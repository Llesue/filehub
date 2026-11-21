use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::db::User;
use crate::error::AppError;
use crate::federation::client::PeerClient;
use crate::federation::crypto;
use crate::federation::db::FederationMount;
use crate::federation::ssrf;
use crate::state::AppState;

fn require_outbound(state: &Arc<AppState>, user: &User) -> Result<(), AppError> {
    if !state.settings().settings.federation.enabled_outbound || !user.feature_federation_mount {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

async fn target_for(state: &Arc<AppState>, url: &str) -> Result<ssrf::ResolvedTarget, AppError> {
    let settings = state.settings();
    ssrf::resolve(
        url,
        settings.settings.federation.allow_loopback,
        settings.settings.federation.allow_plaintext_lan,
    )
    .await
    .map_err(|error| AppError::bad_request(error.message()))
}

async fn client_for_mount(
    state: &Arc<AppState>,
    mount: &FederationMount,
) -> Result<PeerClient, AppError> {
    let key = crypto::mount_key(&state.db)
        .await
        .map_err(AppError::internal)?;
    let token =
        crypto::decrypt(&key, &mount.token_ciphertext).map_err(|_| AppError::TokenRevoked)?;
    let target = target_for(state, &mount.base_url).await?;
    PeerClient::new(
        &mount.base_url,
        &target.host,
        target.ip,
        target.port,
        Some(&token),
        state.settings().settings.federation.request_timeout_seconds,
    )
    .map_err(AppError::internal)
}

pub async fn client_for_root(
    state: &Arc<AppState>,
    user_id: i64,
    root_id: i64,
) -> Result<(Arc<PeerClient>, FederationMount), AppError> {
    let mount = state
        .db
        .federation_mount_by_root(root_id, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if let Some(client) = state.federation_clients.lock().get(&root_id).cloned() {
        return Ok((client, mount));
    }
    let client = Arc::new(client_for_mount(state, &mount).await?);
    state
        .federation_clients
        .lock()
        .insert(root_id, Arc::clone(&client));
    Ok((client, mount))
}

#[derive(Debug, Deserialize)]
pub struct ProbeRequest {
    url: String,
}

pub async fn probe(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ProbeRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    require_outbound(&state, &current.user)?;
    if !state
        .public_rate
        .allow(&format!("federation-probe:{}", current.user.id), 10, 60)
    {
        return Err(AppError::TooManyRequests);
    }
    let target = target_for(&state, &request.url).await?;
    let client = PeerClient::new(
        &request.url,
        &target.host,
        target.ip,
        target.port,
        None,
        state.settings().settings.federation.request_timeout_seconds,
    )
    .map_err(AppError::internal)?;
    let info = client.info().await.map_err(|error| error.into_app())?;
    Ok(Json(json!({
        "ok": true,
        "product": info.get("product").cloned().unwrap_or(Value::Null),
        "api": info.get("api").cloned().unwrap_or(Value::Null),
        "version": info.get("version").cloned().unwrap_or(Value::Null),
        "server_version": info.get("server_version").cloned().unwrap_or(Value::Null),
        "instance_id": info.get("instance_id").cloned().unwrap_or(Value::Null),
        "capabilities": info.get("capabilities").cloned().unwrap_or(json!([])),
        "resolved_ips": [target.ip.to_string()],
        "address_class": target.class,
        "plaintext": target.plaintext,
    })))
}

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    url: String,
    token: String,
}

pub async fn preview(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    require_outbound(&state, &current.user)?;
    let target = target_for(&state, &request.url).await?;
    let client = PeerClient::new(
        &request.url,
        &target.host,
        target.ip,
        target.port,
        Some(&request.token),
        state.settings().settings.federation.request_timeout_seconds,
    )
    .map_err(AppError::internal)?;
    let grants = client.grants().await.map_err(|error| error.into_app())?;
    Ok(Json(json!({
        "grants": grants.get("grants").cloned().unwrap_or(json!([])),
    })))
}

#[derive(Debug, Deserialize)]
pub struct CreateMountRequest {
    url: String,
    token: String,
    grant_id: i64,
    alias: Option<String>,
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateMountRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_outbound(&state, &current.user)?;
    let limit = state.settings().settings.federation.max_mounts_per_user as i64;
    if state
        .db
        .federation_count_mounts(current.user.id)
        .await
        .map_err(AppError::internal)?
        >= limit
    {
        return Err(AppError::TooManyRequests);
    }
    let target = target_for(&state, &request.url).await?;
    let client = PeerClient::new(
        &request.url,
        &target.host,
        target.ip,
        target.port,
        Some(&request.token),
        state.settings().settings.federation.request_timeout_seconds,
    )
    .map_err(AppError::internal)?;
    let info = client.info().await.map_err(|error| error.into_app())?;
    let own_instance = state
        .db
        .federation_instance_id()
        .await
        .map_err(AppError::internal)?;
    if info.get("instance_id").and_then(Value::as_str) == Some(own_instance.as_str()) {
        return Err(AppError::bad_request("cannot mount this instance"));
    }
    let grants = client.grants().await.map_err(|error| error.into_app())?;
    let grant = grants
        .get("grants")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_i64) == Some(request.grant_id))
        })
        .ok_or(AppError::NotFound)?;
    let label = grant
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("Remote")
        .to_owned();
    let mode = grant
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("ro")
        .to_owned();
    let key = crypto::mount_key(&state.db)
        .await
        .map_err(AppError::internal)?;
    let ciphertext = crypto::encrypt(&key, &request.token).map_err(AppError::internal)?;
    let alias = request.alias.unwrap_or_else(|| label.clone());
    let sentinel = format!("/__filehub_remote__/{}", auth::random_token(12));
    let root_id = state
        .db
        .federation_add_remote_root(current.user.id, &alias, &sentinel, mode != "rw")
        .await
        .map_err(AppError::internal)?;
    let created = async {
        let mount_id = state
            .db
            .federation_create_mount(
                current.user.id,
                root_id,
                &request.url,
                request.grant_id,
                &label,
                &mode,
                info.get("server_version")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                &serde_json::to_string(info.get("capabilities").unwrap_or(&json!([])))
                    .unwrap_or_else(|_| "[]".to_owned()),
                &ciphertext,
            )
            .await
            .map_err(AppError::internal)?;
        state
            .db
            .federation_bind_root(root_id, mount_id)
            .await
            .map_err(AppError::internal)?;
        Ok::<i64, AppError>(mount_id)
    }
    .await;
    let mount_id = match created {
        Ok(id) => id,
        Err(error) => {
            let _ = state.db.delete_root(root_id, current.user.id).await;
            return Err(error);
        }
    };
    Ok(Json(json!({
        "id": mount_id,
        "root_id": root_id,
        "alias": alias,
        "mode": mode,
        "grant_id": request.grant_id,
    })))
}

fn directory_size(path: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0_u64;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path()));
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

fn capability_refresh_seen() -> &'static Mutex<HashMap<i64, i64>> {
    static SEEN: OnceLock<Mutex<HashMap<i64, i64>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn schedule_capability_refresh(state: &Arc<AppState>, mount: &FederationMount) {
    let now = crate::db::unix_now();
    {
        let mut seen = capability_refresh_seen()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if now - seen.get(&mount.id).copied().unwrap_or(0) < 300 {
            return;
        }
        seen.insert(mount.id, now);
    }
    let state = Arc::clone(state);
    let mount = mount.clone();
    tokio::spawn(async move {
        let Ok(client) = client_for_mount(&state, &mount).await else {
            return;
        };
        let Ok(info) = client.info().await else {
            return;
        };
        let capabilities = info
            .get("capabilities")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let encoded = serde_json::to_string(&capabilities).unwrap_or_else(|_| "[]".to_owned());
        let _ = state
            .db
            .federation_update_mount_capabilities(mount.id, mount.user_id, &encoded)
            .await;
    });
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let mounts = state
        .db
        .federation_list_mounts(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let mut items = Vec::new();
    for mount in mounts {
        schedule_capability_refresh(&state, &mount);
        let root = state
            .db
            .find_root(mount.root_id, current.user.id)
            .await
            .map_err(AppError::internal)?;
        let thumb_dir = state
            .settings()
            .settings
            .cache_dir
            .join("federation-thumbs")
            .join(mount.id.to_string());
        let thumb_cache_bytes = directory_size(&thumb_dir);
        let state_name = match mount.last_error_code.as_deref() {
            Some("TOKEN_REVOKED" | "TOKEN_EXPIRED") => "token_invalid",
            Some("GRANT_REVOKED") => "grant_revoked",
            Some("REMOTE_ROOT_UNSUPPORTED") => "unsupported",
            Some(_) => "offline",
            None if mount.last_error.is_some() => "offline",
            None => "online",
        };
        items.push(json!({
            "id": mount.id,
            "root_id": mount.root_id,
            "alias": root.as_ref().map(|root| root.alias.clone()).unwrap_or_default(),
            "base_url": mount.base_url,
            "grant_id": mount.grant_id,
            "grant_label": mount.grant_label,
            "mode": mount.mode,
            "capabilities": serde_json::from_str::<Value>(&mount.capabilities)
                .unwrap_or_else(|_| json!([])),
            "thumb_cache_bytes": thumb_cache_bytes,
            "state": state_name,
            "last_ok_at": mount.last_ok_at,
            "last_error": mount.last_error,
            "last_error_code": mount.last_error_code,
            "created_at": mount.created_at,
        }));
    }
    Ok(Json(json!({ "mounts": items })))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let mount = state
        .db
        .federation_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let deleted = state
        .db
        .federation_delete_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    state.federation_clients.lock().remove(&mount.root_id);
    state.federation_streams.lock().remove(&mount.id);
    state
        .federation_cache
        .lock()
        .retain(|key, _| key.0 != mount.root_id);
    let thumb_dir = state
        .settings()
        .settings
        .cache_dir
        .join("federation-thumbs")
        .join(mount.id.to_string());
    let _ = tokio::fs::remove_dir_all(thumb_dir).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn test(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let mount = state
        .db
        .federation_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let started = Instant::now();
    let client = client_for_mount(&state, &mount).await?;
    let result = client.info().await;
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok(info) => {
            state
                .db
                .federation_update_mount_state(id, true, None, None)
                .await
                .map_err(AppError::internal)?;
            Ok(Json(json!({
                "ok": true,
                "latency_ms": latency,
                "server_version": info.get("server_version").cloned().unwrap_or(Value::Null),
            })))
        }
        Err(error) => {
            state
                .db
                .federation_update_mount_state(id, false, Some(&error.message), Some(&error.code))
                .await
                .map_err(AppError::internal)?;
            Ok(Json(json!({
                "ok": false,
                "latency_ms": latency,
                "code": error.code,
                "message": error.message,
            })))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ReauthRequest {
    token: String,
}

pub async fn reauth(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(request): Json<ReauthRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let mount = state
        .db
        .federation_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let target = target_for(&state, &mount.base_url).await?;
    let client = PeerClient::new(
        &mount.base_url,
        &target.host,
        target.ip,
        target.port,
        Some(&request.token),
        state.settings().settings.federation.request_timeout_seconds,
    )
    .map_err(AppError::internal)?;
    client.grants().await.map_err(|error| error.into_app())?;
    let key = crypto::mount_key(&state.db)
        .await
        .map_err(AppError::internal)?;
    let ciphertext = crypto::encrypt(&key, &request.token).map_err(AppError::internal)?;
    state
        .db
        .federation_update_mount_token(id, current.user.id, &ciphertext)
        .await
        .map_err(AppError::internal)?;
    state.federation_clients.lock().remove(&mount.root_id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn activity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let rows = state
        .db
        .federation_activity(current.user.id, 50)
        .await
        .map_err(AppError::internal)?;
    let activity: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "op": row.op,
                "path": row.path,
                "status": row.status,
                "bytes": row.bytes,
                "created_at": row.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "activity": activity })))
}

#[derive(Debug, Deserialize)]
pub struct ThumbWarmRequest {
    root: i64,
    paths: Vec<String>,
    width: Option<u32>,
}

pub async fn thumb_warm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ThumbWarmRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let root = state
        .db
        .find_root(request.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if !root.is_remote() {
        return Err(AppError::bad_request("root is not a remote mount"));
    }
    let settings = state.settings();
    if !settings.settings.federation.enabled_outbound || !current.user.feature_federation_mount {
        return Err(AppError::Forbidden);
    }
    if !settings.settings.federation.thumb_warm {
        return Ok(Json(json!({
            "queued": 0,
            "cached": 0,
            "skipped": 0,
            "disabled": true,
        })));
    }
    if request.paths.is_empty() || request.paths.len() > 500 {
        return Err(AppError::bad_request("paths must contain 1-500 items"));
    }
    let width = request.width.unwrap_or(256).clamp(16, 1024);
    crate::federation::proxy::thumb_warm(&state, current.user.id, &root, &request.paths, width)
        .await
}

#[derive(Debug, Deserialize)]
pub struct RenameMountRequest {
    alias: String,
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(request): Json<RenameMountRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_outbound(&state, &current.user)?;
    let alias = request.alias.trim();
    if alias.is_empty() || alias.chars().count() > 64 {
        return Err(AppError::bad_request("alias must contain 1-64 characters"));
    }
    let changed = state
        .db
        .federation_update_mount_alias(id, current.user.id, alias)
        .await
        .map_err(AppError::internal)?;
    if !changed {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "id": id, "alias": alias })))
}
