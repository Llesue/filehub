use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::db::{Root, unix_now};
use crate::error::AppError;
use crate::federation::db::{FederationGrant, FederationToken};
use crate::fsops;
use crate::pathresolver;
use crate::state::AppState;

pub const REPOSITORY: &str = "https://github.com/Llesue/filehub";
pub const CAPABILITIES: &[&str] = &[
    "list",
    "stat",
    "download",
    "thumb",
    "thumb-warm",
    "dir-size",
    "search",
    "mkdir",
    "touch",
    "rename",
    "copy",
    "move",
    "delete",
    "uploads",
    "unzip",
    "audio-info",
];

struct PeerAuth {
    token: FederationToken,
    grants: Vec<FederationGrant>,
}

impl PeerAuth {
    async fn grant(&self, state: &Arc<AppState>, id: i64) -> Result<&FederationGrant, AppError> {
        if let Some(grant) = self.grants.iter().find(|grant| grant.id == id) {
            return Ok(grant);
        }
        if state
            .db
            .federation_grant_revoked(id, self.token.user_id)
            .await
            .map_err(AppError::internal)?
        {
            return Err(AppError::GrantRevoked);
        }
        Err(AppError::Forbidden)
    }
}

async fn authenticate(state: &Arc<AppState>, headers: &HeaderMap) -> Result<PeerAuth, AppError> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;
    if !raw.starts_with("fhfed_") {
        return Err(AppError::Unauthorized);
    }
    let hash = auth::sha256_hex(raw);
    let token = state
        .db
        .federation_find_token(&hash)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    if token.revoked {
        return Err(AppError::TokenRevoked);
    }
    if token
        .expires_at
        .is_some_and(|expires| expires <= unix_now())
    {
        return Err(AppError::TokenExpired);
    }
    if !state
        .public_rate
        .allow(&format!("federation:{}", token.id), 600, 60)
    {
        return Err(AppError::TooManyRequests);
    }
    let grants = state
        .db
        .federation_token_grants(token.id)
        .await
        .map_err(AppError::internal)?;
    let _ = state.db.federation_touch_token(token.id).await;
    Ok(PeerAuth { token, grants })
}

/// Live presence socket. The mounting side holds this open and treats a
/// disconnect as "peer offline", which is faster than the health loop.
#[derive(Debug, Deserialize)]
pub struct PresenceQuery {
    grant: Option<i64>,
}

async fn presence(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PresenceQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    if let Some(grant) = query.grant
        && !auth
            .grants
            .iter()
            .any(|item| item.id == grant && !item.revoked)
    {
        return Err(AppError::Forbidden);
    }
    let token_id = auth.token.id;
    let user_id = auth.token.user_id;
    let grant_id = query.grant;
    Ok(upgrade
        .on_upgrade(move |socket| presence_session(state, token_id, user_id, grant_id, socket)))
}

async fn presence_session(
    state: Arc<AppState>,
    token_id: i64,
    user_id: i64,
    grant_id: Option<i64>,
    mut socket: WebSocket,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if last_seen.elapsed() >= Duration::from_secs(45) {
                    break;
                }
                if !state.db.federation_token_alive(token_id).await.unwrap_or(false) {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                if let Some(grant) = grant_id
                    && state
                        .db
                        .federation_grant_revoked(grant, user_id)
                        .await
                        .unwrap_or(true)
                {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                if socket
                    .send(Message::Text("{\"type\":\"ping\"}".into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Text(_)))
                    | Some(Ok(Message::Ping(_)))
                    | Some(Ok(Message::Pong(_))) => last_seen = Instant::now(),
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

const IDEMPOTENCY_HEADER: &str = "idempotency-key";

fn idempotency_key(headers: &HeaderMap) -> Result<String, AppError> {
    let value = headers
        .get(IDEMPOTENCY_HEADER)
        .ok_or_else(|| AppError::bad_request("Idempotency-Key is required"))?;
    let raw = value
        .to_str()
        .map_err(|_| AppError::bad_request("invalid Idempotency-Key"))?;
    if raw.is_empty() || raw.len() > 128 {
        return Err(AppError::bad_request(
            "Idempotency-Key must contain 1-128 bytes",
        ));
    }
    Ok(raw.to_owned())
}

fn request_hash<T: serde::Serialize>(value: &T) -> Result<String, AppError> {
    let encoded =
        serde_json::to_string(value).map_err(|error| AppError::internal(anyhow::anyhow!(error)))?;
    Ok(auth::sha256_hex(&encoded))
}

/// Run a write operation at most once per (token, key). The key is reserved
/// before the operation starts, so a concurrent duplicate gets a conflict
/// instead of running twice. Failed operations release the key so the client
/// can retry.
async fn idempotent<F, Fut>(
    state: &Arc<AppState>,
    token_id: i64,
    headers: &HeaderMap,
    method: &str,
    path: Option<&str>,
    body_hash: &str,
    operation: F,
) -> Result<Response, AppError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Response, AppError>>,
{
    let key = idempotency_key(headers)?;
    let started = match state
        .db
        .federation_idempotency_begin(token_id, &key, method, path, body_hash, unix_now())
        .await
        .map_err(AppError::internal)?
    {
        crate::db::IdempotencyStart::New => true,
        crate::db::IdempotencyStart::InProgress => {
            return Err(AppError::Conflict(
                "an identical idempotent request is still running".to_owned(),
            ));
        }
        crate::db::IdempotencyStart::Mismatch => {
            return Err(AppError::Conflict(
                "Idempotency-Key was reused with a different request".to_owned(),
            ));
        }
        crate::db::IdempotencyStart::Replay { status, body } => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
            return Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(body))
                .map_err(|error| AppError::internal(anyhow::anyhow!(error)));
        }
    };
    debug_assert!(started);

    let response = match operation().await {
        Ok(response) => response,
        Err(error) => {
            let _ = state
                .db
                .federation_idempotency_release(token_id, &key)
                .await;
            return Err(error);
        }
    };
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, 256 * 1024)
        .await
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))?;
    let text = String::from_utf8_lossy(&bytes);
    let _ = state
        .db
        .federation_idempotency_finish(token_id, &key, parts.status.as_u16(), &text)
        .await;
    Ok(Response::from_parts(parts, axum::body::Body::from(bytes)))
}

async fn resolve_grant(
    state: &Arc<AppState>,
    grant: &FederationGrant,
    path: &str,
    require_write: bool,
) -> Result<pathresolver::ResolvedPath, AppError> {
    let root = state
        .db
        .find_root(grant.root_id, grant.user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::GrantRevoked)?;
    if root.is_remote() {
        return Err(AppError::GrantRevoked);
    }
    if !std::path::Path::new(&grant.real_path).is_dir() {
        return Err(AppError::GrantRevoked);
    }
    if require_write && (root.read_only || grant.mode != "rw") {
        return Err(AppError::Forbidden);
    }
    let synthetic = Root {
        id: root.id,
        user_id: root.user_id,
        real_path: grant.real_path.clone(),
        alias: root.alias,
        read_only: root.read_only,
        sort_order: 0,
        kind: "local".to_owned(),
        mount_id: None,
    };
    pathresolver::resolve(synthetic, path, require_write).await
}

pub async fn info(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let ip = auth::client_ip(&headers).unwrap_or_else(|| "unknown".to_owned());
    if !state
        .public_rate
        .allow(&format!("federation-info:{ip}"), 30, 60)
    {
        return Err(AppError::TooManyRequests);
    }
    let instance_id = state
        .db
        .federation_instance_id()
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "product": "filehub",
        "api": "federation",
        "version": 1,
        "server_version": env!("CARGO_PKG_VERSION"),
        "instance_id": instance_id,
        "capabilities": CAPABILITIES,
        "chunk_size_max": 64 * 1024 * 1024,
        "path_max": 4096,
        "repository": REPOSITORY,
    })))
}

pub(crate) fn log_read(
    state: &Arc<AppState>,
    user_id: i64,
    token_id: i64,
    grant_id: i64,
    op: &str,
) {
    let mut counts = state.federation_read_counts.lock();
    *counts
        .entry((user_id, token_id, grant_id, op.to_owned()))
        .or_insert(0) += 1;
}

pub(crate) async fn read_flush_loop(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let entries: Vec<((i64, i64, i64, String), i64)> = {
            let mut counts = state.federation_read_counts.lock();
            counts.drain().collect()
        };
        for ((user_id, token_id, grant_id, op), count) in entries {
            let _ = state
                .db
                .federation_log(
                    user_id,
                    token_id,
                    Some(grant_id),
                    &format!("read:{op}"),
                    None,
                    200,
                    count,
                    0,
                    None,
                )
                .await;
        }
    }
}

pub fn router() -> axum::Router<Arc<AppState>> {
    use axum::routing::{get, post, put};
    axum::Router::new()
        .route("/info", get(info))
        .route("/presence", get(presence))
        .route("/grants", get(grants))
        .route("/avatar", get(avatar))
        .route("/list", get(list))
        .route("/stat", get(stat))
        .route("/download", get(download))
        .route("/thumb", get(thumb))
        .route("/dir-size", get(dir_size))
        .route("/search", get(search))
        .route("/uploads", post(create_upload))
        .route("/uploads/{id}", get(upload_status).delete(cancel_upload))
        .route("/uploads/{id}/chunks/{index}", put(upload_chunk))
        .route("/uploads/{id}/complete", post(complete_upload))
        .route("/mkdir", post(mkdir))
        .route("/touch", post(touch))
        .route("/rename", post(rename))
        .route("/copy", post(copy))
        .route("/move", post(move_items))
        .route("/delete", post(delete))
        .route("/unzip", post(unzip))
        .route("/audio-info", get(audio_info))
        .route("/thumb-warm", post(thumb_warm))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response.headers_mut().insert(
                    "x-filehub-federation",
                    header::HeaderValue::from_static("1"),
                );
                response
            },
        ))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(axum::middleware::map_response(
            |response: Response| async move {
                let is_json = response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.starts_with("application/json"));
                if !is_json {
                    return response;
                }
                let (parts, body) = response.into_parts();
                match axum::body::to_bytes(body, 8 * 1024 * 1024).await {
                    Ok(bytes) => Response::from_parts(parts, Body::from(bytes)),
                    Err(_) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": {
                                "code": "TOO_LARGE",
                                "message": "response exceeds the size limit"
                            }
                        })),
                    )
                        .into_response(),
                }
            },
        ))
}

#[derive(Debug, Deserialize)]
pub struct ThumbQuery {
    grant: i64,
    path: Option<String>,
    w: Option<u32>,
    edge: Option<u32>,
    fit: Option<String>,
}

pub async fn thumb(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ThumbQuery>,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, query.grant).await?;
    log_read(&state, auth.token.user_id, auth.token.id, grant.id, "thumb");
    let path = query.path.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &path, false).await?;
    let fit = crate::thumbs::Fit::from_query(query.fit.as_deref());
    let preview_edge = state.settings().settings.thumbnails.preview_edge;
    let edge = query
        .edge
        .unwrap_or_else(|| query.w.unwrap_or(256))
        .clamp(16, preview_edge);
    crate::thumbs::thumbnail_response(
        &state,
        resolved.absolute.clone(),
        edge,
        edge,
        fit,
        grant.user_id,
        &headers,
        Some(crate::thumbs::ThumbScope {
            root_id: Some(grant.root_id),
            share_id: None,
            item_id: None,
            path: resolved.relative.clone(),
        }),
        true,
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct AvatarQuery {
    grant: i64,
}

pub async fn avatar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AvatarQuery>,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let _grant = auth.grant(&state, query.grant).await?;
    let preferences = state
        .db
        .get_user_preferences(auth.token.user_id)
        .await
        .map_err(AppError::internal)?;
    if preferences.avatar_version <= 0 {
        return Err(AppError::NotFound);
    }
    let Some(path) = preferences.avatar_path else {
        return Err(AppError::NotFound);
    };
    let bytes = tokio::fs::read(path)
        .await
        .map_err(crate::fsops::io_error)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "private, max-age=300")
        .body(axum::body::Body::from(bytes))
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))
}

pub async fn grants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let owner = state
        .db
        .find_user_by_id(auth.token.user_id)
        .await
        .map_err(AppError::internal)?;
    let preferences = state
        .db
        .get_user_preferences(auth.token.user_id)
        .await
        .map_err(AppError::internal)?;
    let items: Vec<Value> = auth
        .grants
        .iter()
        .filter(|grant| std::path::Path::new(&grant.real_path).is_dir())
        .map(|grant| {
            json!({
                "id": grant.id,
                "label": grant.label,
                "mode": grant.mode,
                "owner_name": owner.as_ref().map(|user| user.username.clone()).unwrap_or_default(),
                "avatar_version": preferences.avatar_version,
            })
        })
        .collect();
    Ok(Json(json!({ "grants": items })))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    grant: i64,
    path: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
    sort: Option<String>,
    order: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, query.grant).await?;
    log_read(&state, auth.token.user_id, auth.token.id, grant.id, "list");
    let path = query.path.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &path, false).await?;
    let sort = query.sort.unwrap_or_else(|| "name".to_owned());
    let order = query.order.unwrap_or_else(|| "asc".to_owned());
    if !matches!(sort.as_str(), "name" | "modified" | "size") {
        return Err(AppError::bad_request("invalid sort"));
    }
    if !matches!(order.as_str(), "asc" | "desc") {
        return Err(AppError::bad_request("invalid order"));
    }
    let limit = query.limit.unwrap_or(500).clamp(1, 2000);
    let offset = query
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()?
        .unwrap_or(0);
    let entries = tokio::task::spawn_blocking(move || fsops::list_dir(&resolved, &sort, &order))
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))??;
    let preferences = state
        .db
        .get_user_preferences(grant.user_id)
        .await
        .map_err(AppError::internal)?;
    let mut entries = if preferences.show_hidden {
        entries
    } else {
        entries
            .into_iter()
            .filter(|entry| !entry.name.starts_with('.'))
            .collect()
    };
    entries.retain(|entry| !entry.is_symlink);
    let video_enabled = {
        let settings = state.settings();
        settings.settings.thumbnails.enabled
            && settings.settings.thumbnails.video
            && state.ffmpeg.is_some()
    };
    let total = entries.len();
    let page: Vec<Value> = entries
        .drain(..)
        .skip(offset)
        .take(limit)
        .map(|entry| {
            json!({
                "name": entry.name,
                "is_dir": entry.is_dir,
                "is_symlink": entry.is_symlink,
                "size": entry.size,
                "modified": entry.modified,
                "mime_type": entry.mime_type,
                "has_preview": !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled),
            })
        })
        .collect();
    let next_cursor = (offset.saturating_add(limit) < total).then(|| encode_cursor(offset + limit));
    Ok(Json(json!({
        "path": path,
        "read_only": grant.mode != "rw",
        "entries": page,
        "next_cursor": next_cursor,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    grant: i64,
    q: String,
    limit: Option<usize>,
    cursor: Option<String>,
    kind: Option<String>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    modified_after: Option<i64>,
    modified_before: Option<i64>,
    is_dir: Option<bool>,
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    if !state
        .public_rate
        .allow(&format!("federation:search:{}", auth.token.id), 30, 60)
    {
        return Err(AppError::TooManyRequests);
    }
    let grant = auth.grant(&state, query.grant).await?;
    log_read(
        &state,
        auth.token.user_id,
        auth.token.id,
        grant.id,
        "search",
    );
    let text = query.q.trim();
    if text.chars().count() < 2 {
        return Err(AppError::bad_request("search query is too short"));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()?
        .unwrap_or(0);
    let preferences = state
        .db
        .get_user_preferences(grant.user_id)
        .await
        .map_err(AppError::internal)?;
    let (mime_prefix, is_dir) = match query.kind.as_deref() {
        Some("image") => (Some("image/".to_owned()), None),
        Some("video") => (Some("video/".to_owned()), None),
        Some("audio") => (Some("audio/".to_owned()), None),
        Some("text") => (Some("text/".to_owned()), None),
        Some("pdf") => (Some("application/pdf".to_owned()), None),
        Some("dir") => (None, Some(true)),
        Some("file") => (None, Some(false)),
        Some(other) if !other.is_empty() => {
            return Err(AppError::bad_request("invalid search kind"));
        }
        _ => (None, None),
    };
    let is_dir = query.is_dir.or(is_dir);
    let entries = state
        .db
        .search_entries(
            text,
            &grant.real_path,
            preferences.show_hidden,
            offset.saturating_add(limit) as i64,
            crate::db::IndexSearchFilters {
                min_size: query.min_size,
                max_size: query.max_size,
                modified_after: query.modified_after,
                modified_before: query.modified_before,
                is_dir,
                mime_prefix,
            },
        )
        .await
        .map_err(AppError::internal)?;
    let base = grant.real_path.trim_end_matches(['/', '\\']);
    let collected: Vec<Value> = entries
        .into_iter()
        .filter_map(|entry| {
            let relative = entry.real_path.strip_prefix(base)?;
            let path = relative.trim_start_matches(['/', '\\']).replace('\\', "/");
            (!path.is_empty()).then(|| {
                json!({
                    "path": path,
                    "name": entry.name,
                    "is_dir": entry.is_dir,
                    "size": entry.size,
                    "modified": entry.mtime,
                })
            })
        })
        .collect();
    let total = collected.len();
    let results: Vec<Value> = collected.into_iter().skip(offset).take(limit).collect();
    let next_cursor = if offset + results.len() < total {
        Some(encode_cursor(offset + results.len()))
    } else {
        None
    };
    let indexed = state
        .db
        .index_status(&grant.real_path)
        .await
        .map_err(AppError::internal)?
        .is_some_and(|(status, _, _, _, _)| status == "ready");
    Ok(Json(json!({
        "indexed": indexed,
        "results": results,
        "next_cursor": next_cursor,
    })))
}

#[derive(Debug, Deserialize)]
pub struct DirSizeQuery {
    grant: i64,
    path: Option<String>,
}

pub async fn dir_size(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DirSizeQuery>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, query.grant).await?;
    log_read(
        &state,
        auth.token.user_id,
        auth.token.id,
        grant.id,
        "dir-size",
    );
    let path = query.path.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &path, false).await?;
    let absolute = resolved.absolute.clone();
    let (bytes, files) = tokio::task::spawn_blocking(move || fsops::path_usage(&absolute))
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))??;
    Ok(Json(json!({
        "bytes": bytes,
        "files": files,
        "truncated": false,
    })))
}

#[derive(Debug, Deserialize)]
pub struct StatQuery {
    grant: i64,
    path: Option<String>,
}

pub async fn stat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StatQuery>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, query.grant).await?;
    log_read(&state, auth.token.user_id, auth.token.id, grant.id, "stat");
    let path = query.path.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &path, false).await?;
    let is_dir = resolved.absolute.is_dir();
    if is_dir {
        let metadata = tokio::fs::metadata(&resolved.absolute)
            .await
            .map_err(fsops::io_error)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        return Ok(Json(json!({
            "path": path,
            "is_dir": true,
            "size": 0,
            "modified": modified,
            "etag": "",
            "read_only": grant.mode != "rw",
        })));
    }
    let absolute = resolved.absolute.clone();
    let info = tokio::task::spawn_blocking(move || fsops::file_info(&absolute))
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))??;
    Ok(Json(json!({
        "path": path,
        "is_dir": is_dir,
        "size": info.size,
        "modified": info.modified,
        "etag": info.etag,
        "mime_type": mime_guess::from_path(&resolved.absolute)
            .first_or_octet_stream()
            .to_string(),
        "read_only": grant.mode != "rw",
    })))
}

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    grant: i64,
    path: Option<String>,
    inline: Option<String>,
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DownloadQuery>,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, query.grant).await?;
    let path = query.path.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &path, false).await?;
    let inline = query
        .inline
        .as_deref()
        .is_some_and(|value| matches!(value, "1" | "true" | "yes" | "on"));
    let size = tokio::fs::metadata(&resolved.absolute)
        .await
        .map(|metadata| metadata.len() as i64)
        .unwrap_or_default();
    let started = Instant::now();
    let response = crate::files::serve_file(&headers, &resolved.absolute, inline).await;
    let _ = state
        .db
        .federation_log(
            auth.token.user_id,
            auth.token.id,
            Some(grant.id),
            "download",
            Some(path.as_str()),
            if response.is_ok() { 200 } else { 500 },
            size,
            started.elapsed().as_millis() as i64,
            auth::client_ip(&headers).as_deref(),
        )
        .await;
    response
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct MkdirRequest {
    grant: i64,
    path: String,
}

pub async fn mkdir(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<MkdirRequest>,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, request.grant).await?.clone();
    let token_id = auth.token.id;
    let user_id = auth.token.user_id;
    let idempotency_path = request.path.clone();
    idempotent(
        &state,
        token_id,
        &headers,
        "mkdir",
        Some(&idempotency_path),
        &request_hash(&request)?,
        || async {
            let resolved = resolve_grant(&state, &grant, &request.path, true).await?;
            let absolute = resolved.absolute.clone();
            let relative = request.path.clone();
            let started = Instant::now();
            let result = tokio::task::spawn_blocking(move || fsops::create_directory(&absolute))
                .await
                .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
            log_op(
                &state,
                user_id,
                token_id,
                grant.id,
                "mkdir",
                Some(&relative),
                result.is_ok(),
                started,
            )
            .await;
            let created = result?;
            if !created {
                return Err(AppError::Conflict("directory already exists".to_owned()));
            }
            Ok((StatusCode::CREATED, Json(json!({ "path": relative }))).into_response())
        },
    )
    .await
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct TouchRequest {
    grant: i64,
    path: String,
}

pub async fn touch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TouchRequest>,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, request.grant).await?.clone();
    let token_id = auth.token.id;
    let user_id = auth.token.user_id;
    let idempotency_path = request.path.clone();
    idempotent(
        &state,
        token_id,
        &headers,
        "touch",
        Some(&idempotency_path),
        &request_hash(&request)?,
        || async {
            let resolved = resolve_grant(&state, &grant, &request.path, true).await?;
            let absolute = resolved.absolute.clone();
            if absolute.exists() {
                return Err(AppError::Conflict("path already exists".to_owned()));
            }
            let relative = request.path.clone();
            let started = Instant::now();
            let result = tokio::task::spawn_blocking(move || fsops::touch_file(&absolute))
                .await
                .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
            log_op(
                &state,
                user_id,
                token_id,
                grant.id,
                "touch",
                Some(&relative),
                result.is_ok(),
                started,
            )
            .await;
            result?;
            Ok((StatusCode::CREATED, Json(json!({ "path": relative }))).into_response())
        },
    )
    .await
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct RenameRequest {
    grant: i64,
    path: String,
    name: String,
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RenameRequest>,
) -> Result<Response, AppError> {
    validate_name(&request.name)?;
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, request.grant).await?.clone();
    let token_id = auth.token.id;
    let user_id = auth.token.user_id;
    let idempotency_path = request.path.clone();
    idempotent(
        &state,
        token_id,
        &headers,
        "rename",
        Some(&idempotency_path),
        &request_hash(&request)?,
        || async {
            let resolved = resolve_grant(&state, &grant, &request.path, true).await?;
            let target = resolved
                .absolute
                .parent()
                .ok_or(AppError::bad_request("cannot rename root"))?
                .join(&request.name);
            if target.exists() {
                return Err(AppError::Conflict("target already exists".to_owned()));
            }
            let relative = request.path.clone();
            let started = Instant::now();
            let source = resolved.absolute.clone();
            let result = tokio::task::spawn_blocking(move || fsops::rename_path(&source, &target))
                .await
                .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
            log_op(
                &state,
                user_id,
                token_id,
                grant.id,
                "rename",
                Some(&relative),
                result.is_ok(),
                started,
            )
            .await;
            result?;
            let parent = relative.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
            let new_path = if parent.is_empty() {
                request.name.clone()
            } else {
                format!("{parent}/{}", request.name)
            };
            Ok(Json(json!({ "path": new_path })).into_response())
        },
    )
    .await
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct DeleteRequest {
    grant: i64,
    paths: Vec<String>,
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<DeleteRequest>,
) -> Result<Response, AppError> {
    if request.paths.is_empty() || request.paths.len() > 1000 {
        return Err(AppError::bad_request("paths must contain 1-1000 items"));
    }
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, request.grant).await?.clone();
    let token_id = auth.token.id;
    let user_id = auth.token.user_id;
    idempotent(
        &state,
        token_id,
        &headers,
        "delete",
        None,
        &request_hash(&request)?,
        || async {
            let mut done = Vec::new();
            let mut failed = Vec::new();
            for path in request.paths.clone() {
                match resolve_grant(&state, &grant, &path, true).await {
                    Ok(resolved) => {
                        let absolute = resolved.absolute.clone();
                        let relative = path.clone();
                        let started = Instant::now();
                        let result = tokio::task::spawn_blocking(move || {
                            fsops::remove_path(&absolute, &relative)
                        })
                        .await
                        .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
                        log_op(
                            &state,
                            user_id,
                            token_id,
                            grant.id,
                            "delete",
                            Some(&path),
                            result.is_ok(),
                            started,
                        )
                        .await;
                        match result {
                            Ok(()) => done.push(json!({ "path": path })),
                            Err(error) => failed.push(json!({
                                "path": path,
                                "code": error.code(),
                                "message": error.to_string(),
                            })),
                        }
                    }
                    Err(error) => failed.push(json!({
                        "path": path,
                        "code": error.code(),
                        "message": error.to_string(),
                    })),
                }
            }
            if failed.is_empty() {
                Ok(Json(json!({ "done": done, "failed": [] })).into_response())
            } else if done.is_empty() {
                Err(AppError::NotFound)
            } else {
                Ok((
                    StatusCode::MULTI_STATUS,
                    Json(json!({ "done": done, "failed": failed })),
                )
                    .into_response())
            }
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn log_op(
    state: &Arc<AppState>,
    user_id: i64,
    token_id: i64,
    grant_id: i64,
    op: &str,
    path: Option<&str>,
    ok: bool,
    started: Instant,
) {
    let _ = state
        .db
        .federation_log(
            user_id,
            token_id,
            Some(grant_id),
            op,
            path,
            if ok { 200 } else { 500 },
            0,
            started.elapsed().as_millis() as i64,
            None,
        )
        .await;
}

#[derive(Debug, Deserialize)]
pub struct ThumbWarmRequest {
    grant: i64,
    paths: Vec<String>,
    width: Option<u32>,
}

pub async fn thumb_warm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ThumbWarmRequest>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    if !state
        .public_rate
        .allow(&format!("federation:warm:{}", auth.token.id), 2, 60)
    {
        return Err(AppError::TooManyRequests);
    }
    let grant = auth.grant(&state, request.grant).await?;
    if request.paths.is_empty() || request.paths.len() > 500 {
        return Err(AppError::bad_request("paths must contain 1-500 items"));
    }
    let settings = state.settings();
    if !settings.settings.thumbnails.enabled {
        return Ok(Json(json!({
            "queued": 0,
            "cached": 0,
            "skipped": 0,
            "disabled": true,
        })));
    }
    let width = request.width.unwrap_or(256).clamp(16, 1024);
    let edge = width;
    let fit = crate::thumbs::Fit::from_query(None);
    let cache_dir = settings.settings.cache_dir.join("thumbs");
    let video_enabled = settings.settings.thumbnails.video && state.ffmpeg.is_some();
    let started = Instant::now();
    let mut queued = 0_u64;
    let mut cached = 0_u64;
    let mut skipped = 0_u64;
    for path in &request.paths {
        let resolved = match resolve_grant(&state, grant, path, false).await {
            Ok(resolved) => resolved,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !resolved.absolute.is_file() {
            skipped += 1;
            continue;
        }
        let name = resolved
            .absolute
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !crate::thumbs::can_preview(name, video_enabled) {
            skipped += 1;
            continue;
        }
        let Ok((key, cache_path, _)) =
            crate::thumbs::thumb_paths(&cache_dir, &resolved.absolute, width, fit, edge)
        else {
            skipped += 1;
            continue;
        };
        if tokio::fs::metadata(&cache_path).await.is_ok() {
            cached += 1;
            continue;
        }
        if state
            .jobs
            .active_job_exact(crate::jobs::JobKind::Thumbnail, &key)
            .is_some()
        {
            queued += 1;
            continue;
        }
        let scope = crate::thumbs::ThumbScope {
            root_id: Some(grant.root_id),
            share_id: None,
            item_id: None,
            path: resolved.relative.clone(),
        };
        match crate::thumbs::thumbnail_response(
            &state,
            resolved.absolute.clone(),
            width,
            edge,
            fit,
            grant.user_id,
            &headers,
            Some(scope),
            false,
        )
        .await
        {
            Ok(_) => queued += 1,
            Err(_) => skipped += 1,
        }
    }
    let _ = state
        .db
        .federation_log(
            auth.token.user_id,
            auth.token.id,
            Some(grant.id),
            "thumb-warm",
            None,
            200,
            queued as i64,
            started.elapsed().as_millis() as i64,
            None,
        )
        .await;
    Ok(Json(json!({
        "queued": queued,
        "cached": cached,
        "skipped": skipped,
        "disabled": false,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AudioInfoQuery {
    grant: i64,
    path: Option<String>,
}

pub async fn audio_info(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AudioInfoQuery>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    if !state
        .public_rate
        .allow(&format!("federation:audio:{}", auth.token.id), 30, 60)
    {
        return Err(AppError::TooManyRequests);
    }
    let grant = auth.grant(&state, query.grant).await?;
    let path = query.path.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &path, false).await?;
    if !resolved.absolute.is_file() {
        return Err(AppError::NotFound);
    }
    let Some(ffmpeg) = state.ffmpeg.clone() else {
        return Ok(Json(json!({
            "ffmpeg": false,
            "tracks": [],
            "default_track": null,
            "has_supported_track": true,
        })));
    };
    let tracks = crate::media::probe_tracks(&ffmpeg, &resolved.absolute).await?;
    let default_track = tracks
        .iter()
        .position(|track| track.get("default").and_then(Value::as_bool) == Some(true))
        .or(if tracks.is_empty() { None } else { Some(0) });
    let has_supported_track = tracks
        .iter()
        .any(|track| track.get("supported").and_then(Value::as_bool) == Some(true));
    Ok(Json(json!({
        "ffmpeg": true,
        "tracks": tracks,
        "default_track": default_track,
        "has_supported_track": has_supported_track,
    })))
}

#[derive(Debug, Deserialize)]
pub struct UnzipRequest {
    grant: i64,
    path: String,
    target: Option<String>,
}

pub async fn unzip(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UnzipRequest>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    if !state
        .public_rate
        .allow(&format!("federation:unzip:{}", auth.token.id), 10, 60)
    {
        return Err(AppError::TooManyRequests);
    }
    let grant = auth.grant(&state, request.grant).await?;
    let archive = resolve_grant(&state, grant, &request.path, false).await?;
    if !archive.absolute.is_file() {
        return Err(AppError::NotFound);
    }
    let (target, existing) = if let Some(target) = request.target.clone() {
        let resolved = resolve_grant(&state, grant, &target, true).await?;
        let exists = std::fs::symlink_metadata(&resolved.absolute).is_ok();
        if exists && !resolved.absolute.is_dir() {
            return Err(AppError::bad_request("target is a file, not a directory"));
        }
        (resolved.relative, exists)
    } else {
        let file_name = archive
            .absolute
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AppError::invalid_path("invalid archive name"))?;
        let stem = file_name
            .strip_suffix(".zip")
            .or_else(|| file_name.strip_suffix(".ZIP"))
            .unwrap_or(file_name);
        let parent = archive
            .relative
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_owned())
            .unwrap_or_default();
        let relative = if parent.is_empty() {
            stem.to_owned()
        } else {
            format!("{parent}/{stem}")
        };
        let resolved = resolve_grant(&state, grant, &relative, true).await?;
        let exists = std::fs::symlink_metadata(&resolved.absolute).is_ok();
        (resolved.relative, exists)
    };
    if existing {
        let archive_path = archive.absolute.clone();
        let target_path = resolve_grant(&state, grant, &target, true).await?.absolute;
        tokio::task::spawn_blocking(move || {
            crate::unzip::ensure_no_conflicts(&archive_path, &target_path)
        })
        .await
        .map_err(|error| {
            AppError::internal(anyhow::anyhow!("unzip preflight task failed: {error}"))
        })??;
    }
    let archive_path = archive.absolute.clone();
    let target_path = resolve_grant(&state, grant, &target, true).await?.absolute;
    let limits = crate::unzip::UnzipLimits::from_settings(&state.settings().settings.downloads);
    let result = tokio::task::spawn_blocking(move || {
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut progress = |_: usize, _: usize, _: u64| {};
        crate::unzip::extract(&archive_path, &target_path, limits, &cancel, &mut progress)
    })
    .await
    .map_err(|error| AppError::internal(anyhow::anyhow!("unzip task failed: {error}")))??;
    Ok(Json(json!({
        "target": target,
        "entries": result.entries,
        "bytes": result.bytes,
    })))
}

fn validate_name(name: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(char::is_control)
    {
        return Err(AppError::bad_request("invalid name"));
    }
    Ok(())
}

fn encode_cursor(offset: usize) -> String {
    let raw = serde_json::to_vec(&json!({ "o": offset })).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn decode_cursor(cursor: &str) -> Result<usize, AppError> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| AppError::bad_request("invalid cursor"))?;
    let value: Value =
        serde_json::from_slice(&raw).map_err(|_| AppError::bad_request("invalid cursor"))?;
    value
        .get("o")
        .and_then(Value::as_u64)
        .map(|offset| offset as usize)
        .ok_or_else(|| AppError::bad_request("invalid cursor"))
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateUploadRequest {
    grant: i64,
    dir: Option<String>,
    filename: String,
    total_size: i64,
    chunk_size: Option<i64>,
    /// Source file modified time as Unix seconds. Omitted keeps server time.
    mtime: Option<i64>,
    /// Expected ETag of the target file. The upload is rejected with 412 when
    /// the file no longer matches, closing the stat-then-write race.
    if_match: Option<String>,
    /// `*` refuses to overwrite an existing file.
    if_none_match: Option<String>,
}

async fn peer_upload(
    state: &Arc<AppState>,
    auth: &PeerAuth,
    id: &str,
) -> Result<crate::db::UploadSession, AppError> {
    let upload = state
        .db
        .find_upload(id, auth.token.user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if upload.token_id != Some(auth.token.id) {
        return Err(AppError::NotFound);
    }
    let root = state
        .db
        .find_root(upload.root_id, auth.token.user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let relative = if upload.dir_path.is_empty() {
        upload.filename.clone()
    } else {
        format!("{}/{}", upload.dir_path, upload.filename)
    };
    let absolute = std::path::Path::new(&root.real_path).join(&relative);
    let allowed = auth.grants.iter().any(|grant| {
        grant.root_id == upload.root_id
            && grant.mode == "rw"
            && crate::platform::path_within(&absolute, std::path::Path::new(&grant.real_path))
    });
    if !allowed {
        return Err(AppError::NotFound);
    }
    Ok(upload)
}

pub async fn create_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateUploadRequest>,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(&state, request.grant).await?.clone();
    validate_name(&request.filename)?;
    let token_id = auth.token.id;
    let idempotency_path = request.filename.clone();
    idempotent(
        &state,
        token_id,
        &headers,
        "uploads",
        Some(&idempotency_path),
        &request_hash(&request)?,
        || async {
            if request.total_size < 0 {
                return Err(AppError::bad_request("invalid upload size"));
            }
            let mtime = crate::files::validate_upload_mtime(request.mtime)?;
            let chunk_size = request.chunk_size.unwrap_or(4 * 1024 * 1024);
            if !(1..=64 * 1024 * 1024).contains(&chunk_size) {
                return Err(AppError::bad_request("invalid chunk size"));
            }
            let chunks = (request.total_size + chunk_size - 1) / chunk_size;
            if chunks > 100_000 {
                return Err(AppError::bad_request("too many upload chunks"));
            }
            let dir = request.dir.clone().unwrap_or_default();
            let resolved = resolve_grant(&state, &grant, &dir, true).await?;
            if !resolved.absolute.is_dir() {
                return Err(AppError::bad_request("upload target is not a directory"));
            }
            let root = state
                .db
                .find_root(grant.root_id, grant.user_id)
                .await
                .map_err(AppError::internal)?
                .ok_or(AppError::GrantRevoked)?;
            let dir_relative = resolved
                .absolute
                .strip_prefix(std::path::Path::new(&root.real_path))
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let upload_id = format!("up_{}", auth::random_token(16));
            let temp = fsops::create_upload_temp(&resolved.absolute, &upload_id)?;
            let now = unix_now();
            let ttl = state.settings().settings.uploads.session_ttl_hours as i64;
            let upload = crate::db::UploadSession {
                id: upload_id.clone(),
                user_id: grant.user_id,
                root_id: grant.root_id,
                dir_path: dir_relative,
                filename: request.filename.clone(),
                total_size: request.total_size,
                chunk_size,
                received: 0,
                chunk_bitmap: vec![0; ((chunks + 7) / 8) as usize],
                temp_path: temp.to_string_lossy().to_string(),
                expires_at: now + ttl.saturating_mul(3600),
                created_at: now,
                updated_at: now,
                remote_upload_id: None,
                if_match: request.if_match.clone(),
                if_none_match: request.if_none_match.clone(),
                token_id: Some(token_id),
                mtime,
            };
            state
                .db
                .insert_upload(&upload)
                .await
                .map_err(AppError::internal)?;
            Ok(Json(json!({
                "id": upload_id,
                "chunk_size": chunk_size,
                "total_size": request.total_size,
            }))
            .into_response())
        },
    )
    .await
}

pub async fn upload_chunk(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((id, index)): AxumPath<(String, u32)>,
    body: Bytes,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let _ = idempotency_key(&headers)?;
    let mut upload = peer_upload(&state, &auth, &id).await?;
    let index = index as i64;
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    if index < 0 || index >= chunks {
        return Err(AppError::bad_request("chunk index out of range"));
    }
    let expected = if index == chunks - 1 {
        upload.total_size - index * upload.chunk_size
    } else {
        upload.chunk_size
    };
    if body.len() as i64 != expected {
        return Err(AppError::bad_request("unexpected chunk size"));
    }
    let byte_index = (index / 8) as usize;
    let bit = 1_u8 << (index % 8);
    if upload
        .chunk_bitmap
        .get(byte_index)
        .is_some_and(|value| value & bit != 0)
    {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let temp = std::path::PathBuf::from(&upload.temp_path);
    let data = body.to_vec();
    let offset = (index * upload.chunk_size) as u64;
    tokio::task::spawn_blocking(move || fsops::write_upload_chunk(&temp, offset, &data))
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))??;
    if let Some(value) = upload.chunk_bitmap.get_mut(byte_index) {
        *value |= bit;
    }
    let received = (0..chunks)
        .filter(|chunk| {
            upload
                .chunk_bitmap
                .get((chunk / 8) as usize)
                .is_some_and(|value| value & (1_u8 << (chunk % 8)) != 0)
        })
        .map(|chunk| {
            if chunk == chunks - 1 {
                upload.total_size - chunk * upload.chunk_size
            } else {
                upload.chunk_size
            }
        })
        .sum::<i64>();
    state
        .db
        .update_upload_progress(
            &upload.id,
            upload.user_id,
            received,
            upload.chunk_bitmap.clone(),
        )
        .await
        .map_err(AppError::internal)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn upload_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let _ = idempotency_key(&headers)?;
    let upload = peer_upload(&state, &auth, &id).await?;
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    let received_chunks: Vec<u32> = (0..chunks)
        .filter(|chunk| {
            upload
                .chunk_bitmap
                .get((chunk / 8) as usize)
                .is_some_and(|value| value & (1_u8 << (chunk % 8)) != 0)
        })
        .map(|chunk| chunk as u32)
        .collect();
    Ok(Json(json!({
        "received": upload.received,
        "total_chunks": chunks,
        "received_bytes": upload.received,
        "received_chunks": received_chunks,
        "expires_at": upload.expires_at,
    })))
}

pub async fn complete_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let upload = peer_upload(&state, &auth, &id).await?;
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    let received = (0..chunks)
        .filter(|chunk| {
            upload
                .chunk_bitmap
                .get((chunk / 8) as usize)
                .is_some_and(|value| value & (1_u8 << (chunk % 8)) != 0)
        })
        .count() as i64;
    if received != chunks {
        return Err(AppError::bad_request("upload is incomplete"));
    }
    let root = state
        .db
        .find_root(upload.root_id, upload.user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let relative = if upload.dir_path.is_empty() {
        upload.filename.clone()
    } else {
        format!("{}/{}", upload.dir_path, upload.filename)
    };
    let target = std::path::Path::new(&root.real_path).join(&relative);
    let exists = target.exists();
    if let Some(expected) = upload.if_match.as_deref() {
        let current = if exists {
            tokio::task::spawn_blocking({
                let target = target.clone();
                move || fsops::file_info(&target)
            })
            .await
            .map_err(|error| AppError::internal(anyhow::Error::from(error)))?
            .map(|info| info.etag)
            .ok()
        } else {
            None
        };
        if current.as_deref() != Some(expected) {
            return Err(AppError::PreconditionFailed);
        }
    }
    if upload.if_none_match.as_deref() == Some("*") && exists {
        return Err(AppError::PreconditionFailed);
    }
    if exists {
        let _ = fsops::remove_path(&target, &relative);
    }
    let temp = std::path::PathBuf::from(&upload.temp_path);
    tokio::task::spawn_blocking(move || fsops::finalize_upload(&temp, &target, upload.mtime))
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))??;
    let info = tokio::task::spawn_blocking({
        let target = std::path::Path::new(&root.real_path).join(&relative);
        move || fsops::file_info(&target)
    })
    .await
    .map_err(|error| AppError::internal(anyhow::Error::from(error)))??;
    state
        .db
        .delete_upload(&upload.id, upload.user_id)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "path": relative,
        "etag": info.etag,
        "size": info.size,
    })))
}

pub async fn cancel_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let upload = peer_upload(&state, &auth, &id).await?;
    fsops::delete_temp(std::path::Path::new(&upload.temp_path));
    state
        .db
        .delete_upload(&upload.id, upload.user_id)
        .await
        .map_err(AppError::internal)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct TransferRequest {
    grant: i64,
    paths: Vec<String>,
    dest: String,
    overwrite: Option<bool>,
}

pub async fn copy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferRequest>,
) -> Result<Response, AppError> {
    let token_id = authenticate(&state, &headers).await?.token.id;
    let idempotency_path = request.dest.clone();
    let transfer_state = Arc::clone(&state);
    let transfer_headers = headers.clone();
    idempotent(
        &state,
        token_id,
        &headers,
        "copy",
        Some(&idempotency_path),
        &request_hash(&request)?,
        move || {
            let state = Arc::clone(&transfer_state);
            let headers = transfer_headers.clone();
            async move { transfer(&state, &headers, request, false).await }
        },
    )
    .await
}

pub async fn move_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferRequest>,
) -> Result<Response, AppError> {
    let token_id = authenticate(&state, &headers).await?.token.id;
    let idempotency_path = request.dest.clone();
    let transfer_state = Arc::clone(&state);
    let transfer_headers = headers.clone();
    idempotent(
        &state,
        token_id,
        &headers,
        "move",
        Some(&idempotency_path),
        &request_hash(&request)?,
        move || {
            let state = Arc::clone(&transfer_state);
            let headers = transfer_headers.clone();
            async move { transfer(&state, &headers, request, true).await }
        },
    )
    .await
}

async fn transfer(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    request: TransferRequest,
    move_items: bool,
) -> Result<Response, AppError> {
    if request.paths.is_empty() || request.paths.len() > 1000 {
        return Err(AppError::bad_request("paths must contain 1-1000 items"));
    }
    let auth = authenticate(state, headers).await?;
    let grant = auth.grant(state, request.grant).await?;
    let dest = resolve_grant(state, grant, &request.dest, true).await?;
    if !dest.absolute.is_dir() {
        return Err(AppError::bad_request("destination is not a directory"));
    }
    let overwrite = request.overwrite.unwrap_or(false);
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for path in request.paths {
        let source = match resolve_grant(state, grant, &path, move_items).await {
            Ok(source) => source,
            Err(error) => {
                failed.push(json!({
                    "path": path,
                    "code": error.code(),
                    "message": error.to_string(),
                }));
                continue;
            }
        };
        let Some(name) = source.absolute.file_name().map(|name| name.to_owned()) else {
            failed.push(json!({
                "path": path,
                "code": "BAD_REQUEST",
                "message": "source has no file name",
            }));
            continue;
        };
        let target = dest.absolute.join(&name);
        if target == source.absolute {
            done.push(json!({ "path": path, "skipped": true }));
            continue;
        }
        if target.exists() {
            if !overwrite {
                failed.push(json!({
                    "path": path,
                    "code": "CONFLICT",
                    "message": "target already exists",
                }));
                continue;
            }
            let relative = target.to_string_lossy().to_string();
            if let Err(error) = fsops::remove_path(&target, &relative) {
                failed.push(json!({
                    "path": path,
                    "code": error.code(),
                    "message": error.to_string(),
                }));
                continue;
            }
        }
        let source_path = source.absolute.clone();
        let target_path = target.clone();
        let result = tokio::task::spawn_blocking(move || {
            if move_items {
                fsops::move_path(&source_path, &target_path)
            } else {
                fsops::copy_path(&source_path, &target_path)
            }
        })
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
        match result {
            Ok(()) => done.push(json!({ "path": path })),
            Err(error) => failed.push(json!({
                "path": path,
                "code": error.code(),
                "message": error.to_string(),
            })),
        }
    }
    if failed.is_empty() {
        Ok(Json(json!({ "done": done, "failed": [] })).into_response())
    } else if done.is_empty() {
        Err(AppError::Conflict("transfer failed".to_owned()))
    } else {
        Ok((
            StatusCode::MULTI_STATUS,
            Json(json!({ "done": done, "failed": failed })),
        )
            .into_response())
    }
}
