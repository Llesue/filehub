use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::db::{Db, JobRecord, unix_now};
use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    DirSize,
    Copy,
    Move,
    Index,
    Thumbnail,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirSize => "dir_size",
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Index => "index",
            Self::Thumbnail => "thumbnail",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dir_size" => Some(Self::DirSize),
            "copy" => Some(Self::Copy),
            "move" => Some(Self::Move),
            "index" => Some(Self::Index),
            "thumbnail" => Some(Self::Thumbnail),
            _ => None,
        }
    }

    pub fn is_transfer(self) -> bool {
        matches!(self, Self::Copy | Self::Move)
    }

    pub fn uses_shared_io(self) -> bool {
        !matches!(self, Self::Thumbnail)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub user_id: i64,
    pub kind: String,
    #[serde(skip)]
    key: String,
    pub status: String,
    pub progress: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

impl Job {
    fn from_record(record: JobRecord) -> Self {
        Self {
            id: record.id,
            user_id: record.user_id,
            kind: record.kind,
            key: record.job_key,
            status: record.status,
            progress: record.progress.max(0) as usize,
            total: record.total.max(0) as usize,
            bytes_done: record.bytes_done.max(0) as u64,
            bytes_total: record.bytes_total.max(0) as u64,
            result: record
                .result
                .and_then(|raw| serde_json::from_str(&raw).ok()),
            error: record.error,
            created_at: record.created_at,
            started_at: record.started_at,
            finished_at: record.finished_at,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.status.as_str(), "completed" | "failed" | "cancelled")
    }
}

#[derive(Debug, Clone)]
pub struct SubmitOutcome {
    pub job: Job,
    pub created: bool,
}

#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub result: Value,
    pub progress: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

impl JobOutcome {
    pub fn done(result: Value, progress: usize, total: usize) -> Self {
        Self {
            result,
            progress,
            total,
            bytes_done: 0,
            bytes_total: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub kind: String,
    pub queued: i64,
    pub running: i64,
}

struct Inner {
    active: HashMap<(String, String), String>,
    jobs: HashMap<String, Job>,
    cancel: HashMap<String, CancellationToken>,
    last_persist: HashMap<String, Instant>,
    waiters: HashMap<String, Vec<tokio::sync::oneshot::Sender<Job>>>,
}

fn clear_active(inner: &mut Inner, kind: &str, key: &str, id: &str) {
    let map_key = (kind.to_owned(), key.to_owned());
    if inner
        .active
        .get(&map_key)
        .is_some_and(|current| current == id)
    {
        inner.active.remove(&map_key);
    }
}

pub struct JobManager {
    db: Db,
    inner: Mutex<Inner>,
    io: Arc<Semaphore>,
    queue_limit: usize,
    thumbnail_queue_limit: usize,
}

impl JobManager {
    pub fn new(
        db: Db,
        io_workers: usize,
        queue_limit: usize,
        thumbnail_queue_limit: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            inner: Mutex::new(Inner {
                active: HashMap::new(),
                jobs: HashMap::new(),
                cancel: HashMap::new(),
                last_persist: HashMap::new(),
                waiters: HashMap::new(),
            }),
            io: Arc::new(Semaphore::new(io_workers.max(1))),
            queue_limit: queue_limit.max(1),
            thumbnail_queue_limit: thumbnail_queue_limit.max(1),
        })
    }

    pub async fn recover(&self) -> anyhow::Result<usize> {
        let count = self.db.recover_jobs().await?;
        if count > 0 {
            tracing::warn!(count, "marked interrupted background jobs as failed");
        }
        Ok(count)
    }

    pub async fn submit<F, Fut>(
        self: &Arc<Self>,
        kind: JobKind,
        key: String,
        user_id: i64,
        payload: Value,
        total: usize,
        run: F,
    ) -> Result<SubmitOutcome, AppError>
    where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<JobOutcome, AppError>> + Send + 'static,
    {
        {
            let inner = self.inner.lock();
            let active = inner
                .jobs
                .values()
                .filter(|job| matches!(job.status.as_str(), "queued" | "running"))
                .count();
            if active >= self.queue_limit {
                return Err(AppError::QueueFull);
            }
            if kind == JobKind::Thumbnail {
                let queued_thumbnails = inner
                    .jobs
                    .values()
                    .filter(|job| {
                        job.kind == JobKind::Thumbnail.as_str()
                            && matches!(job.status.as_str(), "queued" | "running")
                    })
                    .count();
                if queued_thumbnails >= self.thumbnail_queue_limit {
                    return Err(AppError::QueueFull);
                }
            }
        }

        let id = format!("job_{}", crate::auth::random_token(12));
        let now = unix_now();
        let record = JobRecord {
            id: id.clone(),
            user_id,
            kind: kind.as_str().to_owned(),
            job_key: key.clone(),
            status: "queued".to_owned(),
            progress: 0,
            total: total.min(i64::MAX as usize) as i64,
            bytes_done: 0,
            bytes_total: 0,
            payload: payload.to_string(),
            result: None,
            error: None,
            created_at: now,
            started_at: None,
            finished_at: None,
        };
        let (record, created) = self
            .db
            .create_job(record)
            .await
            .map_err(AppError::internal)?;
        let job = Job::from_record(record);
        if !created {
            tracing::info!(
                job_id = %job.id,
                kind = kind.as_str(),
                key = %key,
                "background job already active; reusing"
            );
            return Ok(SubmitOutcome { job, created });
        }

        let token = CancellationToken::new();
        {
            let mut inner = self.inner.lock();
            inner
                .active
                .insert((kind.as_str().to_owned(), key.clone()), id.clone());
            inner.jobs.insert(id.clone(), job.clone());
            inner.cancel.insert(id.clone(), token.clone());
        }
        tracing::info!(
            job_id = %id,
            kind = kind.as_str(),
            key = %key,
            user_id,
            "background job queued"
        );
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            manager.execute(id, kind, key, token, run).await;
        });
        Ok(SubmitOutcome { job, created })
    }

    async fn execute<F, Fut>(
        self: Arc<Self>,
        id: String,
        kind: JobKind,
        key: String,
        token: CancellationToken,
        run: F,
    ) where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<JobOutcome, AppError>> + Send + 'static,
    {
        let permit = if kind.uses_shared_io() {
            match self.io.clone().acquire_owned().await {
                Ok(permit) => Some(permit),
                Err(_) => {
                    self.finish_row(&id, "failed", None, Some("job worker closed"))
                        .await;
                    self.remove_active(&id, kind, &key);
                    return;
                }
            }
        } else {
            None
        };
        if token.is_cancelled() {
            drop(permit);
            self.finish_row(&id, "cancelled", None, Some("cancelled"))
                .await;
            self.remove_active(&id, kind, &key);
            return;
        }

        if let Err(error) = self.db.mark_job_running(&id).await {
            tracing::warn!(job_id = %id, error = %error, "failed to mark job running");
        }
        {
            let mut inner = self.inner.lock();
            if let Some(job) = inner.jobs.get_mut(&id) {
                job.status = "running".to_owned();
                job.started_at = Some(unix_now());
            }
        }
        tracing::info!(job_id = %id, kind = kind.as_str(), key = %key, "job started");
        let started = Instant::now();
        let context = JobContext {
            manager: Arc::clone(&self),
            id: id.clone(),
            cancel: token.clone(),
        };
        let outcome = tokio::select! {
            biased;
            () = token.cancelled() => Err(AppError::Cancelled),
            result = run(context) => result,
        };
        drop(permit);
        let elapsed_ms = started.elapsed().as_millis();

        let (status, progress, total, bytes_done, bytes_total, result, error) = match &outcome {
            Ok(value) => (
                "completed",
                value.progress,
                value.total,
                value.bytes_done,
                value.bytes_total,
                Some(value.result.to_string()),
                None,
            ),
            Err(AppError::Cancelled) => {
                ("cancelled", 0, 0, 0, 0, None, Some("cancelled".to_owned()))
            }
            Err(err) => ("failed", 0, 0, 0, 0, None, Some(err.to_string())),
        };
        let was_cancelled = {
            let mut inner = self.inner.lock();
            let already = inner
                .jobs
                .get(&id)
                .is_some_and(|job| job.status == "cancelled");
            if let Some(job) = inner.jobs.get_mut(&id)
                && !already
            {
                job.status = status.to_owned();
                job.progress = progress;
                job.total = total;
                job.bytes_done = bytes_done;
                job.bytes_total = bytes_total;
                job.result = result
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok());
                job.error = error.clone();
                job.finished_at = Some(unix_now());
            }
            clear_active(&mut inner, kind.as_str(), &key, &id);
            inner.cancel.remove(&id);
            inner.last_persist.remove(&id);
            already
        };
        if was_cancelled {
            let final_job = {
                let mut inner = self.inner.lock();
                let job = inner.jobs.remove(&id);
                inner.waiters.remove(&id);
                job
            };
            if let Some(job) = final_job {
                self.notify_waiters(&id, &job);
            }
            tracing::info!(
                job_id = %id,
                kind = kind.as_str(),
                key = %key,
                duration_ms = elapsed_ms,
                "job cancelled"
            );
            return;
        }
        if let Err(write_error) = self
            .db
            .finish_job(&id, status, result.as_deref(), error.as_deref())
            .await
        {
            tracing::warn!(job_id = %id, error = %write_error, "failed to persist job result");
        }
        if status == "completed" {
            tracing::info!(
                job_id = %id,
                kind = kind.as_str(),
                key = %key,
                duration_ms = elapsed_ms,
                "job finished"
            );
        } else {
            tracing::warn!(
                job_id = %id,
                kind = kind.as_str(),
                key = %key,
                duration_ms = elapsed_ms,
                error = error.as_deref().unwrap_or("unknown error"),
                "job failed"
            );
        }
        let final_job = {
            let mut inner = self.inner.lock();
            inner.jobs.remove(&id)
        };
        if let Some(job) = final_job {
            self.notify_waiters(&id, &job);
        }
    }

    async fn finish_row(&self, id: &str, status: &str, result: Option<&str>, error: Option<&str>) {
        if let Err(write_error) = self.db.finish_job(id, status, result, error).await {
            tracing::warn!(job_id = %id, error = %write_error, "failed to persist job state");
        }
    }

    fn remove_active(&self, id: &str, kind: JobKind, key: &str) {
        let mut inner = self.inner.lock();
        clear_active(&mut inner, kind.as_str(), key, id);
        inner.cancel.remove(id);
        inner.last_persist.remove(id);
        inner.jobs.remove(id);
    }

    fn notify_waiters(&self, id: &str, job: &Job) {
        let senders = self.inner.lock().waiters.remove(id).unwrap_or_default();
        for sender in senders {
            let _ = sender.send(job.clone());
        }
    }

    pub fn active_job_exact(&self, kind: JobKind, key: &str) -> Option<Job> {
        let inner = self.inner.lock();
        let id = inner
            .active
            .get(&(kind.as_str().to_owned(), key.to_owned()))?;
        inner.jobs.get(id).cloned()
    }

    pub async fn wait_for(&self, id: &str) -> Option<Job> {
        let receiver = {
            let mut inner = self.inner.lock();
            match inner.jobs.get(id) {
                Some(job) if job.is_finished() => return Some(job.clone()),
                Some(_) => {
                    let (sender, receiver) = tokio::sync::oneshot::channel();
                    inner.waiters.entry(id.to_owned()).or_default().push(sender);
                    Some(receiver)
                }
                None => None,
            }
        };
        match receiver {
            Some(receiver) => match receiver.await {
                Ok(job) => Some(job),
                Err(_) => self.get(id).await.ok().flatten(),
            },
            None => self.get(id).await.ok().flatten(),
        }
    }

    pub fn active_job_for_path(&self, kind: JobKind, path: &str) -> Option<Job> {
        let inner = self.inner.lock();
        inner
            .jobs
            .values()
            .find(|job| {
                if job.kind != kind.as_str() || !matches!(job.status.as_str(), "queued" | "running")
                {
                    return false;
                }
                path == job.key
                    || path
                        .strip_prefix(&job.key)
                        .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('\\'))
            })
            .cloned()
    }

    pub async fn cancel_descendants(&self, kind: JobKind, prefix: &str) -> usize {
        let targets: Vec<(String, String)> = {
            let inner = self.inner.lock();
            inner
                .jobs
                .values()
                .filter(|job| {
                    job.kind == kind.as_str()
                        && matches!(job.status.as_str(), "queued" | "running")
                        && job
                            .key
                            .strip_prefix(prefix)
                            .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('\\'))
                })
                .map(|job| (job.id.clone(), job.key.clone()))
                .collect()
        };
        let count = targets.len();
        for (id, key) in targets {
            let token = self.inner.lock().cancel.get(&id).cloned();
            if let Some(token) = token {
                token.cancel();
            }
            {
                let mut inner = self.inner.lock();
                clear_active(&mut inner, kind.as_str(), &key, &id);
                if let Some(job) = inner.jobs.get_mut(&id) {
                    job.status = "cancelled".to_owned();
                    job.error = Some("superseded by parent directory scan".to_owned());
                    job.finished_at = Some(unix_now());
                }
                inner.cancel.remove(&id);
                inner.last_persist.remove(&id);
            }
            self.finish_row(
                &id,
                "cancelled",
                None,
                Some("superseded by parent directory scan"),
            )
            .await;
            if let Ok(Some(job)) = self.db.find_job(&id).await {
                let parsed = Job::from_record(job);
                self.notify_waiters(&id, &parsed);
            }
            tracing::info!(job_id = %id, "cancelled nested job for parent scan");
        }
        count
    }

    pub async fn get(&self, id: &str) -> Result<Option<Job>, AppError> {
        if let Some(job) = self.inner.lock().jobs.get(id).cloned() {
            return Ok(Some(job));
        }
        let record = self.db.find_job(id).await.map_err(AppError::internal)?;
        Ok(record.map(Job::from_record))
    }

    pub async fn cancel(&self, id: &str, user_id: i64, is_admin: bool) -> Result<Job, AppError> {
        let memory_job = self.inner.lock().jobs.get(id).cloned();
        let Some(job) = memory_job else {
            let record = self.db.find_job(id).await.map_err(AppError::internal)?;
            let Some(record) = record else {
                return Err(AppError::NotFound);
            };
            if record.user_id != user_id && !is_admin {
                return Err(AppError::NotFound);
            }
            return Err(AppError::conflict(format!(
                "job is already {}",
                record.status
            )));
        };
        if job.user_id != user_id && !is_admin {
            return Err(AppError::NotFound);
        }
        if job.is_finished() {
            return Err(AppError::conflict("job has already finished"));
        }
        let token = self.inner.lock().cancel.get(id).cloned();
        if let Some(token) = token {
            token.cancel();
        }
        let cancelled = {
            let mut inner = self.inner.lock();
            inner.active.remove(&(job.kind.clone(), job.key.clone()));
            if let Some(current) = inner.jobs.get_mut(id) {
                current.status = "cancelled".to_owned();
                current.error = Some("cancelled by user".to_owned());
                current.finished_at = Some(unix_now());
            }
            inner.cancel.remove(id);
            inner.last_persist.remove(id);
            inner.jobs.get(id).cloned().unwrap_or(job)
        };
        self.finish_row(id, "cancelled", None, Some("cancelled by user"))
            .await;
        self.notify_waiters(id, &cancelled);
        tracing::info!(job_id = %id, "job cancellation requested");
        Ok(cancelled)
    }

    pub fn summary(&self) -> Vec<JobSummary> {
        let inner = self.inner.lock();
        let mut summary: HashMap<&str, (i64, i64)> = HashMap::new();
        for job in inner.jobs.values() {
            let entry = summary.entry(job.kind.as_str()).or_insert((0, 0));
            match job.status.as_str() {
                "queued" => entry.0 += 1,
                "running" => entry.1 += 1,
                _ => {}
            }
        }
        let mut values: Vec<JobSummary> = summary
            .into_iter()
            .map(|(kind, (queued, running))| JobSummary {
                kind: kind.to_owned(),
                queued,
                running,
            })
            .collect();
        values.sort_by(|left, right| left.kind.cmp(&right.kind));
        values
    }

    pub fn active_count(&self) -> usize {
        self.inner
            .lock()
            .jobs
            .values()
            .filter(|job| matches!(job.status.as_str(), "queued" | "running"))
            .count()
    }
}

#[derive(Clone)]
pub struct JobContext {
    manager: Arc<JobManager>,
    id: String,
    cancel: CancellationToken,
}

impl JobContext {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn check_cancelled(&self) -> Result<(), AppError> {
        if self.cancel.is_cancelled() {
            Err(AppError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn report(&self, progress: usize, total: usize, bytes_done: u64, bytes_total: u64) {
        let persist = {
            let mut inner = self.manager.inner.lock();
            if let Some(job) = inner.jobs.get_mut(&self.id) {
                job.progress = progress;
                job.total = total;
                job.bytes_done = bytes_done;
                job.bytes_total = bytes_total;
            }
            let now = Instant::now();
            let due = inner
                .last_persist
                .get(&self.id)
                .is_none_or(|last| now.duration_since(*last) >= Duration::from_millis(900));
            if due {
                inner.last_persist.insert(self.id.clone(), now);
            }
            due
        };
        if !persist {
            return;
        }
        let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
            return;
        };
        let db = self.manager.db.clone();
        let id = self.id.clone();
        handle.spawn(async move {
            let _ = db
                .update_job_progress(
                    &id,
                    progress.min(i64::MAX as usize) as i64,
                    total.min(i64::MAX as usize) as i64,
                    bytes_done.min(i64::MAX as u64) as i64,
                    bytes_total.min(i64::MAX as u64) as i64,
                )
                .await;
        });
    }
}
