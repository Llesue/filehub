use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zip::ZipArchive;

use crate::error::AppError;

#[derive(Debug, Clone, Copy)]
pub struct UnzipLimits {
    pub max_entries: usize,
    pub max_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_ratio: u64,
    pub ratio_check_bytes: u64,
}

impl UnzipLimits {
    pub fn from_settings(settings: &crate::config::DownloadSettings) -> Self {
        Self {
            max_entries: settings.unzip_max_entries,
            max_bytes: settings.unzip_max_bytes,
            max_entry_bytes: settings.unzip_max_entry_bytes,
            max_ratio: settings.unzip_max_ratio,
            ratio_check_bytes: settings.unzip_ratio_check_bytes,
        }
    }
}

#[derive(Debug)]
pub struct UnzipPlan {
    pub entries: usize,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct UnzipResult {
    pub entries: usize,
    pub bytes: u64,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::invalid_path(message.into())
}

fn entry_name(name: &str) -> Result<PathBuf, AppError> {
    if name.is_empty() || name.len() > 4096 || name.contains('\0') {
        return Err(invalid("archive entry has an invalid name"));
    }
    if name.starts_with('/') || name.starts_with('\\') || name.contains('\\') {
        return Err(invalid("archive entry uses an unsupported path"));
    }
    if name.as_bytes().get(1) == Some(&b':') {
        return Err(invalid("archive entry uses a drive prefix"));
    }
    let mut parts = Vec::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| invalid("archive entry has a non-UTF-8 path"))?;
                if part == "." || part == ".." {
                    return Err(invalid("archive entry escapes the target directory"));
                }
                parts.push(part.to_owned());
            }
            _ => return Err(invalid("archive entry escapes the target directory")),
        }
    }
    if parts.is_empty() {
        return Err(invalid("archive entry has an empty path"));
    }
    if parts.len() > 64 {
        return Err(invalid("archive entry is nested too deeply"));
    }
    Ok(parts.iter().collect())
}

fn normalized(name: &Path) -> String {
    name.to_string_lossy().replace('\\', "/")
}

/// Inspect the central directory before writing anything.
pub fn plan(archive: &Path, limits: UnzipLimits) -> Result<UnzipPlan, AppError> {
    let file = std::fs::File::open(archive).map_err(crate::fsops::io_error)?;
    let mut zip = ZipArchive::new(file)
        .map_err(|_| AppError::UnsupportedMedia("not a valid ZIP archive".to_owned()))?;
    if zip.len() > limits.max_entries.max(1) {
        return Err(AppError::TooLarge(
            "ZIP contains too many entries".to_owned(),
        ));
    }
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|_| AppError::UnsupportedMedia("invalid ZIP entry".to_owned()))?;
        if entry.encrypted() {
            return Err(AppError::UnsupportedMedia(
                "encrypted ZIP files are not supported".to_owned(),
            ));
        }
        if entry.is_symlink() {
            return Err(AppError::UnsupportedMedia(
                "ZIP symbolic links are not allowed".to_owned(),
            ));
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                return Err(AppError::UnsupportedMedia(
                    "ZIP contains a special file".to_owned(),
                ));
            }
        }
        let name = entry_name(
            &entry
                .name()
                .map_err(|_| AppError::UnsupportedMedia("invalid ZIP entry name".to_owned()))?,
        )?;
        let key = normalized(&name);
        if !names.insert(key) {
            return Err(AppError::UnsupportedMedia(
                "ZIP contains duplicate entries".to_owned(),
            ));
        }
        if entry.is_dir() {
            continue;
        }
        let size = entry.size();
        if size > limits.max_entry_bytes {
            return Err(AppError::TooLarge(
                "ZIP entry is larger than the configured limit".to_owned(),
            ));
        }
        let compressed = entry.compressed_size();
        if size >= limits.ratio_check_bytes
            && compressed > 0
            && size / compressed.max(1) > limits.max_ratio
        {
            return Err(AppError::TooLarge(
                "ZIP compression ratio is suspicious".to_owned(),
            ));
        }
        total = total.saturating_add(size);
        if total > limits.max_bytes {
            return Err(AppError::TooLarge(
                "ZIP expands beyond the configured limit".to_owned(),
            ));
        }
    }
    Ok(UnzipPlan {
        entries: zip.len(),
        bytes: total,
    })
}

/// Reject an existing target that would be overwritten or followed.
pub fn ensure_no_conflicts(archive: &Path, target: &Path) -> Result<(), AppError> {
    let file = std::fs::File::open(archive).map_err(crate::fsops::io_error)?;
    let mut zip = ZipArchive::new(file)
        .map_err(|_| AppError::UnsupportedMedia("not a valid ZIP archive".to_owned()))?;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|_| AppError::UnsupportedMedia("invalid ZIP entry".to_owned()))?;
        let name = entry_name(
            &entry
                .name()
                .map_err(|_| AppError::UnsupportedMedia("invalid ZIP entry name".to_owned()))?,
        )?;
        let path = target.join(&name);
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err(AppError::conflict(format!(
                "target already exists: {}",
                name.to_string_lossy()
            )));
        }
    }
    Ok(())
}

/// Extract into `target` (which the caller prepared and verified).
pub fn extract(
    archive: &Path,
    target: &Path,
    limits: UnzipLimits,
    cancel: &CancellationToken,
    progress: &mut dyn FnMut(usize, usize, u64),
) -> Result<UnzipResult, AppError> {
    let file = std::fs::File::open(archive).map_err(crate::fsops::io_error)?;
    let mut zip = ZipArchive::new(file)
        .map_err(|_| AppError::UnsupportedMedia("not a valid ZIP archive".to_owned()))?;
    let plan = plan(archive, limits)?;
    std::fs::create_dir_all(target).map_err(crate::fsops::io_error)?;
    let mut created_files = Vec::new();
    let mut created_dirs = Vec::new();
    let mut written_total = 0_u64;

    let result = (|| {
        let mut buffer = vec![0_u8; 256 * 1024];
        for index in 0..zip.len() {
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let mut entry = zip
                .by_index(index)
                .map_err(|_| AppError::UnsupportedMedia("invalid ZIP entry".to_owned()))?;
            let name =
                entry_name(&entry.name().map_err(|_| {
                    AppError::UnsupportedMedia("invalid ZIP entry name".to_owned())
                })?)?;
            let destination = target.join(&name);
            let parent = destination
                .parent()
                .ok_or_else(|| invalid("archive entry has no parent"))?;
            create_parents(target, parent, &mut created_dirs)?;
            if entry.is_dir() {
                if std::fs::symlink_metadata(&destination).is_ok() {
                    return Err(AppError::conflict(format!(
                        "target already exists: {}",
                        name.to_string_lossy()
                    )));
                }
                std::fs::create_dir(&destination).map_err(crate::fsops::io_error)?;
                created_dirs.push(destination);
                progress(index + 1, plan.entries, written_total);
                continue;
            }
            let path = destination.clone();
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            crate::platform::set_private_mode(&mut options);
            let mut output = options.open(&path).map_err(crate::fsops::io_error)?;
            created_files.push(path);
            let mut written = 0_u64;
            loop {
                if cancel.is_cancelled() {
                    return Err(AppError::Cancelled);
                }
                let read = entry.read(&mut buffer).map_err(crate::fsops::io_error)?;
                if read == 0 {
                    break;
                }
                written = written.saturating_add(read as u64);
                written_total = written_total.saturating_add(read as u64);
                if written > limits.max_entry_bytes
                    || written > entry.size()
                    || written_total > limits.max_bytes
                {
                    return Err(AppError::TooLarge(
                        "ZIP expands beyond the declared size".to_owned(),
                    ));
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(crate::fsops::io_error)?;
            }
            output.sync_all().map_err(crate::fsops::io_error)?;
            progress(index + 1, plan.entries, written_total);
        }
        Ok(())
    })();

    if let Err(error) = result {
        for file in created_files.iter().rev() {
            let _ = std::fs::remove_file(file);
        }
        for dir in created_dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
        return Err(error);
    }
    Ok(UnzipResult {
        entries: plan.entries,
        bytes: written_total,
    })
}

fn create_parents(root: &Path, parent: &Path, created: &mut Vec<PathBuf>) -> Result<(), AppError> {
    if parent == root {
        return Ok(());
    }
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| invalid("archive entry escapes the target directory"))?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err(invalid("archive entry escapes the target directory"));
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(AppError::invalid_path(
                        "archive target contains a symbolic link".to_owned(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                crate::platform::create_private_dir_all(&current)
                    .map_err(crate::fsops::io_error)?;
                created.push(current.clone());
            }
            Err(error) => return Err(crate::fsops::io_error(error)),
        }
    }
    Ok(())
}

pub fn job_result(target: &Path, relative: &str, result: &UnzipResult) -> Value {
    json!({
        "path": relative,
        "target": relative,
        "entries": result.entries,
        "bytes": result.bytes,
        "directory": target.to_string_lossy(),
    })
}

pub(crate) async fn submit_unzip_job(
    state: &std::sync::Arc<crate::state::AppState>,
    user_id: i64,
    root_id: i64,
    archive: String,
    target: String,
    existing: bool,
    name: String,
) -> Result<crate::jobs::SubmitOutcome, AppError> {
    let settings = state.settings();
    let limits = UnzipLimits::from_settings(&settings.settings.downloads);
    let payload = json!({
        "root": root_id,
        "archive": archive.clone(),
        "target": target.clone(),
        "existing": existing,
        "name": name.clone(),
    });
    let key = crate::auth::sha256_hex(&format!("{root_id}:{archive}:{target}:{existing}"));
    let job_state = std::sync::Arc::clone(state);
    let job_payload = payload.clone();
    state
        .jobs
        .submit(
            crate::jobs::JobKind::Unzip,
            key,
            user_id,
            payload,
            0,
            false,
            move |ctx| async move {
                run_unzip_job(job_state, user_id, job_payload, ctx, limits).await
            },
        )
        .await
}

async fn run_unzip_job(
    state: std::sync::Arc<crate::state::AppState>,
    user_id: i64,
    payload: Value,
    ctx: crate::jobs::JobContext,
    limits: UnzipLimits,
) -> Result<crate::jobs::JobOutcome, AppError> {
    let user = state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    let root_id = payload
        .get("root")
        .and_then(Value::as_i64)
        .ok_or_else(|| crate::error::AppError::internal(anyhow::anyhow!("missing root")))?;
    let archive_relative = payload
        .get("archive")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let target_relative = payload
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let existing = payload
        .get("existing")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let root = crate::cmdrunner::load_root(&state, user_id, root_id).await?;
    let root_path = std::fs::canonicalize(&root.real_path)
        .map_err(|err| AppError::internal(anyhow::anyhow!("invalid root: {err}")))?;
    let archive = crate::authz::resolve(&state, user_id, root.id, &archive_relative, false).await?;
    if !archive.absolute.is_file() {
        return Err(AppError::bad_request("archive is not a file"));
    }
    let target = crate::authz::resolve(&state, user_id, root.id, &target_relative, true).await?;
    let plan = {
        let archive = archive.absolute.clone();
        tokio::task::spawn_blocking(move || plan(&archive, limits))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("unzip plan failed: {err}")))?
    }?;
    crate::usage::ensure_entry_capacity(&state, &user, plan.entries as u64).await?;
    if let Some(quota_bytes) = user.quota_bytes
        && crate::platform::path_within(&target.absolute, std::path::Path::new(&user.home_dir))
    {
        let used = crate::usage::cached_or_scan(&state, &user).await?;
        if used.saturating_add(plan.bytes) > quota_bytes.max(0) as u64 {
            return Err(AppError::QuotaExceeded);
        }
    }

    let archive_path = archive.absolute.clone();
    let (final_target, summary) = if existing {
        let target_path = target.absolute.clone();
        let extract_target = target_path.clone();
        let archive_path = archive_path.clone();
        let cancel = ctx.token();
        let job_ctx = ctx.clone();
        let summary = tokio::task::spawn_blocking(move || {
            ensure_no_conflicts(&archive_path, &extract_target)?;
            extract(
                &archive_path,
                &extract_target,
                limits,
                &cancel,
                &mut |done, total, bytes| job_ctx.report(done, total, bytes, 0),
            )
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("unzip task failed: {err}")))??;
        (target_path, summary)
    } else {
        let parent = target
            .absolute
            .parent()
            .ok_or_else(|| AppError::invalid_path("target has no parent"))?
            .to_path_buf();
        let intended = crate::fsops::unique_target(&target.absolute);
        let temp = parent.join(format!(
            ".filehub-unzip-{}.part",
            crate::auth::random_token(8)
        ));
        let temp_create = temp.clone();
        tokio::task::spawn_blocking(move || crate::platform::create_private_dir_all(&temp_create))
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("unzip temp task failed: {err}")))?
            .map_err(crate::fsops::io_error)?;
        let archive_path = archive_path.clone();
        let temp_path = temp.clone();
        let cancel = ctx.token();
        let job_ctx = ctx.clone();
        let summary = tokio::task::spawn_blocking(move || {
            match extract(
                &archive_path,
                &temp_path,
                limits,
                &cancel,
                &mut |done, total, bytes| job_ctx.report(done, total, bytes, 0),
            ) {
                Ok(summary) => Ok(summary),
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&temp_path);
                    Err(error)
                }
            }
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("unzip task failed: {err}")))??;
        let rename_target = intended.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::rename(&temp, &rename_target)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &rename_target,
                    std::fs::Permissions::from_mode(0o755),
                );
            }
            Ok::<_, std::io::Error>(())
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("unzip rename failed: {err}")))?
        .map_err(crate::fsops::io_error)?;
        (intended, summary)
    };

    state.adjust_entries(user_id, summary.entries as i64);
    state.adjust_usage(user_id, summary.bytes as i64);
    state
        .invalidate_path_caches_with_usage(
            &final_target,
            Some(user_id),
            false,
            crate::state::UsageInvalidation::Delta(summary.bytes as i64),
        )
        .await;
    let relative = final_target
        .strip_prefix(&root_path)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| target_relative.clone());
    Ok(crate::jobs::JobOutcome {
        result: json!({
            "path": relative,
            "target": final_target.to_string_lossy(),
            "entries": summary.entries,
            "bytes": summary.bytes,
        }),
        progress: summary.entries,
        total: summary.entries,
        bytes_done: summary.bytes,
        bytes_total: summary.bytes,
    })
}
