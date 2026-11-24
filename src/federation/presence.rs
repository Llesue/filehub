//! Live presence channel between instances.
//!
//! Each mount keeps one WebSocket to the peer. A lost connection marks the
//! mount offline immediately, which is much faster than the periodic health
//! loop. Old peers that do not expose the endpoint are left to the health
//! loop, so the feature degrades cleanly.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::state::AppState;

use super::crypto;

const PING_INTERVAL: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(10);
const MAX_BACKOFF_SECS: u64 = 60;

enum PresenceError {
    /// The peer answered the upgrade but rejected the token. Retrying cannot
    /// help until the mount is re-authenticated.
    Auth,
    /// The peer does not implement the endpoint (older version). Fall back to
    /// the health loop for this mount.
    Unsupported,
    /// Transport failure. `connected` says whether the socket had been
    /// established before it broke, which decides if we own the offline state.
    Transport { connected: bool },
}

pub fn start(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            supervise(&state).await;
            tokio::time::sleep(SUPERVISOR_INTERVAL).await;
        }
    });
}

async fn supervise(state: &Arc<AppState>) {
    let mounts = match state.db.federation_all_mounts().await {
        Ok(mounts) => mounts,
        Err(error) => {
            tracing::warn!(%error, "presence supervisor could not list mounts");
            return;
        }
    };
    let live: std::collections::HashSet<i64> = mounts.iter().map(|mount| mount.id).collect();
    let stale: Vec<i64> = {
        let tasks = state.presence_tasks.lock();
        tasks
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect()
    };
    for id in stale {
        if let Some(task) = state.presence_tasks.lock().remove(&id) {
            task.abort();
        }
    }
    for mount in mounts {
        if state.presence_unsupported.lock().contains(&mount.id) {
            continue;
        }
        if state.presence_tasks.lock().contains_key(&mount.id) {
            continue;
        }
        spawn_mount(Arc::clone(state), mount);
    }
}

/// Start a presence task for a freshly created or re-authenticated mount.
pub fn ensure(state: &Arc<AppState>, mount_id: i64) {
    state.presence_unsupported.lock().remove(&mount_id);
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let Ok(Some(mount)) = state.db.federation_mount_any(mount_id).await else {
            return;
        };
        {
            let mut tasks = state.presence_tasks.lock();
            if tasks.get(&mount_id).is_some_and(|task| !task.is_finished()) {
                return;
            }
            tasks.remove(&mount_id);
        }
        spawn_mount(Arc::clone(&state), mount);
    });
}

pub fn stop(state: &Arc<AppState>, mount_id: i64) {
    if let Some(task) = state.presence_tasks.lock().remove(&mount_id) {
        task.abort();
    }
    state.presence_unsupported.lock().remove(&mount_id);
}

fn spawn_mount(state: Arc<AppState>, mount: super::db::FederationMount) {
    let mount_id = mount.id;
    let task = tokio::spawn(run_mount(Arc::clone(&state), mount));
    if let Some(previous) = state.presence_tasks.lock().insert(mount_id, task) {
        previous.abort();
    }
}

async fn run_mount(state: Arc<AppState>, mount: super::db::FederationMount) {
    let mut backoff = 1_u64;
    loop {
        // Re-read the mount so re-authentication and URL failover are picked
        // up without restarting the task.
        let Ok(Some(current)) = state.db.federation_mount_any(mount.id).await else {
            return;
        };
        match run_once(&state, &current).await {
            Ok(()) => {
                mark(&state, &current, false, Some("presence_closed"), None).await;
                backoff = 1;
            }
            Err(PresenceError::Auth) => {
                mark(&state, &current, false, None, Some("UNAUTHORIZED")).await;
                return;
            }
            Err(PresenceError::Unsupported) => {
                state.presence_unsupported.lock().insert(current.id);
                tracing::debug!(
                    mount = current.id,
                    "peer does not expose presence websocket"
                );
                return;
            }
            Err(PresenceError::Transport { connected }) => {
                if connected {
                    mark(&state, &current, false, Some("presence_unreachable"), None).await;
                    backoff = 1;
                }
            }
        }
        tokio::time::sleep(jitter(backoff)).await;
        backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
    }
}

async fn run_once(
    state: &Arc<AppState>,
    mount: &super::db::FederationMount,
) -> Result<(), PresenceError> {
    let token = {
        let key = crypto::mount_key(&state.db)
            .await
            .map_err(|_| PresenceError::Auth)?;
        crypto::decrypt(&key, &mount.token_ciphertext).map_err(|_| PresenceError::Auth)?
    };
    let url = presence_url(&mount.base_url).ok_or(PresenceError::Unsupported)?;
    // TLS verification uses the original host. Unlike the HTTP client this
    // connection is not pinned to a resolved IP, but it only carries presence
    // pings and never reads peer data.
    let mut request = url
        .clone()
        .into_client_request()
        .map_err(|_| PresenceError::Unsupported)?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| PresenceError::Auth)?,
    );

    let socket = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| PresenceError::Transport { connected: false })?;
    let (mut socket, _) = match socket {
        Ok(pair) => pair,
        Err(error) => {
            use tokio_tungstenite::tungstenite::Error as WsError;
            return Err(match error {
                WsError::Http(response) if response.status() == 401 || response.status() == 403 => {
                    PresenceError::Auth
                }
                WsError::Http(response)
                    if response.status() == 404
                        || response.status() == 405
                        || response.status() == 426 =>
                {
                    PresenceError::Unsupported
                }
                _ => PresenceError::Transport { connected: false },
            });
        }
    };

    if !mark(state, mount, true, None, None).await {
        return Err(PresenceError::Auth);
    }
    let lost = || PresenceError::Transport { connected: true };
    tracing::debug!(mount = mount.id, "presence websocket connected");

    let mut ticker = tokio::time::interval(PING_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if last_seen.elapsed() >= READ_TIMEOUT {
                    return Err(lost());
                }
                if socket
                    .send(Message::Text("{\"type\":\"ping\"}".into()))
                    .await
                    .is_err()
                {
                    return Err(lost());
                }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = Instant::now();
                        if let Ok(value) = serde_json::from_str::<Value>(&text)
                            && value.get("type").and_then(Value::as_str) == Some("ping")
                            && socket
                                .send(Message::Text("{\"type\":\"pong\"}".into()))
                                .await
                                .is_err()
                        {
                            return Err(lost());
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        last_seen = Instant::now();
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_seen = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Err(_)) => return Err(lost()),
                    _ => {}
                }
            }
        }
    }
}

/// Mark the mount state and publish a transition. Returns false when the
/// update failed, which the caller treats as an auth problem.
async fn mark(
    state: &Arc<AppState>,
    mount: &super::db::FederationMount,
    online: bool,
    error: Option<&str>,
    code: Option<&str>,
) -> bool {
    if state
        .db
        .federation_update_mount_state(mount.id, online, error, code)
        .await
        .is_err()
    {
        return false;
    }
    state.events.publish(
        "roots.changed",
        Some(mount.user_id),
        serde_json::json!({
            "user_id": mount.user_id,
            "root_id": mount.root_id,
            "online": online,
        }),
    );
    true
}

fn presence_url(base: &str) -> Option<String> {
    let trimmed = base.trim_end_matches('/');
    let ws = trimmed
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| {
            trimmed
                .strip_prefix("http://")
                .map(|rest| format!("ws://{rest}"))
        })?;
    Some(format!("{ws}/api/federation/v1/presence"))
}

fn jitter(seconds: u64) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.subsec_nanos() as u64)
        .unwrap_or(0);
    let spread = seconds / 5 + 1;
    let extra = nanos % (spread * 2);
    Duration::from_secs(seconds.saturating_sub(spread) + extra + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_presence_url_for_both_schemes() {
        assert_eq!(
            presence_url("https://files.example.com").as_deref(),
            Some("wss://files.example.com/api/federation/v1/presence")
        );
        assert_eq!(
            presence_url("http://192.0.2.1:8080/").as_deref(),
            Some("ws://192.0.2.1:8080/api/federation/v1/presence")
        );
        assert!(presence_url("ftp://example.com").is_none());
    }

    #[test]
    fn jitter_stays_near_the_base() {
        for base in [1, 2, 4, 8, 16, 30, 60] {
            let value = jitter(base).as_secs();
            assert!(value >= base - base / 5, "jitter too small: {value}");
            assert!(value <= base + base / 5 + 1, "jitter too large: {value}");
        }
    }
}
