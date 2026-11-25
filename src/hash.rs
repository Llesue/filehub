use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::auth;
use crate::authz;
use crate::db::unix_now;
use crate::error::AppError;
use crate::state::AppState;

const ALGO: &str = "sha256";
const READ_BUFFER: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HashValue {
    pub sha256: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub computed_at: i64,
    pub cached: bool,
}

#[derive(Debug, Clone)]
pub struct HashFailure {
    pub code: String,
    pub message: String,
}

enum BatchItem {
    Value(HashValue),
    Inflight(Arc<Inflight>),
    Error(AppError),
}

#[derive(Debug, Clone)]
pub enum HashOutcome {
    Ready(HashValue),
    Failed(HashFailure),
}

struct Inflight {
    result: Mutex<Option<HashOutcome>>,
    notify: Notify,
}

impl Inflight {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    fn complete(&self, outcome: HashOutcome) {
        *self.result.lock() = Some(outcome);
        self.notify.notify_waiters();
    }

    async fn wait(&self) -> HashOutcome {
        loop {
            if let Some(outcome) = self.result.lock().clone() {
                return outcome;
            }
            let notified = self.notify.notified();
            if let Some(outcome) = self.result.lock().clone() {
                return outcome;
            }
            notified.await;
        }
    }

    fn ready(&self) -> Option<HashOutcome> {
        self.result.lock().clone()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct HashKey {
    real_path: String,
    size: u64,
    mtime_ns: i64,
}

pub struct HashService {
    db: crate::db::Db,
    workers: Arc<Semaphore>,
    upload_workers: Arc<Semaphore>,
    inflight: Mutex<HashMap<HashKey, Arc<Inflight>>>,
    counts: Mutex<HashMap<i64, (usize, usize)>>,
    queue_max: usize,
    per_user_queue: usize,
    shutdown: CancellationToken,
}

impl HashService {
    pub fn new(
        db: crate::db::Db,
        workers: usize,
        queue_max: usize,
        per_user_queue: usize,
        shutdown: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            workers: Arc::new(Semaphore::new(workers.max(1))),
            upload_workers: Arc::new(Semaphore::new(workers.max(1))),
            inflight: Mutex::new(HashMap::new()),
            counts: Mutex::new(HashMap::new()),
            queue_max: queue_max.max(1),
            per_user_queue: per_user_queue.max(1),
            shutdown,
        })
    }

    pub fn cancel(&self) {
        self.shutdown.cancel();
    }

    async fn request(
        self: &Arc<Self>,
        user_id: i64,
        real_path: String,
        size: u64,
        mtime_ns: i64,
        force: bool,
    ) -> Result<Arc<Inflight>, AppError> {
        let key = HashKey {
            real_path: real_path.clone(),
            size,
            mtime_ns,
        };
        {
            let inflight = self.inflight.lock();
            if let Some(existing) = inflight.get(&key) {
                return Ok(Arc::clone(existing));
            }
        }
        let _ = force;
        {
            let mut counts = self.counts.lock();
            let total: usize = counts
                .values()
                .map(|(running, queued)| running + queued)
                .sum();
            if total >= self.queue_max {
                return Err(AppError::HashQueueFull);
            }
            let entry = counts.entry(user_id).or_insert((0, 0));
            if entry.0 + entry.1 >= self.per_user_queue {
                return Err(AppError::HashQueueFull);
            }
            entry.1 += 1;
        }

        let inflight = Arc::new(Inflight::new());
        {
            let mut map = self.inflight.lock();
            if let Some(existing) = map.get(&key) {
                let mut counts = self.counts.lock();
                if let Some(entry) = counts.get_mut(&user_id) {
                    entry.1 = entry.1.saturating_sub(1);
                }
                return Ok(Arc::clone(existing));
            }
            map.insert(key.clone(), Arc::clone(&inflight));
        }

        let service = Arc::clone(self);
        let worker_inflight = Arc::clone(&inflight);
        tokio::spawn(async move {
            let permit = service.workers.clone().acquire_owned().await;
            {
                let mut counts = service.counts.lock();
                if let Some(entry) = counts.get_mut(&user_id) {
                    entry.1 = entry.1.saturating_sub(1);
                    entry.0 += 1;
                }
            }
            let outcome = match permit {
                Ok(permit) => {
                    let value =
                        compute_with_retries(&real_path, size, mtime_ns, service.shutdown.clone())
                            .await;
                    drop(permit);
                    value
                }
                Err(_) => HashOutcome::Failed(HashFailure {
                    code: "HASH_QUEUE_FULL".to_owned(),
                    message: "hash worker pool is closed".to_owned(),
                }),
            };
            {
                let mut counts = service.counts.lock();
                if let Some(entry) = counts.get_mut(&user_id) {
                    entry.0 = entry.0.saturating_sub(1);
                }
            }
            if let HashOutcome::Ready(value) = &outcome
                && let Err(error) = service
                    .db
                    .upsert_file_hash(
                        &real_path,
                        ALGO,
                        &value.sha256,
                        value.size.min(i64::MAX as u64) as i64,
                        value.mtime_ns,
                    )
                    .await
            {
                tracing::warn!(%error, path = %real_path, "failed to cache hash");
            }
            service.inflight.lock().remove(&key);
            worker_inflight.complete(outcome);
        });
        Ok(inflight)
    }

    pub async fn verify_upload(
        self: &Arc<Self>,
        real_path: &Path,
        expected: &str,
        size: u64,
        max_bytes: u64,
    ) -> Result<(String, u64, i64), AppError> {
        if size > max_bytes {
            return Err(AppError::HashTooLarge(format!(
                "file is {size} bytes, limit is {max_bytes}"
            )));
        }
        let permit = self
            .upload_workers
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::internal(anyhow::anyhow!("hash verifier closed")))?;
        let path = real_path.to_path_buf();
        let shutdown = self.shutdown.clone();
        let result =
            tokio::task::spawn_blocking(move || hash_file_blocking(&path, &shutdown, None))
                .await
                .map_err(|error| {
                    AppError::internal(anyhow::anyhow!("hash verify failed: {error}"))
                })?;
        drop(permit);
        let (actual, actual_size, mtime_ns) = result?;
        if actual_size != size {
            return Err(AppError::HashMismatch(format!(
                "size mismatch: expected {size}, read {actual_size}"
            )));
        }
        if actual != expected.to_ascii_lowercase() {
            return Err(AppError::HashMismatch(
                "content hash does not match".to_owned(),
            ));
        }
        Ok((actual, actual_size, mtime_ns))
    }
}

async fn compute_with_retries(
    real_path: &str,
    size: u64,
    mtime_ns: i64,
    shutdown: CancellationToken,
) -> HashOutcome {
    let path = PathBuf::from(real_path);
    for _ in 0..3 {
        let result = tokio::task::spawn_blocking({
            let path = path.clone();
            let shutdown = shutdown.clone();
            move || hash_file_blocking(&path, &shutdown, Some(mtime_ns))
        })
        .await;
        match result {
            Ok(Ok((sha256, actual_size, actual_mtime))) => {
                if actual_size == size && actual_mtime == mtime_ns {
                    return HashOutcome::Ready(HashValue {
                        sha256,
                        size: actual_size,
                        mtime_ns: actual_mtime,
                        computed_at: unix_now(),
                        cached: false,
                    });
                }
            }
            Ok(Err(AppError::Conflict(_))) => continue,
            Ok(Err(error)) => {
                return HashOutcome::Failed(HashFailure {
                    code: error.code().to_owned(),
                    message: error.to_string(),
                });
            }
            Err(error) => {
                return HashOutcome::Failed(HashFailure {
                    code: "INTERNAL_ERROR".to_owned(),
                    message: format!("hash task failed: {error}"),
                });
            }
        }
    }
    HashOutcome::Failed(HashFailure {
        code: "PENDING".to_owned(),
        message: "file changed while hashing".to_owned(),
    })
}

pub fn hash_file_blocking(
    path: &Path,
    cancel: &CancellationToken,
    expected_mtime_ns: Option<i64>,
) -> Result<(String, u64, i64), AppError> {
    let mut file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::NotFound
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::PermissionDenied
        } else {
            AppError::internal(anyhow::Error::from(error))
        }
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
    if !metadata.is_file() {
        return Err(AppError::bad_request("path is not a file"));
    }
    let mut mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or_default();
    if let Some(expected) = expected_mtime_ns
        && mtime_ns != expected
    {
        return Err(AppError::conflict("file changed before hashing"));
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; READ_BUFFER];
    let mut total = 0_u64;
    loop {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    let metadata = file
        .metadata()
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))?;
    mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or_default();
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    Ok((output, total, mtime_ns))
}

#[derive(Debug, Clone, Serialize)]
pub struct HashJson {
    path: String,
    algo: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mtime: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    computed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

fn value_json(path: &str, value: HashValue) -> HashJson {
    HashJson {
        path: path.to_owned(),
        algo: ALGO,
        status: "ready",
        sha256: Some(value.sha256),
        size: Some(value.size),
        mtime: Some(value.mtime_ns / 1_000_000_000),
        cached: Some(value.cached),
        computed_at: Some(value.computed_at),
        retry_after_ms: None,
        error: None,
    }
}

fn pending_json(path: &str, size: u64, mtime_ns: i64) -> HashJson {
    HashJson {
        path: path.to_owned(),
        algo: ALGO,
        status: "pending",
        sha256: None,
        size: Some(size),
        mtime: Some(mtime_ns / 1_000_000_000),
        cached: None,
        computed_at: None,
        retry_after_ms: Some(1000),
        error: None,
    }
}

fn error_json(path: &str, error: &AppError) -> HashJson {
    HashJson {
        path: path.to_owned(),
        algo: ALGO,
        status: "error",
        sha256: None,
        size: None,
        mtime: None,
        cached: None,
        computed_at: None,
        retry_after_ms: None,
        error: Some(json!({
            "code": error.code(),
            "message": error.to_string(),
        })),
    }
}

fn outcome_json(path: &str, outcome: HashOutcome) -> HashJson {
    match outcome {
        HashOutcome::Ready(value) => value_json(path, value),
        HashOutcome::Failed(failure) => HashJson {
            path: path.to_owned(),
            algo: ALGO,
            status: "error",
            sha256: None,
            size: None,
            mtime: None,
            cached: None,
            computed_at: None,
            retry_after_ms: None,
            error: Some(json!({ "code": failure.code, "message": failure.message })),
        },
    }
}

#[derive(Debug, Deserialize)]
pub struct HashQuery {
    root: i64,
    path: String,
    force: Option<bool>,
    wait: Option<bool>,
    timeout_ms: Option<u64>,
}

pub async fn get_hash(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<HashQuery>,
) -> Result<(StatusCode, Json<HashJson>), AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = state
        .db
        .find_root(query.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if root.is_remote() {
        return Err(AppError::HashUnsupported);
    }
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let metadata = tokio::fs::metadata(&resolved.absolute)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound
            } else {
                AppError::internal(anyhow::Error::from(error))
            }
        })?;
    if !metadata.is_file() {
        return Err(AppError::bad_request("path is not a file"));
    }
    let size = metadata.len();
    if size > state.settings().settings.hash.max_file_bytes {
        return Err(AppError::HashTooLarge(format!(
            "file is {size} bytes, limit is {}",
            state.settings().settings.hash.max_file_bytes
        )));
    }
    let mtime_ns = file_mtime_ns(&metadata);
    let real_path = resolved.absolute.to_string_lossy().to_string();
    if !query.force.unwrap_or(false)
        && let Some(cached) = state
            .db
            .file_hash(&real_path, ALGO)
            .await
            .map_err(AppError::internal)?
        && cached.size == size as i64
        && cached.mtime_ns == mtime_ns
    {
        return Ok((
            StatusCode::OK,
            Json(value_json(
                &resolved.relative,
                HashValue {
                    sha256: cached.sha256,
                    size: cached.size.max(0) as u64,
                    mtime_ns: cached.mtime_ns,
                    computed_at: cached.computed_at,
                    cached: true,
                },
            )),
        ));
    }
    let inflight = state
        .hash
        .request(
            current.user.id,
            real_path,
            size,
            mtime_ns,
            query.force.unwrap_or(false),
        )
        .await?;
    let wait = query.wait.unwrap_or(false);
    if !wait {
        if let Some(outcome) = inflight.ready() {
            return Ok((
                StatusCode::OK,
                Json(outcome_json(&resolved.relative, outcome)),
            ));
        }
        return Ok((
            StatusCode::ACCEPTED,
            Json(pending_json(&resolved.relative, size, mtime_ns)),
        ));
    }
    let timeout = Duration::from_millis(
        query
            .timeout_ms
            .unwrap_or(state.settings().settings.hash.wait_timeout_ms)
            .min(state.settings().settings.hash.wait_timeout_ms),
    );
    match tokio::time::timeout(timeout, inflight.wait()).await {
        Ok(HashOutcome::Failed(failure)) if failure.code == "PENDING" => Ok((
            StatusCode::ACCEPTED,
            Json(pending_json(&resolved.relative, size, mtime_ns)),
        )),
        Ok(outcome) => {
            let json = outcome_json(&resolved.relative, outcome);
            let status = if json.status == "ready" {
                StatusCode::OK
            } else {
                StatusCode::ACCEPTED
            };
            Ok((status, Json(json)))
        }
        Err(_) => Ok((
            StatusCode::ACCEPTED,
            Json(pending_json(&resolved.relative, size, mtime_ns)),
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct BatchHashRequest {
    root: i64,
    paths: Vec<String>,
    force: Option<bool>,
}

pub async fn post_hashes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<BatchHashRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    if request.paths.is_empty() || request.paths.len() > 100 {
        return Err(AppError::bad_request("paths must contain 1 to 100 entries"));
    }
    if current.auth_kind == auth::AuthKind::Session {
        auth::require_csrf(&current, &headers)?;
    }
    let root = state
        .db
        .find_root(request.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if root.is_remote() {
        return Err(AppError::HashUnsupported);
    }
    let force = request.force.unwrap_or(false);
    let mut unique = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in request.paths {
        if seen.insert(path.clone()) {
            unique.push(path);
        }
    }
    let resolved: Vec<Result<crate::pathresolver::ResolvedPath, AppError>> =
        match authz::resolve_many(&state, current.user.id, root.id, unique.clone(), false).await {
            Ok(values) => values.into_iter().map(Ok).collect(),
            Err(_) => {
                let mut values = Vec::with_capacity(unique.len());
                for path in &unique {
                    values
                        .push(authz::resolve(&state, current.user.id, root.id, path, false).await);
                }
                values
            }
        };
    let mut started: Vec<(String, u64, i64, BatchItem)> = Vec::new();
    for (index, item) in resolved.into_iter().enumerate() {
        let relative = unique.get(index).cloned().unwrap_or_default();
        match item {
            Err(error) => started.push((relative, 0, 0, BatchItem::Error(error))),
            Ok(resolved_path) => {
                let metadata = match tokio::fs::metadata(&resolved_path.absolute).await {
                    Ok(metadata) if metadata.is_file() => metadata,
                    Ok(_) => {
                        started.push((
                            relative,
                            0,
                            0,
                            BatchItem::Error(AppError::bad_request("path is not a file")),
                        ));
                        continue;
                    }
                    Err(error) => {
                        let error = if error.kind() == std::io::ErrorKind::NotFound {
                            AppError::NotFound
                        } else {
                            AppError::internal(anyhow::Error::from(error))
                        };
                        started.push((relative, 0, 0, BatchItem::Error(error)));
                        continue;
                    }
                };
                let size = metadata.len();
                if size > state.settings().settings.hash.max_file_bytes {
                    started.push((
                        relative,
                        size,
                        file_mtime_ns(&metadata),
                        BatchItem::Error(AppError::HashTooLarge(format!(
                            "file is {size} bytes, limit is {}",
                            state.settings().settings.hash.max_file_bytes
                        ))),
                    ));
                    continue;
                }
                let mtime_ns = file_mtime_ns(&metadata);
                let real_path = resolved_path.absolute.to_string_lossy().to_string();
                if !force
                    && let Some(cached) = state
                        .db
                        .file_hash(&real_path, ALGO)
                        .await
                        .map_err(AppError::internal)?
                    && cached.size == size as i64
                    && cached.mtime_ns == mtime_ns
                {
                    started.push((
                        relative,
                        size,
                        mtime_ns,
                        BatchItem::Value(HashValue {
                            sha256: cached.sha256,
                            size: cached.size.max(0) as u64,
                            mtime_ns: cached.mtime_ns,
                            computed_at: cached.computed_at,
                            cached: true,
                        }),
                    ));
                    continue;
                }
                match state
                    .hash
                    .request(current.user.id, real_path, size, mtime_ns, force)
                    .await
                {
                    Ok(inflight) => {
                        started.push((relative, size, mtime_ns, BatchItem::Inflight(inflight)))
                    }
                    Err(error) => started.push((relative, size, mtime_ns, BatchItem::Error(error))),
                }
            }
        }
    }
    let deadline = Instant::now() + Duration::from_millis(1200);
    let mut hashes = Vec::with_capacity(started.len());
    for (path, size, mtime_ns, item) in started {
        match item {
            BatchItem::Value(value) => hashes.push(value_json(&path, value)),
            BatchItem::Error(error) => hashes.push(error_json(&path, &error)),
            BatchItem::Inflight(inflight) => {
                let outcome = if let Some(outcome) = inflight.ready() {
                    Some(outcome)
                } else {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        None
                    } else {
                        tokio::time::timeout(remaining, inflight.wait()).await.ok()
                    }
                };
                match outcome {
                    Some(HashOutcome::Ready(value)) => {
                        hashes.push(value_json(&path, value));
                    }
                    Some(HashOutcome::Failed(failure)) if failure.code == "PENDING" => {
                        hashes.push(pending_json(&path, size, mtime_ns));
                    }
                    Some(HashOutcome::Failed(failure)) => hashes.push(HashJson {
                        path,
                        algo: ALGO,
                        status: "error",
                        sha256: None,
                        size: None,
                        mtime: None,
                        cached: None,
                        computed_at: None,
                        retry_after_ms: None,
                        error: Some(json!({ "code": failure.code, "message": failure.message })),
                    }),
                    None => hashes.push(pending_json(&path, size, mtime_ns)),
                }
            }
        }
    }
    Ok(Json(json!({ "hashes": hashes })))
}

fn file_mtime_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
pub struct PropertiesHashQuery {
    pub real_path: String,
    pub size: i64,
    pub mtime_ns: i64,
}

pub async fn cached_property_hash(
    state: &Arc<AppState>,
    real_path: &str,
    size: i64,
    mtime_ns: i64,
) -> Result<Option<Value>, AppError> {
    let Some(cached) = state
        .db
        .file_hash(real_path, ALGO)
        .await
        .map_err(AppError::internal)?
    else {
        return Ok(None);
    };
    if cached.size != size || cached.mtime_ns != mtime_ns {
        return Ok(None);
    }
    Ok(Some(json!({
        "algo": cached.algo,
        "sha256": cached.sha256,
        "hash_status": "ready",
        "hash_computed_at": cached.computed_at,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, b"").unwrap();
        let (hash, size, _) = hash_file_blocking(&path, &CancellationToken::new(), None).unwrap();
        assert_eq!(size, 0);
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hashes_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc");
        std::fs::write(&path, b"abc").unwrap();
        let (hash, size, _) = hash_file_blocking(&path, &CancellationToken::new(), None).unwrap();
        assert_eq!(size, 3);
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rejects_mtime_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"data").unwrap();
        let error = hash_file_blocking(&path, &CancellationToken::new(), Some(1)).unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }
}
