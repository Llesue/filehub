use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::auth;
use crate::db::unix_now;
use crate::error::AppError;
use crate::state::AppState;

pub const API_URL: &str = "https://api.github.com/repos/Llesue/filehub/releases/latest";
pub const REPOSITORY: &str = "https://github.com/Llesue/filehub";
pub const RELEASES: &str = "https://github.com/Llesue/filehub/releases";
const MAX_BODY_BYTES: u64 = 256 * 1024;

#[derive(Default)]
struct UpdateState {
    latest: Option<String>,
    checked_at: Option<i64>,
    attempted_at: Option<i64>,
    etag: Option<String>,
}

pub struct UpdateChecker {
    state: Mutex<UpdateState>,
    inflight: AtomicBool,
    api_url: String,
    client: reqwest::Client,
}

impl UpdateChecker {
    pub fn new(api_url: impl Into<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("FileHub")
            .build()?;
        Ok(Self {
            state: Mutex::new(UpdateState::default()),
            inflight: AtomicBool::new(false),
            api_url: api_url.into(),
            client,
        })
    }

    pub fn snapshot(&self, enabled: bool, current: &str) -> Value {
        let guard = self.state.lock();
        let latest = guard.latest.clone();
        let update_available = latest
            .as_deref()
            .is_some_and(|latest| newer(latest, current));
        json!({
            "version": current,
            "repository": REPOSITORY,
            "releases": RELEASES,
            "check_enabled": enabled,
            "latest": latest,
            "update_available": update_available,
            "checked_at": guard.checked_at,
        })
    }

    /// Fire-and-forget refresh used by `GET /version`. Never blocks the request.
    pub fn maybe_refresh(self: &Arc<Self>, enabled: bool, interval_hours: i64, current: &str) {
        if !enabled {
            return;
        }
        let now = unix_now();
        let due = {
            let guard = self.state.lock();
            guard
                .attempted_at
                .is_none_or(|at| now.saturating_sub(at) >= interval_hours.max(1) * 3600)
        };
        if !due || self.inflight.swap(true, Ordering::SeqCst) {
            return;
        }
        let checker = Arc::clone(self);
        let current = current.to_owned();
        tokio::spawn(async move {
            if let Err(error) = checker.refresh_once(&current).await {
                tracing::debug!(error = %error, "update check failed");
            }
            checker.inflight.store(false, Ordering::SeqCst);
        });
    }

    pub async fn refresh_now(&self, current: &str) -> anyhow::Result<()> {
        self.refresh_once(current).await
    }

    async fn refresh_once(&self, _current: &str) -> anyhow::Result<()> {
        let etag = self.state.lock().etag.clone();
        let mut request = self
            .client
            .get(&self.api_url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json");
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                self.record_attempt();
                return Err(error.into());
            }
        };
        let status = response.status();
        if status == reqwest::StatusCode::NOT_MODIFIED {
            self.record_success(None, None);
            return Ok(());
        }
        if !status.is_success() {
            self.record_attempt();
            anyhow::bail!("github returned {status}");
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BODY_BYTES)
        {
            self.record_attempt();
            anyhow::bail!("github response too large");
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body: Value = response.json().await?;
        let tag = body
            .get("tag_name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(latest) = normalize_tag(tag) else {
            self.record_attempt();
            anyhow::bail!("invalid tag name");
        };
        self.record_success(Some(latest), etag);
        Ok(())
    }

    fn record_attempt(&self) {
        self.state.lock().attempted_at = Some(unix_now());
    }

    fn record_success(&self, latest: Option<String>, etag: Option<String>) {
        let now = unix_now();
        let mut guard = self.state.lock();
        guard.attempted_at = Some(now);
        guard.checked_at = Some(now);
        if let Some(latest) = latest {
            guard.latest = Some(latest);
        }
        if let Some(etag) = etag {
            guard.etag = Some(etag);
        }
    }
}

fn normalize_tag(tag: &str) -> Option<String> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = tag.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{major}.{minor}.{patch}"))
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let mut parts = value.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn newer(latest: &str, current: &str) -> bool {
    let Some(latest) = parse_version(latest) else {
        return false;
    };
    let Some(current) = parse_version(current) else {
        return false;
    };
    latest > current
}

pub async fn version(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let _current = auth::current_user(&state, &headers).await?;
    let snapshot = state.settings();
    let enabled = snapshot.settings.updates.check_enabled;
    state.updates.maybe_refresh(
        enabled,
        snapshot.settings.updates.check_interval_hours,
        env!("CARGO_PKG_VERSION"),
    );
    Ok(Json(
        state.updates.snapshot(enabled, env!("CARGO_PKG_VERSION")),
    ))
}

pub async fn admin_check(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let snapshot = state.settings();
    if !snapshot.settings.updates.check_enabled {
        return Err(AppError::Conflict("updates are disabled".to_owned()));
    }
    if let Err(error) = state.updates.refresh_now(env!("CARGO_PKG_VERSION")).await {
        tracing::warn!(error = %error, "manual update check failed");
        return Err(AppError::BadRequest("update check failed".to_owned()));
    }
    Ok(Json(
        state.updates.snapshot(true, env!("CARGO_PKG_VERSION")),
    ))
}

#[cfg(test)]
mod tests {
    use super::{newer, normalize_tag, parse_version};

    #[test]
    fn parses_strict_versions() {
        assert_eq!(parse_version("0.2.1"), Some((0, 2, 1)));
        assert_eq!(parse_version("v1.10.3"), Some((1, 10, 3)));
        assert_eq!(parse_version("0.2.0-rc1"), None);
        assert_eq!(parse_version("0.2"), None);
        assert_eq!(parse_version("x.y.z"), None);
    }

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize_tag("v0.2.1").as_deref(), Some("0.2.1"));
        assert_eq!(normalize_tag("0.2.1").as_deref(), Some("0.2.1"));
        assert_eq!(normalize_tag("v0.2.1-rc1"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(newer("0.2.1", "0.2.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.1.9", "0.2.0"));
        assert!(!newer("bad", "0.2.0"));
    }
}
