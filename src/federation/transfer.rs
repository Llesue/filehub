use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_util::io::StreamReader;

use crate::db::Root;
use crate::error::AppError;
use crate::federation::client::PeerClient;
use crate::federation::mounts::client_for_root;
use crate::files::TransferJobPayload;
use crate::jobs::{JobContext, JobOutcome};
use crate::state::AppState;

const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 64;
const CHUNK_SIZE: usize = 4 * 1024 * 1024;
const COPY_BUFFER: usize = 256 * 1024;

#[derive(Debug)]
struct Entry {
    relative: String,
    is_dir: bool,
    size: u64,
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    }
}

fn basename(path: &str) -> Result<&str, AppError> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.is_empty() || name == "." || name == ".." || name.contains('\\') || name.contains('\0')
    {
        return Err(AppError::invalid_path("invalid source name"));
    }
    Ok(name)
}

fn collect_local(
    absolute: &Path,
    relative: &str,
    entries: &mut Vec<Entry>,
    depth: usize,
) -> Result<(), AppError> {
    if depth > MAX_DEPTH {
        return Err(AppError::bad_request("directory tree is too deep"));
    }
    if entries.len() >= MAX_ENTRIES {
        return Err(AppError::too_large("transfer contains too many entries"));
    }
    let metadata = std::fs::symlink_metadata(absolute).map_err(crate::fsops::io_error)?;
    if metadata.is_dir() {
        entries.push(Entry {
            relative: relative.to_owned(),
            is_dir: true,
            size: 0,
        });
        let mut children = std::fs::read_dir(absolute)
            .map_err(crate::fsops::io_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::fsops::io_error)?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let name = child.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| AppError::invalid_path("non-UTF-8 file name"))?;
            let child_relative = join(relative, name);
            collect_local(&child.path(), &child_relative, entries, depth + 1)?;
        }
    } else if metadata.is_file() {
        entries.push(Entry {
            relative: relative.to_owned(),
            is_dir: false,
            size: metadata.len(),
        });
    }
    Ok(())
}

async fn collect_remote(
    client: &PeerClient,
    grant: i64,
    path: &str,
    relative: &str,
    entries: &mut Vec<Entry>,
    depth: usize,
) -> Result<(), AppError> {
    if depth > MAX_DEPTH {
        return Err(AppError::bad_request("directory tree is too deep"));
    }
    if entries.len() >= MAX_ENTRIES {
        return Err(AppError::too_large("transfer contains too many entries"));
    }
    let stat = client
        .stat(grant, path)
        .await
        .map_err(|error| error.into_app())?;
    let is_dir = stat.get("is_dir").and_then(Value::as_bool) == Some(true);
    if !is_dir {
        entries.push(Entry {
            relative: relative.to_owned(),
            is_dir: false,
            size: stat.get("size").and_then(Value::as_u64).unwrap_or(0),
        });
        return Ok(());
    }
    entries.push(Entry {
        relative: relative.to_owned(),
        is_dir: true,
        size: 0,
    });
    let mut cursor: Option<String> = None;
    loop {
        let page = client
            .list(grant, path, cursor.as_deref(), 1000, "name", "asc")
            .await
            .map_err(|error| error.into_app())?;
        let items = page
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in items {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let child_relative = join(relative, name);
            let child_path = join(path, name);
            if item.get("is_dir").and_then(Value::as_bool) == Some(true) {
                Box::pin(collect_remote(
                    client,
                    grant,
                    &child_path,
                    &child_relative,
                    entries,
                    depth + 1,
                ))
                .await?;
            } else {
                if entries.len() >= MAX_ENTRIES {
                    return Err(AppError::too_large("transfer contains too many entries"));
                }
                entries.push(Entry {
                    relative: child_relative,
                    is_dir: false,
                    size: item.get("size").and_then(Value::as_u64).unwrap_or(0),
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
    Ok(())
}

fn remote_reader(response: reqwest::Response) -> Box<dyn AsyncRead + Unpin + Send> {
    let stream = response
        .bytes_stream()
        .map(|item| item.map_err(std::io::Error::other));
    Box::new(StreamReader::new(stream))
}

struct RemoteWriter {
    client: Arc<PeerClient>,
    upload_id: String,
    chunk_size: usize,
    buffer: Vec<u8>,
    index: u32,
}

impl RemoteWriter {
    async fn write(&mut self, data: &[u8]) -> Result<(), AppError> {
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= self.chunk_size {
            let chunk: Vec<u8> = self.buffer.drain(..self.chunk_size).collect();
            self.client
                .put_chunk(&self.upload_id, self.index, chunk)
                .await
                .map_err(|error| error.into_app())?;
            self.index += 1;
        }
        Ok(())
    }

    async fn finish(mut self) -> Result<(), AppError> {
        if !self.buffer.is_empty() {
            let chunk = std::mem::take(&mut self.buffer);
            self.client
                .put_chunk(&self.upload_id, self.index, chunk)
                .await
                .map_err(|error| error.into_app())?;
        }
        self.client
            .complete_upload(&self.upload_id)
            .await
            .map_err(|error| error.into_app())?;
        Ok(())
    }

    async fn abort(&self) {
        let _ = self.client.cancel_upload(&self.upload_id).await;
    }
}

struct LocalWriter {
    file: tokio::fs::File,
    temp: PathBuf,
    target: PathBuf,
}

impl LocalWriter {
    async fn write(&mut self, data: &[u8]) -> Result<(), AppError> {
        self.file
            .write_all(data)
            .await
            .map_err(crate::fsops::io_error)
    }

    async fn finish(self) -> Result<(), AppError> {
        self.file.sync_all().await.map_err(crate::fsops::io_error)?;
        drop(self.file);
        let _ = tokio::fs::remove_file(&self.target).await;
        tokio::fs::rename(&self.temp, &self.target)
            .await
            .map_err(crate::fsops::io_error)
    }

    async fn abort(&self) {
        let _ = tokio::fs::remove_file(&self.temp).await;
    }
}

enum Writer {
    Local(LocalWriter),
    Remote(RemoteWriter),
}

impl Writer {
    async fn write(&mut self, data: &[u8]) -> Result<(), AppError> {
        match self {
            Self::Local(writer) => writer.write(data).await,
            Self::Remote(writer) => writer.write(data).await,
        }
    }

    async fn finish(self) -> Result<(), AppError> {
        match self {
            Self::Local(writer) => writer.finish().await,
            Self::Remote(writer) => writer.finish().await,
        }
    }

    async fn abort(&self) {
        match self {
            Self::Local(writer) => writer.abort().await,
            Self::Remote(writer) => writer.abort().await,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn copy_one(
    state: &Arc<AppState>,
    user_id: i64,
    source_root: &Root,
    dest_root: &Root,
    source_relative: &str,
    dest_relative: &str,
    size: u64,
    copied: &mut u64,
    total_bytes: u64,
    done: usize,
    total: usize,
    ctx: &JobContext,
) -> Result<(), AppError> {
    ctx.check_cancelled()?;

    let mut reader: Box<dyn AsyncRead + Unpin + Send> = if source_root.is_remote() {
        let (client, mount) = client_for_root(state, user_id, source_root.id).await?;
        let response = client
            .download(
                mount.grant_id,
                source_relative,
                false,
                &axum::http::HeaderMap::new(),
            )
            .await
            .map_err(|error| error.into_app())?;
        remote_reader(response)
    } else {
        let resolved =
            crate::authz::resolve(state, user_id, source_root.id, source_relative, false).await?;
        Box::new(
            tokio::fs::File::open(&resolved.absolute)
                .await
                .map_err(crate::fsops::io_error)?,
        )
    };

    let mut writer = if dest_root.is_remote() {
        let (client, mount) = client_for_root(state, user_id, dest_root.id).await?;
        let (parent, filename) = dest_relative
            .rsplit_once('/')
            .unwrap_or(("", dest_relative));
        let created = client
            .create_upload(
                mount.grant_id,
                parent,
                filename,
                size as i64,
                CHUNK_SIZE as i64,
                None,
                None,
                None,
            )
            .await
            .map_err(|error| error.into_app())?;
        let upload_id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::internal(anyhow::anyhow!("peer did not return an upload id")))?
            .to_owned();
        let chunk_size = created
            .get("chunk_size")
            .and_then(Value::as_u64)
            .unwrap_or(CHUNK_SIZE as u64) as usize;
        Writer::Remote(RemoteWriter {
            client,
            upload_id,
            chunk_size,
            buffer: Vec::with_capacity(chunk_size),
            index: 0,
        })
    } else {
        let resolved =
            crate::authz::resolve(state, user_id, dest_root.id, dest_relative, true).await?;
        if let Some(parent) = resolved.absolute.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(crate::fsops::io_error)?;
        }
        let temp = resolved.absolute.with_extension("filehub-part");
        Writer::Local(LocalWriter {
            file: tokio::fs::File::create(&temp)
                .await
                .map_err(crate::fsops::io_error)?,
            temp,
            target: resolved.absolute,
        })
    };

    let mut buffer = vec![0_u8; COPY_BUFFER];
    let read_result: Result<(), AppError> = async {
        loop {
            ctx.check_cancelled()?;
            let read = reader
                .read(&mut buffer)
                .await
                .map_err(crate::fsops::io_error)?;
            if read == 0 {
                break;
            }
            writer.write(&buffer[..read]).await?;
            *copied = copied.saturating_add(read as u64);
            ctx.report(done, total, *copied, total_bytes);
        }
        Ok(())
    }
    .await;

    if let Err(error) = read_result {
        writer.abort().await;
        return Err(error);
    }
    writer.finish().await
}

async fn ensure_destination_dir(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    relative: &str,
) -> Result<(), AppError> {
    if root.is_remote() {
        let (client, mount) = client_for_root(state, user_id, root.id).await?;
        let result = client
            .write(
                "POST",
                "/mkdir",
                &json!({ "grant": mount.grant_id, "path": relative }),
            )
            .await;
        if let Err(error) = result
            && error.status != 409
        {
            return Err(error.into_app());
        }
        return Ok(());
    }
    let resolved = crate::authz::resolve(state, user_id, root.id, relative, true).await?;
    tokio::fs::create_dir_all(&resolved.absolute)
        .await
        .map_err(crate::fsops::io_error)
}

async fn delete_sources(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    paths: &[String],
) -> Result<(), AppError> {
    if root.is_remote() {
        let (client, mount) = client_for_root(state, user_id, root.id).await?;
        let value = client
            .write(
                "POST",
                "/delete",
                &json!({ "grant": mount.grant_id, "paths": paths }),
            )
            .await
            .map_err(|error| error.into_app())?;
        let failed = value
            .get("failed")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        if failed > 0 {
            return Err(AppError::internal(anyhow::anyhow!(
                "{failed} source items could not be deleted"
            )));
        }
        return Ok(());
    }
    for path in paths {
        let resolved = crate::authz::resolve(state, user_id, root.id, path, true).await?;
        tokio::task::spawn_blocking(move || {
            crate::fsops::remove_path(&resolved.absolute, &resolved.relative)
        })
        .await
        .map_err(|error| AppError::internal(anyhow::anyhow!("delete task failed: {error}")))??;
    }
    Ok(())
}

pub(crate) async fn run(
    state: Arc<AppState>,
    user_id: i64,
    payload: TransferJobPayload,
    ctx: JobContext,
) -> Result<JobOutcome, AppError> {
    let source_root = crate::cmdrunner::load_root(&state, user_id, payload.source_root).await?;
    let dest_root = crate::cmdrunner::load_root(&state, user_id, payload.dest_root).await?;
    let dest_base = payload
        .dest_path
        .as_deref()
        .unwrap_or_default()
        .trim_matches('/')
        .to_owned();

    let mut entries = Vec::new();
    for path in &payload.paths {
        if path.is_empty() {
            return Err(AppError::invalid_path(
                "the root directory cannot be transferred",
            ));
        }
        let name = basename(path)?.to_owned();
        if source_root.is_remote() {
            let (client, mount) = client_for_root(&state, user_id, source_root.id).await?;
            Box::pin(collect_remote(
                &client,
                mount.grant_id,
                path,
                &name,
                &mut entries,
                0,
            ))
            .await?;
        } else {
            let resolved =
                crate::authz::resolve(&state, user_id, source_root.id, path, false).await?;
            let absolute = resolved.absolute.clone();
            let entry_name = name.clone();
            let collected = tokio::task::spawn_blocking(move || {
                let mut collected = Vec::new();
                collect_local(&absolute, &entry_name, &mut collected, 0)?;
                Ok::<Vec<Entry>, AppError>(collected)
            })
            .await
            .map_err(|error| {
                AppError::internal(anyhow::anyhow!("transfer scan failed: {error}"))
            })??;
            entries.extend(collected);
        }
        ctx.check_cancelled()?;
    }

    let total_bytes: u64 = entries.iter().map(|entry| entry.size).sum();
    let total = entries.len();
    let mut copied = 0_u64;
    let mut done = 0_usize;
    let mut failed: Vec<Value> = Vec::new();

    for entry in entries.iter().filter(|entry| entry.is_dir) {
        let relative = if dest_base.is_empty() {
            entry.relative.clone()
        } else {
            join(&dest_base, &entry.relative)
        };
        if let Err(error) = ensure_destination_dir(&state, user_id, &dest_root, &relative).await {
            failed.push(json!({ "path": entry.relative, "message": error.to_string() }));
        }
    }

    for entry in entries.iter().filter(|entry| !entry.is_dir) {
        ctx.check_cancelled()?;
        let relative = if dest_base.is_empty() {
            entry.relative.clone()
        } else {
            join(&dest_base, &entry.relative)
        };
        match copy_one(
            &state,
            user_id,
            &source_root,
            &dest_root,
            &entry.relative,
            &relative,
            entry.size,
            &mut copied,
            total_bytes,
            done,
            total,
            &ctx,
        )
        .await
        {
            Ok(()) => done += 1,
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                failed.push(json!({ "path": entry.relative, "message": error.to_string() }));
            }
        }
    }

    let mut moved = false;
    if payload.move_items && failed.is_empty() {
        delete_sources(&state, user_id, &source_root, &payload.paths).await?;
        moved = true;
    }

    crate::federation::proxy::invalidate_root(&state, source_root.id);
    crate::federation::proxy::invalidate_root(&state, dest_root.id);

    let value = json!({
        "done": done,
        "failed": failed.len(),
        "items": failed,
        "moved": moved,
    });
    Ok(JobOutcome {
        result: value,
        progress: total,
        total,
        bytes_done: copied,
        bytes_total: total_bytes,
    })
}
