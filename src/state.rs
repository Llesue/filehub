use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::auth::{RateLimiter, random_token};
use crate::config::{ConfigFile, Snapshot, build_snapshot, ensure_directories, load_config_file};
use crate::db::{AuditRecord, Db, unix_now};
use crate::events::EventBus;
use crate::fsops::DiskUsage;
use crate::jobs::{JobKind, JobManager, JobOutcome};
use crate::presence::{PresenceEntry, PresenceKey};

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeleteItem {
    pub relative: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct DeletePreview {
    pub user_id: i64,
    pub root_id: i64,
    pub items: Vec<DeleteItem>,
    pub expires_at: i64,
    pub confirm_code: String,
    pub requires_code: bool,
}

#[derive(Debug, Clone)]
pub struct CmdConfirmation {
    pub user_id: i64,
    pub root_id: i64,
    pub cwd: String,
    pub code: String,
    pub targets: Vec<String>,
    pub expires_at: i64,
    pub attempts: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum UsageInvalidation {
    Full,
    Delta(i64),
    None,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageSnapshot {
    pub used_bytes: u64,
    pub entry_count: u64,
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

type DirectorySizeCountsCache = (Instant, Vec<(String, i64)>);

#[allow(clippy::type_complexity)]
pub type FederationCacheKey = (i64, String, String, String);
#[allow(clippy::type_complexity)]
pub type FederationCache = Mutex<HashMap<FederationCacheKey, (i64, serde_json::Value)>>;

pub struct AppState {
    pub db: Db,
    pub config: ConfigFile,
    snapshot: ArcSwap<Snapshot>,
    pub setup: SetupState,
    pub rate_limiter: RateLimiter,
    pub ip_limiter: RateLimiter,
    pub password_semaphore: Arc<tokio::sync::Semaphore>,
    pub delete_tokens: Mutex<HashMap<String, DeletePreview>>,
    pub cmd_confirmations: Mutex<HashMap<String, CmdConfirmation>>,
    pub share_tickets: Mutex<HashMap<String, ShareTicket>>,
    pub ffmpeg: Option<PathBuf>,
    pub thumb_semaphore: Arc<Semaphore>,
    pub thumb_preview_semaphore: Arc<Semaphore>,
    zip_semaphore: Mutex<Arc<Semaphore>>,
    pub jobs: Arc<JobManager>,
    pub gate: Arc<crate::gate::ChangeGate>,
    pub hash: Arc<crate::hash::HashService>,
    pub audit_tx: tokio::sync::mpsc::Sender<AuditMessage>,
    pub usage_cache: Mutex<HashMap<i64, UsageSnapshot>>,
    pub usage_locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    pub usage_semaphore: Arc<Semaphore>,
    directory_size_counts: Mutex<Option<DirectorySizeCountsCache>>,
    directory_size_counts_refreshing: AtomicBool,
    pub public_rate: WindowRateLimiter,
    pub slow_requests: Mutex<VecDeque<SlowRequest>>,
    pub events: Arc<EventBus>,
    pub edit_presence: Mutex<HashMap<PresenceKey, PresenceEntry>>,
    pub updates: Arc<crate::updates::UpdateChecker>,
    pub federation_clients: Mutex<HashMap<i64, Arc<crate::federation::client::PeerClient>>>,
    pub federation_cache: FederationCache,
    pub federation_streams: Mutex<HashMap<i64, Arc<Semaphore>>>,
    pub federation_read_counts: Mutex<HashMap<(i64, i64, i64, String), i64>>,
    pub presence_tasks: Mutex<HashMap<i64, tokio::task::JoinHandle<()>>>,
    pub presence_unsupported: Mutex<std::collections::HashSet<i64>>,
    pub presence_peers: Mutex<HashMap<i64, crate::federation::presence::PresencePeer>>,
    /// Cached explicit index policies; invalidated on every policy mutation.
    pub policy_cache: Mutex<Option<Vec<crate::db::IndexPolicyRecord>>>,
    /// Cancelled once the process starts shutting down. Long-lived streams and
    /// sockets watch this so graceful shutdown does not wait on them forever.
    pub shutdown: CancellationToken,
}

fn path_chain(target: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let mut current = Some(target.to_path_buf());
    while let Some(path) = current {
        paths.push(path.to_string_lossy().to_string());
        current = path.parent().map(Path::to_path_buf);
    }
    paths
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
            },
            "updates": {
                "check_enabled": false
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

        let db = Db::open(&bootstrap.settings.db_path, 8)?;
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
        // Keep a reserved lane for full-size previews. With workers=1 the two
        // lanes oversubscribe by one, but a preview can still make progress.
        let preview_workers = snapshot
            .settings
            .thumbnails
            .preview_workers
            .clamp(1, thumb_workers);
        let thumb_list_workers = thumb_workers.saturating_sub(preview_workers).max(1);
        let ffmpeg =
            if snapshot.settings.thumbnails.video || snapshot.settings.media.audio_transcode {
                crate::thumbs::detect_ffmpeg()
            } else {
                None
            };
        let events = EventBus::new(256);
        let jobs = JobManager::new(
            db.clone(),
            snapshot.settings.jobs.io_workers,
            snapshot.settings.jobs.interactive_workers,
            snapshot.settings.jobs.dir_size_workers,
            snapshot.settings.jobs.index_workers,
            snapshot.settings.jobs.archive_workers,
            snapshot.settings.jobs.media_workers,
            snapshot.settings.jobs.queue_max,
            snapshot.settings.jobs.thumbnail_queue_max,
            Arc::clone(&events),
        );
        jobs.recover().await?;
        let (audit_tx, audit_rx) = tokio::sync::mpsc::channel::<AuditMessage>(1024);
        spawn_audit_writer(db.clone(), audit_rx, Arc::clone(&events));
        let zip_permits = snapshot.settings.downloads.zip_concurrency.max(1);
        let updates = Arc::new(crate::updates::UpdateChecker::new(crate::updates::API_URL)?);
        let shutdown = CancellationToken::new();
        let hash = crate::hash::HashService::new(
            db.clone(),
            snapshot.settings.hash.workers,
            snapshot.settings.hash.queue_max,
            snapshot.settings.hash.per_user_queue,
            shutdown.clone(),
        );

        let state = Arc::new(Self {
            db,
            config,
            snapshot: ArcSwap::from_pointee(snapshot),
            setup: SetupState {
                initialized: AtomicBool::new(initialized),
                token,
            },
            rate_limiter: RateLimiter::new(),
            ip_limiter: RateLimiter::with(30, Duration::from_secs(60), 4096),
            password_semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
            delete_tokens: Mutex::new(HashMap::new()),
            cmd_confirmations: Mutex::new(HashMap::new()),
            share_tickets: Mutex::new(HashMap::new()),
            ffmpeg,
            thumb_semaphore: Arc::new(Semaphore::new(thumb_list_workers)),
            thumb_preview_semaphore: Arc::new(Semaphore::new(preview_workers)),
            zip_semaphore: Mutex::new(Arc::new(Semaphore::new(zip_permits))),
            jobs,
            gate: Arc::new(crate::gate::ChangeGate::default()),
            hash,
            audit_tx,
            usage_cache: Mutex::new(HashMap::new()),
            usage_locks: Mutex::new(HashMap::new()),
            usage_semaphore: Arc::new(Semaphore::new(1)),
            directory_size_counts: Mutex::new(None),
            directory_size_counts_refreshing: AtomicBool::new(false),
            public_rate: WindowRateLimiter::default(),
            slow_requests: Mutex::new(VecDeque::new()),
            events,
            edit_presence: Mutex::new(HashMap::new()),
            updates,
            federation_clients: Mutex::new(HashMap::new()),
            federation_cache: Mutex::new(HashMap::new()),
            federation_streams: Mutex::new(HashMap::new()),
            federation_read_counts: Mutex::new(HashMap::new()),
            presence_tasks: Mutex::new(HashMap::new()),
            presence_unsupported: Mutex::new(std::collections::HashSet::new()),
            presence_peers: Mutex::new(HashMap::new()),
            policy_cache: Mutex::new(None),
            shutdown,
        });
        tokio::spawn(crate::federation::mounts::health_loop(Arc::clone(&state)));
        crate::federation::presence::start(Arc::clone(&state));
        tokio::spawn(crate::federation::peer::read_flush_loop(Arc::clone(&state)));
        state.updates.start_loop(Arc::clone(&state));
        state.schedule_directory_size_counts_refresh();
        state.schedule_bigram_backfill();
        crate::media::warm_cache(&state).await;
        Ok(state)
    }

    pub fn federation_stream_semaphore(&self, mount_id: i64, limit: usize) -> Arc<Semaphore> {
        let mut streams = self.federation_streams.lock();
        streams
            .entry(mount_id)
            .or_insert_with(|| Arc::new(Semaphore::new(limit.max(1))))
            .clone()
    }

    pub fn settings(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }

    /// Directory-size status counts are informational. Scanning 200k rows on
    /// a slow disk can take seconds, so serve a short-lived cache and refresh
    /// in the background instead of blocking the admin page.
    pub fn cached_directory_size_counts(&self) -> Vec<(String, i64)> {
        self.directory_size_counts
            .lock()
            .as_ref()
            .map(|(_, counts)| counts.clone())
            .unwrap_or_default()
    }

    pub fn schedule_directory_size_counts_refresh(self: &Arc<Self>) {
        {
            let cache = self.directory_size_counts.lock();
            if let Some((at, _)) = cache.as_ref()
                && at.elapsed() < Duration::from_secs(60)
            {
                return;
            }
        }
        if self
            .directory_size_counts_refreshing
            .swap(true, Ordering::Relaxed)
        {
            return;
        }
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let counts = state.db.directory_size_counts().await;
            match counts {
                Ok(counts) => {
                    *state.directory_size_counts.lock() = Some((Instant::now(), counts));
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to refresh directory size counts");
                }
            }
            state
                .directory_size_counts_refreshing
                .store(false, Ordering::Relaxed);
        });
    }

    /// Populates name bigrams for entries that existed before M26. The marker
    /// makes the scan restart-safe; the operation is idempotent.
    pub fn schedule_bigram_backfill(self: &Arc<Self>) {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            if state
                .db
                .runtime_state_get("index.bigram_backfill")
                .await
                .ok()
                .flatten()
                == Some(1)
            {
                return;
            }
            let mut after = 0_i64;
            loop {
                match state.db.backfill_name_bigrams_batch(after, 500).await {
                    Ok((0, _)) => break,
                    Ok((_processed, Some(last))) => {
                        after = last;
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Ok((_processed, None)) => break,
                    Err(error) => {
                        tracing::warn!(%error, "name bigram backfill failed");
                        return;
                    }
                }
            }
            if let Err(error) = state.db.runtime_state_set("index.bigram_backfill", 1).await {
                tracing::warn!(%error, "failed to persist bigram backfill marker");
            } else {
                tracing::info!("name bigram backfill completed");
            }
        });
    }

    pub async fn reload_settings(&self, ensure_paths: bool) -> anyhow::Result<()> {
        let overrides = self.db.settings_overrides().await?;
        let snapshot = build_snapshot(&self.config, &overrides)?;
        if ensure_paths {
            ensure_directories(&snapshot)?;
        }
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
        // Auditing is best-effort: never make a user request wait for the
        // audit writer when a long DB operation has filled the channel.
        if let Err(error) = self.audit_tx.try_send(AuditMessage::Record(record)) {
            tracing::warn!(%error, "audit queue is full or closed; dropping audit record");
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
        // Keep the last value and let the next reader refresh it in the
        // background. Dropping it forces a slow full walk on the next request.
        if let Some(snapshot) = self.usage_cache.lock().get_mut(&user_id) {
            snapshot.computed_at = 0;
        }
    }

    pub async fn invalidate_path_caches(
        self: &Arc<Self>,
        target: &Path,
        user_id: Option<i64>,
        subtree: bool,
    ) {
        self.invalidate_path_caches_with_usage(target, user_id, subtree, UsageInvalidation::Full)
            .await;
    }

    pub async fn invalidate_path_caches_with_usage(
        self: &Arc<Self>,
        target: &Path,
        user_id: Option<i64>,
        subtree: bool,
        usage: UsageInvalidation,
    ) {
        let paths = path_chain(target);
        let _ = self.db.invalidate_path_chain(paths).await;
        if subtree {
            self.schedule_cache_cleanup(vec![target.to_path_buf()]);
        }
        self.apply_usage_invalidation(user_id, usage);
    }

    /// Invalidates many targets and their ancestors in one short transaction.
    /// Descendant cache rows are left for `schedule_cache_cleanup`.
    pub async fn invalidate_paths_exact(&self, targets: &[PathBuf], user_id: Option<i64>) {
        self.invalidate_paths_exact_with_usage(targets, user_id, UsageInvalidation::Full)
            .await;
    }

    pub async fn invalidate_paths_exact_with_usage(
        &self,
        targets: &[PathBuf],
        user_id: Option<i64>,
        usage: UsageInvalidation,
    ) {
        if targets.is_empty() {
            return;
        }
        let mut paths = BTreeSet::new();
        for target in targets {
            paths.extend(path_chain(target));
        }
        let _ = self
            .db
            .invalidate_path_chain(paths.into_iter().collect())
            .await;
        self.apply_usage_invalidation(user_id, usage);
    }

    fn apply_usage_invalidation(&self, user_id: Option<i64>, usage: UsageInvalidation) {
        if let Some(user_id) = user_id {
            match usage {
                UsageInvalidation::Full => self.invalidate_usage(user_id),
                UsageInvalidation::Delta(delta) => self.adjust_usage(user_id, delta),
                UsageInvalidation::None => {}
            }
        }
    }

    /// Deletes descendant cache rows in short background batches so a delete,
    /// rename or transfer never holds the SQLite writer lock for long.
    pub fn schedule_cache_cleanup(self: &Arc<Self>, targets: Vec<PathBuf>) {
        let mut unique = BTreeSet::new();
        for target in targets {
            unique.insert(target.to_string_lossy().to_string());
        }
        if unique.is_empty() {
            return;
        }
        let state = Arc::clone(self);
        tokio::spawn(async move {
            for target in unique {
                let _ = state.db.delete_file_hashes_subtree(&target).await;
                loop {
                    match state.db.delete_cached_subtree_batch(&target, 500).await {
                        Ok(0) => break,
                        Ok(deleted) => {
                            tracing::debug!(path = %target, deleted, "cleaned cached descendants");
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        Err(error) => {
                            tracing::warn!(path = %target, %error, "cache cleanup failed");
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Marks descendant size caches stale in short background batches.
    pub fn schedule_sizes_stale(self: &Arc<Self>, target: PathBuf) {
        let state = Arc::clone(self);
        let target = target.to_string_lossy().to_string();
        tokio::spawn(async move {
            loop {
                match state.db.mark_cached_sizes_stale_batch(&target, 500).await {
                    Ok(0) => break,
                    Ok(updated) => {
                        tracing::debug!(path = %target, updated, "marked cached sizes stale");
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Err(error) => {
                        tracing::warn!(path = %target, %error, "size invalidation failed");
                        break;
                    }
                }
            }
        });
    }

    pub fn adjust_usage(&self, user_id: i64, delta: i64) {
        if delta == 0 {
            return;
        }
        let mut cache = self.usage_cache.lock();
        if let Some(snapshot) = cache.get_mut(&user_id) {
            let adjusted = (snapshot.used_bytes as i64).saturating_add(delta).max(0);
            snapshot.used_bytes = adjusted as u64;
        }
    }

    pub fn adjust_entries(&self, user_id: i64, delta: i64) {
        if delta == 0 {
            return;
        }
        let mut cache = self.usage_cache.lock();
        if let Some(snapshot) = cache.get_mut(&user_id) {
            let adjusted = (snapshot.entry_count as i64).saturating_add(delta).max(0);
            snapshot.entry_count = adjusted as u64;
        }
    }

    pub async fn invalidate_index_totals(&self, target: &Path) {
        let mut current = Some(target.to_path_buf());
        while let Some(path) = current {
            let value = path.to_string_lossy().to_string();
            let _ = self.db.mark_index_partial(&value).await;
            let _ = self.db.mark_directory_size_stale(&value).await;
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

    /// True when an interactive request took longer than 1s in the last minute.
    pub fn interactive_pressure(&self) -> bool {
        let now = unix_now();
        self.slow_requests
            .lock()
            .iter()
            .any(|request| now.saturating_sub(request.at) < 60 && request.duration_ms > 1000)
    }

    pub fn index_settle(&self) -> Duration {
        Duration::from_secs(self.settings().settings.index.settle_seconds.max(1))
    }

    pub fn index_max_defer(&self) -> Duration {
        Duration::from_secs(self.settings().settings.index.max_defer_seconds.max(1))
    }

    pub fn note_index_change(&self, path: &Path, reason: crate::gate::ChangeReason) {
        let settings = self.settings();
        self.gate.note_change(
            path,
            reason,
            Duration::from_secs(settings.settings.index.settle_seconds.max(1)),
            Duration::from_secs(settings.settings.index.max_defer_seconds.max(1)),
            settings.settings.index.max_pending,
        );
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

        let shares = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                shares.expire_shares().await;
            }
        });

        let gate = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                gate.dispatch_gate().await;
            }
        });

        let watchdog = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let settings = watchdog.settings();
                let queue_timeout =
                    Duration::from_secs(settings.settings.index.queue_timeout_seconds.max(1));
                let stall_timeout =
                    Duration::from_secs(settings.settings.index.job_stall_seconds.max(1));
                let candidates = watchdog
                    .jobs
                    .watchdog_candidates(queue_timeout, stall_timeout);
                for id in candidates {
                    if let Err(error) = watchdog.jobs.cancel(&id, 0, true).await {
                        tracing::debug!(%error, job_id = %id, "watchdog could not cancel job");
                    } else {
                        tracing::warn!(job_id = %id, "watchdog cancelled stalled background job");
                    }
                }
            }
        });

        let presence = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            loop {
                interval.tick().await;
                crate::presence::cleanup(&presence);
            }
        });
    }

    async fn dispatch_gate(self: &Arc<Self>) {
        let settings = self.settings();
        let settle = Duration::from_secs(settings.settings.index.settle_seconds.max(1));
        let ready = self.gate.take_ready(settle, 8);
        if ready.is_empty() {
            return;
        }
        for item in ready {
            let real_path = item.path.to_string_lossy().to_string();
            let policy = match crate::policy::resolve(self, &item.path).await {
                Ok(policy) => policy,
                Err(_) => continue,
            };
            if policy.is_ignore() {
                continue;
            }
            if policy == crate::policy::IndexPolicy::Background && self.interactive_pressure() {
                self.note_index_change(&item.path, item.reason);
                continue;
            }
            let key = format!("dir:{real_path}");
            if self
                .jobs
                .active_job_for_path(crate::jobs::JobKind::Index, &real_path)
                .is_some()
            {
                self.note_index_change(&item.path, item.reason);
                continue;
            }
            let Some((user_id, root, relative)) =
                crate::indexer::index_target_for_path(self, &real_path)
                    .await
                    .ok()
                    .flatten()
            else {
                continue;
            };
            let job_state = Arc::clone(self);
            let job_path = item.path.clone();
            let gate_path = item.path.clone();
            let forced = item.forced;
            let payload = serde_json::json!({
                "root_id": root.id,
                "path": relative,
                "real_path": real_path,
                "recursive": false,
                "auto": true,
                "forced": forced,
            });
            let result = self
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
                        let result = crate::indexer::index_path(
                            &job_state,
                            user_id,
                            root,
                            &relative,
                            false,
                            forced,
                            &cancel,
                            &mut progress,
                        )
                        .await;
                        match result {
                            Ok(stats) => Ok(JobOutcome::done(serde_json::json!(stats), 1, 1)),
                            Err(error) => {
                                job_state
                                    .gate
                                    .mark_failed(&gate_path, Duration::from_secs(30));
                                Err(error)
                            }
                        }
                    },
                )
                .await;
            match result {
                Ok(outcome) => {
                    if outcome.created {
                        tracing::debug!(path = %job_path.display(), forced, "gate queued index");
                    }
                }
                Err(crate::error::AppError::QueueFull) => {
                    self.note_index_change(&job_path, item.reason);
                }
                Err(error) => {
                    self.gate.mark_failed(&job_path, Duration::from_secs(30));
                    tracing::debug!(%error, path = %job_path.display(), "gate index submit failed");
                }
            }
        }
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
            // A fresh database should get an initial quick scan instead of
            // waiting a full quick_scan_minutes interval.
            Ok(None) => 0,
            Err(error) => {
                tracing::warn!(%error, "failed to read quick scan state");
                return;
            }
        };
        let quick_resume = self
            .db
            .scan_state("quick")
            .await
            .ok()
            .flatten()
            .is_some_and(|state| state.pass_started_at.is_some());
        if quick_resume || now.saturating_sub(last_quick) >= quick_seconds {
            let job_state = Arc::clone(self);
            let result = self
                .jobs
                .submit(
                    JobKind::Index,
                    "scheduled_quick_scan".to_owned(),
                    admin_id,
                    serde_json::json!({ "mode": "quick" }),
                    1,
                    false,
                    move |ctx| async move {
                        let cancel = ctx.token();
                        let mut progress = |checked: usize, total: usize| {
                            ctx.report(checked, total, 0, 0);
                        };
                        let stats =
                            crate::indexer::quick_scan(&job_state, &cancel, &mut progress).await?;
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
        let full_resume = self.db.frontier_len("full").await.unwrap_or(0) > 0;
        if full_resume || now.saturating_sub(last_full) >= full_seconds {
            let job_state = Arc::clone(self);
            let result = self
                .jobs
                .submit(
                    JobKind::Index,
                    "scheduled_full_scan".to_owned(),
                    admin_id,
                    serde_json::json!({ "mode": "full" }),
                    1,
                    false,
                    move |ctx| async move {
                        let cancel = ctx.token();
                        let mut progress = |done: usize, total: usize| {
                            ctx.report(done, total.max(1), 0, 0);
                        };
                        let stats =
                            crate::indexer::scheduled_full_scan(&job_state, &cancel, &mut progress)
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

    /// Persist expired shares as stopped and notify their creators and any
    /// open public share page. Runs often enough that the stored `revoked`
    /// flag follows `expires_at` without waiting for the hourly maintenance.
    pub async fn expire_shares(&self) -> usize {
        const BATCH: i64 = 500;
        let now = unix_now();
        let mut total = 0_usize;
        loop {
            match self.db.revoke_expired_shares(now, BATCH).await {
                Ok(expired) if expired.is_empty() => break,
                Ok(expired) => {
                    let count = expired.len();
                    total += count;
                    for (id, creator_id) in expired {
                        let payload = serde_json::json!({ "id": id });
                        self.events
                            .publish("share.revoked", Some(creator_id), payload.clone());
                        self.events.publish_share("share.revoked", id, payload);
                    }
                    if count < BATCH as usize {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to revoke expired shares");
                    break;
                }
            }
        }
        if total > 0 {
            tracing::info!(count = total, "revoked expired shares");
        }
        total
    }

    pub async fn run_maintenance(&self) {
        let settings = self.settings();
        let jobs = &settings.settings.jobs;
        let now = unix_now();
        let mut audit_removed = 0_usize;
        loop {
            match self
                .db
                .prune_audit_batch(jobs.audit_retention_days, jobs.audit_max_rows, 500)
                .await
            {
                Ok(0) => break,
                Ok(removed) => {
                    audit_removed += removed;
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to prune audit log");
                    break;
                }
            }
        }
        if audit_removed > 0 {
            tracing::info!(removed = audit_removed, "pruned audit log");
        }
        let job_cutoff = now - (jobs.job_retention_days.min(3650) as i64) * 86_400;
        match self.db.prune_jobs(job_cutoff).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "pruned finished jobs"),
            Err(error) => tracing::warn!(%error, "failed to prune jobs"),
        }
        let mut sizes_removed = 0_usize;
        loop {
            match self
                .db
                .prune_directory_sizes_batch(now - 30 * 86_400, 500)
                .await
            {
                Ok(0) => break,
                Ok(removed) => {
                    sizes_removed += removed;
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to prune directory sizes");
                    break;
                }
            }
        }
        if sizes_removed > 0 {
            tracing::info!(removed = sizes_removed, "pruned stale directory sizes");
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
        match self.db.checkpoint_passive().await {
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

fn spawn_audit_writer(
    db: Db,
    mut receiver: tokio::sync::mpsc::Receiver<AuditMessage>,
    events: Arc<EventBus>,
) {
    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(128);
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                message = receiver.recv() => match message {
                    Some(AuditMessage::Record(record)) => {
                        batch.push(record);
                        if batch.len() >= 200 {
                            flush_audit_batch(&db, &mut batch, &events).await;
                        }
                    }
                    Some(AuditMessage::Flush(sender)) => {
                        flush_audit_batch(&db, &mut batch, &events).await;
                        let _ = sender.send(());
                    }
                    None => {
                        flush_audit_batch(&db, &mut batch, &events).await;
                        break;
                    }
                },
                _ = ticker.tick() => {
                    flush_audit_batch(&db, &mut batch, &events).await;
                }
            }
        }
    });
}

async fn flush_audit_batch(db: &Db, batch: &mut Vec<AuditRecord>, events: &Arc<EventBus>) {
    if batch.is_empty() {
        return;
    }
    let count = batch.len();
    let records = std::mem::take(batch);
    match db.audit_batch(records).await {
        Ok(()) => events.publish_admin("audit.appended", serde_json::json!({ "count": count })),
        Err(error) => tracing::warn!(%error, "failed to flush audit batch"),
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
