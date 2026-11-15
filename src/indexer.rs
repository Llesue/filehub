use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::authz;
use crate::db::{IndexEntry, Root};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct IndexStats {
    pub directories: usize,
    pub entries: usize,
}

struct DirScan {
    path: PathBuf,
    mtime_ns: i64,
    entries: Vec<IndexEntry>,
}

pub fn schedule_index(
    state: Arc<AppState>,
    user_id: i64,
    root: Root,
    relative: String,
    recursive: bool,
    key: String,
) {
    if !state.index_jobs.lock().insert(key.clone()) {
        return;
    }
    tokio::spawn(async move {
        let result = index_path(&state, user_id, root, &relative, recursive).await;
        if let Err(error) = &result {
            tracing::warn!(error = %error, path = %relative, "background index failed");
        }
        state.index_jobs.lock().remove(&key);
    });
}

pub async fn index_path(
    state: &Arc<AppState>,
    user_id: i64,
    root: Root,
    relative: &str,
    recursive: bool,
) -> Result<IndexStats, AppError> {
    let resolved = authz::resolve(state, user_id, root.id, relative, false).await?;
    let base = resolved.absolute.clone();

    let scans = tokio::task::spawn_blocking(move || collect_dirs(&base, recursive))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("index task failed: {err}")))??;

    let mut stats = IndexStats {
        directories: 0,
        entries: 0,
    };

    for scan in scans {
        let path = scan.path.to_string_lossy().to_string();
        let dir_id = state
            .db
            .upsert_index_dir(&path, scan.mtime_ns)
            .await
            .map_err(AppError::internal)?;

        let count = match state.db.replace_index_entries(dir_id, scan.entries).await {
            Ok(count) => {
                state
                    .db
                    .finish_index_dir(dir_id, "ready", count as i64, None)
                    .await
                    .map_err(AppError::internal)?;
                count
            }
            Err(err) => {
                let _ = state
                    .db
                    .finish_index_dir(dir_id, "error", 0, Some(&err.to_string()))
                    .await;
                return Err(AppError::internal(err));
            }
        };

        stats.directories += 1;
        stats.entries += count;
    }

    Ok(stats)
}

#[cfg(not(windows))]
fn collect_dirs(base: &Path, recursive: bool) -> Result<Vec<DirScan>, AppError> {
    collect_dirs_std(base, recursive)
}

#[cfg(windows)]
fn collect_dirs(base: &Path, recursive: bool) -> Result<Vec<DirScan>, AppError> {
    if recursive {
        match crate::winindex::scan(base, true) {
            Ok(scans) => {
                return Ok(scans
                    .into_iter()
                    .map(|scan| {
                        let mtime_ns = crate::fsops::dir_mtime_ns(&scan.path).unwrap_or_default();
                        DirScan {
                            path: scan.path,
                            mtime_ns,
                            entries: scan.entries,
                        }
                    })
                    .collect());
            }
            Err(error) => {
                tracing::warn!(%error, path = %base.display(), "MFT index failed; falling back to directory scan");
            }
        }
    }
    collect_dirs_std(base, recursive)
}

fn collect_dirs_std(base: &Path, recursive: bool) -> Result<Vec<DirScan>, AppError> {
    let mut scans = Vec::new();
    collect_one(base, recursive, &mut scans)?;
    Ok(scans)
}

fn collect_one(path: &Path, recursive: bool, scans: &mut Vec<DirScan>) -> Result<(), AppError> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if !metadata.is_dir() {
        return Err(AppError::bad_request("index path is not a directory"));
    }

    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or_default();

    let mut entries = Vec::new();
    let mut child_dirs = Vec::new();

    for entry in std::fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".filehub-upload-") {
            continue;
        }

        let file_type = entry.file_type().map_err(io_error)?;
        if file_type.is_symlink() {
            continue;
        }

        let metadata = entry.metadata().map_err(io_error)?;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();

        entries.push(IndexEntry {
            name,
            real_path: entry.path().to_string_lossy().to_string(),
            is_dir: file_type.is_dir(),
            size: if file_type.is_file() {
                metadata.len() as i64
            } else {
                0
            },
            mtime,
        });

        if recursive && file_type.is_dir() {
            child_dirs.push(entry.path());
        }
    }

    scans.push(DirScan {
        path: path.to_path_buf(),
        mtime_ns,
        entries,
    });

    for child in child_dirs {
        collect_one(&child, true, scans)?;
    }

    Ok(())
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::internal(anyhow::Error::from(error))
}
