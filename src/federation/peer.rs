use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::body::Bytes;
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
    "list", "stat", "download", "thumb", "dir-size", "mkdir", "touch", "rename", "delete",
    "uploads",
];

struct PeerAuth {
    token: FederationToken,
    grants: Vec<FederationGrant>,
}

impl PeerAuth {
    fn grant(&self, id: i64) -> Result<&FederationGrant, AppError> {
        self.grants
            .iter()
            .find(|grant| grant.id == id)
            .ok_or(AppError::Forbidden)
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

pub fn router() -> axum::Router<Arc<AppState>> {
    use axum::routing::{get, post, put};
    axum::Router::new()
        .route("/info", get(info))
        .route("/grants", get(grants))
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
    let grant = auth.grant(query.grant)?;
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

pub async fn grants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let items: Vec<Value> = auth
        .grants
        .iter()
        .map(|grant| {
            json!({
                "id": grant.id,
                "label": grant.label,
                "mode": grant.mode,
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
    let grant = auth.grant(query.grant)?;
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
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(query.grant)?;
    let text = query.q.trim();
    if text.chars().count() < 2 {
        return Err(AppError::bad_request("search query is too short"));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let preferences = state
        .db
        .get_user_preferences(grant.user_id)
        .await
        .map_err(AppError::internal)?;
    let entries = state
        .db
        .search_entries(
            text,
            &grant.real_path,
            preferences.show_hidden,
            limit as i64,
            crate::db::IndexSearchFilters {
                min_size: None,
                max_size: None,
                modified_after: None,
                modified_before: None,
                is_dir: None,
                mime_prefix: None,
            },
        )
        .await
        .map_err(AppError::internal)?;
    let base = grant.real_path.trim_end_matches(['/', '\\']);
    let results: Vec<Value> = entries
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
    Ok(Json(json!({
        "indexed": true,
        "results": results,
        "next_cursor": null,
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
    let grant = auth.grant(query.grant)?;
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
    let grant = auth.grant(query.grant)?;
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
    let grant = auth.grant(query.grant)?;
    let path = query.path.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &path, false).await?;
    let inline = query
        .inline
        .as_deref()
        .is_some_and(|value| matches!(value, "1" | "true" | "yes" | "on"));
    crate::files::serve_file(&headers, &resolved.absolute, inline).await
}

#[derive(Debug, Deserialize)]
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
    let grant = auth.grant(request.grant)?;
    let resolved = resolve_grant(&state, grant, &request.path, true).await?;
    let absolute = resolved.absolute.clone();
    if absolute.exists() {
        return Err(AppError::Conflict("path already exists".to_owned()));
    }
    let relative = request.path.clone();
    let started = Instant::now();
    let result = tokio::task::spawn_blocking(move || fsops::create_directory(&absolute))
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
    log_op(
        &state,
        &auth,
        grant,
        "mkdir",
        Some(&relative),
        result.is_ok(),
        started,
    )
    .await;
    result?;
    Ok((StatusCode::CREATED, Json(json!({ "path": relative }))).into_response())
}

#[derive(Debug, Deserialize)]
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
    let grant = auth.grant(request.grant)?;
    let resolved = resolve_grant(&state, grant, &request.path, true).await?;
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
        &auth,
        grant,
        "touch",
        Some(&relative),
        result.is_ok(),
        started,
    )
    .await;
    result?;
    Ok((StatusCode::CREATED, Json(json!({ "path": relative }))).into_response())
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    grant: i64,
    path: String,
    name: String,
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RenameRequest>,
) -> Result<Json<Value>, AppError> {
    validate_name(&request.name)?;
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(request.grant)?;
    let resolved = resolve_grant(&state, grant, &request.path, true).await?;
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
        &auth,
        grant,
        "rename",
        Some(&relative),
        result.is_ok(),
        started,
    )
    .await;
    result?;
    let parent = relative.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let new_path = if parent.is_empty() {
        request.name
    } else {
        format!("{parent}/{}", request.name)
    };
    Ok(Json(json!({ "path": new_path })))
}

#[derive(Debug, Deserialize)]
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
    let grant = auth.grant(request.grant)?;
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for path in request.paths {
        match resolve_grant(&state, grant, &path, true).await {
            Ok(resolved) => {
                let absolute = resolved.absolute.clone();
                let relative = path.clone();
                let started = Instant::now();
                let result =
                    tokio::task::spawn_blocking(move || fsops::remove_path(&absolute, &relative))
                        .await
                        .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
                log_op(
                    &state,
                    &auth,
                    grant,
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
}

async fn log_op(
    state: &Arc<AppState>,
    auth: &PeerAuth,
    grant: &FederationGrant,
    op: &str,
    path: Option<&str>,
    ok: bool,
    started: Instant,
) {
    let _ = state
        .db
        .federation_log(
            auth.token.user_id,
            auth.token.id,
            Some(grant.id),
            op,
            path,
            if ok { 200 } else { 500 },
            0,
            started.elapsed().as_millis() as i64,
            None,
        )
        .await;
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

#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    grant: i64,
    dir: Option<String>,
    filename: String,
    total_size: i64,
    chunk_size: Option<i64>,
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
) -> Result<Json<Value>, AppError> {
    let auth = authenticate(&state, &headers).await?;
    let grant = auth.grant(request.grant)?;
    validate_name(&request.filename)?;
    if request.total_size < 0 {
        return Err(AppError::bad_request("invalid upload size"));
    }
    let chunk_size = request.chunk_size.unwrap_or(4 * 1024 * 1024);
    if !(1..=64 * 1024 * 1024).contains(&chunk_size) {
        return Err(AppError::bad_request("invalid chunk size"));
    }
    let chunks = (request.total_size + chunk_size - 1) / chunk_size;
    if chunks > 100_000 {
        return Err(AppError::bad_request("too many upload chunks"));
    }
    let dir = request.dir.unwrap_or_default();
    let resolved = resolve_grant(&state, grant, &dir, true).await?;
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
    })))
}

pub async fn upload_chunk(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((id, index)): AxumPath<(String, u32)>,
    body: Bytes,
) -> Result<Response, AppError> {
    let auth = authenticate(&state, &headers).await?;
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
    let upload = peer_upload(&state, &auth, &id).await?;
    Ok(Json(json!({
        "received": upload.received,
        "total_chunks": (upload.total_size + upload.chunk_size - 1) / upload.chunk_size,
        "received_bytes": upload.received,
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
    if target.exists() {
        let _ = fsops::remove_path(&target, &relative);
    }
    let temp = std::path::PathBuf::from(&upload.temp_path);
    tokio::task::spawn_blocking(move || fsops::finalize_upload(&temp, &target))
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

#[derive(Debug, Deserialize)]
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
    transfer(&state, &headers, request, false).await
}

pub async fn move_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferRequest>,
) -> Result<Response, AppError> {
    transfer(&state, &headers, request, true).await
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
    let grant = auth.grant(request.grant)?;
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
