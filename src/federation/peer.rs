use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::{Query, State};
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
    "list", "stat", "download", "mkdir", "touch", "rename", "delete",
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

pub async fn info(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
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
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/info", get(info))
        .route("/grants", get(grants))
        .route("/list", get(list))
        .route("/stat", get(stat))
        .route("/download", get(download))
        .route("/mkdir", post(mkdir))
        .route("/touch", post(touch))
        .route("/rename", post(rename))
        .route("/delete", post(delete))
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
