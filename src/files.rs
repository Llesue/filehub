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
use crate::jobs::{JobContext, JobKind, JobOutcome, SubmitOutcome};
use crate::state::{AppState, DeleteItem, DeletePreview};
use crate::usage;

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
    let settings = state.settings();
    let video_enabled = settings.settings.thumbnails.enabled
        && settings.settings.thumbnails.video
        && state.ffmpeg.is_some();
    let root = load_root(&state, current.user.id, query.root).await?;
    let path = query.path.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, root.id, &path, false).await?;
    let sort = query.sort.unwrap_or_else(|| "name".to_owned());
    let order = query.order.unwrap_or_else(|| "asc".to_owned());
    let sort_by_size = sort == "size";
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

    if !sort_by_size
        && let Some((indexed_mtime, indexed_entries, has_more)) = state
            .db
            .indexed_children_page(&real_path, &sort, &order, show_hidden, offset, limit)
            .await
            .map_err(AppError::internal)?
    {
        if indexed_mtime == current_mtime {
            let mut entries: Vec<fsops::Entry> = indexed_entries
                .into_iter()
                .map(|entry| {
                    // Recompute preview support from the current settings so a
                    // stale indexed flag cannot keep requesting thumbnails
                    // that are now disabled.
                    let has_preview =
                        !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled);
                    fsops::Entry {
                        name: entry.name,
                        is_dir: entry.is_dir,
                        is_symlink: false,
                        size: entry.size,
                        modified: entry.mtime,
                        created_at: entry.created_at,
                        mime_type: entry.mime_type,
                        has_preview,
                    }
                })
                .collect();
            if entries.is_empty() && offset == 0 {
                let _ = state.db.mark_index_stale(&real_path).await;
            } else {
                if query.sizes.unwrap_or(false) {
                    let (enriched, _) =
                        enrich_directory_sizes(&state, entries, &resolved.absolute).await?;
                    entries = enriched;
                }
                return Ok(Json(json!({
                    "root": root_json(&resolved.root),
                    "path": resolved.relative,
                    "read_only": resolved.access_read_only,
                    "entries": entries,
                    "next_cursor": has_more.then(|| (offset + limit).to_string()),
                })));
            }
        } else {
            let _ = state.db.mark_index_stale(&real_path).await;
        }
    }

    let task = resolved.clone();
    let scan_sort = sort.clone();
    let scan_order = order.clone();
    let mut all_entries =
        tokio::task::spawn_blocking(move || fsops::list_dir(&task, &scan_sort, &scan_order))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("list task failed: {err}")))??;

    if sort_by_size {
        if !show_hidden {
            all_entries.retain(|entry| !entry.name.starts_with('.'));
        }
        let (mut entries, unknown) =
            enrich_directory_sizes(&state, all_entries, &resolved.absolute).await?;
        sort_entries_by_size(&mut entries, &order);
        for entry in &mut entries {
            entry.has_preview =
                !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled);
        }
        enqueue_unknown_dir_sizes(&state, current.user.id, &resolved.absolute, unknown);
        let total = entries.len();
        let entries: Vec<fsops::Entry> = entries.into_iter().skip(offset).take(limit).collect();
        if settings.settings.index.auto_index_on_browse {
            schedule_browse_index(
                &state,
                current.user.id,
                resolved.root.clone(),
                resolved.relative.clone(),
                real_path.clone(),
            )
            .await;
        }
        return Ok(Json(json!({
            "root": root_json(&resolved.root),
            "path": resolved.relative,
            "read_only": resolved.access_read_only,
            "entries": entries,
            "next_cursor": (offset.saturating_add(limit) < total)
                .then(|| (offset + limit).to_string()),
        })));
    }

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
    for entry in &mut entries {
        entry.has_preview = !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled);
    }
    if query.sizes.unwrap_or(false) {
        let (enriched, _) = enrich_directory_sizes(&state, entries, &resolved.absolute).await?;
        entries = enriched;
    }
    if settings.settings.index.auto_index_on_browse {
        schedule_browse_index(
            &state,
            current.user.id,
            resolved.root.clone(),
            resolved.relative.clone(),
            real_path.clone(),
        )
        .await;
    }

    Ok(Json(json!({
        "root": root_json(&resolved.root),
        "path": resolved.relative,
        "read_only": resolved.access_read_only,
        "entries": entries,
        "next_cursor": has_more.then(|| (offset + limit).to_string()),
    })))
}

async fn schedule_browse_index(
    state: &Arc<AppState>,
    user_id: i64,
    root: Root,
    relative: String,
    real_path: String,
) {
    let key = format!("dir:{real_path}");
    if state.jobs.active_job_exact(JobKind::Index, &key).is_some() {
        return;
    }
    if state
        .jobs
        .active_job_for_path(JobKind::Index, &real_path)
        .is_some()
    {
        return;
    }
    let payload = json!({
        "root_id": root.id,
        "path": relative,
        "recursive": false,
        "auto": true,
    });
    let job_state = Arc::clone(state);
    let result = state
        .jobs
        .submit(
            JobKind::Index,
            key,
            user_id,
            payload,
            1,
            move |ctx| async move {
                let cancel = ctx.token();
                let mut progress = |directories: usize, _entries: usize| {
                    ctx.report(directories, 1, 0, 0);
                };
                let stats = crate::indexer::index_path(
                    &job_state,
                    user_id,
                    root,
                    &relative,
                    false,
                    &cancel,
                    &mut progress,
                )
                .await?;
                Ok(JobOutcome::done(json!(stats), 1, 1))
            },
        )
        .await;
    match result {
        Ok(_) => {}
        Err(AppError::QueueFull) => {}
        Err(error) => tracing::debug!(%error, path = %real_path, "failed to queue browse index"),
    }
}

async fn enrich_directory_sizes(
    state: &Arc<AppState>,
    mut entries: Vec<fsops::Entry>,
    base: &std::path::Path,
) -> Result<(Vec<fsops::Entry>, Vec<String>), AppError> {
    let dirs: Vec<(usize, String)> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.is_dir)
        .map(|(index, entry)| (index, base.join(&entry.name).to_string_lossy().to_string()))
        .collect();
    if dirs.is_empty() {
        return Ok((entries, Vec::new()));
    }
    let paths: Vec<String> = dirs.iter().map(|(_, path)| path.clone()).collect();
    let indexed = state
        .db
        .indexed_directory_states(paths.clone())
        .await
        .map_err(AppError::internal)?;
    let cached = state
        .db
        .directory_size_states(paths)
        .await
        .map_err(AppError::internal)?;
    let mut unknown = Vec::new();
    for (index, path) in dirs {
        if let Some((_mtime, size, partial, status)) = indexed.get(&path)
            && status == "ready"
            && !partial
        {
            entries[index].size = (*size).min(i64::MAX as u64) as i64;
            continue;
        }
        if let Some((status, size, _error)) = cached.get(&path)
            && status == "ready"
        {
            entries[index].size = (*size).max(0);
            continue;
        }
        unknown.push(path);
    }
    Ok((entries, unknown))
}

fn sort_entries_by_size(entries: &mut [fsops::Entry], order: &str) {
    let descending = order.eq_ignore_ascii_case("desc");
    entries.sort_by(|left, right| {
        let dirs_first = right.is_dir.cmp(&left.is_dir);
        let known = |entry: &fsops::Entry| !entry.is_dir || entry.size > 0;
        let known_first = known(right).cmp(&known(left));
        let size = match (descending, left.size.cmp(&right.size)) {
            (true, std::cmp::Ordering::Less) => std::cmp::Ordering::Greater,
            (true, std::cmp::Ordering::Greater) => std::cmp::Ordering::Less,
            (_, ordering) => ordering,
        };
        dirs_first
            .then(known_first)
            .then(size)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

fn enqueue_unknown_dir_sizes(
    state: &Arc<AppState>,
    user_id: i64,
    base: &std::path::Path,
    unknown: Vec<String>,
) {
    if unknown.is_empty() {
        return;
    }
    let state = Arc::clone(state);
    let base = base.to_path_buf();
    tokio::spawn(async move {
        for path in unknown.into_iter().take(50) {
            let absolute = std::path::PathBuf::from(&path);
            if !crate::platform::path_within(&absolute, &base) {
                continue;
            }
            let stat_path = absolute.clone();
            let Ok(Ok(mtime)) =
                tokio::task::spawn_blocking(move || fsops::dir_mtime_ns(&stat_path)).await
            else {
                continue;
            };
            let _ = enqueue_dir_size(&state, user_id, path, absolute, mtime).await;
        }
    });
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
    serve_file(&headers, &resolved.absolute, query.inline.unwrap_or(false)).await
}

/// Shared file response used by authenticated downloads and public shares.
pub(crate) async fn serve_file(
    headers: &HeaderMap,
    absolute: &std::path::Path,
    inline: bool,
) -> Result<Response, AppError> {
    let info_path = absolute.to_path_buf();
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

    let file = tokio::fs::File::open(absolute)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    let filename = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let mime = mime_guess::from_path(absolute).first_or_octet_stream();
    let is_pdf = filename.to_ascii_lowercase().ends_with(".pdf");
    let safe_inline = is_pdf
        || ((matches!(mime.type_().as_str(), "image" | "video" | "audio"))
            && mime.subtype().as_str() != "svg+xml");
    let disposition = if inline && safe_inline {
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
    zip_response(&state, entries, "filehub.zip")
}

fn zip_options(name: &str) -> SimpleFileOptions {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let method = if matches!(
        extension.as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "avif"
            | "heic"
            | "mp4"
            | "mov"
            | "mkv"
            | "webm"
            | "avi"
            | "m4v"
            | "mp3"
            | "aac"
            | "ogg"
            | "flac"
            | "zip"
            | "7z"
            | "gz"
            | "xz"
            | "bz2"
            | "rar"
            | "zst"
    ) {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };
    SimpleFileOptions::default()
        .compression_method(method)
        .large_file(true)
}

/// Stream a ZIP archive with bounded memory and the configured concurrency
/// limit. The permit lives as long as the blocking writer task, so a client
/// disconnect releases it when the duplex writer fails.
pub(crate) fn zip_response(
    state: &Arc<AppState>,
    entries: Vec<fsops::ZipEntry>,
    filename: &str,
) -> Result<Response, AppError> {
    let permit = state
        .zip_semaphore()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests)?;
    let count = entries.len();
    let (reader, writer) = tokio::io::duplex(64 * 1024);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let started = std::time::Instant::now();
        let sync_writer = SyncIoBridge::new(writer);
        let mut zip = ZipWriter::new_stream(sync_writer);
        for entry in entries {
            let Ok(mut file) = std::fs::File::open(&entry.path) else {
                continue;
            };
            if zip
                .start_file(&entry.name, zip_options(&entry.name))
                .is_err()
            {
                return;
            }
            if std::io::copy(&mut file, &mut zip).is_err() {
                return;
            }
        }
        let _ = zip.finish();
        tracing::debug!(
            files = count,
            duration_ms = started.elapsed().as_millis() as u64,
            "zip stream finished"
        );
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename*=UTF-8''{}",
                percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC)
            ),
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
    let concurrency = state
        .settings()
        .settings
        .uploads
        .concurrency_per_user
        .max(1);
    let active = state
        .db
        .count_active_uploads(current.user.id, unix_now())
        .await
        .map_err(AppError::internal)?;
    if active >= concurrency {
        return Err(AppError::TooManyRequests);
    }

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

pub async fn cancel_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    if let Some(upload) = state
        .db
        .find_upload(&id, current.user.id)
        .await
        .map_err(AppError::internal)?
    {
        fsops::delete_temp(std::path::Path::new(&upload.temp_path));
        state
            .db
            .delete_upload(&id, current.user.id)
            .await
            .map_err(AppError::internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
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

    let finalize_target = target_path.clone();
    tokio::task::spawn_blocking(move || fsops::finalize_upload(&temp, &finalize_target))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("finalize task failed: {err}")))??;

    state
        .db
        .delete_upload(&upload.id, current.user.id)
        .await
        .map_err(AppError::internal)?;

    state
        .invalidate_path_caches(&target_path, Some(current.user.id), false)
        .await;

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
        state
            .invalidate_path_caches(&resolved.absolute, Some(current.user.id), true)
            .await;
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
        fsops::delete_temp(std::path::Path::new(&upload.temp_path));
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

pub(crate) fn validate_filename(filename: &str) -> Result<String, AppError> {
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

pub(crate) fn validate_upload_size(state: &Arc<AppState>, size: i64) -> Result<i64, AppError> {
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
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: action.to_owned(),
            path: path.map(str::to_owned),
            detail: None,
            ip: auth::client_ip(headers),
        })
        .await;
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
    let real_path = resolved.absolute.to_string_lossy().to_string();
    let path = resolved.absolute.clone();
    let mtime = tokio::task::spawn_blocking({
        let path = path.clone();
        move || fsops::dir_mtime_ns(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("dir size metadata task failed: {err}")))??;
    let settings = state.settings();
    let ttl = settings.settings.jobs.dir_size_ttl_seconds as i64;
    let retry = settings.settings.jobs.dir_size_error_retry_seconds as i64;
    let now = unix_now();

    if let Some((indexed_mtime, size, partial, status)) = state
        .db
        .indexed_directory_state(&real_path)
        .await
        .map_err(AppError::internal)?
    {
        if status == "ready" && !partial && indexed_mtime == mtime {
            return Ok(Json(json!({
                "status": "ready",
                "size": size,
                "indexed": true,
                "partial": false,
                "stale": false,
                "error": null,
            })));
        }
        if status == "ready" || status == "error" {
            let _ = state.db.mark_index_stale(&real_path).await;
        }
    }

    if let Some((status, size, cached_mtime, error, updated_at)) = state
        .db
        .directory_size_state(&real_path)
        .await
        .map_err(AppError::internal)?
    {
        if status == "ready" && cached_mtime == mtime {
            let stale = now.saturating_sub(updated_at) > ttl;
            if stale {
                let _ = enqueue_dir_size(
                    &state,
                    current.user.id,
                    real_path.clone(),
                    path.clone(),
                    mtime,
                )
                .await;
            }
            return Ok(Json(json!({
                "status": "ready",
                "size": size.max(0) as u64,
                "indexed": false,
                "partial": error.is_some(),
                "stale": stale,
                "error": error,
            })));
        }
        if status == "error" && now.saturating_sub(updated_at) <= retry {
            return Ok(Json(json!({
                "status": "error",
                "size": null,
                "indexed": false,
                "partial": false,
                "stale": false,
                "error": error,
            })));
        }
    }

    let outcome = enqueue_dir_size(&state, current.user.id, real_path, path, mtime).await?;
    let status = if outcome.job.status == "running" {
        "running"
    } else {
        "queued"
    };
    Ok(Json(json!({
        "status": status,
        "size": null,
        "indexed": false,
        "partial": false,
        "stale": false,
        "error": null,
        "job_id": (outcome.job.user_id == current.user.id).then_some(outcome.job.id),
    })))
}

async fn enqueue_dir_size(
    state: &Arc<AppState>,
    user_id: i64,
    real_path: String,
    absolute: std::path::PathBuf,
    mtime_ns: i64,
) -> Result<SubmitOutcome, AppError> {
    let payload = json!({
        "real_path": real_path,
        "mtime_ns": mtime_ns,
    });
    let key = real_path.clone();
    if let Some(job) = state.jobs.active_job_for_path(JobKind::DirSize, &key) {
        return Ok(SubmitOutcome {
            job,
            created: false,
        });
    }
    state.jobs.cancel_descendants(JobKind::DirSize, &key).await;
    let job_state = Arc::clone(state);
    let job_path = absolute;
    let job_real_path = real_path;
    let settings = state.settings();
    let ignore = settings.settings.index.ignore.clone();
    let use_logical = settings.settings.jobs.dir_size_use_logical;
    state
        .jobs
        .submit(
            JobKind::DirSize,
            key,
            user_id,
            payload,
            1,
            move |ctx| async move {
                let _ = job_state
                    .db
                    .set_directory_size(&job_real_path, "running", 0, mtime_ns, None)
                    .await;
                let cancel = ctx.token();
                let (sender, mut receiver) =
                    tokio::sync::mpsc::channel::<(String, i64, i64, Option<String>)>(1024);
                let scan_cancel = cancel.clone();
                let scan_path = job_path;
                let scan = tokio::task::spawn_blocking(move || {
                    fsops::dir_size_scan(
                        &scan_path,
                        &ignore,
                        &scan_cancel,
                        use_logical,
                        &mut |path, total, mtime, partial| {
                            let error = partial
                                .then(|| "some entries could not be read".to_owned());
                            let _ = sender.blocking_send((
                                path.to_string_lossy().to_string(),
                                total.min(i64::MAX as u64) as i64,
                                mtime,
                                error,
                            ));
                        },
                    )
                });
                let mut batch: Vec<(String, i64, i64, Option<String>)> = Vec::with_capacity(256);
                let mut directories = 0_usize;
                while let Some(row) = receiver.recv().await {
                    batch.push(row);
                    directories += 1;
                    if batch.len() >= 256 {
                        let rows = std::mem::take(&mut batch);
                        if let Err(error) = job_state.db.set_directory_sizes_batch(rows).await {
                            tracing::warn!(%error, path = %job_real_path, "failed to persist nested directory sizes");
                        }
                        ctx.report(directories, 0, 0, 0);
                    }
                }
                if !batch.is_empty()
                    && let Err(error) = job_state.db.set_directory_sizes_batch(batch).await
                {
                    tracing::warn!(%error, path = %job_real_path, "failed to persist nested directory sizes");
                }
                ctx.report(directories, 0, 0, 0);
                let result = scan.await.map_err(|err| {
                    AppError::internal(anyhow::anyhow!("dir size task failed: {err}"))
                })?;
                match result {
                    Ok(scan) => {
                        let _ = job_state
                            .db
                            .set_directory_size(
                                &job_real_path,
                                "ready",
                                scan.total.min(i64::MAX as u64) as i64,
                                mtime_ns,
                                scan.error.as_deref(),
                            )
                            .await;
                        Ok(JobOutcome::done(
                            json!({
                                "size": scan.total,
                                "partial": scan.partial,
                                "error": scan.error,
                            }),
                            directories,
                            directories,
                        ))
                    }
                    Err(AppError::Cancelled) => {
                        let _ = job_state
                            .db
                            .set_directory_size(
                                &job_real_path,
                                "stale",
                                0,
                                mtime_ns,
                                None,
                            )
                            .await;
                        Err(AppError::Cancelled)
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = job_state
                            .db
                            .set_directory_size(
                                &job_real_path,
                                "error",
                                0,
                                mtime_ns,
                                Some(&message),
                            )
                            .await;
                        Err(error)
                    }
                }
            },
        )
        .await
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
    state
        .invalidate_path_caches(&resolved.absolute, Some(current.user.id), false)
        .await;
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
    state
        .invalidate_path_caches(&resolved.absolute, Some(current.user.id), false)
        .await;
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
    state
        .invalidate_path_caches(&source.absolute, Some(current.user.id), true)
        .await;
    state
        .invalidate_path_caches(&target.absolute, Some(current.user.id), true)
        .await;
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TransferJobPayload {
    source_root: i64,
    paths: Vec<String>,
    dest_root: i64,
    dest_path: Option<String>,
    move_items: bool,
}

pub async fn copy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferRequest>,
) -> Result<Json<Value>, AppError> {
    submit_transfer(
        state,
        headers,
        TransferJobPayload {
            source_root: request.source_root,
            paths: request.paths,
            dest_root: request.dest_root,
            dest_path: request.dest_path,
            move_items: false,
        },
    )
    .await
}

pub async fn move_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferRequest>,
) -> Result<Json<Value>, AppError> {
    submit_transfer(
        state,
        headers,
        TransferJobPayload {
            source_root: request.source_root,
            paths: request.paths,
            dest_root: request.dest_root,
            dest_path: request.dest_path,
            move_items: true,
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct TransferJobRequest {
    source_root: i64,
    paths: Vec<String>,
    dest_root: i64,
    dest_path: Option<String>,
    move_items: Option<bool>,
}

pub async fn transfer_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TransferJobRequest>,
) -> Result<Json<Value>, AppError> {
    submit_transfer(
        state,
        headers,
        TransferJobPayload {
            source_root: request.source_root,
            paths: request.paths,
            dest_root: request.dest_root,
            dest_path: request.dest_path,
            move_items: request.move_items.unwrap_or(false),
        },
    )
    .await
}

async fn submit_transfer(
    state: Arc<AppState>,
    headers: HeaderMap,
    payload: TransferJobPayload,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    if payload.paths.is_empty() || payload.paths.len() > 1000 {
        return Err(AppError::bad_request("paths must contain 1-1000 items"));
    }
    let kind = if payload.move_items {
        JobKind::Move
    } else {
        JobKind::Copy
    };
    let key = auth::sha256_hex(&format!(
        "{}:{}:{}:{}:{}",
        payload.source_root,
        payload.dest_root,
        payload.dest_path.as_deref().unwrap_or_default(),
        payload.move_items,
        payload.paths.join("\u{1f}")
    ));
    let payload_value = serde_json::to_value(&payload)
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    let user_id = current.user.id;
    let total = payload.paths.len();
    let job_state = Arc::clone(&state);
    let job_payload = payload;
    let outcome =
        state
            .jobs
            .submit(
                kind,
                key,
                user_id,
                payload_value,
                total,
                move |ctx| async move {
                    execute_transfer_job(job_state, user_id, job_payload, ctx).await
                },
            )
            .await?;
    let status = if outcome.job.status == "running" {
        "running"
    } else {
        "queued"
    };
    Ok(Json(json!({
        "job_id": outcome.job.id,
        "status": status,
        "created": outcome.created,
    })))
}

async fn execute_transfer_job(
    state: Arc<AppState>,
    user_id: i64,
    payload: TransferJobPayload,
    ctx: JobContext,
) -> Result<JobOutcome, AppError> {
    let user = state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let source_root = load_root(&state, user_id, payload.source_root).await?;
    let dest_root = load_root(&state, user_id, payload.dest_root).await?;
    let dest_path = payload.dest_path.clone().unwrap_or_default();
    let dest = authz::resolve(&state, user_id, dest_root.id, &dest_path, true).await?;
    if !dest.absolute.is_dir() {
        return Err(AppError::bad_request("destination is not a directory"));
    }

    let mut sources = Vec::new();
    for path in &payload.paths {
        if path.is_empty() {
            return Err(AppError::invalid_path(
                "the root directory cannot be transferred",
            ));
        }
        sources
            .push(authz::resolve(&state, user_id, source_root.id, path, payload.move_items).await?);
    }

    let total_items = sources.len();
    let cancel = ctx.token();
    let mut source_sizes = Vec::with_capacity(total_items);
    let mut total_bytes = 0_u64;
    for source in &sources {
        let path = source.absolute.clone();
        let scan_cancel = cancel.clone();
        let size = tokio::task::spawn_blocking(move || {
            fsops::path_size_cancellable(&path, &scan_cancel, &mut |_| {})
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("size task failed: {err}")))??;
        source_sizes.push(size);
        total_bytes = total_bytes.saturating_add(size);
        ctx.check_cancelled()?;
    }

    if !payload.move_items && user.quota_bytes.is_some() {
        let used = usage::cached_or_scan(&state, &user).await?;
        if used.saturating_add(total_bytes) > user.quota_bytes.unwrap_or_default().max(0) as u64 {
            return Err(AppError::QuotaExceeded);
        }
    }

    let mut results = Vec::with_capacity(total_items);
    let mut failed = 0_usize;
    let mut bytes_done_total = 0_u64;
    for (index, source) in sources.iter().enumerate() {
        ctx.check_cancelled()?;
        let source_again = authz::resolve(
            &state,
            user_id,
            source_root.id,
            &source.relative,
            payload.move_items,
        )
        .await?;
        let dest_again =
            authz::resolve(&state, user_id, dest_root.id, &dest.relative, true).await?;
        if source_again.absolute != source.absolute || dest_again.absolute != dest.absolute {
            failed += 1;
            results.push(json!({
                "source": source.relative,
                "status": "failed",
                "error": "path changed during transfer",
            }));
            continue;
        }
        let Some(name) = source.absolute.file_name() else {
            failed += 1;
            results.push(json!({
                "source": source.relative,
                "status": "failed",
                "error": "source has no file name",
            }));
            continue;
        };
        let candidate = dest.absolute.join(name);
        if payload.move_items && source.absolute == candidate {
            results.push(json!({
                "source": source.relative,
                "target": source.relative,
                "status": "skipped",
            }));
            continue;
        }
        let target = fsops::unique_target(&candidate);
        let source_path = source.absolute.clone();
        let target_path = target.clone();
        let expected = source_sizes.get(index).copied().unwrap_or(0);
        let job_cancel = ctx.token();
        let base_done = bytes_done_total;
        let move_items = payload.move_items;
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<u64>(32);
        let progress_ctx = ctx.clone();
        let progress_task = tokio::spawn(async move {
            while let Some(done) = receiver.recv().await {
                progress_ctx.report(
                    index,
                    total_items,
                    base_done.saturating_add(done),
                    total_bytes,
                );
            }
        });
        let result = tokio::task::spawn_blocking(move || {
            let mut last = 0_u64;
            let mut progress = |done: u64, total: u64, _files: u64| {
                if done.saturating_sub(last) >= 32 * 1024 * 1024 || done == total {
                    last = done;
                    let _ = sender.blocking_send(done);
                }
            };
            if move_items {
                fsops::move_path_with_progress(
                    &source_path,
                    &target_path,
                    &job_cancel,
                    expected,
                    &mut progress,
                )
            } else {
                fsops::copy_path_with_progress(
                    &source_path,
                    &target_path,
                    &job_cancel,
                    expected,
                    &mut progress,
                )
            }
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("transfer task failed: {err}")))?;
        let _ = progress_task.await;

        match result {
            Ok(()) => {
                bytes_done_total = bytes_done_total.saturating_add(expected);
                ctx.report(index + 1, total_items, bytes_done_total, total_bytes);
                let target_relative = target
                    .strip_prefix(&dest.absolute)
                    .unwrap_or(&target)
                    .to_string_lossy()
                    .to_string();
                state
                    .invalidate_path_caches(&target, Some(user_id), true)
                    .await;
                if payload.move_items {
                    state
                        .invalidate_path_caches(&source.absolute, Some(user_id), true)
                        .await;
                }
                results.push(json!({
                    "source": source.relative,
                    "target": target_relative,
                    "status": "completed",
                    "size": expected,
                }));
            }
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    path = %source.relative,
                    %error,
                    "background transfer item failed"
                );
                results.push(json!({
                    "source": source.relative,
                    "status": "failed",
                    "error": error.to_string(),
                }));
            }
        }
    }

    if failed == total_items && total_items > 0 {
        return Err(AppError::bad_request(format!(
            "all {failed} transfer items failed"
        )));
    }
    let action = if payload.move_items { "move" } else { "copy" };
    if let Err(error) = state
        .db
        .audit(Some(user_id), action, Some(&dest.relative), None, None)
        .await
    {
        tracing::warn!(%error, action, "failed to write transfer audit log");
    }
    Ok(JobOutcome {
        result: json!({
            "items": results,
            "failed": failed,
            "partial": failed > 0,
        }),
        progress: total_items,
        total: total_items,
        bytes_done: bytes_done_total,
        bytes_total: total_bytes,
    })
}

async fn can_view_job(
    state: &Arc<AppState>,
    user: &crate::db::User,
    job: &crate::jobs::Job,
) -> Result<bool, AppError> {
    if job.user_id == user.id || user.role == "admin" {
        return Ok(true);
    }
    let roots = state
        .db
        .list_roots(user.id)
        .await
        .map_err(AppError::internal)?;
    Ok(roots.iter().any(|root| {
        crate::platform::path_within(
            std::path::Path::new(job.key()),
            std::path::Path::new(&root.real_path),
        )
    }))
}

pub async fn job_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let job = state.jobs.get(&id).await?.ok_or(AppError::NotFound)?;
    if !can_view_job(&state, &current.user, &job).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::to_value(job).map_err(|err| {
        AppError::internal(anyhow::Error::from(err))
    })?))
}

pub async fn job_cancel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let job = state.jobs.get(&id).await?.ok_or(AppError::NotFound)?;
    let is_admin = current.user.role == "admin";
    let owner = job.user_id == current.user.id;
    let shared = matches!(job.kind.as_str(), "dir_size" | "index")
        && can_view_job(&state, &current.user, &job).await?;
    if !owner && !is_admin && !shared {
        return Err(AppError::NotFound);
    }
    let cancelled = if owner || is_admin {
        state.jobs.cancel(&id, current.user.id, is_admin).await?
    } else {
        // A shared directory/index job belongs to the queue, not to the user
        // who happened to enqueue it; any user with access to the path may
        // cancel it. Passing the owner id satisfies the ownership check.
        state.jobs.cancel(&id, job.user_id, false).await?
    };
    Ok(Json(serde_json::to_value(cancelled).map_err(|err| {
        AppError::internal(anyhow::Error::from(err))
    })?))
}

fn join_relative(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    }
}
