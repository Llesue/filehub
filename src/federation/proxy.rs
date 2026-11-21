use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};

use crate::db::Root;
use crate::error::AppError;
use crate::federation::mounts::client_for_root;
use crate::state::AppState;

#[allow(clippy::too_many_arguments)]
pub async fn list(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: Option<&str>,
    cursor: Option<&str>,
    limit: Option<usize>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let path_text = path.unwrap_or("");
    let sort_text = sort.unwrap_or("name");
    let order_text = order.unwrap_or("asc");
    let key = (
        root.id,
        path_text.to_owned(),
        sort_text.to_owned(),
        order_text.to_owned(),
    );
    let revalidate = state
        .settings()
        .settings
        .federation
        .revalidate_seconds
        .max(1);
    let now = crate::db::unix_now();
    let cached = state.federation_cache.lock().get(&key).cloned();
    if let Some((at, value)) = cached {
        if now.saturating_sub(at) < revalidate {
            return Ok(render_list(root, &mount, &value, path_text));
        }
        let state_clone = Arc::clone(state);
        let client_clone = Arc::clone(&client);
        let grant = mount.grant_id;
        let key_clone = key.clone();
        let path_owned = key.1.clone();
        let sort_owned = key.2.clone();
        let order_owned = key.3.clone();
        let limit_value = limit.unwrap_or(200);
        let cursor_owned = cursor.map(str::to_owned);
        tokio::spawn(async move {
            if let Ok(value) = client_clone
                .list(
                    grant,
                    &path_owned,
                    cursor_owned.as_deref(),
                    limit_value,
                    &sort_owned,
                    &order_owned,
                )
                .await
            {
                let mut cache = state_clone.federation_cache.lock();
                if cache.len() > 500 {
                    cache.clear();
                }
                cache.insert(key_clone, (crate::db::unix_now(), value));
            }
        });
        return Ok(render_list(root, &mount, &value, path_text));
    }
    let value = client
        .list(
            mount.grant_id,
            path_text,
            cursor,
            limit.unwrap_or(200),
            sort_text,
            order_text,
        )
        .await
        .map_err(|error| error.into_app())?;
    {
        let mut cache = state.federation_cache.lock();
        if cache.len() > 500 {
            cache.clear();
        }
        cache.insert(key, (now, value.clone()));
    }
    Ok(render_list(root, &mount, &value, path_text))
}

fn render_list(
    root: &Root,
    mount: &crate::federation::db::FederationMount,
    value: &Value,
    path: &str,
) -> Json<Value> {
    let mut entries = value.get("entries").cloned().unwrap_or(json!([]));
    if let Some(array) = entries.as_array_mut() {
        for entry in array {
            if let Some(object) = entry.as_object_mut() {
                object.entry("is_symlink").or_insert(json!(false));
                object.entry("created_at").or_insert(json!(0));
                object.entry("symlink_state").or_insert(Value::Null);
                object.entry("symlink_target").or_insert(Value::Null);
            }
        }
    }
    let total = entries.as_array().map(Vec::len).unwrap_or(0);
    Json(json!({
        "root": {
            "id": root.id,
            "alias": root.alias,
            "read_only": mount.mode != "rw",
            "kind": root.kind,
            "mount_id": root.mount_id,
        },
        "path": value.get("path").cloned().unwrap_or_else(|| json!(path)),
        "entries": entries,
        "next_cursor": value.get("next_cursor").cloned().unwrap_or(Value::Null),
        "total": total,
        "read_only": mount.mode != "rw",
        "read_only_reason": if mount.mode != "rw" { json!("policy") } else { Value::Null },
        "root_id": root.id,
        "root_alias": root.alias,
    }))
}

pub(crate) fn invalidate_root(state: &Arc<AppState>, root_id: i64) {
    state
        .federation_cache
        .lock()
        .retain(|key, _| key.0 != root_id);
}

pub async fn download(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
    inline: bool,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let response = client
        .download(mount.grant_id, path, inline, headers)
        .await
        .map_err(|error| error.into_app())?;
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ETAG,
        header::ACCEPT_RANGES,
        header::CONTENT_DISPOSITION,
        header::LAST_MODIFIED,
        header::CACHE_CONTROL,
    ] {
        if let Some(value) = response.headers().get(&name) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))
}

pub(crate) async fn audit_remote(
    state: &Arc<AppState>,
    user_id: i64,
    action: &str,
    path: Option<&str>,
    detail: Option<String>,
) {
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: action.to_owned(),
            path: path.map(str::to_owned),
            detail,
            ip: None,
        })
        .await;
}

fn join_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

pub async fn mkdir(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
    name: &str,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let target = join_path(path, name);
    let value = client
        .write(
            "POST",
            "/mkdir",
            &json!({ "grant": mount.grant_id, "path": target }),
        )
        .await
        .map_err(|error| error.into_app())?;
    invalidate_root(state, root.id);
    audit_remote(state, user_id, "mkdir", Some(&target), None).await;
    Ok(Json(value))
}

pub async fn touch(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
    name: &str,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let target = join_path(path, name);
    let value = client
        .write(
            "POST",
            "/touch",
            &json!({ "grant": mount.grant_id, "path": target }),
        )
        .await
        .map_err(|error| error.into_app())?;
    invalidate_root(state, root.id);
    audit_remote(state, user_id, "touch", Some(&target), None).await;
    Ok(Json(value))
}

pub async fn rename(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
    name: &str,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .write(
            "POST",
            "/rename",
            &json!({ "grant": mount.grant_id, "path": path, "name": name }),
        )
        .await
        .map_err(|error| error.into_app())?;
    invalidate_root(state, root.id);
    audit_remote(state, user_id, "rename", Some(path), None).await;
    Ok(Json(value))
}

pub async fn delete_many(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    paths: &[String],
) -> Result<Value, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .write(
            "POST",
            "/delete",
            &json!({ "grant": mount.grant_id, "paths": paths }),
        )
        .await
        .map_err(|error| error.into_app())?;
    invalidate_root(state, root.id);
    audit_remote(
        state,
        user_id,
        "delete",
        None,
        Some(format!("items={}", paths.len())),
    )
    .await;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_upload(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    dir: &str,
    filename: &str,
    total_size: i64,
    chunk_size: i64,
    if_match: Option<&str>,
    if_none_match: Option<&str>,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .create_upload(
            mount.grant_id,
            dir,
            filename,
            total_size,
            chunk_size,
            if_match,
            if_none_match,
        )
        .await
        .map_err(|error| error.into_app())?;
    let peer_id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("peer did not return an upload id")))?
        .to_owned();
    let chunk_size = value
        .get("chunk_size")
        .and_then(Value::as_i64)
        .unwrap_or(chunk_size);
    let chunks = (total_size + chunk_size - 1) / chunk_size;
    let local_id = format!("up_{}", crate::auth::random_token(16));
    let now = crate::db::unix_now();
    let ttl = state.settings().settings.uploads.session_ttl_hours as i64;
    let upload = crate::db::UploadSession {
        id: local_id.clone(),
        user_id,
        root_id: root.id,
        dir_path: dir.to_owned(),
        filename: filename.to_owned(),
        total_size,
        chunk_size,
        received: 0,
        chunk_bitmap: vec![0; ((chunks + 7) / 8) as usize],
        temp_path: String::new(),
        expires_at: now + ttl.saturating_mul(3600),
        created_at: now,
        updated_at: now,
        remote_upload_id: Some(peer_id),
        if_match: None,
        if_none_match: None,
    };
    state
        .db
        .insert_upload(&upload)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "upload_id": local_id,
        "chunk_size": chunk_size,
        "chunks": chunks,
        "expires_at": upload.expires_at,
    })))
}

pub async fn upload_chunk(
    state: &Arc<AppState>,
    user_id: i64,
    upload: &crate::db::UploadSession,
    index: u32,
    body: &[u8],
) -> Result<Json<Value>, AppError> {
    let peer_id = upload
        .remote_upload_id
        .as_deref()
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("missing remote upload id")))?;
    let (client, _mount) = client_for_root(state, user_id, upload.root_id).await?;
    client
        .put_chunk(peer_id, index, body.to_vec())
        .await
        .map_err(|error| error.into_app())?;
    let mut bitmap = upload.chunk_bitmap.clone();
    let byte = (index / 8) as usize;
    if let Some(value) = bitmap.get_mut(byte) {
        *value |= 1_u8 << (index % 8);
    }
    let chunks = (upload.total_size + upload.chunk_size - 1) / upload.chunk_size;
    let received = (0..chunks)
        .filter(|chunk| {
            bitmap
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
        .update_upload_progress(&upload.id, user_id, received, bitmap.clone())
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "upload_id": upload.id,
        "received": received,
        "complete": received == upload.total_size,
    })))
}

pub async fn complete_upload(
    state: &Arc<AppState>,
    user_id: i64,
    upload: &crate::db::UploadSession,
) -> Result<Json<Value>, AppError> {
    let peer_id = upload
        .remote_upload_id
        .as_deref()
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("missing remote upload id")))?;
    let (client, _mount) = client_for_root(state, user_id, upload.root_id).await?;
    let value = client
        .complete_upload(peer_id)
        .await
        .map_err(|error| error.into_app())?;
    state
        .db
        .delete_upload(&upload.id, user_id)
        .await
        .map_err(AppError::internal)?;
    invalidate_root(state, upload.root_id);
    audit_remote(
        state,
        user_id,
        "upload",
        Some(&upload.filename),
        Some(format!("bytes={}", upload.total_size)),
    )
    .await;
    Ok(Json(value))
}

pub async fn cancel_upload(
    state: &Arc<AppState>,
    user_id: i64,
    upload: &crate::db::UploadSession,
) -> Result<(), AppError> {
    if let Some(peer_id) = upload.remote_upload_id.as_deref()
        && let Ok((client, _mount)) = client_for_root(state, user_id, upload.root_id).await
    {
        let _ = client.cancel_upload(peer_id).await;
    }
    state
        .db
        .delete_upload(&upload.id, user_id)
        .await
        .map_err(AppError::internal)?;
    Ok(())
}

pub async fn thumb_warm(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    paths: &[String],
    width: u32,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .thumb_warm(mount.grant_id, paths, width)
        .await
        .map_err(|error| error.into_app())?;
    Ok(Json(value))
}

pub async fn audio_info(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .audio_info(mount.grant_id, path)
        .await
        .map_err(|error| error.into_app())?;
    Ok(Json(value))
}

pub async fn unzip(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
    target: Option<&str>,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .unzip(mount.grant_id, path, target)
        .await
        .map_err(|error| error.into_app())?;
    invalidate_root(state, root.id);
    audit_remote(state, user_id, "unzip", Some(path), None).await;
    let name = value
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or(path)
        .to_owned();
    Ok(Json(json!({
        "job_id": null,
        "status": "completed",
        "name": name,
        "entries": value.get("entries").cloned().unwrap_or(Value::Null),
    })))
}

pub async fn thumb(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
    width: u32,
    edge: u32,
    fit: &str,
) -> Result<Response, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let response = client
        .thumb(mount.grant_id, path, width, edge, fit)
        .await
        .map_err(|error| error.into_app())?;
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::ETAG,
        header::CACHE_CONTROL,
    ] {
        if let Some(value) = response.headers().get(&name) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))
}

pub async fn search(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    query: &str,
    limit: usize,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .search(mount.grant_id, query, limit)
        .await
        .map_err(|error| error.into_app())?;
    let mut entries = Vec::new();
    for item in value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        entries.push(json!({
            "name": item.get("name").cloned().unwrap_or(Value::Null),
            "path": item.get("path").cloned().unwrap_or_else(|| json!("")),
            "is_dir": item.get("is_dir").cloned().unwrap_or(json!(false)),
            "size": item.get("size").cloned().unwrap_or(json!(0)),
            "modified": item.get("modified").cloned().unwrap_or(json!(0)),
            "rootId": root.id,
            "root_id": root.id,
        }));
    }
    Ok(Json(json!({
        "entries": entries,
        "next_cursor": Value::Null,
    })))
}

struct RemoteZipEntry {
    name: String,
    remote_path: Option<String>,
    is_dir: bool,
}

fn zip_entry_name(name: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(AppError::invalid_path("invalid archive entry name"));
    }
    Ok(())
}

async fn collect_remote_zip(
    client: &crate::federation::client::PeerClient,
    grant: i64,
    selected: &[String],
    max_bytes: u64,
    max_files: usize,
) -> Result<(Vec<RemoteZipEntry>, u64, usize), AppError> {
    let mut entries: Vec<RemoteZipEntry> = Vec::new();
    let mut total_bytes = 0_u64;
    let mut file_count = 0_usize;
    for path in selected {
        if path.is_empty() {
            return Err(AppError::invalid_path("cannot archive the root directory"));
        }
        let stat = client
            .stat(grant, path)
            .await
            .map_err(|error| error.into_app())?;
        let is_dir = stat.get("is_dir").and_then(Value::as_bool) == Some(true);
        if !is_dir {
            let size = stat.get("size").and_then(Value::as_u64).unwrap_or(0);
            total_bytes = total_bytes.saturating_add(size);
            if total_bytes > max_bytes {
                return Err(AppError::too_large(
                    "file exceeds the configured size limit",
                ));
            }
            file_count += 1;
            if file_count > max_files {
                return Err(AppError::too_large("zip exceeds the configured file limit"));
            }
            let name = path.rsplit('/').next().unwrap_or(path.as_str()).to_owned();
            entries.push(RemoteZipEntry {
                name,
                remote_path: Some(path.clone()),
                is_dir: false,
            });
            continue;
        }
        let prefix = path.rsplit('/').next().unwrap_or(path.as_str()).to_owned();
        let mut stack = vec![(path.clone(), prefix, 0_usize)];
        while let Some((directory, prefix, depth)) = stack.pop() {
            if depth > 64 {
                return Err(AppError::bad_request("directory tree is too deep"));
            }
            if entries.len() > max_files.saturating_mul(4).saturating_add(1000) {
                return Err(AppError::too_large("zip contains too many entries"));
            }
            entries.push(RemoteZipEntry {
                name: prefix.clone(),
                remote_path: None,
                is_dir: true,
            });
            let mut cursor: Option<String> = None;
            loop {
                let page = client
                    .list(grant, &directory, cursor.as_deref(), 1000, "name", "asc")
                    .await
                    .map_err(|error| error.into_app())?;
                let items = page
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for item in items {
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    zip_entry_name(&name)?;
                    let child_prefix = format!("{prefix}/{name}");
                    let child_path = if directory.is_empty() {
                        name
                    } else {
                        format!("{directory}/{name}")
                    };
                    if item.get("is_dir").and_then(Value::as_bool) == Some(true) {
                        stack.push((child_path, child_prefix, depth + 1));
                    } else {
                        let size = item.get("size").and_then(Value::as_u64).unwrap_or(0);
                        total_bytes = total_bytes.saturating_add(size);
                        if total_bytes > max_bytes {
                            return Err(AppError::too_large(
                                "file exceeds the configured size limit",
                            ));
                        }
                        file_count += 1;
                        if file_count > max_files {
                            return Err(AppError::too_large(
                                "zip exceeds the configured file limit",
                            ));
                        }
                        entries.push(RemoteZipEntry {
                            name: child_prefix,
                            remote_path: Some(child_path),
                            is_dir: false,
                        });
                    }
                }
                cursor = page
                    .get("next_cursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if cursor.is_none() {
                    break;
                }
            }
        }
    }
    Ok((entries, total_bytes, file_count))
}

pub(crate) async fn spool_download(
    client: &crate::federation::client::PeerClient,
    grant: i64,
    remote: &str,
    target: &std::path::Path,
    max_bytes: u64,
) -> Result<u64, AppError> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let response = client
        .download(grant, remote, false, &HeaderMap::new())
        .await
        .map_err(|error| error.into_app())?;
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes)
    {
        return Err(AppError::too_large(
            "file exceeds the configured size limit",
        ));
    }
    let mut file = tokio::fs::File::create(target)
        .await
        .map_err(crate::fsops::io_error)?;
    let mut stream = response.bytes_stream();
    let mut written = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            AppError::internal(anyhow::anyhow!("peer download failed: {error}"))
        })?;
        written = written.saturating_add(chunk.len() as u64);
        if written > max_bytes {
            let _ = tokio::fs::remove_file(target).await;
            return Err(AppError::too_large(
                "file exceeds the configured size limit",
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(crate::fsops::io_error)?;
    }
    file.flush().await.map_err(crate::fsops::io_error)?;
    Ok(written)
}

pub async fn zip_files(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    paths: Vec<String>,
) -> Result<Response, AppError> {
    use tokio_util::io::{ReaderStream, SyncIoBridge};

    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let settings = state.settings();
    let max_bytes = settings.settings.downloads.zip_max_bytes;
    let max_files = settings.settings.downloads.zip_max_files;
    let (entries, total_bytes, file_count) =
        collect_remote_zip(&client, mount.grant_id, &paths, max_bytes, max_files).await?;
    if file_count == 0 {
        return Err(AppError::bad_request("selection contains no files"));
    }

    let spool = settings
        .settings
        .cache_dir
        .join("zip-spool")
        .join(crate::auth::random_token(8));
    crate::platform::create_private_dir_all(&spool).map_err(crate::fsops::io_error)?;
    let mut downloaded: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut directories: Vec<String> = Vec::new();
    for entry in &entries {
        if entry.is_dir {
            directories.push(entry.name.clone());
            continue;
        }
        let Some(remote) = entry.remote_path.as_deref() else {
            continue;
        };
        let destination = spool.join(format!("{}", downloaded.len()));
        if let Err(error) =
            spool_download(&client, mount.grant_id, remote, &destination, max_bytes).await
        {
            let _ = tokio::fs::remove_dir_all(&spool).await;
            return Err(error);
        }
        downloaded.push((entry.name.clone(), destination));
    }

    let permit = state
        .zip_semaphore()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests)?;
    let count = downloaded.len();
    let (reader, writer) = tokio::io::duplex(64 * 1024);
    let spool_cleanup = spool.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut zip = zip::ZipWriter::new_stream(SyncIoBridge::new(writer));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for name in directories {
            let _ = zip.add_directory(format!("{name}/"), options);
        }
        for (name, path) in downloaded {
            if zip.start_file(name, options).is_err() {
                continue;
            }
            match std::fs::File::open(&path) {
                Ok(mut file) => {
                    let _ = std::io::copy(&mut file, &mut zip);
                }
                Err(error) => {
                    tracing::warn!(%error, "remote zip spool file disappeared");
                }
            }
            let _ = std::fs::remove_file(&path);
        }
        let _ = zip.finish();
        let _ = std::fs::remove_dir(&spool_cleanup);
        tracing::debug!(
            files = count,
            bytes = total_bytes,
            "remote zip stream finished"
        );
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename*=UTF-8''filehub.zip",
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(ReaderStream::new(reader)))
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))
}
