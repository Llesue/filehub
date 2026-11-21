use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use futures_util::StreamExt;
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
    let url = ssrf::normalize_url(&request.url);
    let target = target_for(&state, &url).await?;
    let client = PeerClient::new(
        &url,
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
    let url = ssrf::normalize_url(&request.url);
    let target = target_for(&state, &url).await?;
    let client = PeerClient::new(
        &url,
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

async fn ensure_mount_limit(state: &Arc<AppState>, user_id: i64) -> Result<(), AppError> {
    let limit = state.settings().settings.federation.max_mounts_per_user as i64;
    if state
        .db
        .federation_count_mounts(user_id)
        .await
        .map_err(AppError::internal)?
        >= limit
    {
        return Err(AppError::TooManyRequests);
    }
    Ok(())
}

async fn mount_inner(
    state: &Arc<AppState>,
    user: &User,
    url: &str,
    token: &str,
    grant_id: i64,
    alias: Option<String>,
) -> Result<(i64, i64, String, String), AppError> {
    let url = ssrf::normalize_url(url);
    let target = target_for(state, &url).await?;
    let client = PeerClient::new(
        &url,
        &target.host,
        target.ip,
        target.port,
        Some(token),
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
                .find(|item| item.get("id").and_then(Value::as_i64) == Some(grant_id))
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
    let owner_name = grant
        .get("owner_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let avatar_version = grant
        .get("avatar_version")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let key = crypto::mount_key(&state.db)
        .await
        .map_err(AppError::internal)?;
    let ciphertext = crypto::encrypt(&key, token).map_err(AppError::internal)?;
    let alias = alias.unwrap_or_else(|| label.clone());
    let existing_roots = state
        .db
        .list_roots(user.id)
        .await
        .map_err(AppError::internal)?;
    let mut unique = alias.clone();
    let mut suffix = 2;
    while existing_roots.iter().any(|root| root.alias == unique) {
        unique = format!("{alias} ({suffix})");
        suffix += 1;
    }
    let alias = unique;
    let sentinel = format!("/__filehub_remote__/{}", auth::random_token(12));
    let root_id = state
        .db
        .federation_add_remote_root(user.id, &alias, &sentinel, mode != "rw")
        .await
        .map_err(AppError::internal)?;
    let created = async {
        let mount_id = state
            .db
            .federation_create_mount(
                user.id,
                root_id,
                &url,
                grant_id,
                &label,
                &mode,
                info.get("server_version")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                &serde_json::to_string(info.get("capabilities").unwrap_or(&json!([])))
                    .unwrap_or_else(|_| "[]".to_owned()),
                &ciphertext,
                &owner_name,
                avatar_version,
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
            let _ = state.db.delete_root(root_id, user.id).await;
            return Err(error);
        }
    };
    state
        .db
        .federation_update_mount_capabilities(
            mount_id,
            user.id,
            &serde_json::to_string(info.get("capabilities").unwrap_or(&json!([])))
                .unwrap_or_else(|_| "[]".to_owned()),
        )
        .await
        .ok();
    Ok((mount_id, root_id, alias, mode))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateMountRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_outbound(&state, &current.user)?;
    ensure_mount_limit(&state, current.user.id).await?;
    let (mount_id, root_id, alias, mode) = mount_inner(
        &state,
        &current.user,
        &request.url,
        &request.token,
        request.grant_id,
        request.alias,
    )
    .await?;
    state.events.publish(
        "roots.changed",
        Some(current.user.id),
        json!({ "user_id": current.user.id }),
    );
    Ok(Json(json!({
        "id": mount_id,
        "root_id": root_id,
        "alias": alias,
        "mode": mode,
        "grant_id": request.grant_id,
    })))
}

pub async fn mount_grants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    require_outbound(&state, &current.user)?;
    let mount = state
        .db
        .federation_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let client = client_for_mount(&state, &mount).await?;
    let value = client.grants().await.map_err(|error| error.into_app())?;
    let mounts = state
        .db
        .federation_list_mounts(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let items: Vec<Value> = value
        .get("grants")
        .and_then(Value::as_array)
        .map(|grants| {
            grants
                .iter()
                .filter_map(|grant| {
                    let grant_id = grant.get("id").and_then(Value::as_i64)?;
                    let existing = mounts
                        .iter()
                        .find(|m| m.base_url == mount.base_url && m.grant_id == grant_id);
                    Some(json!({
                        "id": grant_id,
                        "label": grant.get("label").cloned().unwrap_or(Value::Null),
                        "mode": grant.get("mode").cloned().unwrap_or(Value::Null),
                        "mounted": existing.is_some(),
                        "mount_id": existing.map(|m| m.id),
                        "current": grant_id == mount.grant_id,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(json!({
        "mount_id": mount.id,
        "base_url": mount.base_url,
        "grants": items,
    })))
}

pub async fn attach_grant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((id, grant_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_outbound(&state, &current.user)?;
    ensure_mount_limit(&state, current.user.id).await?;
    let source = state
        .db
        .federation_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let mounts = state
        .db
        .federation_list_mounts(current.user.id)
        .await
        .map_err(AppError::internal)?;
    if mounts
        .iter()
        .any(|m| m.base_url == source.base_url && m.grant_id == grant_id)
    {
        return Err(AppError::Conflict(
            "this grant is already mounted".to_owned(),
        ));
    }
    let key = crypto::mount_key(&state.db)
        .await
        .map_err(AppError::internal)?;
    let token = crypto::decrypt(&key, &source.token_ciphertext)
        .map_err(|_| AppError::RemoteTokenInvalid)?;
    let (mount_id, root_id, alias, mode) = mount_inner(
        &state,
        &current.user,
        &source.base_url,
        &token,
        grant_id,
        None,
    )
    .await?;
    state.events.publish(
        "roots.changed",
        Some(current.user.id),
        json!({ "user_id": current.user.id }),
    );
    Ok(Json(json!({
        "id": mount_id,
        "root_id": root_id,
        "alias": alias,
        "mode": mode,
        "grant_id": grant_id,
    })))
}

pub(crate) async fn health_loop(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let mounts = match state.db.federation_all_mounts().await {
            Ok(mounts) => mounts,
            Err(error) => {
                tracing::warn!(%error, "federation health check could not list mounts");
                continue;
            }
        };
        let check_state = Arc::clone(&state);
        futures_util::stream::iter(mounts)
            .for_each_concurrent(4, move |mount| {
                let state = Arc::clone(&check_state);
                async move {
                    health_check(&state, mount).await;
                }
            })
            .await;
    }
}

fn has_grant(value: &Value, grant_id: i64) -> bool {
    value
        .get("grants")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("id").and_then(Value::as_i64) == Some(grant_id))
        })
}

async fn health_check(state: &Arc<AppState>, mount: FederationMount) {
    let client = match client_for_mount(state, &mount).await {
        Ok(client) => client,
        Err(error) => {
            let _ = state
                .db
                .federation_update_mount_state(
                    mount.id,
                    false,
                    Some(&error.to_string()),
                    Some("OFFLINE"),
                )
                .await;
            return;
        }
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), client.grants()).await {
        Ok(Ok(value)) => {
            if !has_grant(&value, mount.grant_id) {
                tracing::info!(mount = mount.id, "remote grant is gone; unmounting");
                let _ = remove_mount(state, mount.user_id, &mount, Some("GRANT_REVOKED")).await;
                return;
            }
            let _ = state
                .db
                .federation_update_mount_state(mount.id, true, None, None)
                .await;
            if let Some(grant) = value
                .get("grants")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("id").and_then(Value::as_i64) == Some(mount.grant_id))
                })
            {
                let name = grant
                    .get("owner_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let version = grant
                    .get("avatar_version")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let _ = state
                    .db
                    .federation_update_mount_owner(mount.id, name, version)
                    .await;
                prune_avatar_cache(state, mount.id, version);
            }
        }
        Ok(Err(error)) => {
            if is_auth_error(&error) {
                tracing::info!(
                    mount = mount.id,
                    code = %error.code,
                    "remote token is invalid; unmounting"
                );
                let _ = remove_mount(state, mount.user_id, &mount, Some(error.code.as_str())).await;
            } else {
                let _ = state
                    .db
                    .federation_update_mount_state(
                        mount.id,
                        false,
                        Some(&error.message),
                        Some(&error.code),
                    )
                    .await;
            }
        }
        Err(_) => {
            let _ = state
                .db
                .federation_update_mount_state(
                    mount.id,
                    false,
                    Some("peer check timed out"),
                    Some("OFFLINE"),
                )
                .await;
        }
    }
}

fn prune_avatar_cache(state: &Arc<AppState>, mount_id: i64, keep_version: i64) {
    let dir = state
        .settings()
        .settings
        .cache_dir
        .join("federation-avatars");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{mount_id}-");
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with(&prefix) && name != format!("{mount_id}-{keep_version}.jpg") {
            let _ = std::fs::remove_file(path);
        }
    }
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
            "owner_name": mount.owner_name,
            "avatar_version": mount.avatar_version,
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

pub(crate) async fn remove_mount(
    state: &Arc<AppState>,
    user_id: i64,
    mount: &FederationMount,
    reason: Option<&str>,
) -> Result<bool, AppError> {
    if let Ok(client) = client_for_mount(state, mount).await
        && let Ok(uploads) = state.db.uploads_for_root(user_id, mount.root_id).await
    {
        for upload in uploads {
            if let Some(peer_id) = upload.remote_upload_id.as_deref() {
                let _ = client.cancel_upload(peer_id).await;
            }
            let _ = state.db.delete_upload(&upload.id, user_id).await;
        }
    }
    let limit = state
        .settings()
        .settings
        .federation
        .stream_concurrency
        .max(1) as u32;
    let stream_semaphore = state.federation_streams.lock().get(&mount.id).cloned();
    if let Some(semaphore) = stream_semaphore {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Ok(permit) = semaphore.clone().try_acquire_many_owned(limit) {
                drop(permit);
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    let deleted = state
        .db
        .federation_delete_mount(mount.id, user_id)
        .await
        .map_err(AppError::internal)?;
    if deleted {
        let _ = state.db.delete_root(mount.root_id, user_id).await;
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
    let avatar_dir = state
        .settings()
        .settings
        .cache_dir
        .join("federation-avatars");
    if let Ok(entries) = std::fs::read_dir(avatar_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&format!("{}-", mount.id)))
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    state.events.publish(
        "roots.changed",
        Some(user_id),
        json!({ "user_id": user_id }),
    );
    if let Some(reason) = reason {
        state.events.publish(
            "remote.unmounted",
            Some(user_id),
            json!({
                "user_id": user_id,
                "mount_id": mount.id,
                "alias": mount.grant_label,
                "reason": reason,
            }),
        );
    }
    Ok(deleted)
}

fn is_auth_error(error: &crate::federation::client::PeerError) -> bool {
    matches!(
        error.code.as_str(),
        "UNAUTHORIZED" | "TOKEN_REVOKED" | "TOKEN_EXPIRED" | "GRANT_REVOKED"
    ) || error.status == 401
}

pub async fn avatar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let mount = state
        .db
        .federation_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if mount.avatar_version <= 0 {
        return Err(AppError::NotFound);
    }
    let cache_dir = state
        .settings()
        .settings
        .cache_dir
        .join("federation-avatars");
    let cache_path = cache_dir.join(format!("{}-{}.jpg", mount.id, mount.avatar_version));
    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return Response::builder()
            .status(StatusCode::OK)
            .header(axum::http::header::CONTENT_TYPE, "image/jpeg")
            .header(axum::http::header::CACHE_CONTROL, "private, max-age=300")
            .body(axum::body::Body::from(bytes))
            .map_err(|error| AppError::internal(anyhow::anyhow!(error)));
    }
    let client = client_for_mount(&state, &mount).await?;
    let response = client
        .avatar(mount.grant_id)
        .await
        .map_err(|error| error.into_app())?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(AppError::NotFound);
    }
    let _ = crate::platform::create_private_dir_all(&cache_dir);
    let _ = crate::platform::write_private_file(&cache_path, &bytes);
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "image/jpeg")
        .header(axum::http::header::CACHE_CONTROL, "private, max-age=300")
        .body(axum::body::Body::from(bytes))
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))
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
    let deleted = remove_mount(&state, current.user.id, &mount, None).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
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
    let info = match client.info().await {
        Ok(info) => info,
        Err(error) => {
            state
                .db
                .federation_update_mount_state(id, false, Some(&error.message), Some(&error.code))
                .await
                .map_err(AppError::internal)?;
            return Ok(Json(json!({
                "ok": false,
                "removed": false,
                "latency_ms": started.elapsed().as_millis() as i64,
                "code": error.code,
                "message": error.message,
            })));
        }
    };
    let latency = started.elapsed().as_millis() as i64;
    if let Err(error) = client.grants().await {
        if is_auth_error(&error) {
            remove_mount(&state, current.user.id, &mount, Some(error.code.as_str())).await?;
            return Ok(Json(json!({
                "ok": false,
                "removed": true,
                "latency_ms": latency,
                "code": error.code,
                "message": error.message,
            })));
        }
        state
            .db
            .federation_update_mount_state(id, false, Some(&error.message), Some(&error.code))
            .await
            .map_err(AppError::internal)?;
        return Ok(Json(json!({
            "ok": false,
            "removed": false,
            "latency_ms": latency,
            "code": error.code,
            "message": error.message,
        })));
    }
    state
        .db
        .federation_update_mount_state(id, true, None, None)
        .await
        .map_err(AppError::internal)?;
    let capabilities = info
        .get("capabilities")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let _ = state
        .db
        .federation_update_mount_capabilities(
            id,
            current.user.id,
            &serde_json::to_string(&capabilities).unwrap_or_else(|_| "[]".to_owned()),
        )
        .await;
    Ok(Json(json!({
        "ok": true,
        "removed": false,
        "latency_ms": latency,
        "server_version": info.get("server_version").cloned().unwrap_or(Value::Null),
    })))
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
