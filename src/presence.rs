use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::db::unix_now;
use crate::error::AppError;
use crate::state::AppState;

/// Presence is advisory only. A stale entry disappears after this many
/// seconds; the editor never blocks a save based on another tab.
pub const PRESENCE_TTL_SECONDS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresenceKey {
    pub user_id: i64,
    pub root_id: i64,
    pub path: String,
    pub client: String,
}

#[derive(Debug, Clone)]
pub struct PresenceEntry {
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct PresenceRequest {
    root: i64,
    path: String,
    state: String,
    client: String,
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PresenceRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let open = match request.state.as_str() {
        "open" => true,
        "close" => false,
        _ => return Err(AppError::bad_request("state must be open or close")),
    };
    let client = request.client.trim();
    if client.len() < 6
        || client.len() > 64
        || !client
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '-' || value == '_')
    {
        return Err(AppError::bad_request("invalid presence client"));
    }
    let resolved =
        authz::resolve(&state, current.user.id, request.root, &request.path, false).await?;
    let key = PresenceKey {
        user_id: current.user.id,
        root_id: resolved.root.id,
        path: resolved.relative.clone(),
        client: client.to_owned(),
    };
    let now = unix_now();
    let expires_at = now.saturating_add(PRESENCE_TTL_SECONDS);
    let (others_count, peers) = {
        let mut presence = state.edit_presence.lock();
        presence.retain(|_, entry| entry.expires_at > now);
        if open {
            presence.insert(key.clone(), PresenceEntry { expires_at });
        } else {
            presence.remove(&key);
        }
        let peers: Vec<_> = presence
            .iter()
            .filter(|(candidate, _)| {
                candidate.user_id == key.user_id
                    && candidate.root_id == key.root_id
                    && candidate.path == key.path
                    && candidate.client != key.client
            })
            .map(|(candidate, entry)| (candidate.client.clone(), entry.expires_at))
            .collect();
        (peers.len(), peers)
    };
    state.events.publish(
        "edit.presence",
        Some(current.user.id),
        json!({
            "root": key.root_id,
            "path": key.path,
            "client": key.client,
            "state": if open { "open" } else { "close" },
            "expires_at": expires_at,
            "others": others_count,
        }),
    );
    Ok(Json(json!({
        "ok": true,
        "others": others_count,
        "peers": peers.iter().map(|(client, expires_at)| json!({
            "client": client,
            "expires_at": expires_at,
        })).collect::<Vec<_>>(),
        "expires_at": expires_at,
    })))
}

pub fn cleanup(state: &AppState) {
    let now = unix_now();
    let expired = {
        let mut presence = state.edit_presence.lock();
        let expired: Vec<_> = presence
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &expired {
            presence.remove(key);
        }
        expired
    };
    for key in expired {
        state.events.publish(
            "edit.presence",
            Some(key.user_id),
            json!({
                "root": key.root_id,
                "path": key.path,
                "client": key.client,
                "state": "close",
                "expires_at": now,
                "others": 0,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn expired_entries_are_reported_as_closed() {
        let mut presence = HashMap::new();
        let key = PresenceKey {
            user_id: 1,
            root_id: 2,
            path: "notes.txt".to_owned(),
            client: "tab-one".to_owned(),
        };
        presence.insert(
            key,
            PresenceEntry {
                expires_at: unix_now().saturating_sub(1),
            },
        );
        let now = unix_now();
        presence.retain(|_, entry| entry.expires_at > now);
        assert!(presence.is_empty());
    }
}
