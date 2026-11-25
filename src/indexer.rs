use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::authz;
use crate::db::{IndexEntry, Root};
use crate::error::AppError;
use crate::jobs::{JobKind, JobOutcome};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct IndexStats {
    pub directories: usize,
    pub entries: usize,
    pub partial: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct IndexTarget {
    pub root: Root,
    pub relative: String,
    pub covered_by: Option<String>,
}

struct ScanOutcome {
    removed: Vec<PathBuf>,
    usn: Option<crate::db::UsnState>,
}

struct DirScan {
    path: PathBuf,
    mtime_ns: i64,
    entries: Vec<IndexEntry>,
    partial: bool,
    error: Option<String>,
}

pub async fn resolve_index_target(
    state: &Arc<AppState>,
    user_id: i64,
    root: Root,
    relative: &str,
) -> Result<IndexTarget, AppError> {
    let resolved = authz::resolve(state, user_id, root.id, relative, false).await?;
    let roots = state
        .db
        .list_roots(user_id)
        .await
        .map_err(AppError::internal)?;
    let mut covering: Option<&Root> = None;
    for other in &roots {
        if other.id == root.id {
            continue;
        }
        if crate::platform::path_within(&resolved.absolute, Path::new(&other.real_path))
            && covering.is_none_or(|current| other.real_path.len() < current.real_path.len())
        {
            covering = Some(other);
        }
    }
    if let Some(covering) = covering {
        return Ok(IndexTarget {
            root: covering.clone(),
            relative: String::new(),
            covered_by: Some(covering.alias.clone()),
        });
    }
    Ok(IndexTarget {
        root,
        relative: resolved.relative,
        covered_by: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn index_path(
    state: &Arc<AppState>,
    user_id: i64,
    root: Root,
    relative: &str,
    recursive: bool,
    cancel: &CancellationToken,
    progress: &mut (dyn FnMut(usize, usize) + Send),
) -> Result<IndexStats, AppError> {
    let resolved = authz::resolve(state, user_id, root.id, relative, false).await?;
    let base = resolved.absolute.clone();
    let settings = state.settings();
    let ignore = settings.settings.index.ignore.clone();
    let video_enabled = settings.settings.thumbnails.enabled
        && settings.settings.thumbnails.video
        && state.ffmpeg.is_some();
    let usn_states = state.db.usn_states().await.unwrap_or_default();
    let scan_limits = ScanLimits {
        follow_symlinks: settings.settings.symlinks.follow_in_index,
        max_depth: settings.settings.symlinks.max_depth,
        max_entries: settings.settings.index.max_entries_per_dir,
    };
    let scan_roots: Vec<String> = state
        .db
        .list_roots(user_id)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .map(|root| root.real_path)
        .collect();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let scan_cancel = cancel.clone();
    let scan_base = base.clone();
    let task = tokio::task::spawn_blocking(move || {
        collect_dirs_channel(
            &scan_base,
            recursive,
            &ignore,
            &scan_cancel,
            sender,
            &usn_states,
            &scan_limits,
            &scan_roots,
        )
    });

    let mut stats = IndexStats {
        directories: 0,
        entries: 0,
        partial: !recursive,
        errors: Vec::new(),
    };
    let mut totals: HashMap<String, u64> = HashMap::new();

    let mut cancelled = false;
    while let Some(mut scan) = receiver.recv().await {
        enrich_entries(&mut scan.entries, video_enabled);
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let path = scan.path.to_string_lossy().to_string();
        let dir_id = state
            .db
            .upsert_index_dir(&path, scan.mtime_ns)
            .await
            .map_err(AppError::internal)?;

        let mut total_size = 0_u64;
        let mut partial = scan.partial || !recursive;
        let mut child_dirs = Vec::new();
        for entry in &scan.entries {
            if entry.is_dir {
                child_dirs.push(entry.real_path.clone());
            } else {
                total_size = total_size.saturating_add(entry.size.max(0) as u64);
            }
        }
        let count = match state.db.replace_index_entries(dir_id, scan.entries).await {
            Ok(count) => count,
            Err(error) => {
                stats.partial = true;
                if stats.errors.len() < 20 {
                    stats.errors.push(format!("{path}: {error}"));
                }
                let _ = state
                    .db
                    .finish_index_dir(dir_id, "error", 0, 0, true, Some(&error.to_string()))
                    .await;
                continue;
            }
        };
        let child_sizes = if child_dirs.is_empty() {
            std::collections::HashMap::new()
        } else {
            state
                .db
                .directory_size_values(child_dirs.clone())
                .await
                .unwrap_or_default()
        };
        for child in child_dirs {
            match totals.remove(&child) {
                Some(child_size) => total_size = total_size.saturating_add(child_size),
                None => {
                    // Quick scans index one directory at a time, so child
                    // totals are usually not in `totals`. Reuse the child's
                    // cached directory size instead of marking the parent
                    // permanently partial and forcing a full rescan.
                    if let Some((status, child_size)) = child_sizes.get(&child)
                        && matches!(status.as_str(), "ready" | "stale")
                    {
                        total_size = total_size.saturating_add((*child_size).max(0) as u64);
                    } else {
                        partial = true;
                    }
                }
            }
        }
        let _ = state
            .db
            .finish_index_dir(
                dir_id,
                "ready",
                count as i64,
                total_size.min(i64::MAX as u64) as i64,
                partial,
                scan.error.as_deref(),
            )
            .await;
        totals.insert(path, total_size);
        stats.directories += 1;
        stats.entries += count;
        if partial {
            stats.partial = true;
        }
        if let Some(error) = scan.error
            && stats.errors.len() < 20
        {
            stats.errors.push(error);
        }
        progress(stats.directories, stats.entries);
    }

    drop(receiver);
    let scan_result = task
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("index task failed: {err}")))?;
    if cancelled {
        return Err(AppError::Cancelled);
    }
    let outcome = scan_result?;
    let removed: Vec<PathBuf> = outcome.removed.iter().map(PathBuf::from).collect();
    if !removed.is_empty() {
        state.invalidate_paths_exact(&removed, None).await;
        state.schedule_cache_cleanup(removed);
    }
    if let Some(usn) = outcome.usn {
        let _ = state.db.upsert_usn_state(&usn).await;
    }

    let mut current = base.parent().map(Path::to_path_buf);
    while let Some(path) = current {
        let value = path.to_string_lossy().to_string();
        let _ = state.db.mark_index_partial(&value).await;
        let _ = state.db.mark_directory_size_stale(&value).await;
        current = path.parent().map(Path::to_path_buf);
    }
    state.schedule_sizes_stale(base.clone());

    Ok(stats)
}

#[derive(Debug, Clone, Serialize)]
pub struct QuickScanStats {
    pub checked: usize,
    pub changed: usize,
    pub missing: usize,
    pub entries: usize,
    pub new_directories: usize,
    pub errors: Vec<String>,
}

#[derive(Clone)]
struct ScanLimits {
    follow_symlinks: bool,
    max_depth: usize,
    max_entries: usize,
}

struct QuickScanChange {
    path: String,
    mtime_ns: i64,
    entries: Vec<IndexEntry>,
    child_dirs: Vec<PathBuf>,
    partial: bool,
    error: Option<String>,
}

struct QuickScanBatch {
    checked: usize,
    changes: Vec<QuickScanChange>,
    missing: Vec<String>,
    errors: Vec<String>,
}

fn quick_scan_batch(
    paths: Vec<(String, i64, String)>,
    roots: &[String],
    ignore: &[String],
    cancel: &CancellationToken,
    video_enabled: bool,
    limits: &ScanLimits,
) -> Result<QuickScanBatch, AppError> {
    let mut batch = QuickScanBatch {
        checked: 0,
        changes: Vec::new(),
        missing: Vec::new(),
        errors: Vec::new(),
    };
    for (path, database_mtime, status) in paths {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        batch.checked += 1;
        if !roots
            .iter()
            .any(|root| crate::platform::path_within(Path::new(&path), Path::new(root)))
        {
            batch.missing.push(path);
            continue;
        }
        let real_path = Path::new(&path);
        let metadata = match std::fs::symlink_metadata(real_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                batch.missing.push(path);
                continue;
            }
            Err(error) => {
                if batch.errors.len() < 20 {
                    batch.errors.push(format!("{path}: {error}"));
                }
                continue;
            }
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            batch.missing.push(path);
            continue;
        }
        let mtime_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos() as i64)
            .unwrap_or_default();
        if mtime_ns == database_mtime && status == "ready" {
            continue;
        }
        match read_directory_contents(real_path, ignore, cancel, limits, roots, 0) {
            Ok(mut contents) => {
                enrich_entries(&mut contents.entries, video_enabled);
                batch.changes.push(QuickScanChange {
                    path,
                    mtime_ns: contents.mtime_ns,
                    entries: contents.entries,
                    child_dirs: contents.child_dirs,
                    partial: contents.partial,
                    error: (!contents.errors.is_empty()).then(|| contents.errors.join("; ")),
                });
            }
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                if batch.errors.len() < 20 {
                    batch.errors.push(format!("{path}: {error}"));
                }
            }
        }
    }
    Ok(batch)
}

pub async fn quick_scan(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    progress: &mut (dyn FnMut(usize, usize) + Send),
) -> Result<QuickScanStats, AppError> {
    let settings = state.settings();
    let ignore = settings.settings.index.ignore.clone();
    let video_enabled = settings.settings.thumbnails.enabled
        && settings.settings.thumbnails.video
        && state.ffmpeg.is_some();
    let users = state.db.list_users().await.map_err(AppError::internal)?;
    let mut root_entries: Vec<(i64, Root)> = Vec::new();
    for user in users {
        for root in state
            .db
            .list_roots(user.id)
            .await
            .map_err(AppError::internal)?
        {
            root_entries.push((user.id, root));
        }
    }
    let roots: Vec<String> = root_entries
        .iter()
        .map(|(_, root)| root.real_path.clone())
        .collect();
    let total = state
        .db
        .count_indexed_dirs()
        .await
        .map_err(AppError::internal)? as usize;
    let mut stats = QuickScanStats {
        checked: 0,
        changed: 0,
        missing: 0,
        entries: 0,
        new_directories: 0,
        errors: Vec::new(),
    };
    let mut after: Option<String> = None;
    let mut parents: HashSet<PathBuf> = HashSet::new();
    let mut new_dir_candidates: HashSet<String> = HashSet::new();
    match state
        .db
        .take_pending_index_dirs(QUICK_SCAN_CHILD_LIMIT as i64)
        .await
    {
        Ok(pending) if !pending.is_empty() => {
            let checked = tokio::task::spawn_blocking(move || {
                pending
                    .into_iter()
                    .filter(|path| Path::new(path).is_dir())
                    .collect::<Vec<_>>()
            })
            .await;
            match checked {
                Ok(paths) => new_dir_candidates.extend(paths),
                Err(error) => {
                    tracing::warn!(%error, "failed to validate deferred index directories");
                }
            }
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "failed to load deferred index directories"),
    }

    loop {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let rows = state
            .db
            .indexed_dirs_page(after.clone(), 2000)
            .await
            .map_err(AppError::internal)?;
        if rows.is_empty() {
            break;
        }
        after = rows.last().map(|row| row.0.clone());
        let paths: Vec<(String, i64, String)> = rows
            .into_iter()
            .map(|(path, _id, mtime, status)| (path, mtime, status))
            .collect();
        let workers = state.settings().settings.index.scan_workers.clamp(1, 16);
        let scan_limits = Arc::new(ScanLimits {
            follow_symlinks: state.settings().settings.symlinks.follow_in_index,
            max_depth: state.settings().settings.symlinks.max_depth,
            max_entries: state.settings().settings.index.max_entries_per_dir,
        });
        let chunk_size = 250_usize;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(workers));
        let batch_roots = Arc::new(roots.clone());
        let batch_ignore = Arc::new(ignore.clone());
        for group in paths.chunks(workers.saturating_mul(chunk_size)) {
            let mut handles = Vec::new();
            for chunk in group.chunks(chunk_size) {
                let permit = Arc::clone(&semaphore)
                    .acquire_owned()
                    .await
                    .map_err(|_| AppError::internal(anyhow::anyhow!("index scanner closed")))?;
                let roots = Arc::clone(&batch_roots);
                let ignore = Arc::clone(&batch_ignore);
                let cancel = cancel.clone();
                let chunk = chunk.to_vec();
                let limits = Arc::clone(&scan_limits);
                handles.push(tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    quick_scan_batch(chunk, &roots, &ignore, &cancel, video_enabled, &limits)
                }));
            }
            for handle in handles {
                let batch = handle.await.map_err(|error| {
                    AppError::internal(anyhow::anyhow!("quick index scan task failed: {error}"))
                })??;
                apply_quick_scan_batch(
                    state,
                    batch,
                    &roots,
                    &mut stats,
                    &mut parents,
                    &mut new_dir_candidates,
                    cancel,
                )
                .await?;
                progress(stats.checked, total);
            }
        }
    }

    for parent in parents {
        state.invalidate_index_totals(&parent).await;
    }

    if !new_dir_candidates.is_empty() {
        let candidates: Vec<String> = new_dir_candidates.into_iter().collect();
        queue_new_directory_indexes(
            state,
            &root_entries,
            candidates,
            cancel,
            &mut stats.new_directories,
        )
        .await?;
    }

    Ok(stats)
}

async fn apply_quick_scan_batch(
    state: &Arc<AppState>,
    batch: QuickScanBatch,
    roots: &[String],
    stats: &mut QuickScanStats,
    parents: &mut HashSet<PathBuf>,
    new_dir_candidates: &mut HashSet<String>,
    cancel: &CancellationToken,
) -> Result<(), AppError> {
    stats.checked += batch.checked;
    for error in batch.errors {
        if stats.errors.len() < 20 {
            stats.errors.push(error);
        }
    }

    for change in batch.changes {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let dir_id = state
            .db
            .upsert_index_dir(&change.path, change.mtime_ns)
            .await
            .map_err(AppError::internal)?;
        let direct_size: u64 = change
            .entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.size.max(0) as u64)
            .sum();
        let child_paths: Vec<String> = change
            .child_dirs
            .iter()
            .map(|child| child.to_string_lossy().to_string())
            .collect();
        let child_sizes = if child_paths.is_empty() {
            std::collections::HashMap::new()
        } else {
            state
                .db
                .directory_size_values(child_paths)
                .await
                .unwrap_or_default()
        };
        let mut total_size = direct_size;
        let mut partial = change.partial;
        for child in &change.child_dirs {
            let child = child.to_string_lossy().to_string();
            if let Some((status, child_size)) = child_sizes.get(&child)
                && matches!(status.as_str(), "ready" | "stale")
            {
                total_size = total_size.saturating_add((*child_size).max(0) as u64);
            } else {
                partial = true;
            }
        }
        let count = state
            .db
            .replace_index_entries(dir_id, change.entries)
            .await
            .map_err(AppError::internal)?;
        let _ = state
            .db
            .finish_index_dir(
                dir_id,
                "ready",
                count as i64,
                total_size.min(i64::MAX as u64) as i64,
                partial,
                change.error.as_deref(),
            )
            .await;
        let _ = state.db.mark_directory_size_stale(&change.path).await;
        for child in change.child_dirs {
            new_dir_candidates.insert(child.to_string_lossy().to_string());
        }
        if let Some(parent) = Path::new(&change.path).parent() {
            parents.insert(parent.to_path_buf());
        }
        stats.changed += 1;
        stats.entries += count;
    }
    let mut missing_targets = Vec::with_capacity(batch.missing.len());
    for missing in batch.missing {
        if let Some(parent) = Path::new(&missing).parent() {
            parents.insert(parent.to_path_buf());
        }
        missing_targets.push(PathBuf::from(missing));
        stats.missing += 1;
    }
    if !missing_targets.is_empty() {
        let mut stale_only = Vec::new();
        // If the root itself is reachable, a vanished parent means the
        // directory tree was deleted or moved, not that a disk is offline.
        let reachable: Vec<&String> = roots
            .iter()
            .filter(|root| std::path::Path::new(root).is_dir())
            .collect();
        for target in &missing_targets {
            let parent_exists = target.parent().is_some_and(|parent| parent.is_dir());
            let remove = if parent_exists {
                match tokio::fs::metadata(target).await {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                    Ok(metadata) if !metadata.is_dir() => true,
                    _ => false,
                }
            } else {
                reachable
                    .iter()
                    .any(|root| crate::platform::path_within(target, std::path::Path::new(root)))
            };
            if remove {
                let prefix = target.to_string_lossy().to_string();
                loop {
                    match state.db.delete_cached_subtree_batch(&prefix, 2000).await {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(%error, path = %prefix, "failed to purge missing index subtree");
                            break;
                        }
                    }
                }
                let _ = state.db.delete_pending_subtree(&prefix).await;
            } else {
                stale_only.push(target.clone());
            }
        }
        if !stale_only.is_empty() {
            state.invalidate_paths_exact(&stale_only, None).await;
        }
        state.schedule_cache_cleanup(missing_targets);
    }
    Ok(())
}

const QUICK_SCAN_CHILD_LIMIT: usize = 500;

async fn queue_new_directory_indexes(
    state: &Arc<AppState>,
    root_entries: &[(i64, Root)],
    candidates: Vec<String>,
    cancel: &CancellationToken,
    new_directories: &mut usize,
) -> Result<(), AppError> {
    if candidates.is_empty() {
        return Ok(());
    }
    let existing = state
        .db
        .existing_index_dirs(candidates.clone())
        .await
        .map_err(AppError::internal)?;
    let mut new_dirs: Vec<String> = candidates
        .into_iter()
        .filter(|path| !existing.contains(path))
        .collect();
    new_dirs.sort();
    let mut queued = 0_usize;
    let mut cursor = 0_usize;
    while cursor < new_dirs.len() {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if queued >= QUICK_SCAN_CHILD_LIMIT {
            break;
        }
        let path = new_dirs[cursor].clone();
        let Some((user_id, root, relative)) = best_root_for_path(root_entries, &path) else {
            cursor += 1;
            continue;
        };
        let key = format!("dir:{path}");
        if state.jobs.active_job_exact(JobKind::Index, &key).is_some()
            || state
                .jobs
                .active_job_for_path(JobKind::Index, &path)
                .is_some()
        {
            cursor += 1;
            continue;
        }
        let job_state = Arc::clone(state);
        let job_root = root.clone();
        let job_relative = relative.clone();
        let result = state
            .jobs
            .submit(
                JobKind::Index,
                key,
                user_id,
                serde_json::json!({
                    "root_id": root.id,
                    "path": relative,
                    "real_path": path,
                    "recursive": true,
                    "auto": "quick-scan",
                }),
                1,
                false,
                move |ctx| async move {
                    let cancel = ctx.token();
                    let mut progress = |directories: usize, _entries: usize| {
                        ctx.report(directories, 1, 0, 0);
                    };
                    let stats = index_path(
                        &job_state,
                        user_id,
                        job_root,
                        &job_relative,
                        true,
                        &cancel,
                        &mut progress,
                    )
                    .await?;
                    let directories = stats.directories;
                    Ok(JobOutcome::done(
                        serde_json::json!(stats),
                        directories,
                        directories.max(1),
                    ))
                },
            )
            .await;
        match result {
            Ok(outcome) => {
                if outcome.created {
                    *new_directories += 1;
                }
                cursor += 1;
                queued += 1;
            }
            Err(AppError::QueueFull) => break,
            Err(error) => {
                tracing::debug!(%error, path, "failed to queue quick-scan child index");
                break;
            }
        }
    }

    if cursor < new_dirs.len() {
        let deferred = &new_dirs[cursor..];
        match state.db.append_pending_index_dirs(deferred).await {
            Ok(inserted) => tracing::info!(
                inserted,
                remaining = deferred.len(),
                "deferred new directories to a later quick scan"
            ),
            Err(error) => tracing::warn!(%error, "failed to persist deferred index directories"),
        }
    }
    Ok(())
}

fn best_root_for_path(roots: &[(i64, Root)], path: &str) -> Option<(i64, Root, String)> {
    let path = Path::new(path);
    roots
        .iter()
        .filter(|(_, root)| crate::platform::path_within(path, Path::new(&root.real_path)))
        .max_by_key(|(_, root)| root.real_path.len())
        .and_then(|(user_id, root)| {
            let relative = path.strip_prefix(Path::new(&root.real_path)).ok()?;
            let relative = relative
                .components()
                .filter_map(|component| component.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join("/");
            Some((*user_id, root.clone(), relative))
        })
}

pub async fn scheduled_full_scan(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    progress: &mut (dyn FnMut(usize, usize) + Send),
) -> Result<IndexStats, AppError> {
    let users = state.db.list_users().await.map_err(AppError::internal)?;
    let mut candidates: Vec<(i64, Root)> = Vec::new();
    for user in users {
        for root in state
            .db
            .list_roots(user.id)
            .await
            .map_err(AppError::internal)?
        {
            candidates.push((user.id, root));
        }
    }
    candidates.sort_by_key(|(_, root)| root.real_path.len());
    let mut selected: Vec<(i64, Root)> = Vec::new();
    for (user_id, root) in candidates {
        let root_path = Path::new(&root.real_path);
        if selected.iter().any(|(_, existing)| {
            crate::platform::paths_equal(root_path, Path::new(&existing.real_path))
                || crate::platform::path_within(root_path, Path::new(&existing.real_path))
        }) {
            continue;
        }
        selected.push((user_id, root));
    }

    let total = selected.len();
    let mut stats = IndexStats {
        directories: 0,
        entries: 0,
        partial: false,
        errors: Vec::new(),
    };
    for (index, (user_id, root)) in selected.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        #[cfg(windows)]
        if let Ok(volume) = crate::winindex::volume_name(Path::new(&root.real_path)) {
            let _ = state.db.delete_usn_state(&volume).await;
        }
        progress(index, total);
        let mut root_progress = |_directories: usize, _entries: usize| {
            progress(index, total);
        };
        match index_path(
            state,
            user_id,
            root.clone(),
            "",
            true,
            cancel,
            &mut root_progress,
        )
        .await
        {
            Ok(result) => {
                stats.directories += result.directories;
                stats.entries += result.entries;
                stats.partial |= result.partial;
                for error in result.errors {
                    if stats.errors.len() < 20 {
                        stats.errors.push(error);
                    }
                }
            }
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                stats.partial = true;
                if stats.errors.len() < 20 {
                    stats.errors.push(format!("{}: {error}", root.real_path));
                }
            }
        }
    }
    progress(total, total);
    Ok(stats)
}

fn enrich_entries(entries: &mut [IndexEntry], video_enabled: bool) {
    for entry in entries {
        entry.mime_type = mime_guess::from_path(&entry.name)
            .first_or_octet_stream()
            .to_string();
        entry.is_hidden = entry.name.starts_with('.');
        entry.has_preview = !entry.is_dir && crate::thumbs::can_preview(&entry.name, video_enabled);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_dirs_channel(
    base: &Path,
    recursive: bool,
    ignore: &[String],
    cancel: &CancellationToken,
    sender: tokio::sync::mpsc::Sender<DirScan>,
    usn_states: &[crate::db::UsnState],
    limits: &ScanLimits,
    roots: &[String],
) -> Result<ScanOutcome, AppError> {
    let mut visited: HashSet<(i64, i64)> = HashSet::new();
    #[cfg(windows)]
    if recursive {
        let volume = crate::winindex::volume_name(base).ok();
        let previous = volume
            .as_deref()
            .and_then(|volume| usn_states.iter().find(|state| state.volume == volume));
        match crate::winindex::incremental(base, previous, ignore) {
            Ok(Some(changes)) => {
                tracing::debug!(
                    removed = changes.removed.len(),
                    changed_dirs = changes.scans.len(),
                    new_dirs = changes.new_dirs.len(),
                    "USN incremental index applied"
                );
                let mut complete = changes.complete;
                for scan in changes.scans {
                    if cancel.is_cancelled() {
                        return Err(AppError::Cancelled);
                    }
                    let mtime_ns = crate::fsops::dir_mtime_ns(&scan.path).unwrap_or_default();
                    if sender
                        .blocking_send(DirScan {
                            path: scan.path,
                            mtime_ns,
                            entries: scan.entries,
                            partial: false,
                            error: None,
                        })
                        .is_err()
                    {
                        return Ok(ScanOutcome {
                            removed: changes.removed,
                            usn: None,
                        });
                    }
                }
                for new_dir in &changes.new_dirs {
                    match scan_walk(
                        new_dir,
                        true,
                        ignore,
                        cancel,
                        &sender,
                        limits,
                        roots,
                        0,
                        &mut visited,
                    ) {
                        Ok(_) => {}
                        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                        Err(error) => {
                            complete = false;
                            tracing::warn!(%error, path = %new_dir.display(), "failed to index new USN directory");
                        }
                    }
                }
                return Ok(ScanOutcome {
                    removed: changes.removed,
                    usn: complete.then_some(changes.state),
                });
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, path = %base.display(), "USN incremental index failed; falling back to MFT");
            }
        }

        let journal_before = crate::winindex::journal_state(base).ok();
        match crate::winindex::scan(base, true) {
            Ok(mut scans) => {
                scans.sort_by_key(|scan| std::cmp::Reverse(scan.path.components().count()));
                for scan in scans {
                    if cancel.is_cancelled() {
                        return Err(AppError::Cancelled);
                    }
                    let mtime_ns = crate::fsops::dir_mtime_ns(&scan.path).unwrap_or_default();
                    if sender
                        .blocking_send(DirScan {
                            path: scan.path,
                            mtime_ns,
                            entries: scan.entries,
                            partial: false,
                            error: None,
                        })
                        .is_err()
                    {
                        return Ok(ScanOutcome {
                            removed: Vec::new(),
                            usn: None,
                        });
                    }
                }
                return Ok(ScanOutcome {
                    removed: Vec::new(),
                    usn: journal_before,
                });
            }
            Err(error) => {
                tracing::warn!(%error, path = %base.display(), "MFT index failed; falling back to directory scan");
            }
        }
    }
    let _ = usn_states;
    scan_walk(
        base,
        recursive,
        ignore,
        cancel,
        &sender,
        limits,
        roots,
        0,
        &mut visited,
    )?;
    Ok(ScanOutcome {
        removed: Vec::new(),
        usn: None,
    })
}

struct DirectoryContents {
    mtime_ns: i64,
    entries: Vec<IndexEntry>,
    child_dirs: Vec<PathBuf>,
    partial: bool,
    errors: Vec<String>,
}

fn read_directory_contents(
    path: &Path,
    ignore: &[String],
    cancel: &CancellationToken,
    limits: &ScanLimits,
    roots: &[String],
    depth: usize,
) -> Result<DirectoryContents, AppError> {
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
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
    let mut errors = Vec::new();
    let mut partial = false;

    let read_dir = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => return Err(io_error(error)),
    };
    let mut seen_entries = 0usize;
    for entry in read_dir {
        seen_entries += 1;
        if seen_entries > limits.max_entries {
            partial = true;
            if errors.len() < 10 {
                errors.push(format!(
                    "{}: entry limit {} reached",
                    path.display(),
                    limits.max_entries
                ));
            }
            break;
        }
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                partial = true;
                if errors.len() < 10 {
                    errors.push(format!("{}: {error}", path.display()));
                }
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with(".filehub-upload-")
            || matches!(
                name.as_str(),
                "$RECYCLE.BIN" | "System Volume Information" | "@eaDir"
            )
            || ignore.iter().any(|ignored| ignored == &name)
        {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                partial = true;
                if errors.len() < 10 {
                    errors.push(format!("{}: {error}", entry.path().display()));
                }
                continue;
            }
        };
        if file_type.is_symlink() {
            if !limits.follow_symlinks || depth >= limits.max_depth {
                continue;
            }
            let Ok(target) = std::fs::canonicalize(entry.path()) else {
                continue;
            };
            if !roots
                .iter()
                .any(|root| crate::platform::path_within(&target, std::path::Path::new(root)))
            {
                continue;
            }
            match std::fs::metadata(&target) {
                Ok(metadata) if metadata.is_dir() => child_dirs.push(target),
                _ => {}
            }
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                partial = true;
                if errors.len() < 10 {
                    errors.push(format!("{}: {error}", entry.path().display()));
                }
                continue;
            }
        };
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        let created_at = metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();

        entries.push(IndexEntry {
            name: name.clone(),
            real_path: entry.path().to_string_lossy().to_string(),
            is_dir: file_type.is_dir(),
            size: if file_type.is_file() {
                metadata.len().min(i64::MAX as u64) as i64
            } else {
                0
            },
            mtime,
            created_at,
            mime_type: String::new(),
            has_preview: false,
            is_hidden: false,
        });

        if file_type.is_dir() {
            child_dirs.push(entry.path());
        }
    }

    Ok(DirectoryContents {
        mtime_ns,
        entries,
        child_dirs,
        partial,
        errors,
    })
}

#[allow(clippy::too_many_arguments)]
fn scan_walk(
    path: &Path,
    recursive: bool,
    ignore: &[String],
    cancel: &CancellationToken,
    sender: &tokio::sync::mpsc::Sender<DirScan>,
    limits: &ScanLimits,
    roots: &[String],
    depth: usize,
    visited: &mut HashSet<(i64, i64)>,
) -> Result<(bool, Vec<String>), AppError> {
    if depth > limits.max_depth {
        return Ok((
            true,
            vec![format!(
                "{}: max depth {} reached",
                path.display(),
                limits.max_depth
            )],
        ));
    }
    if let Ok(metadata) = std::fs::metadata(path) {
        let identity = crate::platform::metadata_identity(&metadata);
        if !visited.insert(identity) {
            return Ok((
                true,
                vec![format!("{}: symlink loop skipped", path.display())],
            ));
        }
    }
    let DirectoryContents {
        mtime_ns,
        entries,
        child_dirs,
        mut partial,
        mut errors,
    } = read_directory_contents(path, ignore, cancel, limits, roots, depth)?;

    if recursive {
        for child in child_dirs {
            match scan_walk(
                &child,
                true,
                ignore,
                cancel,
                sender,
                limits,
                roots,
                depth + 1,
                visited,
            ) {
                Ok((child_partial, child_errors)) => {
                    partial |= child_partial;
                    for error in child_errors {
                        if errors.len() < 10 {
                            errors.push(error);
                        }
                    }
                }
                Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                Err(error) => {
                    partial = true;
                    if errors.len() < 10 {
                        errors.push(format!("{}: {error}", child.display()));
                    }
                }
            }
        }
    }

    if sender
        .blocking_send(DirScan {
            path: path.to_path_buf(),
            mtime_ns,
            entries,
            partial,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
        })
        .is_err()
    {
        return Ok((partial, errors));
    }

    Ok((partial, errors))
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::internal(anyhow::Error::from(error))
}

#[cfg(all(test, unix))]
mod scan_limit_tests {
    use super::{ScanLimits, read_directory_contents};
    use std::os::unix::fs::symlink;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn follows_symlinks_only_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = root.join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file.txt"), b"x").unwrap();
        symlink(&target, root.join("link")).unwrap();

        let roots = vec![root.to_string_lossy().to_string()];
        let cancel = CancellationToken::new();
        let enabled = ScanLimits {
            follow_symlinks: true,
            max_depth: 8,
            max_entries: 100,
        };
        let contents = read_directory_contents(root, &[], &cancel, &enabled, &roots, 0).unwrap();
        // The real directory plus the followed symlink both resolve to `target`.
        assert_eq!(
            contents
                .child_dirs
                .iter()
                .filter(|path| path.ends_with("target"))
                .count(),
            2
        );

        let disabled = ScanLimits {
            follow_symlinks: false,
            max_depth: 8,
            max_entries: 100,
        };
        let contents = read_directory_contents(root, &[], &cancel, &disabled, &roots, 0).unwrap();
        assert_eq!(
            contents
                .child_dirs
                .iter()
                .filter(|path| path.ends_with("target"))
                .count(),
            1
        );
    }

    #[test]
    fn caps_entries_per_directory() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..5 {
            std::fs::write(dir.path().join(format!("file-{index}.txt")), b"x").unwrap();
        }
        let limits = ScanLimits {
            follow_symlinks: false,
            max_depth: 8,
            max_entries: 2,
        };
        let contents =
            read_directory_contents(dir.path(), &[], &CancellationToken::new(), &limits, &[], 0)
                .unwrap();
        assert_eq!(contents.entries.len(), 2);
        assert!(contents.partial);
    }
}
