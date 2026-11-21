use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use serde_json::{Value, json};

use crate::auth;
use crate::db::{User, unix_now};
use crate::error::AppError;
use crate::fsops;
use crate::state::{AppState, UsageSnapshot};

pub async fn usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let ttl = state.settings().settings.jobs.usage_cache_seconds as i64;
    if let Some(snapshot) = state.usage_cache.lock().get(&current.user.id).cloned() {
        let stale = unix_now().saturating_sub(snapshot.computed_at) > ttl.max(1);
        if stale {
            spawn_refresh(Arc::clone(&state), current.user.clone());
        }
        return Ok(Json(json!({
            "quota_bytes": current.user.quota_bytes,
            "used_bytes": snapshot.used_bytes,
            "entry_count": snapshot.entry_count,
            "max_entries": state.settings().settings.max_entries_per_user,
            "disks": snapshot.disks,
            "computed_at": snapshot.computed_at,
            "stale": stale,
            "computing": stale,
        })));
    }
    spawn_refresh(Arc::clone(&state), current.user.clone());
    Ok(Json(json!({
        "quota_bytes": current.user.quota_bytes,
        "used_bytes": null,
        "entry_count": null,
        "max_entries": state.settings().settings.max_entries_per_user,
        "disks": [],
        "computed_at": null,
        "stale": false,
        "computing": true,
    })))
}

pub async fn admin_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    let users = state.db.list_users().await.map_err(AppError::internal)?;
    let ttl = state.settings().settings.jobs.usage_cache_seconds as i64;
    let mut result = serde_json::Map::new();
    for user in users {
        let cached = state.usage_cache.lock().get(&user.id).cloned();
        match cached {
            Some(snapshot) => {
                let stale = unix_now().saturating_sub(snapshot.computed_at) > ttl.max(1);
                if stale {
                    spawn_refresh(Arc::clone(&state), user.clone());
                }
                result.insert(
                    user.id.to_string(),
                    json!({
                        "used_bytes": snapshot.used_bytes,
                        "entry_count": snapshot.entry_count,
                        "quota_bytes": snapshot.quota_bytes,
                        "computed_at": snapshot.computed_at,
                        "computing": stale,
                    }),
                );
            }
            None => {
                spawn_refresh(Arc::clone(&state), user.clone());
                result.insert(
                    user.id.to_string(),
                    json!({
                        "used_bytes": null,
                        "entry_count": null,
                        "quota_bytes": user.quota_bytes,
                        "computed_at": null,
                        "computing": true,
                    }),
                );
            }
        }
    }
    Ok(Json(json!({ "users": result })))
}

pub async fn cached_or_scan(state: &Arc<AppState>, user: &User) -> Result<u64, AppError> {
    if let Some(snapshot) = state.usage_cache.lock().get(&user.id) {
        return Ok(snapshot.used_bytes);
    }
    let ttl = state.settings().settings.jobs.usage_cache_seconds as i64;
    let snapshot = load_snapshot(state, user, ttl).await?;
    Ok(snapshot.used_bytes)
}

/// Reject new entries once the per-user entry budget is exhausted.
///
/// The count comes from the same cached scan as byte usage, so a create does
/// not walk the whole home on every request.
pub async fn ensure_entry_capacity(
    state: &Arc<AppState>,
    user: &User,
    additional: u64,
) -> Result<(), AppError> {
    let max = state.settings().settings.max_entries_per_user;
    if max == 0 {
        return Ok(());
    }
    let ttl = state.settings().settings.jobs.usage_cache_seconds as i64;
    let snapshot = load_snapshot(state, user, ttl).await?;
    if snapshot.entry_count.saturating_add(additional) > max {
        return Err(AppError::EntryLimitExceeded);
    }
    Ok(())
}

pub async fn load_snapshot(
    state: &Arc<AppState>,
    user: &User,
    ttl_seconds: i64,
) -> Result<UsageSnapshot, AppError> {
    if let Some(snapshot) = state.usage_cache.lock().get(&user.id).cloned() {
        if unix_now().saturating_sub(snapshot.computed_at) <= ttl_seconds.max(1) {
            return Ok(snapshot);
        }
        spawn_refresh(Arc::clone(state), user.clone());
        return Ok(snapshot);
    }

    let lock = usage_lock(state, user.id);
    let _guard = lock.lock().await;
    if let Some(snapshot) = state.usage_cache.lock().get(&user.id).cloned()
        && unix_now().saturating_sub(snapshot.computed_at) <= ttl_seconds.max(1)
    {
        return Ok(snapshot);
    }
    let _permit = state
        .usage_semaphore
        .acquire()
        .await
        .map_err(|_| AppError::internal(anyhow::anyhow!("usage worker closed")))?;
    let snapshot = compute_snapshot(state, user).await?;
    state.usage_cache.lock().insert(user.id, snapshot.clone());
    state.events.publish(
        "usage.updated",
        Some(user.id),
        json!({
            "used_bytes": snapshot.used_bytes,
            "quota_bytes": snapshot.quota_bytes,
            "computed_at": snapshot.computed_at,
        }),
    );
    Ok(snapshot)
}

fn usage_lock(state: &Arc<AppState>, user_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    state
        .usage_locks
        .lock()
        .entry(user_id)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub(crate) fn spawn_refresh(state: Arc<AppState>, user: User) {
    tokio::spawn(async move {
        let lock = usage_lock(&state, user.id);
        let guard = lock.lock().await;
        if let Some(snapshot) = state.usage_cache.lock().get(&user.id).cloned() {
            let ttl = state.settings().settings.jobs.usage_cache_seconds as i64;
            if unix_now().saturating_sub(snapshot.computed_at) <= ttl.max(1) {
                return;
            }
        }
        match compute_snapshot(&state, &user).await {
            Ok(snapshot) => {
                state.usage_cache.lock().insert(user.id, snapshot.clone());
                state.events.publish(
                    "usage.updated",
                    Some(user.id),
                    json!({
                        "used_bytes": snapshot.used_bytes,
                        "quota_bytes": snapshot.quota_bytes,
                        "computed_at": snapshot.computed_at,
                    }),
                );
            }
            Err(error) => {
                tracing::warn!(user_id = user.id, %error, "failed to refresh usage cache");
            }
        }
        drop(guard);
    });
}

async fn compute_snapshot(state: &Arc<AppState>, user: &User) -> Result<UsageSnapshot, AppError> {
    let roots = state
        .db
        .list_roots(user.id)
        .await
        .map_err(AppError::internal)?;
    let home = PathBuf::from(user.home_dir.clone());
    let roots_for_task: Vec<_> = roots
        .into_iter()
        .filter(|root| {
            !root.is_remote() && !crate::platform::paths_equal(Path::new(&root.real_path), &home)
        })
        .collect();
    let home_for_task = home.clone();
    let (used_bytes, entry_count, disks) = tokio::task::spawn_blocking(move || {
        let (used, entries) = fsops::path_usage(&home_for_task)?;
        let disks = fsops::disk_usage(&roots_for_task)?;
        Ok::<_, AppError>((used, entries, disks))
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("usage task failed: {err}")))??;

    Ok(UsageSnapshot {
        used_bytes,
        entry_count,
        quota_bytes: user.quota_bytes,
        disks,
        computed_at: unix_now(),
    })
}
