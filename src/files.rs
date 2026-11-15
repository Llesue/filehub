use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::{ReaderStream, SyncIoBridge};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::auth;
use crate::authz;
use crate::db::{Root, UploadSession, unix_now};
use crate::error::AppError;
use crate::fsops;
use crate::state::{AppState, DeleteItem, DeletePreview};

pub async fn roots(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let roots = state
        .db
        .list_roots(current.user.id)
        .await
        .map_err(AppError::internal)?;

    Ok(Json(json!({
        "roots": roots.iter().map(root_json).collect::<Vec<_>>()
    })))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    root: i64,
    path: Option<String>,
    sort: Option<String>,
    order: Option<String>,
    sizes: Option<bool>,
    limit: Option<usize>,
    cursor: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let show_hidden = preferences.show_hidden;
    let root = load_root(&state, current.user.id, query.root).await?;
    let path = query.path.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, root.id, &path, false).await?;
    let sort = query.sort.unwrap_or_else(|| "name".to_owned());
    let order = query.order.unwrap_or_else(|| "asc".to_owned());
    let real_path = resolved.absolute.to_string_lossy().to_string();
    let current_mtime = fsops::dir_mtime_ns(&resolved.absolute).unwrap_or_default();
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
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

    if let Some((indexed_mtime, indexed_entries, has_more)) = state
        .db
        .indexed_children_page(&real_path, &sort, &order, show_hidden, offset, limit)
        .await
        .map_err(AppError::internal)?
    {
        if indexed_mtime == current_mtime {
            let mut entries: Vec<fsops::Entry> = indexed_entries
                .into_iter()
                .map(|entry| fsops::Entry {
                    name: entry.name,
                    is_dir: entry.is_dir,
                    is_symlink: false,
                    size: entry.size,
                    modified: entry.mtime,
                })
                .collect();
            if entries.is_empty() && offset == 0 {
                let _ = state.db.mark_index_stale(&real_path).await;
            } else {
                if query.sizes.unwrap_or(false) {
                    entries = enrich_directory_sizes(entries, resolved.absolute.clone()).await?;
                }
                return Ok(Json(json!({
                    "root": root_json(&resolved.root),
                    "path": resolved.relative,
                    "entries": entries,
                    "next_cursor": has_more.then(|| (offset + limit).to_string()),
                })));
            }
        } else {
            let _ = state.db.mark_index_stale(&real_path).await;
        }
    }

    let task = resolved.clone();
    let mut all_entries =
        tokio::task::spawn_blocking(move || fsops::list_dir(&task, &sort, &order))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("list task failed: {err}")))??;
    let has_more = all_entries.len() > offset.saturating_add(limit);
    let mut entries = if offset >= all_entries.len() {
        Vec::new()
    } else {
        all_entries
            .drain(offset..std::cmp::min(offset + limit, all_entries.len()))
            .collect()
    };
    if !show_hidden {
        entries.retain(|entry| !entry.name.starts_with('.'));
    }
    if query.sizes.unwrap_or(false) {
        entries = enrich_directory_sizes(entries, resolved.absolute.clone()).await?;
    }

    Ok(Json(json!({
        "root": root_json(&resolved.root),
        "path": resolved.relative,
        "entries": entries,
        "next_cursor": has_more.then(|| (offset + limit).to_string()),
    })))
}

async fn enrich_directory_sizes(
    mut entries: Vec<fsops::Entry>,
    base: std::path::PathBuf,
) -> Result<Vec<fsops::Entry>, AppError> {
    tokio::task::spawn_blocking(move || {
        for entry in &mut entries {
            if entry.is_dir {
                entry.size = fsops::path_size_limited(&base.join(&entry.name), 10_000) as i64;
            }
        }
        entries
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("directory size task failed: {err}")))
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    root: i64,
    path: String,
    inline: Option<bool>,
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, query.root).await?;
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let info_path = resolved.absolute.clone();
    let info = tokio::task::spawn_blocking(move || fsops::file_info(&info_path))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("metadata task failed: {err}")))??;

    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(info.etag.as_str())
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }

    let file = tokio::fs::File::open(&resolved.absolute)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    let filename = resolved
        .absolute
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let mime = mime_guess::from_path(&resolved.absolute).first_or_octet_stream();
    let is_pdf = filename.to_ascii_lowercase().ends_with(".pdf");
    let safe_inline = is_pdf
        || ((matches!(mime.type_().as_str(), "image" | "video" | "audio"))
            && mime.subtype().as_str() != "svg+xml");
    let disposition = if query.inline.unwrap_or(false) && safe_inline {
        "inline".to_owned()
    } else {
        format!(
            "attachment; filename*=UTF-8''{}",
            percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC)
        )
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, info.size));

    let content_type = if is_pdf {
        "application/pdf"
    } else {
        mime.as_ref()
    };
    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, &info.etag)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CACHE_CONTROL, "no-store");

    let response = if let Some((start, end)) = range {
        let length = end - start + 1;
        let mut file = file;
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        let reader = file.take(length);
        builder = builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{}", info.size),
            )
            .header(header::CONTENT_LENGTH, length.to_string());
        builder
            .body(Body::from_stream(ReaderStream::new(reader)))
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?
    } else if headers.get(header::RANGE).is_some() {
        return Ok((
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{}", info.size))],
        )
            .into_response());
    } else {
        builder = builder.header(header::CONTENT_LENGTH, info.size.to_string());
        builder
            .body(Body::from_stream(ReaderStream::new(file)))
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?
    };

    let _ = info.modified;
    Ok(response)
}

#[derive(Debug, Deserialize)]
pub struct ZipRequest {
    root: i64,
    paths: Vec<String>,
}

pub async fn zip(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ZipRequest>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    if request.paths.is_empty() || request.paths.len() > 1000 {
        return Err(AppError::bad_request("select between 1 and 1000 paths"));
    }

    let mut selected = Vec::with_capacity(request.paths.len());
    for path in request.paths {
        selected.push(authz::resolve(&state, current.user.id, root.id, &path, false).await?);
    }

    let settings = state.settings();
    let max_bytes = settings.settings.downloads.zip_max_bytes;
    let max_files = settings.settings.downloads.zip_max_files;
    let entries =
        tokio::task::spawn_blocking(move || fsops::collect_zip(&selected, max_bytes, max_files))
            .await
            .map_err(|err| {
                AppError::internal(anyhow::anyhow!("zip preparation failed: {err}"))
            })??;

    let (reader, writer) = tokio::io::duplex(64 * 1024);
    tokio::task::spawn_blocking(move || {
        let sync_writer = SyncIoBridge::new(writer);
        let mut zip = ZipWriter::new_stream(sync_writer);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .large_file(true);

        for entry in entries {
            let Ok(mut file) = std::fs::File::open(&entry.path) else {
                continue;
            };
            if zip.start_file(&entry.name, options).is_err() {
                return;
            }
            if std::io::copy(&mut file, &mut zip).is_err() {
                return;
            }
        }
        let _ = zip.finish();
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"filehub.zip\"",
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(ReaderStream::new(reader)))
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))
}

#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    root: i64,
    path: Option<String>,
    filename: String,
    total_size: i64,
    chunk_size: Option<i64>,
}

pub async fn create_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateUploadRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let path = request.path.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, root.id, &path, true).await?;
    let filename = validate_filename(&request.filename)?;
    let size = validate_upload_size(&state, request.total_size)?;

    if let Some(quota_bytes) = current.user.quota_bytes
        && crate::platform::path_within(
            &resolved.absolute,
            std::path::Path::new(&current.user.home_dir),
        )
    {
        let home = std::path::PathBuf::from(&current.user.home_dir);
        let usage = tokio::task::spawn_blocking(move || fsops::path_size(&home))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("quota task failed: {err}")))??;
        if usage.saturating_add(size as u64) > quota_bytes.max(0) as u64 {
            return Err(AppError::QuotaExceeded);
        }
    }

    let chunk_size = request
        .chunk_size
        .unwrap_or_else(|| state.settings().settings.uploads.chunk_size as i64);

    if !(1..=64 * 1024 * 1024).contains(&chunk_size) {
        return Err(AppError::bad_request(
            "chunk_size must be between 1 and 64 MiB",
        ));
    }
    if !resolved.absolute.is_dir() {
        return Err(AppError::bad_request("upload target is not a directory"));
    }

    let chunks = (size + chunk_size - 1) / chunk_size;
    if chunks > 100_000 {
        return Err(AppError::bad_request("too many upload chunks"));
    }

    let upload_id = format!("up_{}", auth::random_token(16));
    let temp = fsops::create_upload_temp(&resolved.absolute, &upload_id)?;
    let now = unix_now();
    let ttl = state.settings().settings.uploads.session_ttl_hours as i64;
    let upload = UploadSession {
        id: upload_id.clone(),
        user_id: current.user.id,
        root_id: root.id,
        dir_path: resolved.relative.clone(),
        filename: filename.clone(),
        total_size: size,
        chunk_size,
        received: 0,
        chunk_bitmap: vec![0; ((chunks + 7) / 8) as usize],
        temp_path: temp.to_string_lossy().to_string(),
        expires_at: now + ttl.saturating_mul(3600),
        created_at: now,
        updated_at: now,
    };

    if let Err(err) = state.db.insert_upload(&upload).await {
        fsops::delete_temp(&temp);
        return Err(AppError::internal(err));
    }

    Ok(Json(json!({
        "upload_id": upload_id,
        "chunk_size": chunk_size,
        "chunks": chunks,
        "expires_at": upload.expires_at,
    })))
}

pub async fn upload_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let upload = load_upload(&state, current.user.id, &id).await?;
    let received_chunks = received_chunks(&upload);

    Ok(Json(json!({
        "upload_id": upload.id,
        "total_size": upload.total_size,
        "chunk_size": upload.chunk_size,
        "received": upload.received,
        "received_chunks": received_chunks,
        "expires_at": upload.expires_at,
    })))
}

pub async fn upload_chunk(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((id, index)): AxumPath<(String, u32)>,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let mut upload = load_upload(&state, current.user.id, &id).await?;
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
    let already_received = upload
        .chunk_bitmap
        .get(byte_index)
        .is_some_and(|value| value & bit != 0);

    if !already_received {
        let temp_path = PathBuf::from(&upload.temp_path);
        let data = body.to_vec();
        let offset = (index * upload.chunk_size) as u64;
        tokio::task::spawn_blocking(move || fsops::write_upload_chunk(&temp_path, offset, &data))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("chunk task failed: {err}")))??;

        if let Some(value) = upload.chunk_bitmap.get_mut(byte_index) {
            *value |= bit;
        }
        upload.received = received_bytes(&upload);
        state
            .db
            .update_upload_progress(
                &upload.id,
                current.user.id,
                upload.received,
                upload.chunk_bitmap.clone(),
            )
            .await
            .map_err(AppError::internal)?;
    }

    Ok(Json(json!({
        "upload_id": upload.id,
        "received": upload.received,
        "complete": received_chunks(&upload).len() as i64 == chunks,
    })))
}

pub async fn complete_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let upload = load_upload(&state, current.user.id, &id).await?;
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    if received_chunks(&upload).len() as i64 != chunks {
        return Err(AppError::bad_request("upload is incomplete"));
    }

    let root = load_root(&state, current.user.id, upload.root_id).await?;
    let target_relative = if upload.dir_path.is_empty() {
        upload.filename.clone()
    } else {
        format!("{}/{}", upload.dir_path, upload.filename)
    };
    let target = authz::resolve(&state, current.user.id, root.id, &target_relative, true).await?;
    let temp = PathBuf::from(&upload.temp_path);
    let conflict = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?
        .upload_conflict;
    let mut target_path = target.absolute.clone();
    let mut target_relative = target_relative;
    if target_path.exists() {
        match conflict.as_str() {
            "skip" => {
                fsops::delete_temp(&temp);
                state
                    .db
                    .delete_upload(&upload.id, current.user.id)
                    .await
                    .map_err(AppError::internal)?;
                return Ok(Json(json!({
                    "path": target_relative,
                    "size": upload.total_size,
                    "skipped": true,
                })));
            }
            "overwrite" => {
                if target_path.is_dir() {
                    return Err(AppError::conflict("target is a directory"));
                }
                tokio::fs::remove_file(&target_path)
                    .await
                    .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
            }
            _ => {
                target_path = fsops::unique_target(&target_path);
                if let Some(name) = target_path.file_name().and_then(|name| name.to_str()) {
                    target_relative = if upload.dir_path.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{}/{}", upload.dir_path, name)
                    };
                }
            }
        }
    }

    tokio::task::spawn_blocking(move || fsops::finalize_upload(&temp, &target_path))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("finalize task failed: {err}")))??;

    state
        .db
        .delete_upload(&upload.id, current.user.id)
        .await
        .map_err(AppError::internal)?;

    if let Some(parent) = target.absolute.parent() {
        let _ = state.db.mark_index_stale(&parent.to_string_lossy()).await;
    }

    audit(
        &state,
        current.user.id,
        "upload",
        Some(&target_relative),
        &headers,
    )
    .await;

    Ok(Json(json!({
        "path": target_relative,
        "size": upload.total_size,
    })))
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    root: i64,
    paths: Vec<String>,
}

pub async fn delete_preview(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<DeleteRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    if request.paths.is_empty() || request.paths.len() > 1000 {
        return Err(AppError::bad_request("select between 1 and 1000 paths"));
    }

    let root = load_root(&state, current.user.id, request.root).await?;
    let mut items = Vec::with_capacity(request.paths.len());

    for path in request.paths {
        let resolved = authz::resolve(&state, current.user.id, root.id, &path, true).await?;
        if resolved.relative.is_empty() {
            return Err(AppError::invalid_path(
                "the root directory cannot be deleted",
            ));
        }
        if !resolved.absolute.exists() {
            return Err(AppError::NotFound);
        }
        items.push(DeleteItem {
            relative: resolved.relative,
            is_dir: resolved.absolute.is_dir(),
        });
    }

    let token = auth::random_token(24);
    let expires_at = unix_now() + 300;
    let random = auth::sha256_hex(&auth::random_token(8));
    let confirm_code = format!(
        "{:04}",
        u32::from_str_radix(&random[..8], 16).unwrap_or_default() % 10_000
    );
    let mut tokens = state.delete_tokens.lock();
    tokens.retain(|_, preview| preview.expires_at > unix_now());
    tokens.insert(
        token.clone(),
        DeletePreview {
            user_id: current.user.id,
            root_id: root.id,
            items: items.clone(),
            expires_at,
            confirm_code: confirm_code.clone(),
        },
    );
    drop(tokens);

    Ok(Json(json!({
        "token": token,
        "expires_at": expires_at,
        "items": items,
        "confirm_code": confirm_code,
    })))
}

#[derive(Debug, Deserialize)]
pub struct DeleteConfirmRequest {
    token: String,
    confirm_name: Option<String>,
    confirm_code: Option<String>,
}

pub async fn delete_confirm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<DeleteConfirmRequest>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preview = {
        let tokens = state.delete_tokens.lock();
        tokens.get(&request.token).cloned()
    }
    .ok_or(AppError::NotFound)?;

    if preview.user_id != current.user.id || preview.expires_at <= unix_now() {
        return Err(AppError::NotFound);
    }

    let directories: Vec<&DeleteItem> = preview.items.iter().filter(|item| item.is_dir).collect();
    if !directories.is_empty()
        && request.confirm_code.as_deref() != Some(preview.confirm_code.as_str())
        && (directories.len() != 1
            || request.confirm_name.as_deref()
                != Some(
                    std::path::Path::new(&directories[0].relative)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default(),
                ))
    {
        return Err(AppError::bad_request("confirmation code is required"));
    }

    {
        let mut tokens = state.delete_tokens.lock();
        tokens.remove(&request.token);
    }
    let root = load_root(&state, current.user.id, preview.root_id).await?;
    for item in &preview.items {
        let resolved =
            authz::resolve(&state, current.user.id, root.id, &item.relative, true).await?;
        let relative = resolved.relative.clone();
        let path = resolved.absolute.clone();
        tokio::task::spawn_blocking(move || fsops::remove_path(&path, &relative))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("delete task failed: {err}")))??;

        if let Some(parent) = resolved.absolute.parent() {
            let _ = state.db.mark_index_stale(&parent.to_string_lossy()).await;
        }
    }

    audit(&state, current.user.id, "delete", None, &headers).await;

    Ok(StatusCode::NO_CONTENT)
}

fn load_root<'a>(
    state: &'a Arc<AppState>,
    user_id: i64,
    root_id: i64,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Root, AppError>> + Send + 'a>> {
    Box::pin(async move {
        state
            .db
            .find_root(root_id, user_id)
            .await
            .map_err(AppError::internal)?
            .ok_or(AppError::NotFound)
    })
}

async fn load_upload(
    state: &Arc<AppState>,
    user_id: i64,
    id: &str,
) -> Result<UploadSession, AppError> {
    let upload = state
        .db
        .find_upload(id, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;

    if upload.expires_at <= unix_now() {
        let _ = state.db.delete_upload(id, user_id).await;
        return Err(AppError::NotFound);
    }

    Ok(upload)
}

fn received_chunks(upload: &UploadSession) -> Vec<i64> {
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

fn received_bytes(upload: &UploadSession) -> i64 {
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    let mut total = 0_i64;
    for index in 0..chunks {
        let byte = (index / 8) as usize;
        let bit = 1_u8 << (index % 8);
        if upload
            .chunk_bitmap
            .get(byte)
            .is_some_and(|value| value & bit != 0)
        {
            total += if index == chunks - 1 {
                upload.total_size - index * upload.chunk_size
            } else {
                upload.chunk_size
            };
        }
    }
    total
}

fn validate_filename(filename: &str) -> Result<String, AppError> {
    let filename = filename.trim();
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return Err(AppError::invalid_path("invalid filename"));
    }
    if filename.len() > 255 {
        return Err(AppError::invalid_path("filename is too long"));
    }
    #[cfg(windows)]
    {
        let stem = filename
            .split_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(filename);
        let reserved = matches!(
            stem.to_ascii_uppercase().as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        );
        if reserved
            || filename.contains(':')
            || filename
                .chars()
                .any(|value| matches!(value, '<' | '>' | '"' | '|' | '?' | '*'))
            || filename.ends_with(' ')
            || filename.ends_with('.')
        {
            return Err(AppError::invalid_path("filename is not valid on Windows"));
        }
    }
    Ok(filename.to_owned())
}

fn validate_upload_size(state: &Arc<AppState>, size: i64) -> Result<i64, AppError> {
    if size <= 0 {
        return Err(AppError::bad_request("total_size must be positive"));
    }
    let max = state.settings().settings.uploads.max_file_bytes;
    if size as u64 > max {
        return Err(AppError::too_large(
            "file exceeds the configured upload limit",
        ));
    }
    Ok(size)
}

fn root_json(root: &Root) -> Value {
    json!({
        "id": root.id,
        "alias": root.alias,
        "read_only": root.read_only,
    })
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    let (start, end) = if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 || size == 0 {
            return None;
        }
        (size.saturating_sub(suffix), size - 1)
    } else {
        let start: u64 = start.parse().ok()?;
        let end = if end.is_empty() {
            size.checked_sub(1)?
        } else {
            end.parse().ok()?
        };
        (start, end)
    };

    if start > end || start >= size {
        return None;
    }
    Some((start, end.min(size - 1)))
}

async fn audit(
    state: &Arc<AppState>,
    user_id: i64,
    action: &str,
    path: Option<&str>,
    headers: &HeaderMap,
) {
    let ip = auth::client_ip(headers);
    if let Err(err) = state
        .db
        .audit(Some(user_id), action, path, None, ip.as_deref())
        .await
    {
        tracing::warn!(error = %err, action, "failed to write audit log");
    }
}

#[derive(Debug, Deserialize)]
pub struct DirSizeQuery {
    root: i64,
    path: String,
}

pub async fn dir_size(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DirSizeQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, query.root).await?;
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let path = resolved.absolute.clone();
    let size = tokio::task::spawn_blocking(move || fsops::path_size_limited(&path, 100_000))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("dir size task failed: {err}")))?;
    Ok(Json(json!({ "size": size })))
}

#[derive(Debug, Deserialize)]
pub struct MkdirRequest {
    root: i64,
    path: String,
    name: String,
    recursive: Option<bool>,
}

pub async fn mkdir(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<MkdirRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let name = validate_filename(&request.name)?;
    let target = join_relative(&request.path, &name);
    let resolved = authz::resolve(&state, current.user.id, root.id, &target, true).await?;
    let absolute = resolved.absolute.clone();
    let recursive = request.recursive.unwrap_or(false);
    tokio::task::spawn_blocking(move || {
        if recursive {
            fsops::create_directory_all(&absolute)
        } else {
            fsops::create_directory(&absolute)
        }
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("mkdir task failed: {err}")))??;
    audit(
        &state,
        current.user.id,
        "mkdir",
        Some(&resolved.relative),
        &headers,
    )
    .await;
    Ok(Json(json!({ "path": resolved.relative })))
}

#[derive(Debug, Deserialize)]
pub struct TouchRequest {
    root: i64,
    path: String,
    name: String,
}

pub async fn touch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TouchRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let name = validate_filename(&request.name)?;
    let target = join_relative(&request.path, &name);
    let resolved = authz::resolve(&state, current.user.id, root.id, &target, true).await?;
    let absolute = resolved.absolute.clone();
    tokio::task::spawn_blocking(move || fsops::touch_file(&absolute))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("touch task failed: {err}")))??;
    audit(
        &state,
        current.user.id,
        "touch",
        Some(&resolved.relative),
        &headers,
    )
    .await;
    Ok(Json(json!({ "path": resolved.relative })))
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    root: i64,
    path: String,
    new_name: String,
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RenameRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    if request.path.is_empty() {
        return Err(AppError::invalid_path(
            "the root directory cannot be renamed",
        ));
    }
    let new_name = validate_filename(&request.new_name)?;
    let source = authz::resolve(&state, current.user.id, root.id, &request.path, true).await?;
    let (parent, _) = request.path.rsplit_once('/').unwrap_or(("", &request.path));
    let target_path = join_relative(parent, &new_name);
    let target = authz::resolve(&state, current.user.id, root.id, &target_path, true).await?;
    if target.absolute.exists() {
        return Err(AppError::conflict("target already exists"));
    }
    let source_absolute = source.absolute.clone();
    let target_absolute = target.absolute.clone();
    tokio::task::spawn_blocking(move || fsops::rename_path(&source_absolute, &target_absolute))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("rename task failed: {err}")))??;
    audit(
        &state,
        current.user.id,
        "rename",
        Some(&source.relative),
        &headers,
    )
    .await;
    Ok(Json(json!({ "path": target.relative })))
}

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    source_root: i64,
    paths: Vec<String>,
    dest_root: i64,
    dest_path: Option<String>,
}

pub async fn copy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferRequest>,
) -> Result<Json<Value>, AppError> {
    transfer(state, headers, request, false).await
}

pub async fn move_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferRequest>,
) -> Result<Json<Value>, AppError> {
    transfer(state, headers, request, true).await
}

async fn transfer(
    state: Arc<AppState>,
    headers: HeaderMap,
    request: TransferRequest,
    move_items: bool,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let source_root = load_root(&state, current.user.id, request.source_root).await?;
    let dest_root = load_root(&state, current.user.id, request.dest_root).await?;
    if request.paths.is_empty() || request.paths.len() > 1000 {
        return Err(AppError::bad_request("paths must contain 1-1000 items"));
    }
    let dest_path = request.dest_path.unwrap_or_default();
    let dest = authz::resolve(&state, current.user.id, dest_root.id, &dest_path, true).await?;
    if !dest.absolute.is_dir() {
        return Err(AppError::bad_request("destination is not a directory"));
    }

    let mut sources = Vec::new();
    for path in request.paths {
        if path.is_empty() {
            return Err(AppError::invalid_path(
                "the root directory cannot be transferred",
            ));
        }
        let source =
            authz::resolve(&state, current.user.id, source_root.id, &path, move_items).await?;
        sources.push(source);
    }

    if !move_items && current.user.quota_bytes.is_some() {
        let roots = state
            .db
            .list_roots(current.user.id)
            .await
            .map_err(AppError::internal)?;
        let mut added = 0_u64;
        for source in &sources {
            let path = source.absolute.clone();
            added = added.saturating_add(
                tokio::task::spawn_blocking(move || fsops::path_size(&path))
                    .await
                    .map_err(|err| {
                        AppError::internal(anyhow::anyhow!("size task failed: {err}"))
                    })??,
            );
        }
        let used = tokio::task::spawn_blocking(move || fsops::directory_usage(&roots))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("quota task failed: {err}")))??;
        if used.saturating_add(added) > current.user.quota_bytes.unwrap_or_default().max(0) as u64 {
            return Err(AppError::QuotaExceeded);
        }
    }

    let mut results = Vec::new();
    for source in sources {
        let name = source
            .absolute
            .file_name()
            .ok_or_else(|| AppError::invalid_path("source has no file name"))?;
        let candidate = dest.absolute.join(name);
        if move_items && source.absolute == candidate {
            continue;
        }
        let target = fsops::unique_target(&candidate);
        let source_path = source.absolute.clone();
        let target_path = target.clone();
        tokio::task::spawn_blocking(move || {
            if move_items {
                fsops::move_path(&source_path, &target_path)
            } else {
                fsops::copy_path(&source_path, &target_path)
            }
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("transfer task failed: {err}")))??;
        let _ = state
            .db
            .mark_index_stale(&source.absolute.to_string_lossy())
            .await;
        let _ = state
            .db
            .mark_index_stale(&dest.absolute.to_string_lossy())
            .await;
        results.push(json!({
            "source": source.relative,
            "target": target.strip_prefix(&dest.absolute).unwrap_or(&target).to_string_lossy(),
        }));
    }
    let action = if move_items { "move" } else { "copy" };
    audit(
        &state,
        current.user.id,
        action,
        Some(&dest.relative),
        &headers,
    )
    .await;
    Ok(Json(json!({ "items": results })))
}

fn join_relative(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    }
}
