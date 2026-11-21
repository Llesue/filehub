use std::io::{Seek, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

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
use crate::state::{AppState, DeleteItem, DeletePreview, UsageInvalidation};
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
    let grants = state
        .db
        .federation_list_grants(current.user.id)
        .await
        .unwrap_or_default();
    let shared: Vec<std::path::PathBuf> = grants
        .iter()
        .filter(|grant| !grant.revoked)
        .map(|grant| std::path::PathBuf::from(&grant.real_path))
        .collect();
    let values: Vec<Value> = roots
        .iter()
        .map(|root| {
            let mut value = root_json(root);
            if !root.is_remote()
                && shared
                    .iter()
                    .any(|path| path == std::path::Path::new(&root.real_path))
            {
                value["shared"] = json!(true);
            }
            value
        })
        .collect();

    Ok(Json(json!({ "roots": values })))
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
    focus: Option<String>,
}

async fn mark_shared_entries(
    state: &Arc<AppState>,
    user_id: i64,
    entries: Vec<fsops::Entry>,
    directory: &std::path::Path,
) -> Vec<Value> {
    let grants = state
        .db
        .federation_list_grants(user_id)
        .await
        .unwrap_or_default();
    let shared: Vec<std::path::PathBuf> = grants
        .iter()
        .filter(|grant| !grant.revoked)
        .map(|grant| std::path::PathBuf::from(&grant.real_path))
        .collect();
    entries
        .into_iter()
        .map(|entry| {
            let mut value = serde_json::to_value(&entry).unwrap_or_else(|_| json!({}));
            if !shared.is_empty()
                && shared
                    .iter()
                    .any(|path| path == &directory.join(&entry.name))
            {
                value["shared"] = json!(true);
            }
            value
        })
        .collect()
}

fn map_indexed_entry(entry: crate::db::IndexEntry, video_enabled: bool) -> fsops::Entry {
    // Recompute preview support from the current settings so a stale indexed
    // flag cannot keep requesting thumbnails that are now disabled.
    let has_preview = !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled);
    fsops::Entry {
        name: entry.name,
        is_dir: entry.is_dir,
        is_symlink: false,
        size: entry.size,
        modified: entry.mtime,
        created_at: entry.created_at,
        mime_type: entry.mime_type,
        has_preview,
        symlink_state: None,
        symlink_target: None,
    }
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
    if root.is_remote() {
        return crate::federation::proxy::list(
            &state,
            current.user.id,
            &root,
            query.path.as_deref(),
            query.cursor.as_deref(),
            query.limit,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
        .await;
    }
    let path = query.path.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, root.id, &path, false).await?;
    let policy_read_only = resolved.access_read_only;
    let probe_path = resolved.absolute.clone();
    let filesystem_read_only = if policy_read_only {
        false
    } else {
        tokio::task::spawn_blocking(move || fsops::directory_writable(&probe_path))
            .await
            .ok()
            .flatten()
            == Some(false)
    };
    let read_only_reason = if policy_read_only {
        Some("policy")
    } else if filesystem_read_only {
        Some("filesystem")
    } else {
        None
    };
    let sort = query.sort.unwrap_or_else(|| "name".to_owned());
    let order = query.order.unwrap_or_else(|| "asc".to_owned());
    let sort_by_size = sort == "size";
    let real_path = resolved.absolute.to_string_lossy().to_string();
    let probe_path = resolved.absolute.clone();
    let (current_mtime, has_symlinks) = tokio::task::spawn_blocking(move || {
        (
            fsops::dir_mtime_ns(&probe_path).unwrap_or_default(),
            fsops::has_symlinks(&probe_path),
        )
    })
    .await
    .unwrap_or((0, true));
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let focus = query
        .focus
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if focus.is_some()
        && query
            .cursor
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    {
        return Err(AppError::bad_request("cursor and focus cannot be combined"));
    }
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

    if !sort_by_size && !has_symlinks {
        if let Some(focus_name) = focus.as_deref() {
            if let Some((indexed_mtime, focus_offset, total)) = state
                .db
                .locate_indexed_child(&real_path, focus_name, &sort, &order, show_hidden)
                .await
                .map_err(AppError::internal)?
                && indexed_mtime == current_mtime
            {
                let page_start = focus_offset
                    .saturating_sub(limit / 2)
                    .min(total.saturating_sub(limit));
                if let Some((page_mtime, indexed_entries, has_more)) = state
                    .db
                    .indexed_children_page(
                        &real_path,
                        &sort,
                        &order,
                        show_hidden,
                        page_start,
                        limit,
                    )
                    .await
                    .map_err(AppError::internal)?
                    && page_mtime == current_mtime
                {
                    let mut entries: Vec<fsops::Entry> = indexed_entries
                        .into_iter()
                        .map(|entry| map_indexed_entry(entry, video_enabled))
                        .collect();
                    if query.sizes.unwrap_or(false) {
                        let (enriched, _) =
                            enrich_directory_sizes(&state, entries, &resolved.absolute).await?;
                        entries = enriched;
                    }
                    let focus_index = focus_offset.saturating_sub(page_start);
                    return Ok(Json(json!({
                        "root": root_json(&resolved.root),
                        "path": resolved.relative,
                        "read_only": read_only_reason.is_some(),
                        "read_only_reason": read_only_reason,
                        "entries": mark_shared_entries(
                            &state,
                            current.user.id,
                            entries,
                            &resolved.absolute,
                        )
                        .await,
                        "next_cursor": has_more.then(|| (page_start + limit).to_string()),
                        "focus_index": focus_index,
                        "focus_offset": focus_offset,
                        "total": total,
                    })));
                }
            }
            // The index is stale or does not know this name; the filesystem
            // scan below is authoritative.
        } else if let Some((indexed_mtime, indexed_entries, has_more)) = state
            .db
            .indexed_children_page(&real_path, &sort, &order, show_hidden, offset, limit)
            .await
            .map_err(AppError::internal)?
        {
            if indexed_mtime == current_mtime {
                let mut entries: Vec<fsops::Entry> = indexed_entries
                    .into_iter()
                    .map(|entry| map_indexed_entry(entry, video_enabled))
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
                        "read_only": read_only_reason.is_some(),
                        "read_only_reason": read_only_reason,
                        "entries": mark_shared_entries(
                            &state,
                            current.user.id,
                            entries,
                            &resolved.absolute,
                        )
                        .await,
                        "next_cursor": has_more.then(|| (offset + limit).to_string()),
                    })));
                }
            } else {
                let _ = state.db.mark_index_stale(&real_path).await;
            }
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
        let (page_start, focus_index) = if let Some(focus_name) = focus.as_deref() {
            let index = entries
                .iter()
                .position(|entry| entry.name == focus_name)
                .ok_or(AppError::NotFound)?;
            let start = index
                .saturating_sub(limit / 2)
                .min(total.saturating_sub(limit));
            (start, Some(index - start))
        } else {
            (offset, None)
        };
        let entries: Vec<fsops::Entry> = entries.into_iter().skip(page_start).take(limit).collect();
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
        let mut payload = json!({
            "root": root_json(&resolved.root),
            "path": resolved.relative,
            "read_only": read_only_reason.is_some(),
            "read_only_reason": read_only_reason,
            "entries": mark_shared_entries(
                &state,
                current.user.id,
                entries,
                &resolved.absolute,
            )
            .await,
            "next_cursor": (page_start.saturating_add(limit) < total)
                .then(|| (page_start + limit).to_string()),
        });
        if let Some(focus_index) = focus_index {
            payload["focus_index"] = json!(focus_index);
            payload["focus_offset"] = json!(page_start + focus_index);
            payload["total"] = json!(total);
        }
        return Ok(Json(payload));
    }

    if let Some(focus_name) = focus.as_deref() {
        if !show_hidden {
            all_entries.retain(|entry| !entry.name.starts_with('.'));
        }
        let total = all_entries.len();
        let focus_offset = all_entries
            .iter()
            .position(|entry| entry.name == focus_name)
            .ok_or(AppError::NotFound)?;
        let page_start = focus_offset
            .saturating_sub(limit / 2)
            .min(total.saturating_sub(limit));
        let mut entries: Vec<fsops::Entry> = all_entries
            .into_iter()
            .skip(page_start)
            .take(limit)
            .collect();
        for entry in &mut entries {
            entry.has_preview =
                !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled);
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
        let focus_index = focus_offset - page_start;
        return Ok(Json(json!({
            "root": root_json(&resolved.root),
            "path": resolved.relative,
            "read_only": read_only_reason.is_some(),
            "read_only_reason": read_only_reason,
            "entries": mark_shared_entries(
                &state,
                current.user.id,
                entries,
                &resolved.absolute,
            )
            .await,
            "next_cursor": (page_start.saturating_add(limit) < total)
                .then(|| (page_start + limit).to_string()),
            "focus_index": focus_index,
            "focus_offset": focus_offset,
            "total": total,
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
        "read_only": read_only_reason.is_some(),
        "read_only_reason": read_only_reason,
        "entries": mark_shared_entries(
                &state,
                current.user.id,
                entries,
                &resolved.absolute,
            )
            .await,
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
        "real_path": real_path,
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
            false,
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
            && matches!(status.as_str(), "ready" | "stale")
        {
            // Keep showing the last known size while a refresh runs. The
            // per-directory /dir-size request reports the stale flag.
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
        let name = left.name.to_lowercase().cmp(&right.name.to_lowercase());
        let name = if descending { name.reverse() } else { name };
        dirs_first.then(known_first).then(size).then(name)
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
    if root.is_remote() {
        return crate::federation::proxy::download(
            &state,
            current.user.id,
            &root,
            &query.path,
            query.inline.unwrap_or(false),
            &headers,
        )
        .await;
    }
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

    let mut file = tokio::fs::File::open(absolute)
        .await
        .map_err(fsops::io_error)?;
    let filename = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let mime = mime_guess::from_path(absolute).first_or_octet_stream();
    let is_pdf = filename.to_ascii_lowercase().ends_with(".pdf");
    // Some recorders (notably WeChat) prepend a tiny primer ADTS frame that
    // Chrome treats as a fatal decode error. Inline playback skips it while
    // downloads keep the original bytes untouched.
    let aac_skip = if inline && filename.to_ascii_lowercase().ends_with(".aac") {
        let mut head = [0_u8; 15];
        let read = file
            .read(&mut head)
            .await
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        aac_primer_skip(&head[..read]).unwrap_or(0)
    } else {
        0
    };
    let effective_size = info.size.saturating_sub(aac_skip);
    // `image/svg+xml` reports its subtype as "svg", not "svg+xml". SVG must
    // never be served inline: opening the URL directly would render the
    // document and any embedded script in this origin.
    let is_svg = mime.type_() == "image" && mime.subtype() == "svg";
    let safe_inline =
        is_pdf || (!is_svg && matches!(mime.type_().as_str(), "image" | "video" | "audio"));
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
        .and_then(|value| parse_range(value, effective_size));

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
        file.seek(SeekFrom::Start(aac_skip + start))
            .await
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        let reader = file.take(length);
        builder = builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{effective_size}"),
            )
            .header(header::CONTENT_LENGTH, length.to_string());
        builder
            .body(Body::from_stream(ReaderStream::new(reader)))
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?
    } else if headers.get(header::RANGE).is_some() {
        return Ok((
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{effective_size}"))],
        )
            .into_response());
    } else {
        file.seek(SeekFrom::Start(aac_skip))
            .await
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        builder = builder.header(header::CONTENT_LENGTH, effective_size.to_string());
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
    crate::capabilities::require_zip(&state, &current.user).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    if request.paths.is_empty() || request.paths.len() > 1000 {
        return Err(AppError::bad_request("select between 1 and 1000 paths"));
    }
    if root.is_remote() {
        return crate::federation::proxy::zip_files(&state, current.user.id, &root, request.paths)
            .await;
    }

    let selected =
        authz::resolve_many(&state, current.user.id, root.id, request.paths, false).await?;

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
        let _ = write_zip_entries(&mut zip, &entries, None);
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

fn write_zip_entries<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    entries: &[fsops::ZipEntry],
    mut progress: Option<&mut dyn FnMut(usize, u64)>,
) -> Result<(), AppError> {
    let mut done = 0_usize;
    let mut bytes = 0_u64;
    for entry in entries {
        let mut file = match std::fs::File::open(&entry.path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        zip.start_file(&entry.name, zip_options(&entry.name))
            .map_err(|err| AppError::internal(anyhow::anyhow!("zip entry failed: {err}")))?;
        std::io::copy(&mut file, zip)
            .map_err(|err| AppError::internal(anyhow::anyhow!("zip write failed: {err}")))?;
        done += 1;
        bytes = bytes.saturating_add(entry.size);
        if let Some(progress) = progress.as_deref_mut() {
            progress(done, bytes);
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ArchiveZipRequest {
    root: i64,
    dir: Option<String>,
    paths: Vec<String>,
    name: String,
}

pub async fn archive_zip(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ArchiveZipRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    crate::capabilities::require_zip(&state, &current.user).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    if request.paths.is_empty() || request.paths.len() > 1000 {
        return Err(AppError::bad_request("select between 1 and 1000 paths"));
    }
    let dir = request.dir.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, root.id, &dir, true).await?;
    if !resolved.absolute.is_dir() {
        return Err(AppError::bad_request(
            "archive destination is not a directory",
        ));
    }
    let mut name = request.name.trim().to_owned();
    if !name.to_ascii_lowercase().ends_with(".zip") {
        name.push_str(".zip");
    }
    let name = validate_filename(&name)?;
    for path in &request.paths {
        if path.is_empty()
            || path == "."
            || path == ".."
            || path.contains('/')
            || path.contains('\\')
            || path.contains('\0')
        {
            return Err(AppError::invalid_path("invalid archive entry"));
        }
    }
    let joined: Vec<String> = request
        .paths
        .iter()
        .map(|path| join_relative(&dir, path))
        .collect();
    let selected = authz::resolve_many(&state, current.user.id, root.id, joined, false).await?;
    if selected.iter().any(|item| item.relative.is_empty()) {
        return Err(AppError::invalid_path("cannot archive the root directory"));
    }
    let settings = state.settings();
    let max_bytes = settings.settings.downloads.zip_max_bytes;
    let max_files = settings.settings.downloads.zip_max_files;
    let names = request.paths.clone();
    let entries = tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        let mut total = 0_u64;
        for (resolved, name) in selected.iter().zip(names.iter()) {
            fsops::collect_zip_named(
                &resolved.absolute,
                name,
                &mut entries,
                &mut total,
                max_bytes,
                max_files,
                false,
            )?;
        }
        Ok::<_, AppError>(entries)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("zip preparation failed: {err}")))??;
    if entries.is_empty() {
        return Err(AppError::bad_request("nothing to archive"));
    }
    if let Some(quota_bytes) = current.user.quota_bytes
        && crate::platform::path_within(
            &resolved.absolute,
            std::path::Path::new(&current.user.home_dir),
        )
    {
        let used = usage::cached_or_scan(&state, &current.user).await?;
        let source_bytes: u64 = entries.iter().map(|entry| entry.size).sum();
        if used.saturating_add(source_bytes) > quota_bytes.max(0) as u64 {
            return Err(AppError::QuotaExceeded);
        }
    }

    let outcome = submit_zip_job(
        &state,
        current.user.id,
        root.id,
        resolved.relative,
        request.paths,
        name.clone(),
        entries.len(),
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
        "name": name,
    })))
}

pub(crate) async fn submit_zip_job(
    state: &Arc<AppState>,
    user_id: i64,
    root_id: i64,
    dir: String,
    paths: Vec<String>,
    name: String,
    total: usize,
) -> Result<SubmitOutcome, AppError> {
    let limit = state.settings().settings.jobs.archive_per_user;
    if limit > 0 {
        let active = state
            .jobs
            .list_for_user(user_id, 0)
            .await
            .map_err(AppError::internal)?
            .into_iter()
            .filter(|job| {
                matches!(job.kind.as_str(), "archive" | "unzip")
                    && matches!(job.status.as_str(), "queued" | "running")
            })
            .count();
        if active >= limit {
            return Err(AppError::TooManyRequests);
        }
    }
    let settings = state.settings();
    let max_bytes = settings.settings.downloads.zip_max_bytes;
    let max_files = settings.settings.downloads.zip_max_files;
    let payload = json!({
        "root": root_id,
        "dir": dir.clone(),
        "paths": paths.clone(),
        "name": name.clone(),
    });
    let key = auth::sha256_hex(&format!(
        "{}:{}:{}:{}",
        root_id,
        dir,
        name,
        paths.join("\u{1f}")
    ));
    let job_state = Arc::clone(state);
    let job_payload = payload.clone();
    state
        .jobs
        .submit(
            JobKind::Archive,
            key,
            user_id,
            payload,
            total,
            false,
            move |ctx| async move {
                run_zip_job(
                    job_state,
                    user_id,
                    job_payload,
                    ctx,
                    total,
                    max_bytes,
                    max_files,
                )
                .await
            },
        )
        .await
}

async fn run_zip_job(
    state: Arc<AppState>,
    user_id: i64,
    payload: Value,
    ctx: JobContext,
    total: usize,
    max_bytes: u64,
    max_files: usize,
) -> Result<JobOutcome, AppError> {
    let user = state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    let root_id = payload
        .get("root")
        .and_then(Value::as_i64)
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("archive payload missing root")))?;
    let dir = payload
        .get("dir")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("archive payload missing name")))?
        .to_owned();
    let paths: Vec<String> = payload
        .get("paths")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let root = crate::cmdrunner::load_root(&state, user_id, root_id).await?;
    let dir_resolved = authz::resolve(&state, user_id, root.id, &dir, true).await?;
    if !dir_resolved.absolute.is_dir() {
        return Err(AppError::bad_request(
            "archive destination is not a directory",
        ));
    }
    let joined: Vec<String> = paths.iter().map(|path| join_relative(&dir, path)).collect();
    let selected = authz::resolve_many(&state, user_id, root.id, joined, false).await?;
    let names = paths.clone();
    let entries = tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        let mut total = 0_u64;
        for (resolved, name) in selected.iter().zip(names.iter()) {
            fsops::collect_zip_named(
                &resolved.absolute,
                name,
                &mut entries,
                &mut total,
                max_bytes,
                max_files,
                false,
            )?;
        }
        Ok::<_, AppError>(entries)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("zip preparation failed: {err}")))??;
    if entries.is_empty() {
        return Err(AppError::bad_request("nothing to archive"));
    }
    let source_bytes: u64 = entries.iter().map(|entry| entry.size).sum();
    if let Some(quota_bytes) = user.quota_bytes
        && crate::platform::path_within(
            &dir_resolved.absolute,
            std::path::Path::new(&user.home_dir),
        )
    {
        let used = usage::cached_or_scan(&state, &user).await?;
        if used.saturating_add(source_bytes) > quota_bytes.max(0) as u64 {
            return Err(AppError::QuotaExceeded);
        }
    }
    let total = total.max(entries.len());
    let base = dir_resolved.absolute.clone();
    let temp = base.join(format!(".filehub-zip-{}.part", auth::random_token(8)));
    let create = {
        let temp = temp.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<std::fs::File> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            crate::platform::set_private_mode(&mut options);
            options.open(temp)
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("zip temp task failed: {err}")))?
    };
    let file = match create {
        Ok(file) => file,
        Err(error) => return Err(AppError::internal(anyhow::Error::from(error))),
    };
    let worker_temp = temp.clone();
    let worker_entries = entries.clone();
    let worker_name = name.clone();
    let worker_base = base.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(PathBuf, u64), AppError> {
        let mut zip = ZipWriter::new(file);
        let mut scratch = std::io::BufWriter::new(std::io::sink());
        let _ = &mut scratch;
        let mut last_progress = 0_usize;
        let mut written = 0_u64;
        for (index, entry) in worker_entries.iter().enumerate() {
            if ctx.check_cancelled().is_err() {
                let _ = std::fs::remove_file(&worker_temp);
                return Err(AppError::Cancelled);
            }
            let mut input = std::fs::File::open(&entry.path)
                .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
            zip.start_file(&entry.name, zip_options(&entry.name))
                .map_err(|err| AppError::internal(anyhow::anyhow!("zip entry failed: {err}")))?;
            std::io::copy(&mut input, &mut zip)
                .map_err(|err| AppError::internal(anyhow::anyhow!("zip write failed: {err}")))?;
            written = written.saturating_add(entry.size);
            if index + 1 > last_progress {
                last_progress = index + 1;
                ctx.report(index + 1, total, written, source_bytes);
            }
        }
        let file = zip
            .finish()
            .map_err(|err| AppError::internal(anyhow::anyhow!("zip finish failed: {err}")))?;
        file.sync_all()
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        drop(file);
        let mut target = crate::fsops::unique_target(&worker_base.join(&worker_name));
        let mut linked = false;
        for _ in 0..100 {
            match std::fs::hard_link(&worker_temp, &target) {
                Ok(()) => {
                    linked = true;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    target = crate::fsops::unique_target(&target);
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&worker_temp);
                    return Err(AppError::internal(anyhow::Error::from(error)));
                }
            }
        }
        let _ = std::fs::remove_file(&worker_temp);
        if !linked {
            return Err(AppError::conflict("could not allocate an archive name"));
        }
        let size = std::fs::metadata(&target)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok((target, size))
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("zip task failed: {err}")))?;
    let (target, size) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
    };
    let relative = target
        .strip_prefix(&base)
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| name.clone());
    state.adjust_entries(user_id, 1);
    state.adjust_usage(user_id, size as i64);
    state
        .invalidate_path_caches_with_usage(
            &target,
            Some(user_id),
            false,
            UsageInvalidation::Delta(size as i64),
        )
        .await;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&name)
        .to_owned();
    let _ = &user;
    Ok(JobOutcome {
        result: json!({
            "path": relative,
            "name": name,
            "size": size,
            "files": entries.len(),
            "bytes": source_bytes,
        }),
        progress: entries.len(),
        total,
        bytes_done: source_bytes,
        bytes_total: source_bytes,
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    root: i64,
    path: Option<String>,
    filename: String,
    total_size: i64,
    chunk_size: Option<i64>,
}

fn warn_upload(user_id: i64, filename: &str, action: &str, error: &AppError) {
    tracing::warn!(
        user_id,
        filename,
        action,
        error = %error,
        "upload request failed"
    );
}

pub async fn create_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateUploadRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let path = request.path.unwrap_or_default();
    if root.is_remote() {
        let filename = validate_filename(&request.filename)?;
        let size = validate_upload_size(&state, request.total_size)?;
        let chunk_size = request
            .chunk_size
            .unwrap_or_else(|| state.settings().settings.uploads.chunk_size as i64);
        if !(1..=64 * 1024 * 1024).contains(&chunk_size) {
            return Err(AppError::bad_request(
                "chunk_size must be between 1 and 64 MiB",
            ));
        }
        return crate::federation::proxy::create_upload(
            &state,
            current.user.id,
            &root,
            &path,
            &filename,
            size,
            chunk_size,
            None,
            None,
        )
        .await;
    }
    let resolved = authz::resolve(&state, current.user.id, root.id, &path, true).await?;
    let filename = validate_filename(&request.filename)
        .inspect_err(|error| warn_upload(current.user.id, &request.filename, "filename", error))?;
    let size = validate_upload_size(&state, request.total_size)
        .inspect_err(|error| warn_upload(current.user.id, &filename, "size", error))?;
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
        let error = AppError::TooManyRequests;
        warn_upload(current.user.id, &filename, "concurrency", &error);
        return Err(error);
    }

    if let Some(quota_bytes) = current.user.quota_bytes
        && crate::platform::path_within(
            &resolved.absolute,
            std::path::Path::new(&current.user.home_dir),
        )
    {
        let usage = crate::usage::cached_or_scan(&state, &current.user).await?;
        if usage.saturating_add(size as u64) > quota_bytes.max(0) as u64 {
            let error = AppError::QuotaExceeded;
            warn_upload(current.user.id, &filename, "quota", &error);
            return Err(error);
        }
    }
    if std::fs::symlink_metadata(resolved.absolute.join(&filename)).is_err() {
        crate::usage::ensure_entry_capacity(&state, &current.user, 1)
            .await
            .inspect_err(|error| {
                warn_upload(current.user.id, &filename, "entry_limit", error);
            })?;
    }

    let chunk_size = request
        .chunk_size
        .unwrap_or_else(|| state.settings().settings.uploads.chunk_size as i64);

    if !(1..=64 * 1024 * 1024).contains(&chunk_size) {
        let error = AppError::bad_request("chunk_size must be between 1 and 64 MiB");
        warn_upload(current.user.id, &filename, "chunk_size", &error);
        return Err(error);
    }
    if !resolved.absolute.is_dir() {
        let error = AppError::bad_request("upload target is not a directory");
        warn_upload(current.user.id, &filename, "target", &error);
        return Err(error);
    }

    let chunks = (size + chunk_size - 1) / chunk_size;
    if chunks > 100_000 {
        let error = AppError::bad_request("too many upload chunks");
        warn_upload(current.user.id, &filename, "chunks", &error);
        return Err(error);
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
        remote_upload_id: None,
        if_match: None,
        if_none_match: None,
        token_id: None,
    };

    if let Err(err) = state.db.insert_upload(&upload).await {
        fsops::delete_temp(&temp);
        let error = AppError::internal(err);
        warn_upload(current.user.id, &filename, "insert", &error);
        return Err(error);
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
    if upload.remote_upload_id.is_some() {
        return crate::federation::proxy::upload_status(&state, current.user.id, &upload).await;
    }
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
        if upload.remote_upload_id.is_some() {
            crate::federation::proxy::cancel_upload(&state, current.user.id, &upload).await?;
            return Ok(StatusCode::NO_CONTENT);
        }
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
        let error = AppError::bad_request("chunk index out of range");
        warn_upload(current.user.id, &upload.filename, "chunk_index", &error);
        return Err(error);
    }

    let expected = if index == chunks - 1 {
        upload.total_size - index * upload.chunk_size
    } else {
        upload.chunk_size
    };
    if body.len() as i64 != expected {
        let error = AppError::bad_request("unexpected chunk size");
        warn_upload(current.user.id, &upload.filename, "chunk_size", &error);
        return Err(error);
    }

    let byte_index = (index / 8) as usize;
    let bit = 1_u8 << (index % 8);
    let already_received = upload
        .chunk_bitmap
        .get(byte_index)
        .is_some_and(|value| value & bit != 0);

    if upload.remote_upload_id.is_some() && !already_received {
        return crate::federation::proxy::upload_chunk(
            &state,
            current.user.id,
            &upload,
            index as u32,
            &body,
        )
        .await;
    }

    if !already_received {
        let temp_path = PathBuf::from(&upload.temp_path);
        let data = body.to_vec();
        let offset = (index * upload.chunk_size) as u64;
        let written = tokio::task::spawn_blocking(move || {
            fsops::write_upload_chunk(&temp_path, offset, &data)
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("chunk task failed: {err}")))?;
        if let Err(error) = written {
            warn_upload(current.user.id, &upload.filename, "chunk_write", &error);
            return Err(error);
        }

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
        let error = AppError::bad_request("upload is incomplete");
        warn_upload(current.user.id, &upload.filename, "incomplete", &error);
        return Err(error);
    }

    let root = load_root(&state, current.user.id, upload.root_id).await?;
    if root.is_remote() {
        return crate::federation::proxy::complete_upload(&state, current.user.id, &upload).await;
    }
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
    let target_existed = target_path.exists();
    if target_existed {
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
                    let error = AppError::conflict("target is a directory");
                    warn_upload(current.user.id, &upload.filename, "complete", &error);
                    return Err(error);
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
    let finalized =
        tokio::task::spawn_blocking(move || fsops::finalize_upload(&temp, &finalize_target))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("finalize task failed: {err}")))?;
    if let Err(error) = finalized {
        warn_upload(current.user.id, &upload.filename, "finalize", &error);
        return Err(error);
    }
    if !target_existed {
        state.adjust_entries(current.user.id, 1);
    }

    state
        .db
        .delete_upload(&upload.id, current.user.id)
        .await
        .map_err(AppError::internal)?;

    let usage =
        if crate::platform::path_within(&target_path, std::path::Path::new(&current.user.home_dir))
        {
            UsageInvalidation::Delta(upload.total_size)
        } else {
            UsageInvalidation::None
        };
    state
        .invalidate_path_caches_with_usage(&target_path, Some(current.user.id), false, usage)
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

pub(crate) fn delete_requires_code(items: &[(bool, u64)]) -> bool {
    const ITEM_LIMIT: usize = 5;
    const SIZE_LIMIT: u64 = 10 * 1024 * 1024;
    items.iter().any(|(is_dir, _)| *is_dir)
        || items.len() > ITEM_LIMIT
        || items.iter().map(|(_, size)| *size).sum::<u64>() > SIZE_LIMIT
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
    let requested_paths = request.paths;
    if root.is_remote() {
        let (client, mount) =
            crate::federation::mounts::client_for_root(&state, current.user.id, root.id).await?;
        let mut items = Vec::with_capacity(requested_paths.len());
        for path in &requested_paths {
            if path.is_empty() {
                return Err(AppError::invalid_path(
                    "the root directory cannot be deleted",
                ));
            }
            let stat = client
                .stat(mount.grant_id, path)
                .await
                .map_err(|error| error.into_app())?;
            items.push(DeleteItem {
                relative: path.clone(),
                is_dir: stat.get("is_dir").and_then(Value::as_bool).unwrap_or(false),
                size: stat.get("size").and_then(Value::as_u64).unwrap_or(0),
            });
        }
        let requires_code = delete_requires_code(
            &items
                .iter()
                .map(|item| (item.is_dir, item.size))
                .collect::<Vec<_>>(),
        );
        let token = auth::random_token(24);
        let expires_at = unix_now() + 300;
        let random = auth::sha256_hex(&auth::random_token(8));
        let confirm_code = format!(
            "{:04}",
            u32::from_str_radix(&random[..8], 16).unwrap_or_default() % 10_000
        );
        state.delete_tokens.lock().insert(
            token.clone(),
            DeletePreview {
                user_id: current.user.id,
                root_id: root.id,
                items: items.clone(),
                expires_at,
                confirm_code: confirm_code.clone(),
                requires_code,
            },
        );
        return Ok(Json(json!({
            "token": token,
            "expires_at": expires_at,
            "items": items,
            "confirm_code": confirm_code,
            "requires_code": requires_code,
        })));
    }
    let resolved_paths = authz::resolve_many(
        &state,
        current.user.id,
        root.id,
        requested_paths.clone(),
        true,
    )
    .await?;
    let mut items = Vec::with_capacity(requested_paths.len());
    let grants = state
        .db
        .federation_list_grants(current.user.id)
        .await
        .map_err(AppError::internal)?;

    for resolved in resolved_paths {
        if resolved.relative.is_empty() {
            return Err(AppError::invalid_path(
                "the root directory cannot be deleted",
            ));
        }
        if resolved.absolute.is_dir()
            && grants.iter().any(|grant| {
                !grant.revoked
                    && crate::platform::path_within(
                        std::path::Path::new(&grant.real_path),
                        &resolved.absolute,
                    )
            })
        {
            return Err(AppError::Conflict(
                "the directory contains a remote share; remove the share first".to_owned(),
            ));
        }
        if !resolved.absolute.exists() {
            return Err(AppError::NotFound);
        }
        let metadata = std::fs::symlink_metadata(&resolved.absolute).map_err(AppError::internal)?;
        items.push(DeleteItem {
            relative: resolved.relative,
            is_dir: metadata.is_dir(),
            size: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
        });
    }

    let requires_code = delete_requires_code(
        &items
            .iter()
            .map(|item| (item.is_dir, item.size))
            .collect::<Vec<_>>(),
    );
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
            requires_code,
        },
    );
    drop(tokens);

    Ok(Json(json!({
        "token": token,
        "expires_at": expires_at,
        "items": items,
        "confirm_code": confirm_code,
        "requires_code": requires_code,
    })))
}

#[derive(Debug, Deserialize)]
pub struct DeleteConfirmRequest {
    token: String,
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

    if preview.requires_code
        && request.confirm_code.as_deref() != Some(preview.confirm_code.as_str())
    {
        return Err(AppError::bad_request("confirmation code is required"));
    }

    {
        let mut tokens = state.delete_tokens.lock();
        tokens.remove(&request.token);
    }
    let started = Instant::now();
    let mut resolve_ns = 0_u128;
    let mut remove_ns = 0_u128;
    let mut cache_ns = 0_u128;
    let root = load_root(&state, current.user.id, preview.root_id).await?;
    if !root.is_remote() {
        let grants = state
            .db
            .federation_list_grants(current.user.id)
            .await
            .map_err(AppError::internal)?;
        for item in &preview.items {
            let resolved =
                authz::resolve(&state, current.user.id, root.id, &item.relative, false).await?;
            if resolved.absolute.is_dir()
                && grants.iter().any(|grant| {
                    !grant.revoked
                        && crate::platform::path_within(
                            std::path::Path::new(&grant.real_path),
                            &resolved.absolute,
                        )
                })
            {
                return Err(AppError::Conflict(
                    "the directory contains a remote share; remove the share first".to_owned(),
                ));
            }
        }
    }
    let mut cache_targets = Vec::with_capacity(preview.items.len());
    let paths: Vec<String> = preview
        .items
        .iter()
        .map(|item| item.relative.clone())
        .collect();
    if root.is_remote() {
        let value =
            crate::federation::proxy::delete_many(&state, current.user.id, &root, &paths).await?;
        let failed = value
            .get("failed")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        audit(
            &state,
            current.user.id,
            "delete",
            preview.items.first().map(|item| item.relative.as_str()),
            &headers,
        )
        .await;
        if failed > 0 {
            return Err(AppError::internal(anyhow::anyhow!(
                "some remote items could not be deleted"
            )));
        }
        return Ok(StatusCode::NO_CONTENT);
    }
    let phase = Instant::now();
    let resolved_paths = authz::resolve_many(&state, current.user.id, root.id, paths, true).await?;
    resolve_ns += phase.elapsed().as_nanos();
    for resolved in resolved_paths {
        let relative = resolved.relative.clone();
        let path = resolved.absolute.clone();
        let phase = Instant::now();
        tokio::task::spawn_blocking(move || fsops::remove_path(&path, &relative))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("delete task failed: {err}")))??;
        remove_ns += phase.elapsed().as_nanos();
        cache_targets.push(resolved.absolute.clone());
    }
    let phase = Instant::now();
    state
        .invalidate_paths_exact(&cache_targets, Some(current.user.id))
        .await;
    state.schedule_cache_cleanup(cache_targets);
    cache_ns += phase.elapsed().as_nanos();

    let audit_started = Instant::now();
    let audit_path = preview.items.first().map(|item| item.relative.as_str());
    let audit_detail = (preview.items.len() > 1).then(|| format!("items={}", preview.items.len()));
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(current.user.id),
            action: "delete".to_owned(),
            path: audit_path.map(str::to_owned),
            detail: audit_detail,
            ip: auth::client_ip(&headers),
        })
        .await;
    let total_ms = started.elapsed().as_millis() as u64;
    if total_ms >= 500 {
        tracing::warn!(
            user_id = current.user.id,
            items = preview.items.len(),
            resolve_ms = resolve_ns / 1_000_000,
            remove_ms = remove_ns / 1_000_000,
            cache_ms = cache_ns / 1_000_000,
            audit_ms = audit_started.elapsed().as_millis() as u64,
            total_ms,
            "slow delete request"
        );
    }

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
    // NTFS limits names by UTF-16 code units, not UTF-8 bytes. A long Chinese
    // filename that Explorer accepts must not be rejected by a byte count.
    let too_long = if cfg!(windows) {
        filename.encode_utf16().count() > 255
    } else {
        filename.len() > 255
    };
    if too_long {
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
    if size < 0 {
        return Err(AppError::bad_request("total_size must not be negative"));
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
        "kind": root.kind,
        "mount_id": root.mount_id,
    })
}

/// Detects a tiny leading ADTS frame with no meaningful audio payload, as
/// produced by some WeChat recordings. Returns the number of bytes to skip.
fn aac_primer_skip(head: &[u8]) -> Option<u64> {
    if head.len() < 9 || head[0] != 0xFF || (head[1] & 0xF0) != 0xF0 {
        return None;
    }
    // ADTS layer bits must be 00.
    if head[1] & 0x06 != 0 {
        return None;
    }
    let header_len = if head[1] & 1 == 1 { 7 } else { 9 };
    let frame_len =
        (((head[3] & 3) as usize) << 11) | ((head[4] as usize) << 3) | ((head[5] as usize) >> 5);
    if frame_len <= header_len || frame_len > header_len + 4 {
        return None;
    }
    let next = frame_len;
    if head.len() < next + 2 || head[next] != 0xFF || (head[next + 1] & 0xF0) != 0xF0 {
        return None;
    }
    Some(frame_len as u64)
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
    if root.is_remote() {
        let (client, mount) =
            crate::federation::mounts::client_for_root(&state, current.user.id, root.id).await?;
        let value = client
            .dir_size(mount.grant_id, &query.path)
            .await
            .map_err(|error| error.into_app())?;
        let bytes = value.get("bytes").and_then(Value::as_u64).unwrap_or(0);
        let truncated = value
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        return Ok(Json(json!({
            "status": "ready",
            "size": bytes,
            "indexed": false,
            "partial": truncated,
            "stale": false,
            "refreshing": false,
            "error": null,
        })));
    }
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

    let indexed_state = state
        .db
        .indexed_directory_state(&real_path)
        .await
        .map_err(AppError::internal)?;
    let cached_state = state
        .db
        .directory_size_state(&real_path)
        .await
        .map_err(AppError::internal)?;

    if let Some((indexed_mtime, size, partial, status)) = &indexed_state {
        if status == "ready" && !partial && *indexed_mtime == mtime {
            return Ok(Json(json!({
                "status": "ready",
                "size": size,
                "indexed": true,
                "partial": false,
                "stale": false,
                "refreshing": false,
                "error": null,
            })));
        }
        if status == "ready" || status == "error" {
            let _ = state.db.mark_index_stale(&real_path).await;
        }
    }

    if let Some((status, size, cached_mtime, error, updated_at, has_value)) = cached_state {
        let mtime_matches = cached_mtime == mtime;
        let fresh = status == "ready" && mtime_matches;
        let ttl_expired = now.saturating_sub(updated_at) > ttl;
        if has_value && status == "stale" && mtime_matches && !ttl_expired {
            // Index invalidation only marks the value stale; keep showing it
            // until the TTL expires instead of queueing a scan on every list.
            return Ok(Json(json!({
                "status": "ready",
                "size": size.max(0) as u64,
                "indexed": false,
                "partial": error.is_some(),
                "stale": true,
                "refreshing": false,
                "error": error,
            })));
        }
        if fresh && !ttl_expired {
            return Ok(Json(json!({
                "status": "ready",
                "size": size.max(0) as u64,
                "indexed": false,
                "partial": error.is_some(),
                "stale": false,
                "refreshing": false,
                "error": error,
            })));
        }
        if has_value {
            // Serve the last known value while a refresh runs. The value is
            // marked stale so the UI can render it differently.
            let retry_blocked =
                status == "error" && mtime_matches && now.saturating_sub(updated_at) <= retry;
            let refreshing = if retry_blocked {
                false
            } else {
                try_enqueue_dir_size(&state, current.user.id, &real_path, &path, mtime).await
            };
            return Ok(Json(json!({
                "status": "ready",
                "size": size.max(0) as u64,
                "indexed": false,
                "partial": error.is_some(),
                "stale": true,
                "refreshing": refreshing,
                "error": error,
            })));
        }
        if status == "error" && mtime_matches && now.saturating_sub(updated_at) <= retry {
            return Ok(Json(json!({
                "status": "error",
                "size": null,
                "indexed": false,
                "partial": false,
                "stale": false,
                "refreshing": false,
                "error": error,
            })));
        }
    }

    // No usable cache row: fall back to a previous index aggregate when the
    // directory changed since the index was built.
    if let Some((indexed_mtime, size, partial, status)) = indexed_state
        && indexed_mtime != mtime
        && matches!(status.as_str(), "ready" | "stale")
    {
        let refreshing =
            try_enqueue_dir_size(&state, current.user.id, &real_path, &path, mtime).await;
        return Ok(Json(json!({
            "status": "ready",
            "size": size,
            "indexed": true,
            "partial": partial,
            "stale": true,
            "refreshing": refreshing,
            "error": null,
        })));
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
        "refreshing": true,
        "error": null,
        "job_id": (outcome.job.user_id == current.user.id).then_some(outcome.job.id),
    })))
}

async fn try_enqueue_dir_size(
    state: &Arc<AppState>,
    user_id: i64,
    real_path: &str,
    path: &std::path::Path,
    mtime: i64,
) -> bool {
    match enqueue_dir_size(
        state,
        user_id,
        real_path.to_owned(),
        path.to_path_buf(),
        mtime,
    )
    .await
    {
        Ok(outcome) => !outcome.job.is_finished(),
        Err(AppError::QueueFull) => false,
        Err(error) => {
            tracing::warn!(%error, path = %real_path, "failed to enqueue directory size refresh");
            false
        }
    }
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
        "recursive": true,
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
            false,
            move |ctx| async move {
                let _ = job_state
                    .db
                    .begin_directory_size(&job_real_path, mtime_ns)
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
                            .fail_directory_size(&job_real_path, "stale", mtime_ns, None)
                            .await;
                        Err(AppError::Cancelled)
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = job_state
                            .db
                            .fail_directory_size(
                                &job_real_path,
                                "error",
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
    if root.is_remote() {
        return crate::federation::proxy::mkdir(
            &state,
            current.user.id,
            &root,
            &request.path,
            &name,
        )
        .await;
    }
    let target = join_relative(&request.path, &name);
    let resolved = authz::resolve(&state, current.user.id, root.id, &target, true).await?;
    crate::usage::ensure_entry_capacity(&state, &current.user, 1).await?;
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
    state.adjust_entries(current.user.id, 1);
    state
        .invalidate_path_caches_with_usage(
            &resolved.absolute,
            Some(current.user.id),
            false,
            UsageInvalidation::None,
        )
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
    if root.is_remote() {
        return crate::federation::proxy::touch(
            &state,
            current.user.id,
            &root,
            &request.path,
            &name,
        )
        .await;
    }
    let target = join_relative(&request.path, &name);
    let resolved = authz::resolve(&state, current.user.id, root.id, &target, true).await?;
    crate::usage::ensure_entry_capacity(&state, &current.user, 1).await?;
    let absolute = resolved.absolute.clone();
    tokio::task::spawn_blocking(move || fsops::touch_file(&absolute))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("touch task failed: {err}")))??;
    state.adjust_entries(current.user.id, 1);
    state
        .invalidate_path_caches_with_usage(
            &resolved.absolute,
            Some(current.user.id),
            false,
            UsageInvalidation::None,
        )
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
    if root.is_remote() {
        return crate::federation::proxy::rename(
            &state,
            current.user.id,
            &root,
            &request.path,
            &new_name,
        )
        .await;
    }
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
    let cache_targets = vec![source.absolute.clone(), target.absolute.clone()];
    state
        .invalidate_paths_exact_with_usage(
            &cache_targets,
            Some(current.user.id),
            UsageInvalidation::None,
        )
        .await;
    state.schedule_cache_cleanup(cache_targets);
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
pub(crate) struct TransferJobPayload {
    pub(crate) source_root: i64,
    pub(crate) paths: Vec<String>,
    pub(crate) dest_root: i64,
    pub(crate) dest_path: Option<String>,
    pub(crate) move_items: bool,
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
    let user_id = current.user.id;
    let outcome = submit_transfer_job(&state, user_id, payload).await?;
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

pub(crate) async fn submit_transfer_job(
    state: &Arc<AppState>,
    user_id: i64,
    payload: TransferJobPayload,
) -> Result<SubmitOutcome, AppError> {
    if payload.paths.is_empty() || payload.paths.len() > 1000 {
        return Err(AppError::bad_request("paths must contain 1-1000 items"));
    }
    let source_root = state
        .db
        .find_root(payload.source_root, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let dest_root = state
        .db
        .find_root(payload.dest_root, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if source_root.is_remote() || dest_root.is_remote() {
        let same_remote_root = source_root.id == dest_root.id && source_root.is_remote();
        let kind = if payload.move_items {
            JobKind::Move
        } else {
            JobKind::Copy
        };
        if same_remote_root {
            let key = auth::sha256_hex(&format!(
                "remote:{}:{}:{}:{}",
                payload.source_root,
                payload.dest_path.as_deref().unwrap_or_default(),
                payload.move_items,
                payload.paths.join("\u{1f}")
            ));
            let payload_value = serde_json::to_value(&payload)
                .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
            let total = payload.paths.len();
            let job_state = Arc::clone(state);
            let job_payload = payload;
            return state
                .jobs
                .submit(
                    kind,
                    key,
                    user_id,
                    payload_value,
                    total,
                    false,
                    move |ctx| async move {
                        execute_remote_transfer(job_state, user_id, job_payload, ctx).await
                    },
                )
                .await;
        }
        let key = auth::sha256_hex(&format!(
            "cross:{}:{}:{}:{}:{}",
            payload.source_root,
            payload.dest_root,
            payload.dest_path.as_deref().unwrap_or_default(),
            payload.move_items,
            payload.paths.join("\u{1f}")
        ));
        let payload_value = serde_json::to_value(&payload)
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        let total = payload.paths.len();
        let job_state = Arc::clone(state);
        let job_payload = payload;
        return state
            .jobs
            .submit(
                kind,
                key,
                user_id,
                payload_value,
                total,
                false,
                move |ctx| async move {
                    crate::federation::transfer::run(job_state, user_id, job_payload, ctx).await
                },
            )
            .await;
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
    let total = payload.paths.len();
    let job_state = Arc::clone(state);
    let job_payload = payload;
    state
        .jobs
        .submit(
            kind,
            key,
            user_id,
            payload_value,
            total,
            false,
            move |ctx| async move {
                execute_transfer_job(job_state, user_id, job_payload, ctx).await
            },
        )
        .await
}

async fn execute_remote_transfer(
    state: Arc<AppState>,
    user_id: i64,
    payload: TransferJobPayload,
    ctx: JobContext,
) -> Result<JobOutcome, AppError> {
    let (client, mount) =
        crate::federation::mounts::client_for_root(&state, user_id, payload.source_root).await?;
    let dest = payload.dest_path.clone().unwrap_or_default();
    let body = json!({
        "grant": mount.grant_id,
        "paths": payload.paths,
        "dest": dest,
        "overwrite": false,
    });
    let value = client
        .write(
            "POST",
            if payload.move_items { "/move" } else { "/copy" },
            &body,
        )
        .await
        .map_err(|error| error.into_app())?;
    ctx.report(1, 1, 0, 0);
    crate::federation::proxy::invalidate_root(&state, payload.source_root);
    Ok(JobOutcome::done(value, 1, 1))
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
    let transfer_workers = state.settings().settings.jobs.transfer_workers.max(1);
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

    if !payload.move_items {
        let mut wanted = 0_u64;
        for source in &sources {
            wanted = wanted.saturating_add(fsops::count_entries_limited(&source.absolute, 10_000));
        }
        usage::ensure_entry_capacity(&state, &user, wanted).await?;
        state.adjust_entries(user_id, wanted as i64);
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
    let mut cache_targets = Vec::with_capacity(total_items.saturating_mul(2));
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
                fsops::move_path_parallel(
                    &source_path,
                    &target_path,
                    &job_cancel,
                    expected,
                    transfer_workers,
                    &mut progress,
                )
            } else {
                fsops::copy_path_parallel(
                    &source_path,
                    &target_path,
                    &job_cancel,
                    expected,
                    transfer_workers,
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
                cache_targets.push(target.clone());
                if payload.move_items {
                    cache_targets.push(source.absolute.clone());
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

    if !cache_targets.is_empty() {
        state
            .invalidate_paths_exact(&cache_targets, Some(user_id))
            .await;
        state.schedule_cache_cleanup(cache_targets);
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

#[derive(Debug, Deserialize)]
pub struct JobsQuery {
    since: Option<i64>,
}

pub async fn list_jobs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<JobsQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let since = query.since.unwrap_or(0).max(0);
    let jobs = state.jobs.list_for_user(current.user.id, since).await?;
    Ok(Json(json!({ "jobs": jobs })))
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

#[cfg(test)]
mod tests {
    use super::aac_primer_skip;

    #[test]
    fn detects_wechat_aac_primer() {
        let mut head = [0_u8; 15];
        head[..9].copy_from_slice(&[0xFF, 0xF9, 0x60, 0x80, 0x01, 0x3F, 0xFC, 0x00, 0x00]);
        head[9..11].copy_from_slice(&[0xFF, 0xF1]);
        assert_eq!(aac_primer_skip(&head), Some(9));
    }

    #[test]
    fn ignores_normal_aac_and_garbage() {
        let mut head = [0_u8; 15];
        head[..7].copy_from_slice(&[0xFF, 0xF1, 0x60, 0x40, 0x0D, 0x1F, 0xFC]);
        assert_eq!(aac_primer_skip(&head), None);
        assert_eq!(aac_primer_skip(&[0_u8; 4]), None);
        assert_eq!(aac_primer_skip(b"not aac data"), None);
    }

    #[test]
    fn requires_a_valid_following_frame() {
        let mut head = [0_u8; 15];
        head[..9].copy_from_slice(&[0xFF, 0xF9, 0x60, 0x80, 0x01, 0x3F, 0xFC, 0x00, 0x00]);
        head[9..11].copy_from_slice(b"xx");
        assert_eq!(aac_primer_skip(&head), None);
    }
}

#[derive(Debug, Deserialize)]
pub struct DirVersionQuery {
    root: i64,
    path: String,
}

/// Cheap change probe for the currently viewed directory. The client polls
/// this while the tab is visible and reloads the listing in place when the
/// mtime changes (for example after an SMB or rsync write).
pub async fn dir_version(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DirVersionQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, query.root).await?;
    if root.is_remote() {
        let (client, mount) =
            crate::federation::mounts::client_for_root(&state, current.user.id, root.id).await?;
        let stat = client
            .stat(mount.grant_id, &query.path)
            .await
            .map_err(|error| error.into_app())?;
        let mtime = stat
            .get("modified")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .saturating_mul(1000);
        return Ok(Json(json!({ "mtime": mtime })));
    }
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let metadata = tokio::fs::metadata(&resolved.absolute)
        .await
        .map_err(fsops::io_error)?;
    if !metadata.is_dir() {
        return Err(AppError::NotFound);
    }
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();
    Ok(Json(json!({ "mtime": mtime })))
}
