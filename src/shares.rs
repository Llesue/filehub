use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::io::ReaderStream;

use crate::auth;
use crate::authz;
use crate::db::{Root, Share, unix_now};
use crate::error::AppError;
use crate::fsops;
use crate::pathresolver;
use crate::state::{AppState, ShareTicket};

#[derive(Debug, Deserialize)]
pub struct CreateShareRequest {
    root: i64,
    path: String,
    password: Option<String>,
    expires_at: Option<i64>,
    read_only: Option<bool>,
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateShareRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let resolved = authz::resolve(&state, current.user.id, root.id, &request.path, false).await?;
    if !resolved.absolute.exists() {
        return Err(AppError::NotFound);
    }
    if request
        .expires_at
        .is_some_and(|expires| expires <= unix_now())
    {
        return Err(AppError::bad_request("expires_at must be in the future"));
    }

    let password_hash = match request
        .password
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(password) => Some(
            auth::hash_password_async(password.to_owned())
                .await
                .map_err(AppError::internal)?,
        ),
        None => None,
    };
    let token = format!("sh_{}", auth::random_token(12));
    let token_hash = auth::sha256_hex(&token);
    let real_path = resolved.absolute.to_string_lossy().to_string();
    let id = state
        .db
        .insert_share(
            &token_hash,
            current.user.id,
            &real_path,
            resolved.absolute.is_dir(),
            password_hash.as_deref(),
            request.expires_at,
            request.read_only.unwrap_or(true),
        )
        .await
        .map_err(AppError::internal)?;

    audit(
        &state,
        current.user.id,
        "share_created",
        &real_path,
        &headers,
    )
    .await;
    Ok(Json(json!({
        "id": id,
        "token": token,
        "url": format!("/s/{token}"),
        "expires_at": request.expires_at,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
    cursor: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let offset = query
        .cursor
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let mut shares = state
        .db
        .list_shares(current.user.id, offset, limit + 1)
        .await
        .map_err(AppError::internal)?;
    let has_more = shares.len() as i64 > limit;
    shares.truncate(limit as usize);
    Ok(Json(json!({
        "next_cursor": has_more.then(|| (offset + limit).to_string()),
        "shares": shares.iter().map(share_json).collect::<Vec<_>>()
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateShareRequest {
    read_only: Option<bool>,
    expires_at: Option<i64>,
    clear_expires: Option<bool>,
    password: Option<String>,
    clear_password: Option<bool>,
    revoked: Option<bool>,
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(request): Json<UpdateShareRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let mut changed = false;
    if let Some(read_only) = request.read_only {
        changed |= state
            .db
            .update_share_read_only(id, current.user.id, read_only)
            .await
            .map_err(AppError::internal)?;
    }
    if request.clear_expires.unwrap_or(false) {
        changed |= state
            .db
            .update_share_expiry(id, current.user.id, None)
            .await
            .map_err(AppError::internal)?;
    } else if let Some(expires_at) = request.expires_at {
        changed |= state
            .db
            .update_share_expiry(id, current.user.id, Some(expires_at))
            .await
            .map_err(AppError::internal)?;
    }
    if request.clear_password.unwrap_or(false) {
        changed |= state
            .db
            .update_share_password(id, current.user.id, None)
            .await
            .map_err(AppError::internal)?;
    } else if let Some(password) = request.password {
        let hash = if password.is_empty() {
            None
        } else {
            Some(
                auth::hash_password_async(password)
                    .await
                    .map_err(AppError::internal)?,
            )
        };
        changed |= state
            .db
            .update_share_password(id, current.user.id, hash.as_deref())
            .await
            .map_err(AppError::internal)?;
    }
    if let Some(revoked) = request.revoked {
        changed |= state
            .db
            .update_share_revoked(id, current.user.id, revoked)
            .await
            .map_err(AppError::internal)?;
    }
    if !changed {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn rotate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let token = format!("sh_{}", auth::random_token(12));
    let token_hash = auth::sha256_hex(&token);
    let changed = state
        .db
        .rotate_share_token(id, current.user.id, &token_hash)
        .await
        .map_err(AppError::internal)?;
    if !changed {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({
        "token": token,
        "url": format!("/s/{token}"),
    })))
}

#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    path: Option<String>,
    filename: String,
}

pub async fn upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers)?;
    if !share.is_dir || share.read_only {
        return Err(AppError::Forbidden);
    }
    let path = query.path.unwrap_or_default();
    let resolved = resolve_share_path(&share, &path).await?;
    if !resolved.absolute.is_dir() {
        return Err(AppError::bad_request("upload target is not a directory"));
    }
    let filename = query.filename.trim();
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return Err(AppError::invalid_path("invalid filename"));
    }
    #[cfg(windows)]
    if filename.contains(':')
        || filename
            .chars()
            .any(|value| matches!(value, '<' | '>' | '"' | '|' | '?' | '*'))
        || filename.ends_with(' ')
        || filename.ends_with('.')
    {
        return Err(AppError::invalid_path("invalid Windows filename"));
    }
    let target = resolved.absolute.join(filename);
    let target = fsops::unique_target(&target);
    tokio::fs::write(&target, body)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    Ok(Json(
        json!({ "name": target.file_name().and_then(|value| value.to_str()).unwrap_or(filename) }),
    ))
}

pub async fn purge(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let deleted = state
        .db
        .purge_share(id, current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    audit(
        &state,
        current.user.id,
        "share_purged",
        &id.to_string(),
        &headers,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let revoked = state
        .db
        .revoke_share(id, current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !revoked {
        return Err(AppError::NotFound);
    }
    audit(
        &state,
        current.user.id,
        "share_revoked",
        &id.to_string(),
        &headers,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ShareQuery {
    path: Option<String>,
}

pub async fn public_meta(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<ShareQuery>,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers)?;
    state
        .db
        .increment_share_visit(share.id)
        .await
        .map_err(AppError::internal)?;
    let path = query.path.unwrap_or_default();

    if !share.is_dir {
        if !path.is_empty() {
            return Err(AppError::NotFound);
        }
        let file_path = PathBuf::from(&share.real_path);
        let info = tokio::task::spawn_blocking(move || fsops::file_info(&file_path))
            .await
            .map_err(|err| {
                AppError::internal(anyhow::anyhow!("share metadata task failed: {err}"))
            })??;
        return Ok(Json(json!({
            "name": Path::new(&share.real_path).file_name().and_then(|name| name.to_str()).unwrap_or("file"),
            "path": "",
            "is_dir": false,
            "read_only": share.read_only,
            "size": info.size,
            "modified": info.modified,
        })));
    }

    let resolved = resolve_share_path(&share, &path).await?;

    if resolved.absolute.is_file() {
        let info = tokio::task::spawn_blocking({
            let path = resolved.absolute.clone();
            move || fsops::file_info(&path)
        })
        .await
        .map_err(|err| {
            AppError::internal(anyhow::anyhow!("share metadata task failed: {err}"))
        })??;
        return Ok(Json(json!({
            "name": resolved.absolute.file_name().and_then(|name| name.to_str()).unwrap_or("file"),
            "path": resolved.relative,
            "is_dir": false,
            "read_only": share.read_only,
            "size": info.size,
            "modified": info.modified,
        })));
    }

    let entries = tokio::task::spawn_blocking({
        let resolved = resolved.clone();
        move || fsops::list_dir(&resolved, "name", "asc")
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("share list task failed: {err}")))??;

    Ok(Json(json!({
        "name": resolved.absolute.file_name().and_then(|name| name.to_str()).unwrap_or("分享"),
        "path": resolved.relative,
        "is_dir": true,
        "entries": entries,
    })))
}

#[derive(Debug, Deserialize)]
pub struct UnlockRequest {
    password: String,
}

pub async fn unlock(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Json(request): Json<UnlockRequest>,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    let Some(password_hash) = share.password_hash.as_deref() else {
        return Err(AppError::bad_request("share does not require a password"));
    };
    let key = format!("share:{}", share.id);
    if state.rate_limiter.is_blocked(&key) {
        return Err(AppError::TooManyRequests);
    }

    let valid = auth::verify_password_async(request.password, password_hash.to_owned())
        .await
        .map_err(AppError::internal)?;
    if !valid {
        state.rate_limiter.record_failure(&key);
        return Err(AppError::Unauthorized);
    }
    state.rate_limiter.clear(&key);

    let ticket = auth::random_token(24);
    let expires_at = (unix_now() + 3600).min(share.expires_at.unwrap_or(i64::MAX));
    let mut tickets = state.share_tickets.lock();
    tickets.retain(|_, value| value.expires_at > unix_now());
    tickets.insert(
        ticket.clone(),
        ShareTicket {
            share_id: share.id,
            expires_at,
        },
    );
    drop(tickets);

    let _ = headers;
    Ok(Json(json!({ "ticket": ticket, "expires_at": expires_at })))
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<ShareQuery>,
) -> Result<Response, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers)?;
    state
        .db
        .increment_share_visit(share.id)
        .await
        .map_err(AppError::internal)?;
    let path = query.path.unwrap_or_default();
    let resolved = if share.is_dir {
        resolve_share_path(&share, &path).await?
    } else {
        if !path.is_empty() {
            return Err(AppError::NotFound);
        }
        pathresolver::ResolvedPath {
            root: Root {
                id: 0,
                user_id: 0,
                real_path: share.real_path.clone(),
                alias: "share".to_owned(),
                read_only: share.read_only,
                sort_order: 0,
            },
            relative: String::new(),
            absolute: PathBuf::from(&share.real_path),
            access_read_only: share.read_only,
        }
    };
    let info = tokio::task::spawn_blocking({
        let path = resolved.absolute.clone();
        move || fsops::file_info(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("share download task failed: {err}")))??;

    let file = tokio::fs::File::open(&resolved.absolute)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    let filename = resolved
        .absolute
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let disposition = format!(
        "attachment; filename*=UTF-8''{}",
        percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC)
    );
    let mime = mime_guess::from_path(&resolved.absolute).first_or_octet_stream();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CONTENT_LENGTH, info.size.to_string())
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))
}

async fn load_public_share(state: &Arc<AppState>, token: &str) -> Result<Share, AppError> {
    let hash = auth::sha256_hex(token);
    let share = state
        .db
        .find_share_by_hash(&hash)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if share.revoked
        || share
            .expires_at
            .is_some_and(|expires| expires <= unix_now())
    {
        return Err(AppError::NotFound);
    }
    Ok(share)
}

fn require_ticket(
    state: &Arc<AppState>,
    share: &Share,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    if share.password_hash.is_none() {
        return Ok(());
    }
    let ticket = headers
        .get("x-share-ticket")
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    let tickets = state.share_tickets.lock();
    let valid = tickets
        .get(ticket)
        .is_some_and(|value| value.share_id == share.id && value.expires_at > unix_now());
    if valid {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

async fn resolve_share_path(
    share: &Share,
    path: &str,
) -> Result<pathresolver::ResolvedPath, AppError> {
    let root = Root {
        id: 0,
        user_id: 0,
        real_path: share.real_path.clone(),
        alias: "share".to_owned(),
        read_only: share.read_only,
        sort_order: 0,
    };
    pathresolver::resolve(root, path, false).await
}

fn share_json(share: &Share) -> Value {
    json!({
        "id": share.id,
        "real_path": share.real_path,
        "is_dir": share.is_dir,
        "has_password": share.password_hash.is_some(),
        "expires_at": share.expires_at,
        "read_only": share.read_only,
        "revoked": share.revoked,
        "created_at": share.created_at,
        "visit_count": share.visit_count,
    })
}

async fn load_root(state: &Arc<AppState>, user_id: i64, root_id: i64) -> Result<Root, AppError> {
    state
        .db
        .find_root(root_id, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)
}

async fn audit(state: &Arc<AppState>, user_id: i64, action: &str, path: &str, headers: &HeaderMap) {
    let ip = auth::client_ip(headers);
    if let Err(error) = state
        .db
        .audit(Some(user_id), action, Some(path), None, ip.as_deref())
        .await
    {
        tracing::warn!(error = %error, action, "failed to write share audit log");
    }
}
