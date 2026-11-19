use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Form, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::time::Duration;

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
    path: Option<String>,
    paths: Option<Vec<String>>,
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
    let candidates: Vec<String> = request
        .paths
        .clone()
        .filter(|paths| !paths.is_empty())
        .unwrap_or_else(|| vec![request.path.clone().unwrap_or_default()]);
    if candidates.is_empty() || candidates.len() > 100 {
        return Err(AppError::bad_request("share between 1 and 100 paths"));
    }
    if request
        .expires_at
        .is_some_and(|expires| expires <= unix_now())
    {
        return Err(AppError::bad_request("expires_at must be in the future"));
    }

    let mut items = Vec::with_capacity(candidates.len());
    for path in candidates {
        let resolved = authz::resolve(&state, current.user.id, root.id, &path, false).await?;
        if !resolved.absolute.exists() {
            return Err(AppError::NotFound);
        }
        items.push(resolved);
    }
    let collection = items.len() > 1;
    if collection && items.iter().any(|item| item.absolute.is_dir()) {
        return Err(AppError::bad_request(
            "directories must be shared separately",
        ));
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

    let (id, audit_path, item_count) = if collection {
        let entries: Vec<(String, String, bool)> = items
            .iter()
            .map(|item| {
                let absolute = item.absolute.to_string_lossy().to_string();
                let name = item
                    .absolute
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
                    .to_owned();
                (absolute, name, false)
            })
            .collect();
        let id = state
            .db
            .insert_collection_share(
                &token_hash,
                current.user.id,
                &entries,
                password_hash.as_deref(),
                request.expires_at,
                true,
            )
            .await
            .map_err(AppError::internal)?;
        (id, "collection".to_owned(), entries.len())
    } else {
        let item = &items[0];
        let real_path = item.absolute.to_string_lossy().to_string();
        let id = state
            .db
            .insert_share(
                &token_hash,
                current.user.id,
                &real_path,
                item.absolute.is_dir(),
                password_hash.as_deref(),
                request.expires_at,
                request.read_only.unwrap_or(true),
            )
            .await
            .map_err(AppError::internal)?;
        (id, real_path, 1)
    };

    audit(
        &state,
        current.user.id,
        "share_created",
        &audit_path,
        &headers,
    )
    .await;
    state.events.publish(
        "share.created",
        Some(current.user.id),
        json!({ "id": id, "kind": if collection { "collection" } else { "single" } }),
    );
    Ok(Json(json!({
        "id": id,
        "token": token,
        "url": format!("/s/{token}"),
        "expires_at": request.expires_at,
        "kind": if collection { "collection" } else { "single" },
        "item_count": item_count,
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
    state
        .events
        .publish("share.updated", Some(current.user.id), json!({ "id": id }));
    state
        .events
        .publish_share("share.updated", id, json!({ "id": id }));
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
    ticket: Option<String>,
}

pub async fn upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    if !share.is_dir || share.read_only {
        return Err(AppError::Forbidden);
    }
    let path = query.path.unwrap_or_default();
    ensure_public_path(&share, &path)?;
    let resolved = resolve_share_path(&share, &path).await?;
    if !resolved.absolute.is_dir() {
        return Err(AppError::bad_request("upload target is not a directory"));
    }
    let creator = state
        .db
        .find_user_by_id(share.creator_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Forbidden)?;
    let filename = query.filename.trim();
    if !share_root_is_hidden(&share) && filename.starts_with('.') {
        return Err(AppError::invalid_path(
            "hidden files cannot be uploaded to a public share",
        ));
    }
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
    let base_target = resolved.absolute.join(filename);
    crate::usage::ensure_entry_capacity(&state, &creator, 1).await?;
    let data = body.to_vec();
    let existed = std::fs::symlink_metadata(&base_target).is_ok();
    let target = tokio::task::spawn_blocking(move || {
        let mut candidate = fsops::unique_target(&base_target);
        for _ in 0..16 {
            match fsops::write_new_file(&candidate, &data) {
                Ok(()) => return Ok(candidate),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    candidate = fsops::unique_target(&candidate);
                }
                Err(error) => {
                    return Err(AppError::internal(anyhow::Error::from(error)));
                }
            }
        }
        Err(AppError::conflict("upload target keeps changing"))
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("upload task failed: {err}")))??;
    if !existed || target != resolved.absolute.join(filename) {
        state.adjust_entries(share.creator_id, 1);
    }
    state
        .invalidate_path_caches(&target, Some(share.creator_id), false)
        .await;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(filename)
        .to_owned();
    let activity = json!({ "id": share.id, "name": name.clone(), "path": path });
    state
        .events
        .publish("share.uploaded", Some(share.creator_id), activity.clone());
    state
        .events
        .publish_share("share.uploaded", share.id, activity);
    Ok(Json(json!({ "name": name })))
}

#[derive(Debug, Deserialize)]
pub struct CreateShareUploadRequest {
    path: Option<String>,
    filename: String,
    total_size: i64,
    chunk_size: Option<i64>,
    ticket: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShareUploadQuery {
    ticket: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShareChunkQuery {
    ticket: Option<String>,
}

pub async fn create_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Json(request): Json<CreateShareUploadRequest>,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, request.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    if !share.is_dir || share.read_only {
        return Err(AppError::Forbidden);
    }
    let path = request.path.unwrap_or_default();
    ensure_public_path(&share, &path)?;
    let resolved = resolve_share_path(&share, &path).await?;
    if !resolved.absolute.is_dir() {
        return Err(AppError::bad_request("upload target is not a directory"));
    }
    let filename = crate::files::validate_filename(&request.filename)?;
    if !share_root_is_hidden(&share) && filename.starts_with('.') {
        return Err(AppError::invalid_path(
            "hidden files cannot be uploaded to a public share",
        ));
    }
    let size = crate::files::validate_upload_size(&state, request.total_size)?;
    let chunk_size = request
        .chunk_size
        .unwrap_or_else(|| state.settings().settings.uploads.chunk_size as i64);
    if !(1..=64 * 1024 * 1024).contains(&chunk_size) {
        return Err(AppError::bad_request(
            "chunk_size must be between 1 and 64 MiB",
        ));
    }
    let chunks = (size + chunk_size - 1) / chunk_size;
    if chunks > 100_000 {
        return Err(AppError::bad_request("too many upload chunks"));
    }

    let creator = state
        .db
        .find_user_by_id(share.creator_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Forbidden)?;
    crate::usage::ensure_entry_capacity(&state, &creator, 1).await?;
    let upload_id = format!("su_{}", auth::random_token(16));
    let temp = fsops::create_upload_temp(&resolved.absolute, &upload_id)?;
    let now = unix_now();
    let ttl = state.settings().settings.uploads.session_ttl_hours as i64;
    let upload = crate::db::ShareUploadSession {
        id: upload_id.clone(),
        share_id: share.id,
        dir_path: resolved.relative.clone(),
        filename,
        total_size: size,
        chunk_size,
        received: 0,
        chunk_bitmap: vec![0; ((chunks + 7) / 8) as usize],
        temp_path: temp.to_string_lossy().to_string(),
        expires_at: now + ttl.saturating_mul(3600),
        created_at: now,
        updated_at: now,
    };
    if let Err(error) = state.db.insert_share_upload(&upload).await {
        fsops::delete_temp(&temp);
        return Err(AppError::internal(error));
    }
    Ok(Json(json!({
        "upload_id": upload_id,
        "chunk_size": chunk_size,
        "chunks": chunks,
        "expires_at": upload.expires_at,
    })))
}

async fn load_share_upload(
    state: &Arc<AppState>,
    share: &Share,
    id: &str,
) -> Result<crate::db::ShareUploadSession, AppError> {
    let upload = state
        .db
        .find_share_upload(id, share.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if upload.expires_at <= unix_now() {
        let _ = state.db.delete_share_upload(id, share.id).await;
        fsops::delete_temp(std::path::Path::new(&upload.temp_path));
        return Err(AppError::NotFound);
    }
    Ok(upload)
}

fn share_received_chunks(upload: &crate::db::ShareUploadSession) -> Vec<i64> {
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    let mut received = Vec::new();
    for index in 0..chunks {
        let byte = (index / 8) as usize;
        let bit = 1_u8 << (index % 8);
        if upload
            .chunk_bitmap
            .get(byte)
            .is_some_and(|value| value & bit != 0)
        {
            received.push(index);
        }
    }
    received
}

pub async fn upload_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((token, id)): AxumPath<(String, String)>,
    Query(query): Query<ShareUploadQuery>,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    let upload = load_share_upload(&state, &share, &id).await?;
    Ok(Json(json!({
        "upload_id": upload.id,
        "received": upload.received,
        "total_size": upload.total_size,
        "chunks": (upload.total_size + upload.chunk_size - 1) / upload.chunk_size,
        "received_chunks": share_received_chunks(&upload),
        "expires_at": upload.expires_at,
    })))
}

pub async fn upload_chunk(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((token, id, index)): AxumPath<(String, String, i64)>,
    Query(query): Query<ShareChunkQuery>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    if !share.is_dir || share.read_only {
        return Err(AppError::Forbidden);
    }
    let upload = load_share_upload(&state, &share, &id).await?;
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    if index < 0 || index >= chunks {
        return Err(AppError::bad_request("chunk index out of range"));
    }
    let offset = index * upload.chunk_size;
    let expected = (upload.total_size - offset).min(upload.chunk_size) as usize;
    if body.len() != expected {
        return Err(AppError::bad_request("invalid chunk size"));
    }
    let temp = std::path::PathBuf::from(&upload.temp_path);
    let bytes = body.to_vec();
    tokio::task::spawn_blocking(move || fsops::write_upload_chunk(&temp, offset as u64, &bytes))
        .await
        .map_err(|error| AppError::internal(anyhow::anyhow!("chunk task failed: {error}")))??;

    let byte = (index / 8) as usize;
    let bit = 1_u8 << (index % 8);
    let mut bitmap = upload.chunk_bitmap.clone();
    let was_set = bitmap.get(byte).is_some_and(|value| value & bit != 0);
    if bitmap.len() <= byte {
        bitmap.resize(byte + 1, 0);
    }
    bitmap[byte] |= bit;
    let received = if was_set {
        upload.received
    } else {
        upload.received + expected as i64
    };
    state
        .db
        .update_share_upload_progress(&id, share.id, received, bitmap)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({ "received": received, "chunk": index })))
}

pub async fn complete_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((token, id)): AxumPath<(String, String)>,
    Query(query): Query<ShareUploadQuery>,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    if !share.is_dir || share.read_only {
        return Err(AppError::Forbidden);
    }
    let upload = load_share_upload(&state, &share, &id).await?;
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    if share_received_chunks(&upload).len() as i64 != chunks {
        return Err(AppError::bad_request("upload is incomplete"));
    }
    let resolved = resolve_share_path(&share, &upload.dir_path).await?;
    if !resolved.absolute.is_dir() {
        return Err(AppError::NotFound);
    }
    let temp = std::path::PathBuf::from(&upload.temp_path);
    let target = fsops::unique_target(&resolved.absolute.join(&upload.filename));
    tokio::fs::rename(&temp, &target)
        .await
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
    state
        .invalidate_path_caches(&target, Some(share.creator_id), false)
        .await;
    let _ = state.db.delete_share_upload(&id, share.id).await;
    state.adjust_entries(share.creator_id, 1);
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&upload.filename)
        .to_owned();
    let activity = json!({
        "id": share.id,
        "name": name.clone(),
        "path": upload.dir_path,
    });
    state
        .events
        .publish("share.uploaded", Some(share.creator_id), activity.clone());
    state
        .events
        .publish_share("share.uploaded", share.id, activity);
    Ok(Json(json!({ "name": name })))
}

pub async fn cancel_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((token, id)): AxumPath<(String, String)>,
    Query(query): Query<ShareUploadQuery>,
) -> Result<StatusCode, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    if let Ok(upload) = load_share_upload(&state, &share, &id).await {
        fsops::delete_temp(std::path::Path::new(&upload.temp_path));
        let _ = state.db.delete_share_upload(&id, share.id).await;
    }
    Ok(StatusCode::NO_CONTENT)
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
    state
        .events
        .publish_share("share.revoked", id, json!({ "id": id }));
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
    state
        .events
        .publish("share.revoked", Some(current.user.id), json!({ "id": id }));
    state
        .events
        .publish_share("share.revoked", id, json!({ "id": id }));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ShareQuery {
    path: Option<String>,
    ticket: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
    item: Option<i64>,
    inline: Option<bool>,
}

pub async fn public_meta(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<ShareQuery>,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    let path = query.path.unwrap_or_default();
    ensure_public_path(&share, &path)?;
    // Only the root page load counts as a visit; thumbnails, downloads and
    // sub-directory paging must not inflate the counter.
    if path.is_empty() && query.cursor.is_none() {
        state
            .db
            .increment_share_visit(share.id)
            .await
            .map_err(AppError::internal)?;
        let activity = json!({
            "id": share.id,
            "name": if share.kind == "collection" {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(share_name(&share))
            },
            "path": path,
            "at": unix_now(),
        });
        state
            .events
            .publish("share.visited", Some(share.creator_id), activity.clone());
        state
            .events
            .publish_share("share.visited", share.id, activity);
    }
    let zip_allowed = state
        .db
        .find_user_by_id(share.creator_id)
        .await
        .map_err(AppError::internal)?
        .is_some_and(|user| user.feature_zip);
    let video_enabled = {
        let settings = state.settings();
        settings.settings.thumbnails.enabled
            && settings.settings.thumbnails.video
            && state.ffmpeg.is_some()
    };

    if share.kind == "collection" {
        if !path.is_empty() {
            return Err(AppError::NotFound);
        }
        let items = state
            .db
            .share_items(share.id)
            .await
            .map_err(AppError::internal)?;
        let mut entries = Vec::new();
        for item in items {
            if item.is_dir {
                continue;
            }
            let item_path = PathBuf::from(&item.real_path);
            let info = tokio::task::spawn_blocking(move || fsops::file_info(&item_path)).await;
            let Ok(Ok(info)) = info else {
                continue;
            };
            entries.push(json!({
                "item_id": item.id,
                "name": item.display_name,
                "is_dir": false,
                "is_symlink": false,
                "size": info.size,
                "modified": info.modified,
                "mime_type": mime_guess::from_path(&item.real_path).first_or_octet_stream().to_string(),
                "has_preview": crate::thumbs::can_preview(&item.display_name, video_enabled),
            }));
        }
        return Ok(Json(json!({
            "name": format!("{} 个文件", entries.len()),
            "path": "",
            "is_dir": true,
            "collection": true,
            "read_only": true,
            "zip_allowed": zip_allowed,
            "entries": entries,
        })));
    }

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
        let name = share_name(&share);
        return Ok(Json(json!({
            "name": name,
            "path": "",
            "is_dir": false,
            "read_only": share.read_only,
            "zip_allowed": zip_allowed,
            "size": info.size,
            "modified": info.modified,
            "mime_type": mime_guess::from_path(&share.real_path).first_or_octet_stream().to_string(),
            "has_preview": crate::thumbs::can_preview(&name, video_enabled),
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
        let name = resolved
            .absolute
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_owned();
        return Ok(Json(json!({
            "name": name,
            "path": resolved.relative,
            "is_dir": false,
            "read_only": share.read_only,
            "zip_allowed": zip_allowed,
            "size": info.size,
            "modified": info.modified,
            "mime_type": mime_guess::from_path(&resolved.absolute).first_or_octet_stream().to_string(),
            "has_preview": crate::thumbs::can_preview(&name, video_enabled),
        })));
    }

    let mut entries = tokio::task::spawn_blocking({
        let resolved = resolved.clone();
        move || fsops::list_dir(&resolved, "name", "asc")
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("share list task failed: {err}")))??;
    if !share_root_is_hidden(&share) {
        entries.retain(|entry| !entry.name.starts_with('.'));
    }
    let limit = query.limit.unwrap_or(200).clamp(1, 1000) as usize;
    let offset = query
        .cursor
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| AppError::bad_request("invalid cursor"))
        })
        .transpose()?
        .unwrap_or(0);
    let total = entries.len();
    let entries: Vec<fsops::Entry> = entries
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|mut entry| {
            entry.has_preview =
                !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled);
            entry
        })
        .collect();

    let filesystem_read_only = if share.read_only {
        false
    } else {
        let path = resolved.absolute.clone();
        tokio::task::spawn_blocking(move || fsops::directory_writable(&path))
            .await
            .ok()
            .flatten()
            == Some(false)
    };

    Ok(Json(json!({
        "name": resolved.absolute.file_name().and_then(|name| name.to_str()).unwrap_or("分享"),
        "path": resolved.relative,
        "is_dir": true,
        "read_only": share.read_only || filesystem_read_only,
        "zip_allowed": zip_allowed,
        "entries": entries,
        "next_cursor": (offset.saturating_add(limit) < total)
            .then(|| (offset + limit).to_string()),
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
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    if share.kind == "collection" {
        let item_id = query.item.ok_or(AppError::NotFound)?;
        let item = state
            .db
            .share_item(share.id, item_id)
            .await
            .map_err(AppError::internal)?
            .ok_or(AppError::NotFound)?;
        if item.is_dir {
            return Err(AppError::NotFound);
        }
        let activity = json!({
            "id": share.id,
            "item": item.id,
            "name": item.display_name.clone(),
            "path": item.display_name,
        });
        if !query.inline.unwrap_or(false) && !headers.contains_key("range") {
            state
                .events
                .publish("share.downloaded", Some(share.creator_id), activity.clone());
            state
                .events
                .publish_share("share.downloaded", share.id, activity);
        }
        return crate::files::serve_file(
            &headers,
            Path::new(&item.real_path),
            query.inline.unwrap_or(false),
        )
        .await;
    }
    let path = query.path.unwrap_or_default();
    ensure_public_path(&share, &path)?;
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
    if resolved.absolute.is_dir() {
        return Err(AppError::NotFound);
    }
    let name = resolved
        .absolute
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let activity = json!({ "id": share.id, "name": name, "path": path });
    if !query.inline.unwrap_or(false) && !headers.contains_key("range") {
        state
            .events
            .publish("share.downloaded", Some(share.creator_id), activity.clone());
        state
            .events
            .publish_share("share.downloaded", share.id, activity);
    }
    crate::files::serve_file(&headers, &resolved.absolute, false).await
}

pub async fn events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<ShareQuery>,
) -> Result<Response, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    let share_id = share.id;
    let deadline = share.expires_at.map(|expires| {
        tokio::time::Instant::now()
            + Duration::from_secs(expires.saturating_sub(unix_now()).max(0) as u64)
    });
    let receiver = state.events.subscribe();
    let stream = futures_util::stream::unfold(
        (receiver, deadline),
        move |(mut receiver, deadline)| async move {
            loop {
                if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                    return None;
                }
                let received = match deadline {
                    Some(deadline) => tokio::select! {
                        received = receiver.recv() => Some(received),
                        () = tokio::time::sleep_until(deadline) => None,
                    },
                    None => Some(receiver.recv().await),
                };
                let received = received?;
                match received {
                    Ok(event) => {
                        if !event.visible_to_share(share_id) {
                            continue;
                        }
                        let data = serde_json::to_string(&event.data).unwrap_or_default();
                        let out = SseEvent::default()
                            .id(event.id.to_string())
                            .event(event.event)
                            .data(data);
                        return Some((Ok::<_, Infallible>(out), (receiver, deadline)));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

#[derive(Debug, Deserialize)]
pub struct PublicThumbQuery {
    path: Option<String>,
    ticket: Option<String>,
    w: Option<u32>,
    edge: Option<u32>,
    fit: Option<String>,
    item: Option<i64>,
    wait: Option<String>,
}

pub async fn thumb(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<PublicThumbQuery>,
) -> Result<Response, AppError> {
    let share = load_public_share(&state, &token).await?;
    require_ticket(&state, &share, &headers, query.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    if share.kind == "collection" {
        let item_id = query.item.ok_or(AppError::NotFound)?;
        let item = state
            .db
            .share_item(share.id, item_id)
            .await
            .map_err(AppError::internal)?
            .ok_or(AppError::NotFound)?;
        if item.is_dir {
            return Err(AppError::NotFound);
        }
        let fit = crate::thumbs::Fit::from_query(query.fit.as_deref());
        let edge = query
            .edge
            .unwrap_or_else(|| query.w.unwrap_or(256))
            .clamp(16, 2048);
        return crate::thumbs::thumbnail_response(
            &state,
            PathBuf::from(&item.real_path),
            edge,
            edge,
            fit,
            share.creator_id,
            &headers,
            Some(crate::thumbs::ThumbScope {
                root_id: None,
                share_id: Some(share.id),
                item_id: Some(item.id),
                path: item.display_name.clone(),
            }),
            crate::thumbs::wants_wait(query.wait.as_deref()),
        )
        .await;
    }
    let path = query.path.unwrap_or_default();
    ensure_public_path(&share, &path)?;
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
    if resolved.absolute.is_dir() {
        return Err(AppError::NotFound);
    }
    let fit = crate::thumbs::Fit::from_query(query.fit.as_deref());
    let edge = query
        .edge
        .unwrap_or_else(|| query.w.unwrap_or(256))
        .clamp(16, 2048);
    let scope_path = resolved.relative.clone();
    crate::thumbs::thumbnail_response(
        &state,
        resolved.absolute,
        edge,
        edge,
        fit,
        share.creator_id,
        &headers,
        Some(crate::thumbs::ThumbScope {
            root_id: None,
            share_id: Some(share.id),
            item_id: None,
            path: scope_path,
        }),
        crate::thumbs::wants_wait(query.wait.as_deref()),
    )
    .await
}

#[derive(Debug, Deserialize, Default)]
pub struct PublicZipRequest {
    path: Option<String>,
    paths: Option<Vec<String>>,
    item_ids: Option<Vec<i64>>,
    ticket: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PublicZipForm {
    path: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    item_ids: Vec<i64>,
    ticket: Option<String>,
}

fn share_root_resolved(share: &Share) -> pathresolver::ResolvedPath {
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
}

struct ZipTarget {
    resolved: pathresolver::ResolvedPath,
    name: Option<String>,
}

async fn resolve_zip_selection(
    state: &Arc<AppState>,
    share: &Share,
    path: Option<String>,
    paths: Vec<String>,
    item_ids: Vec<i64>,
) -> Result<(Vec<ZipTarget>, String), AppError> {
    if share.kind == "collection" {
        let items = if item_ids.is_empty() {
            state
                .db
                .share_items(share.id)
                .await
                .map_err(AppError::internal)?
        } else {
            if item_ids.len() > 1000 {
                return Err(AppError::bad_request("select at most 1000 items"));
            }
            let mut selected = Vec::new();
            for item_id in item_ids {
                if let Some(item) = state
                    .db
                    .share_item(share.id, item_id)
                    .await
                    .map_err(AppError::internal)?
                {
                    selected.push(item);
                }
            }
            selected
        };
        let root = Root {
            id: 0,
            user_id: 0,
            real_path: String::new(),
            alias: "share".to_owned(),
            read_only: true,
            sort_order: 0,
        };
        let mut selected = Vec::new();
        for item in items {
            if item.is_dir || !Path::new(&item.real_path).exists() {
                continue;
            }
            selected.push(ZipTarget {
                resolved: pathresolver::ResolvedPath {
                    root: root.clone(),
                    relative: item.display_name.clone(),
                    absolute: PathBuf::from(&item.real_path),
                    access_read_only: true,
                },
                name: Some(item.display_name),
            });
        }
        if selected.is_empty() {
            return Err(AppError::NotFound);
        }
        return Ok((selected, "files.zip".to_owned()));
    }

    if !share.is_dir {
        return Ok((
            vec![ZipTarget {
                resolved: share_root_resolved(share),
                name: None,
            }],
            resolved_filename(share, ""),
        ));
    }
    let candidates: Vec<String> = if paths.is_empty() {
        vec![path.unwrap_or_default()]
    } else {
        paths
    };
    if candidates.len() > 1000 {
        return Err(AppError::bad_request("select at most 1000 paths"));
    }
    let filename = if candidates.len() == 1 {
        resolved_filename(share, &candidates[0])
    } else {
        "filehub.zip".to_owned()
    };
    let mut selected = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        ensure_public_path(share, &candidate)?;
        let resolved = resolve_share_path(share, &candidate).await?;
        if !resolved.absolute.exists() {
            return Err(AppError::NotFound);
        }
        selected.push(ZipTarget {
            resolved,
            name: None,
        });
    }
    Ok((selected, filename))
}

async fn collect_public_zip_entries(
    state: &Arc<AppState>,
    share: &Share,
    selected: Vec<ZipTarget>,
) -> Result<Vec<fsops::ZipEntry>, AppError> {
    let settings = state.settings();
    let max_bytes = settings.settings.downloads.zip_max_bytes;
    let max_files = settings.settings.downloads.zip_max_files;
    let skip_hidden = !share_root_is_hidden(share);
    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        let mut total_bytes = 0_u64;
        for target in selected {
            if let Some(name) = target.name {
                fsops::collect_zip_named(
                    &target.resolved.absolute,
                    &name,
                    &mut entries,
                    &mut total_bytes,
                    max_bytes,
                    max_files,
                    skip_hidden,
                )?;
            } else {
                fsops::collect_zip_path(
                    &target.resolved,
                    &mut entries,
                    &mut total_bytes,
                    max_bytes,
                    max_files,
                    skip_hidden,
                )?;
            }
        }
        Ok::<_, AppError>(entries)
    })
    .await
    .map_err(|error| AppError::internal(anyhow::anyhow!("share zip task failed: {error}")))?
}

#[allow(clippy::too_many_arguments)]
async fn public_zip_response(
    state: Arc<AppState>,
    headers: &HeaderMap,
    token: String,
    path: Option<String>,
    paths: Vec<String>,
    item_ids: Vec<i64>,
    ticket: Option<String>,
) -> Result<Response, AppError> {
    let share = load_public_share(&state, &token).await?;
    let creator = state
        .db
        .find_user_by_id(share.creator_id)
        .await
        .map_err(AppError::internal)?;
    let Some(creator) = creator else {
        return Err(AppError::Forbidden);
    };
    let capabilities = crate::capabilities::load(&state, &creator).await?;
    if !capabilities.zip {
        return Err(AppError::Forbidden);
    }
    require_ticket(&state, &share, headers, ticket.as_deref())?;
    check_public_rate(&state, &share, headers)?;
    let (selected, filename) = resolve_zip_selection(&state, &share, path, paths, item_ids).await?;
    let entries = collect_public_zip_entries(&state, &share, selected).await?;
    let activity = json!({
        "id": share.id,
        "name": filename,
        "path": filename,
        "items": entries.len(),
        "bytes": entries.iter().map(|entry| entry.size).sum::<u64>(),
        "zip": true,
    });
    state
        .events
        .publish("share.downloaded", Some(share.creator_id), activity.clone());
    state
        .events
        .publish_share("share.downloaded", share.id, activity);
    crate::files::zip_response(&state, entries, &filename)
}

pub async fn zip(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Json(request): Json<PublicZipRequest>,
) -> Result<Response, AppError> {
    public_zip_response(
        state,
        &headers,
        token,
        request.path,
        request.paths.unwrap_or_default(),
        request.item_ids.unwrap_or_default(),
        request.ticket,
    )
    .await
}

pub async fn zip_form(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Form(request): Form<PublicZipForm>,
) -> Result<Response, AppError> {
    public_zip_response(
        state,
        &headers,
        token,
        request.path,
        request.paths,
        request.item_ids,
        request.ticket,
    )
    .await
}

pub async fn zip_preflight(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Json(request): Json<PublicZipRequest>,
) -> Result<Json<Value>, AppError> {
    let share = load_public_share(&state, &token).await?;
    let creator = state
        .db
        .find_user_by_id(share.creator_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Forbidden)?;
    let capabilities = crate::capabilities::load(&state, &creator).await?;
    if !capabilities.zip {
        return Ok(Json(json!({
            "allowed": false,
            "files": 0,
            "bytes": 0,
            "max_bytes": state.settings().settings.downloads.zip_max_bytes,
            "max_files": state.settings().settings.downloads.zip_max_files,
            "message": "zip is disabled for the share owner",
        })));
    }
    require_ticket(&state, &share, &headers, request.ticket.as_deref())?;
    check_public_rate(&state, &share, &headers)?;
    let (selected, _filename) = resolve_zip_selection(
        &state,
        &share,
        request.path,
        request.paths.unwrap_or_default(),
        request.item_ids.unwrap_or_default(),
    )
    .await?;
    let settings = state.settings();
    let max_bytes = settings.settings.downloads.zip_max_bytes;
    let max_files = settings.settings.downloads.zip_max_files;
    let skip_hidden = !share_root_is_hidden(&share);
    let outcome = tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        let mut total_bytes = 0_u64;
        for target in selected {
            let result = if let Some(name) = target.name {
                fsops::collect_zip_named(
                    &target.resolved.absolute,
                    &name,
                    &mut entries,
                    &mut total_bytes,
                    max_bytes,
                    max_files,
                    skip_hidden,
                )
            } else {
                fsops::collect_zip_path(
                    &target.resolved,
                    &mut entries,
                    &mut total_bytes,
                    max_bytes,
                    max_files,
                    skip_hidden,
                )
            };
            match result {
                Ok(()) => {}
                Err(AppError::TooLarge(message)) => return Err(message),
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok((true, entries.len() as u64, total_bytes))
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("share zip preflight failed: {err}")))?;
    match outcome {
        Ok((allowed, files, bytes)) => Ok(Json(json!({
            "allowed": allowed,
            "files": files,
            "bytes": bytes,
            "max_bytes": max_bytes,
            "max_files": max_files,
        }))),
        Err(message) => Ok(Json(json!({
            "allowed": false,
            "message": message,
            "max_bytes": max_bytes,
            "max_files": max_files,
        }))),
    }
}

fn resolved_filename(share: &Share, path: &str) -> String {
    if path.is_empty() {
        return format!("{}.zip", share_name(share));
    }
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.zip"))
        .unwrap_or_else(|| "filehub.zip".to_owned())
}

async fn load_public_share(state: &Arc<AppState>, token: &str) -> Result<Share, AppError> {
    let hash = auth::sha256_hex(token);
    let share = state
        .db
        .find_share_by_hash(&hash)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if share.revoked {
        return Err(AppError::ShareRevoked);
    }
    if share
        .expires_at
        .is_some_and(|expires| expires <= unix_now())
    {
        return Err(AppError::ShareExpired);
    }
    Ok(share)
}

fn check_public_rate(
    state: &Arc<AppState>,
    share: &Share,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    let limit = state
        .settings()
        .settings
        .security
        .public_rate_limit_per_minute;
    if limit == 0 {
        return Ok(());
    }
    let ip = auth::client_ip(headers).unwrap_or_else(|| "unknown".to_owned());
    let key = format!("share:{}:{ip}", share.id);
    if state.public_rate.allow(&key, limit, 60) {
        Ok(())
    } else {
        Err(AppError::TooManyRequests)
    }
}

fn require_ticket(
    state: &Arc<AppState>,
    share: &Share,
    headers: &HeaderMap,
    query_ticket: Option<&str>,
) -> Result<(), AppError> {
    if share.password_hash.is_none() {
        return Ok(());
    }
    let ticket = headers
        .get("x-share-ticket")
        .and_then(|value| value.to_str().ok())
        .or(query_ticket)
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

fn share_name(share: &Share) -> String {
    Path::new(&share.real_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_owned()
}

fn share_root_is_hidden(share: &Share) -> bool {
    Path::new(&share.real_path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn ensure_public_path(share: &Share, path: &str) -> Result<(), AppError> {
    if share_root_is_hidden(share) {
        return Ok(());
    }
    if path
        .split(['/', '\\'])
        .any(|part| part.starts_with('.') && part != "." && part != "..")
    {
        return Err(AppError::NotFound);
    }
    Ok(())
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
        "kind": share.kind,
        "item_count": share.item_count,
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
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: action.to_owned(),
            path: Some(path.to_owned()),
            detail: None,
            ip: auth::client_ip(headers),
        })
        .await;
}
