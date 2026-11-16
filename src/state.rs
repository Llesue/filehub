use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::auth::{RateLimiter, random_token};
use crate::config::{ConfigFile, Snapshot, build_snapshot, ensure_directories, load_config_file};
use crate::db::{AuditRecord, Db, unix_now};
use crate::fsops::DiskUsage;
use crate::jobs::{JobKind, JobManager, JobOutcome};

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeleteItem {
    pub relative: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct DeletePreview {
    pub user_id: i64,
    pub root_id: i64,
    pub items: Vec<DeleteItem>,
    pub expires_at: i64,
    pub confirm_code: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageSnapshot {
    pub used_bytes: u64,
    pub quota_bytes: Option<i64>,
    pub disks: Vec<DiskUsage>,
    pub computed_at: i64,
}

pub enum AuditMessage {
    Record(AuditRecord),
    Flush(tokio::sync::oneshot::Sender<()>),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SlowRequest {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u128,
    pub request_id: Option<String>,
    pub at: i64,
}

pub struct ShareTicket {
    pub share_id: i64,
    pub expires_at: i64,
}

pub struct SetupState {
    pub initialized: AtomicBool,
    pub token: String,
}

#[derive(Default)]
pub struct WindowRateLimiter {
    windows: Mutex<HashMap<String, (i64, u32)>>,
}

impl WindowRateLimiter {
    pub fn allow(&self, key: &str, limit: u32, window_secs: i64) -> bool {
        if limit == 0 {
            return true;
        }
        let now = unix_now();
        let mut windows = self.windows.lock();
        if windows.len() > 10_000 {
            windows.retain(|_, (start, _)| now.saturating_sub(*start) < window_secs);
        }
        let entry = windows.entry(key.to_owned()).or_insert((now, 0));
        if now.saturating_sub(entry.0) >= window_secs {
            *entry = (now, 0);
        }
        entry.1 = entry.1.saturating_add(1);
        entry.1 <= limit
    }
}

pub struct AppState {
    pub db: Db,
    pub config: ConfigFile,
    snapshot: ArcSwap<Snapshot>,
    pub setup: SetupState,
    pub rate_limiter: RateLimiter,
    pub delete_tokens: Mutex<HashMap<String, DeletePreview>>,
    pub share_tickets: Mutex<HashMap<String, ShareTicket>>,
    pub ffmpeg: Option<PathBuf>,
    pub thumb_semaphore: Arc<Semaphore>,
    pub thumb_large_semaphore: Option<Arc<Semaphore>>,
    zip_semaphore: Mutex<Arc<Semaphore>>,
    pub jobs: Arc<JobManager>,
    pub audit_tx: tokio::sync::mpsc::Sender<AuditMessage>,
    pub usage_cache: Mutex<HashMap<i64, UsageSnapshot>>,
    pub usage_locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    pub public_rate: WindowRateLimiter,
    pub slow_requests: Mutex<VecDeque<SlowRequest>>,
}

impl AppState {
    pub async fn new(config_path: Option<&Path>) -> anyhow::Result<Arc<Self>> {
        Self::build(config_path, None).await
    }

    pub async fn new_test(base: &Path, setup_token: &str) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(base.join("users"))?;
        std::fs::create_dir_all(base.join("cache"))?;
        let config_path = base.join("config.json");
        let config = serde_json::json!({
            "listen": "127.0.0.1:0",
            "db_path": base.join("filehub.db").to_string_lossy(),
            "users_root": base.join("users").to_string_lossy(),
            "cache_dir": base.join("cache").to_string_lossy(),
            "security": {
                "session_ttl_hours": 1
            }
        });
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config)?)?;
        Self::build(Some(&config_path), Some(setup_token.to_owned())).await
    }

    async fn build(
        config_path: Option<&Path>,
        explicit_setup_token: Option<String>,
    ) -> anyhow::Result<Arc<Self>> {
        let config = load_config_file(config_path)?;
        let bootstrap = build_snapshot(&config, &[])?;
        ensure_directories(&bootstrap)?;

        let db = Db::open(&bootstrap.settings.db_path, 4)?;
        ensure_default_roots(&db).await?;
        let overrides = db.settings_overrides().await?;
        let snapshot = build_snapshot(&config, &overrides)?;
        ensure_directories(&snapshot)?;

        let initialized = db.count_users().await? > 0;
        let token = match explicit_setup_token {
            Some(token) => token,
            None => match std::env::var("FILEHUB_SETUP_TOKEN") {
                Ok(token) if !token.is_empty() => token,
                _ => {
                    let token = random_token(24);
                    if !initialized {
                        tracing::info!(setup_token = %token, "first-run setup token");
                    }
                    token
                }
            },
        };

        if initialized {
            tracing::info!("users already present; setup is disabled");
        }

        let thumb_workers = snapshot.settings.thumbnails.workers.max(1);
        let (thumb_small_workers, thumb_large_workers) = if thumb_workers <= 1 {
            (thumb_workers, None)
        } else {
            let large = if thumb_workers >= 3 { 2 } else { 1 };
            (thumb_workers - large, Some(large))
        };
        let ffmpeg = if snapshot.settings.thumbnails.video {
            crate::thumbs::detect_ffmpeg()
        } else {
            None
        };
        let jobs = JobManager::new(
            db.clone(),
            snapshot.settings.jobs.io_workers,
            snapshot.settings.jobs.queue_max,
        );
        jobs.recover().await?;
        let (audit_tx, audit_rx) = tokio::sync::mpsc::channel::<AuditMessage>(1024);
        spawn_audit_writer(db.clone(), audit_rx);
        let zip_permits = snapshot.settings.downloads.zip_concurrency.max(1);

        Ok(Arc::new(Self {
            db,
            config,
            snapshot: ArcSwap::from_pointee(snapshot),
            setup: SetupState {
                initialized: AtomicBool::new(initialized),
                token,
            },
            rate_limiter: RateLimiter::new(),
            delete_tokens: Mutex::new(HashMap::new()),
            share_tickets: Mutex::new(HashMap::new()),
            ffmpeg,
            thumb_semaphore: Arc::new(Semaphore::new(thumb_small_workers)),
            thumb_large_semaphore: thumb_large_workers
                .map(|workers| Arc::new(Semaphore::new(workers))),
            zip_semaphore: Mutex::new(Arc::new(Semaphore::new(zip_permits))),
            jobs,
            audit_tx,
            usage_cache: Mutex::new(HashMap::new()),
            usage_locks: Mutex::new(HashMap::new()),
            public_rate: WindowRateLimiter::default(),
            slow_requests: Mutex::new(VecDeque::new()),
        }))
    }

    pub fn settings(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }

    pub async fn reload_settings(&self) -> anyhow::Result<()> {
        let overrides = self.db.settings_overrides().await?;
        let snapshot = build_snapshot(&self.config, &overrides)?;
        ensure_directories(&snapshot)?;
        let permits = snapshot.settings.downloads.zip_concurrency.max(1);
        self.snapshot.store(Arc::new(snapshot));
        *self.zip_semaphore.lock() = Arc::new(Semaphore::new(permits));
        Ok(())
    }

    pub fn zip_semaphore(&self) -> Arc<Semaphore> {
        Arc::clone(&self.zip_semaphore.lock())
    }

    pub fn is_initialized(&self) -> bool {
        self.setup.initialized.load(Ordering::Relaxed)
    }

    pub async fn enqueue_audit(&self, record: AuditRecord) {
        if self
            .audit_tx
            .send(AuditMessage::Record(record))
            .await
            .is_err()
        {
            tracing::warn!("audit writer is closed; dropping audit record");
        }
    }

    pub async fn flush_audit(&self) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if self
            .audit_tx
            .send(AuditMessage::Flush(sender))
            .await
            .is_ok()
        {
            let _ = receiver.await;
        }
    }

    pub fn invalidate_usage(&self, user_id: i64) {
        self.usage_cache.lock().remove(&user_id);
    }

    pub async fn invalidate_path_caches(&self, target: &Path, user_id: Option<i64>, subtree: bool) {
        let mut current = Some(target.to_path_buf());
        let mut first = true;
        while let Some(path) = current {
            let value = path.to_string_lossy().to_string();
            if first && subtree {
                let _ = self.db.invalidate_directory_sizes(&value).await;
                let _ = self.db.invalidate_index_subtree(&value).await;
            } else {
                let _ = self.db.delete_directory_size_exact(&value).await;
            }
            let _ = self.db.mark_index_stale(&value).await;
            first = false;
            current = path.parent().map(Path::to_path_buf);
        }
        if let Some(user_id) = user_id {
            self.invalidate_usage(user_id);
        }
    }

    pub async fn invalidate_index_totals(&self, target: &Path) {
        let mut current = Some(target.to_path_buf());
        while let Some(path) = current {
            let value = path.to_string_lossy().to_string();
            let _ = self.db.mark_index_partial(&value).await;
            let _ = self.db.delete_directory_size_exact(&value).await;
            current = path.parent().map(Path::to_path_buf);
        }
    }

    pub fn push_slow_request(&self, request: SlowRequest) {
        let mut requests = self.slow_requests.lock();
        requests.push_back(request);
        while requests.len() > 100 {
            requests.pop_front();
        }
    }

    pub fn recent_slow_requests(&self) -> Vec<SlowRequest> {
        self.slow_requests.lock().iter().rev().cloned().collect()
    }

    pub fn start_background_tasks(self: &Arc<Self>) {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                state.run_maintenance().await;
            }
        });

        let scheduler = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                scheduler.schedule_index_scans().await;
            }
        });
    }

    async fn schedule_index_scans(self: &Arc<Self>) {
        let settings = self.settings();
        if !settings.settings.index.scheduled_scans {
            return;
        }
        let now = unix_now();
        let users = match self.db.list_users().await {
            Ok(users) => users,
            Err(error) => {
                tracing::warn!(%error, "failed to load users for index scheduler");
                return;
            }
        };
        let Some(admin) = users
            .iter()
            .find(|user| user.role == "admin" && !user.disabled)
        else {
            return;
        };
        let admin_id = admin.id;
        let quick_seconds = (settings
            .settings
            .index
            .quick_scan_minutes
            .min(10_080)
            .saturating_mul(60)) as i64;
        let full_seconds = (settings
            .settings
            .index
            .full_scan_hours
            .min(8_760)
            .saturating_mul(3600)) as i64;

        let last_quick = match self.db.runtime_state_get("index.last_quick_scan").await {
            Ok(Some(value)) => value,
            Ok(None) => {
                let _ = self
                    .db
                    .runtime_state_set("index.last_quick_scan", now)
                    .await;
                now
            }
            Err(error) => {
                tracing::warn!(%error, "failed to read quick scan state");
                return;
            }
        };
        if now.saturating_sub(last_quick) >= quick_seconds {
            let job_state = Arc::clone(self);
            let result = self
                .jobs
                .submit(
                    JobKind::Index,
                    "scheduled_quick_scan".to_owned(),
                    admin_id,
                    serde_json::json!({ "mode": "quick" }),
                    1,
                    move |ctx| async move {
                        let cancel = ctx.token();
                        let mut progress = |checked: usize, total: usize| {
                            ctx.report(checked, total, 0, 0);
                        };
                        let stats =
                            crate::indexer::quick_scan(&job_state, &cancel, &mut progress).await?;
                        let _ = job_state
                            .db
                            .runtime_state_set("index.last_quick_scan", unix_now())
                            .await;
                        let checked = stats.checked;
                        Ok(JobOutcome::done(
                            serde_json::json!(stats),
                            checked,
                            checked.max(1),
                        ))
                    },
                )
                .await;
            match result {
                Ok(outcome) if outcome.created => {
                    tracing::info!(job_id = %outcome.job.id, "scheduled quick index scan queued");
                }
                Ok(_) => {}
                Err(crate::error::AppError::QueueFull) => {
                    tracing::debug!("index queue full; quick scan will retry");
                }
                Err(error) => tracing::warn!(%error, "failed to queue quick index scan"),
            }
        }

        let last_full = match self.db.runtime_state_get("index.last_full_scan").await {
            Ok(Some(value)) => value,
            Ok(None) => {
                let _ = self.db.runtime_state_set("index.last_full_scan", now).await;
                now
            }
            Err(error) => {
                tracing::warn!(%error, "failed to read full scan state");
                return;
            }
        };
        if now.saturating_sub(last_full) >= full_seconds {
            let job_state = Arc::clone(self);
            let result = self
                .jobs
                .submit(
                    JobKind::Index,
                    "scheduled_full_scan".to_owned(),
                    admin_id,
                    serde_json::json!({ "mode": "full" }),
                    1,
                    move |ctx| async move {
                        let cancel = ctx.token();
                        let mut progress = |done: usize, total: usize| {
                            ctx.report(done, total.max(1), 0, 0);
                        };
                        let stats =
                            crate::indexer::scheduled_full_scan(&job_state, &cancel, &mut progress)
                                .await?;
                        let _ = job_state
                            .db
                            .runtime_state_set("index.last_full_scan", unix_now())
                            .await;
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
                Ok(outcome) if outcome.created => {
                    tracing::info!(job_id = %outcome.job.id, "scheduled full index scan queued");
                }
                Ok(_) => {}
                Err(crate::error::AppError::QueueFull) => {
                    tracing::debug!("index queue full; full scan will retry");
                }
                Err(error) => tracing::warn!(%error, "failed to queue full index scan"),
            }
        }
    }

    pub async fn run_maintenance(&self) {
        let settings = self.settings();
        let jobs = &settings.settings.jobs;
        let now = unix_now();
        match self
            .db
            .prune_audit(jobs.audit_retention_days, jobs.audit_max_rows)
            .await
        {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "pruned audit log"),
            Err(error) => tracing::warn!(%error, "failed to prune audit log"),
        }
        let job_cutoff = now - (jobs.job_retention_days.min(3650) as i64) * 86_400;
        match self.db.prune_jobs(job_cutoff).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "pruned finished jobs"),
            Err(error) => tracing::warn!(%error, "failed to prune jobs"),
        }
        match self.db.prune_directory_sizes(now - 30 * 86_400).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "pruned stale directory sizes"),
            Err(error) => tracing::warn!(%error, "failed to prune directory sizes"),
        }
        match self.db.prune_expired_sessions(now).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "pruned expired sessions"),
            Err(error) => tracing::warn!(%error, "failed to prune expired sessions"),
        }
        match self.db.prune_indexed_pending_dirs().await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "pruned indexed pending-directory candidates"),
            Err(error) => tracing::warn!(%error, "failed to prune pending index directories"),
        }
        match self.db.take_expired_upload_temps(now).await {
            Ok(paths) if !paths.is_empty() => {
                let mut removed = 0_usize;
                for path in &paths {
                    let temp = std::path::Path::new(path);
                    let expected = temp
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(".filehub-upload-"));
                    if expected && std::fs::remove_file(temp).is_ok() {
                        removed += 1;
                    }
                }
                tracing::info!(
                    sessions = paths.len(),
                    removed,
                    "pruned expired upload sessions"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "failed to prune expired upload sessions"),
        }
        match self.db.take_expired_share_upload_temps(now).await {
            Ok(paths) if !paths.is_empty() => {
                let mut removed = 0_usize;
                for path in &paths {
                    let temp = std::path::Path::new(path);
                    let expected = temp
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(".filehub-upload-"));
                    if expected && std::fs::remove_file(temp).is_ok() {
                        removed += 1;
                    }
                }
                tracing::info!(
                    sessions = paths.len(),
                    removed,
                    "pruned expired share upload sessions"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "failed to prune expired share upload sessions"),
        }
        self.cleanup_editor_temps(now).await;
        #[cfg(windows)]
        match crate::platform::prune_service_logs(settings.settings.log.retention_days) {
            0 => {}
            removed => tracing::info!(removed, "pruned old service logs"),
        }
        match self.db.checkpoint().await {
            Ok(true) => tracing::debug!("WAL checkpoint was busy; will retry next pass"),
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "WAL checkpoint failed"),
        }
        self.usage_cache
            .lock()
            .retain(|_, snapshot| now.saturating_sub(snapshot.computed_at) < 600);
    }

    /// Editor temp files only leak when the process dies mid-save, so a daily
    /// bounded walk is enough; it is not worth scanning the whole users tree
    /// every hour.
    async fn cleanup_editor_temps(&self, now: i64) {
        const INTERVAL: i64 = 24 * 3600;
        const MAX_DIRS: usize = 100_000;

        let last = match self
            .db
            .runtime_state_get("maintenance.edit_temp_cleanup_at")
            .await
        {
            Ok(Some(value)) => value,
            Ok(None) => 0,
            Err(error) => {
                tracing::warn!(%error, "failed to read edit temp cleanup state");
                return;
            }
        };
        if now.saturating_sub(last) < INTERVAL {
            return;
        }

        let users_root = self.settings().settings.users_root.clone();
        let cutoff = now - INTERVAL;
        let outcome = tokio::task::spawn_blocking(move || {
            crate::fsops::cleanup_edit_temp(&users_root, cutoff, MAX_DIRS)
        })
        .await;
        match outcome {
            Ok((removed, visited, truncated)) => {
                if removed > 0 || truncated {
                    tracing::info!(removed, visited, truncated, "cleaned editor temp files");
                }
                if let Err(error) = self
                    .db
                    .runtime_state_set("maintenance.edit_temp_cleanup_at", now)
                    .await
                {
                    tracing::warn!(%error, "failed to persist edit temp cleanup state");
                }
            }
            Err(error) => tracing::warn!(%error, "edit temp cleanup task failed"),
        }
    }
}

fn spawn_audit_writer(db: Db, mut receiver: tokio::sync::mpsc::Receiver<AuditMessage>) {
    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(128);
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                message = receiver.recv() => match message {
                    Some(AuditMessage::Record(record)) => {
                        batch.push(record);
                        if batch.len() >= 200 {
                            flush_audit_batch(&db, &mut batch).await;
                        }
                    }
                    Some(AuditMessage::Flush(sender)) => {
                        flush_audit_batch(&db, &mut batch).await;
                        let _ = sender.send(());
                    }
                    None => {
                        flush_audit_batch(&db, &mut batch).await;
                        break;
                    }
                },
                _ = ticker.tick() => {
                    flush_audit_batch(&db, &mut batch).await;
                }
            }
        }
    });
}

async fn flush_audit_batch(db: &Db, batch: &mut Vec<AuditRecord>) {
    if batch.is_empty() {
        return;
    }
    let records = std::mem::take(batch);
    if let Err(error) = db.audit_batch(records).await {
        tracing::warn!(%error, "failed to flush audit batch");
    }
}

async fn ensure_default_roots(db: &Db) -> anyhow::Result<()> {
    for user in db.list_users().await? {
        if !db.list_roots(user.id).await?.is_empty() {
            continue;
        }

        let home = std::path::PathBuf::from(&user.home_dir);
        let canonical =
            crate::fsops::ensure_directory(&home).map_err(|err| anyhow::anyhow!(err))?;
        let real_path = canonical
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("user home is not valid UTF-8"))?;
        db.ensure_root(user.id, real_path, "My Files", false)
            .await?;
    }
    Ok(())
}
