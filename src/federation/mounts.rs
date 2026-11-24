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
    .map_err(|error| {
        // A DNS failure is a transport-level outage, not a bad request from
        // the caller's point of view. Other SSRF rejections stay 400.
        if error == ssrf::SsrfError::ResolveFailed {
            AppError::RemoteOffline
        } else {
            AppError::bad_request(error.message())
        }
    })
}

async fn client_for_url(
    state: &Arc<AppState>,
    mount: &FederationMount,
    url: &str,
) -> Result<PeerClient, AppError> {
    let key = crypto::mount_key(&state.db)
        .await
        .map_err(AppError::internal)?;
    let token =
        crypto::decrypt(&key, &mount.token_ciphertext).map_err(|_| AppError::TokenRevoked)?;
    let target = target_for(state, url).await?;
    PeerClient::new(
        url,
        &target.host,
        target.ip,
        target.port,
        Some(&token),
        state.settings().settings.federation.request_timeout_seconds,
    )
    .map_err(AppError::internal)
}

async fn client_for_mount(
    state: &Arc<AppState>,
    mount: &FederationMount,
) -> Result<PeerClient, AppError> {
    client_for_url(state, mount, &mount.base_url).await
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

/// Liveness state for `GET /api/v1/roots`.
///
/// The background health loop already checks every mount, so a recent success
/// is trusted without another round trip. Only stale or never-checked mounts
/// are probed here, with a timeout that respects the configured request
/// timeout (bounded so the roots endpoint cannot hang).
pub async fn remote_root_online(state: &Arc<AppState>, user_id: i64, root_id: i64) -> bool {
    let Ok(Some(mount)) = state.db.federation_mount_by_root(root_id, user_id).await else {
        return false;
    };
    if mount.last_error_code.is_some() {
        return false;
    }
    let now = crate::db::unix_now();
    if mount
        .last_ok_at
        .is_some_and(|at| now.saturating_sub(at) <= 120)
    {
        return true;
    }
    let timeout = std::time::Duration::from_secs(
        state
            .settings()
            .settings
            .federation
            .request_timeout_seconds
            .clamp(3, 8),
    );
    let probe = async {
        let (client, mount) = client_for_root(state, user_id, root_id).await.ok()?;
        let grants = client.grants().await.ok()?;
        Some(has_grant(&grants, mount.grant_id))
    };
    tokio::time::timeout(timeout, probe)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
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
    url: Option<String>,
    urls: Option<Vec<String>>,
    token: String,
    grant_id: i64,
    alias: Option<String>,
}

fn requested_urls(url: Option<&str>, urls: Option<&[String]>) -> Result<Vec<String>, AppError> {
    let raw: Vec<&str> = urls
        .filter(|items| !items.is_empty())
        .map(|items| items.iter().map(String::as_str).collect())
        .or_else(|| url.map(|value| vec![value]))
        .unwrap_or_default();
    let mut result: Vec<String> = Vec::new();
    for item in raw {
        let normalized = ssrf::normalize_url(item);
        if normalized.is_empty() || result.iter().any(|existing| existing == &normalized) {
            continue;
        }
        result.push(normalized);
    }
    if result.is_empty() {
        return Err(AppError::bad_request("at least one address is required"));
    }
    if result.len() > 5 {
        return Err(AppError::bad_request("at most 5 addresses are supported"));
    }
    Ok(result)
}

fn endpoint_state(error_code: &str) -> &'static str {
    match error_code {
        "TOKEN_REVOKED" | "TOKEN_EXPIRED" | "UNAUTHORIZED" => "token_invalid",
        "GRANT_REVOKED" => "grant_revoked",
        "REMOTE_ROOT_UNSUPPORTED" => "unsupported",
        _ => "offline",
    }
}

async fn probe_endpoint(
    state: &Arc<AppState>,
    token: &str,
    url: &str,
) -> Result<(Value, Value), AppError> {
    let check = async {
        let target = target_for(state, url).await?;
        let client = PeerClient::new(
            url,
            &target.host,
            target.ip,
            target.port,
            Some(token),
            state.settings().settings.federation.request_timeout_seconds,
        )
        .map_err(AppError::internal)?;
        let info = client.info().await.map_err(|error| error.into_app())?;
        let grants = client.grants().await.map_err(|error| error.into_app())?;
        Ok::<_, AppError>((info, grants))
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), check)
        .await
        .map_err(|_| AppError::bad_request("peer check timed out"))?
}

fn grant_for(grants: &Value, grant_id: i64) -> Option<&Value> {
    grants
        .get("grants")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_i64) == Some(grant_id))
        })
}

async fn update_endpoint_states(
    state: &Arc<AppState>,
    mount_id: i64,
    urls: &[String],
    working: &std::collections::HashMap<String, (Value, Value)>,
    errors: &std::collections::HashMap<String, (String, String)>,
) {
    for url in urls {
        if working.contains_key(url) {
            let _ = state
                .db
                .federation_update_endpoint_state(mount_id, url, "online", true, None, None)
                .await;
        } else if let Some((code, message)) = errors.get(url) {
            let _ = state
                .db
                .federation_update_endpoint_state(
                    mount_id,
                    url,
                    endpoint_state(code),
                    false,
                    Some(message),
                    Some(code),
                )
                .await;
        }
    }
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
    urls: &[String],
    token: &str,
    grant_id: i64,
    alias: Option<String>,
) -> Result<(i64, i64, String, String), AppError> {
    let own_instance = state
        .db
        .federation_instance_id()
        .await
        .map_err(AppError::internal)?;
    let mut working: std::collections::HashMap<String, (Value, Value)> =
        std::collections::HashMap::new();
    let mut errors: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    let mut peer_instance_id: Option<String> = None;
    for url in urls {
        match probe_endpoint(state, token, url).await {
            Ok((info, grants)) => {
                let instance = info
                    .get("instance_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if instance.is_empty() {
                    errors.insert(
                        url.clone(),
                        (
                            "OFFLINE".to_owned(),
                            "peer did not return an instance id".to_owned(),
                        ),
                    );
                    continue;
                }
                if instance == own_instance {
                    return Err(AppError::bad_request("cannot mount this instance"));
                }
                if let Some(expected) = peer_instance_id.as_deref()
                    && expected != instance
                {
                    return Err(AppError::bad_request(
                        "all addresses must point to the same FileHub instance",
                    ));
                }
                peer_instance_id = Some(instance);
                if grant_for(&grants, grant_id).is_none() {
                    errors.insert(
                        url.clone(),
                        (
                            "GRANT_REVOKED".to_owned(),
                            "grant is not available".to_owned(),
                        ),
                    );
                    continue;
                }
                working.insert(url.clone(), (info, grants));
            }
            Err(error) => {
                errors.insert(url.clone(), (error.code().to_owned(), error.to_string()));
            }
        }
    }
    let Some(primary_url) = urls.iter().find(|url| working.contains_key(*url)).cloned() else {
        let detail = errors
            .values()
            .next()
            .map(|(_, message)| message.clone())
            .unwrap_or_else(|| "no address is reachable".to_owned());
        return Err(AppError::bad_request(format!(
            "no remote address is reachable: {detail}"
        )));
    };
    let Some(peer_instance_id) = peer_instance_id else {
        return Err(AppError::bad_request(
            "could not identify the remote instance",
        ));
    };
    let (info, grants) = working
        .get(&primary_url)
        .cloned()
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("primary endpoint vanished")))?;
    let grant = grant_for(&grants, grant_id).ok_or(AppError::NotFound)?;
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

    // Adding the same instance/grant through another address should update the
    // existing mount instead of creating a second root.
    if let Some(existing) = state
        .db
        .federation_find_mount_by_peer(user.id, &peer_instance_id, grant_id)
        .await
        .map_err(AppError::internal)?
    {
        let mut merged: Vec<String> = state
            .db
            .federation_list_endpoints(existing.id)
            .await
            .map_err(AppError::internal)?
            .into_iter()
            .map(|endpoint| endpoint.url)
            .collect();
        for url in urls {
            if !merged.contains(url) {
                merged.push(url.clone());
            }
        }
        state
            .db
            .federation_replace_endpoints(existing.id, &merged)
            .await
            .map_err(AppError::internal)?;
        update_endpoint_states(state, existing.id, urls, &working, &errors).await;
        let _ = state
            .db
            .federation_update_mount_token(existing.id, user.id, &ciphertext)
            .await;
        let active = if working.contains_key(&existing.base_url) {
            existing.base_url.clone()
        } else {
            primary_url.clone()
        };
        let _ = state
            .db
            .federation_set_mount_active_url(existing.id, user.id, &active)
            .await;
        let _ = state
            .db
            .federation_update_mount_state(existing.id, true, None, None)
            .await;
        state.federation_clients.lock().remove(&existing.root_id);
        state.events.publish(
            "roots.changed",
            Some(user.id),
            json!({ "user_id": user.id }),
        );
        let root = state
            .db
            .find_root(existing.root_id, user.id)
            .await
            .map_err(AppError::internal)?;
        return Ok((
            existing.id,
            existing.root_id,
            root.map(|root| root.alias).unwrap_or(existing.grant_label),
            existing.mode,
        ));
    }

    ensure_mount_limit(state, user.id).await?;
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
                &primary_url,
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
                &peer_instance_id,
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
        .federation_replace_endpoints(mount_id, urls)
        .await
        .map_err(AppError::internal)?;
    update_endpoint_states(state, mount_id, urls, &working, &errors).await;
    let _ = state
        .db
        .federation_update_mount_state(mount_id, true, None, None)
        .await;
    crate::federation::presence::ensure(state, mount_id);
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
    let urls = requested_urls(request.url.as_deref(), request.urls.as_deref())?;
    let (mount_id, root_id, alias, mode) = mount_inner(
        &state,
        &current.user,
        &urls,
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
    if mounts.iter().any(|m| {
        m.grant_id == grant_id
            && (m.base_url == source.base_url
                || (!m.peer_instance_id.is_empty()
                    && m.peer_instance_id == source.peer_instance_id))
    }) {
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
        std::slice::from_ref(&source.base_url),
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
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
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

fn is_auth_code(code: &str) -> bool {
    matches!(
        code,
        "UNAUTHORIZED" | "TOKEN_REVOKED" | "TOKEN_EXPIRED" | "GRANT_REVOKED"
    )
}

struct EndpointCheck {
    url: String,
    grants: Option<Value>,
    code: String,
    message: String,
    latency_ms: i64,
}

async fn check_mount_endpoint(
    state: &Arc<AppState>,
    mount: &FederationMount,
    url: &str,
) -> EndpointCheck {
    let started = Instant::now();
    let client = match client_for_url(state, mount, url).await {
        Ok(client) => client,
        Err(error) => {
            return EndpointCheck {
                url: url.to_owned(),
                grants: None,
                code: error.code().to_owned(),
                message: error.to_string(),
                latency_ms: started.elapsed().as_millis() as i64,
            };
        }
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), client.grants()).await {
        Ok(Ok(value)) => {
            if has_grant(&value, mount.grant_id) {
                EndpointCheck {
                    url: url.to_owned(),
                    grants: Some(value),
                    code: String::new(),
                    message: String::new(),
                    latency_ms: started.elapsed().as_millis() as i64,
                }
            } else {
                EndpointCheck {
                    url: url.to_owned(),
                    grants: None,
                    code: "GRANT_REVOKED".to_owned(),
                    message: "grant is no longer available".to_owned(),
                    latency_ms: started.elapsed().as_millis() as i64,
                }
            }
        }
        Ok(Err(error)) => EndpointCheck {
            url: url.to_owned(),
            grants: None,
            code: error.code,
            message: error.message,
            latency_ms: started.elapsed().as_millis() as i64,
        },
        Err(_) => EndpointCheck {
            url: url.to_owned(),
            grants: None,
            code: "OFFLINE".to_owned(),
            message: "peer check timed out".to_owned(),
            latency_ms: started.elapsed().as_millis() as i64,
        },
    }
}

struct HealthOutcome {
    online: bool,
    removed: bool,
    code: Option<String>,
    message: Option<String>,
    active_url: Option<String>,
    latency_ms: i64,
}

async fn refresh_mount_health(
    state: &Arc<AppState>,
    mount: &FederationMount,
) -> Result<HealthOutcome, AppError> {
    let mut urls: Vec<String> = state
        .db
        .federation_list_endpoints(mount.id)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .map(|endpoint| endpoint.url)
        .collect();
    if urls.is_empty() {
        urls.push(mount.base_url.clone());
        state
            .db
            .federation_replace_endpoints(mount.id, &urls)
            .await
            .map_err(AppError::internal)?;
    }

    let checks = futures_util::future::join_all(
        urls.iter()
            .map(|url| check_mount_endpoint(state, mount, url)),
    )
    .await;

    let mut working: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let mut failures: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for check in &checks {
        if let Some(grants) = &check.grants {
            working.insert(check.url.clone(), grants.clone());
        } else {
            failures.insert(
                check.url.clone(),
                (check.code.clone(), check.message.clone()),
            );
        }
    }
    let working_check = checks.iter().find(|check| check.grants.is_some());
    if let Some(active) = working_check {
        let mut active_url = mount.base_url.clone();
        let mut active_changed = false;
        let mut owner_changed = false;
        for check in &checks {
            if check.grants.is_some() {
                state
                    .db
                    .federation_update_endpoint_state(
                        mount.id, &check.url, "online", true, None, None,
                    )
                    .await
                    .map_err(AppError::internal)?;
            } else {
                state
                    .db
                    .federation_update_endpoint_state(
                        mount.id,
                        &check.url,
                        endpoint_state(&check.code),
                        false,
                        Some(&check.message),
                        Some(&check.code),
                    )
                    .await
                    .map_err(AppError::internal)?;
            }
        }
        // Prefer the first healthy address in priority order. This also
        // provides failback when the LAN address recovers.
        if active.url != mount.base_url {
            active_url = active.url.clone();
            active_changed = true;
            state
                .db
                .federation_set_mount_active_url(mount.id, mount.user_id, &active_url)
                .await
                .map_err(AppError::internal)?;
            state.federation_clients.lock().remove(&mount.root_id);
        }
        if mount.peer_instance_id.is_empty()
            && let Ok(client) = client_for_url(state, mount, &active.url).await
            && let Ok(info) = client.info().await
            && let Some(instance) = info.get("instance_id").and_then(Value::as_str)
        {
            let _ = state
                .db
                .federation_set_mount_peer_instance(mount.id, instance)
                .await;
        }
        if let Some(grants) = &active.grants
            && let Some(grant) = grant_for(grants, mount.grant_id)
        {
            let name = grant
                .get("owner_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = grant
                .get("avatar_version")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if name != mount.owner_name || version != mount.avatar_version {
                owner_changed = true;
                let _ = state
                    .db
                    .federation_update_mount_owner(mount.id, name, version)
                    .await;
                prune_avatar_cache(state, mount.id, version);
            }
        }
        state
            .db
            .federation_update_mount_state(mount.id, true, None, None)
            .await
            .map_err(AppError::internal)?;
        let recovered = mount.last_error_code.is_some();
        if active_changed || owner_changed || recovered {
            state.events.publish(
                "roots.changed",
                Some(mount.user_id),
                json!({ "user_id": mount.user_id, "root_id": mount.root_id, "online": true }),
            );
        }
        return Ok(HealthOutcome {
            online: true,
            removed: false,
            code: None,
            message: None,
            active_url: Some(active_url),
            latency_ms: active.latency_ms,
        });
    }

    for check in &checks {
        state
            .db
            .federation_update_endpoint_state(
                mount.id,
                &check.url,
                endpoint_state(&check.code),
                false,
                Some(&check.message),
                Some(&check.code),
            )
            .await
            .map_err(AppError::internal)?;
    }
    let all_auth = !checks.is_empty() && checks.iter().all(|check| is_auth_code(&check.code));
    let first = checks.first();
    let code = first.map(|check| check.code.clone());
    let message = first.map(|check| check.message.clone());
    if all_auth {
        let reason = checks
            .iter()
            .find(|check| check.code == "GRANT_REVOKED")
            .map(|check| check.code.clone())
            .or_else(|| code.clone())
            .unwrap_or_else(|| "GRANT_REVOKED".to_owned());
        let _ = remove_mount(state, mount.user_id, mount, Some(&reason)).await;
        return Ok(HealthOutcome {
            online: false,
            removed: true,
            code: Some(reason),
            message,
            active_url: None,
            latency_ms: first.map(|check| check.latency_ms).unwrap_or_default(),
        });
    }
    state
        .db
        .federation_update_mount_state(mount.id, false, message.as_deref(), code.as_deref())
        .await
        .map_err(AppError::internal)?;
    if mount.last_error_code.is_none() && mount.last_ok_at.is_some() {
        state.events.publish(
            "roots.changed",
            Some(mount.user_id),
            json!({ "user_id": mount.user_id, "root_id": mount.root_id, "online": false }),
        );
    }
    Ok(HealthOutcome {
        online: false,
        removed: false,
        code,
        message,
        active_url: None,
        latency_ms: first.map(|check| check.latency_ms).unwrap_or_default(),
    })
}

async fn health_check(state: &Arc<AppState>, mount: FederationMount) {
    if let Err(error) = refresh_mount_health(state, &mount).await {
        tracing::warn!(mount = mount.id, %error, "federation health check failed");
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
    refresh_capabilities_now(state, mount);
}

/// Fetch the peer capability list without the periodic throttle. Used when the
/// peer tells us its capabilities changed.
pub(crate) fn refresh_capabilities_now(state: &Arc<AppState>, mount: &FederationMount) {
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
        state.events.publish(
            "roots.changed",
            Some(mount.user_id),
            json!({ "user_id": mount.user_id, "root_id": mount.root_id }),
        );
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
        let endpoints = state
            .db
            .federation_list_endpoints(mount.id)
            .await
            .map_err(AppError::internal)?;
        let addresses: Vec<Value> = endpoints
            .iter()
            .map(|endpoint| {
                json!({
                    "url": endpoint.url,
                    "state": endpoint.state,
                    "last_ok_at": endpoint.last_ok_at,
                    "last_error": endpoint.last_error,
                    "last_error_code": endpoint.last_error_code,
                    "active": endpoint.url == mount.base_url,
                })
            })
            .collect();
        items.push(json!({
            "id": mount.id,
            "root_id": mount.root_id,
            "alias": root.as_ref().map(|root| root.alias.clone()).unwrap_or_default(),
            "base_url": mount.base_url,
            "addresses": addresses,
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
    crate::federation::presence::stop(state, mount.id);
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

#[derive(Debug, Deserialize)]
pub struct UpdateAddressesRequest {
    urls: Vec<String>,
}

pub async fn update_addresses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(request): Json<UpdateAddressesRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_outbound(&state, &current.user)?;
    let mount = state
        .db
        .federation_mount(id, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let urls = requested_urls(None, Some(&request.urls))?;
    let key = crypto::mount_key(&state.db)
        .await
        .map_err(AppError::internal)?;
    let token =
        crypto::decrypt(&key, &mount.token_ciphertext).map_err(|_| AppError::TokenRevoked)?;
    let own_instance = state
        .db
        .federation_instance_id()
        .await
        .map_err(AppError::internal)?;
    let mut working: std::collections::HashMap<String, (Value, Value)> =
        std::collections::HashMap::new();
    let mut errors: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    let mut instance: Option<String> = if mount.peer_instance_id.is_empty() {
        None
    } else {
        Some(mount.peer_instance_id.clone())
    };
    for url in &urls {
        match probe_endpoint(&state, &token, url).await {
            Ok((info, grants)) => {
                let found = info
                    .get("instance_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if found.is_empty() || found == own_instance {
                    errors.insert(
                        url.clone(),
                        (
                            "BAD_REQUEST".to_owned(),
                            "address does not identify a remote FileHub instance".to_owned(),
                        ),
                    );
                    continue;
                }
                if let Some(expected) = instance.as_deref()
                    && expected != found
                {
                    return Err(AppError::bad_request(
                        "all addresses must point to the same FileHub instance",
                    ));
                }
                instance = Some(found);
                if grant_for(&grants, mount.grant_id).is_none() {
                    errors.insert(
                        url.clone(),
                        (
                            "GRANT_REVOKED".to_owned(),
                            "grant is not available".to_owned(),
                        ),
                    );
                    continue;
                }
                working.insert(url.clone(), (info, grants));
            }
            Err(error) => {
                errors.insert(url.clone(), (error.code().to_owned(), error.to_string()));
            }
        }
    }
    let Some(active) = urls.iter().find(|url| working.contains_key(*url)).cloned() else {
        let detail = errors
            .values()
            .next()
            .map(|(_, message)| message.clone())
            .unwrap_or_else(|| "no address is reachable".to_owned());
        return Err(AppError::bad_request(format!(
            "no remote address is reachable: {detail}"
        )));
    };
    if let Some(instance) = instance
        && mount.peer_instance_id.is_empty()
    {
        let _ = state
            .db
            .federation_set_mount_peer_instance(mount.id, &instance)
            .await;
    }
    state
        .db
        .federation_replace_endpoints(mount.id, &urls)
        .await
        .map_err(AppError::internal)?;
    update_endpoint_states(&state, mount.id, &urls, &working, &errors).await;
    state
        .db
        .federation_set_mount_active_url(mount.id, current.user.id, &active)
        .await
        .map_err(AppError::internal)?;
    state
        .db
        .federation_update_mount_state(mount.id, true, None, None)
        .await
        .map_err(AppError::internal)?;
    state.federation_clients.lock().remove(&mount.root_id);
    state.events.publish(
        "roots.changed",
        Some(current.user.id),
        json!({ "user_id": current.user.id }),
    );
    Ok(Json(json!({ "ok": true, "active_url": active })))
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
    let outcome = refresh_mount_health(&state, &mount).await?;
    let mut server_version = Value::Null;
    if outcome.online
        && let Some(active_url) = outcome.active_url.as_deref()
        && let Ok(client) = client_for_url(&state, &mount, active_url).await
        && let Ok(info) = client.info().await
    {
        server_version = info.get("server_version").cloned().unwrap_or(Value::Null);
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
    }
    if outcome.online {
        crate::federation::presence::ensure(&state, id);
    }
    Ok(Json(json!({
        "ok": outcome.online,
        "removed": outcome.removed,
        "latency_ms": outcome.latency_ms,
        "code": outcome.code,
        "message": outcome.message,
        "active_url": outcome.active_url,
        "server_version": server_version,
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
    let mut urls: Vec<String> = state
        .db
        .federation_list_endpoints(mount.id)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .map(|endpoint| endpoint.url)
        .collect();
    if urls.is_empty() {
        urls.push(mount.base_url.clone());
    }
    let mut last_error: Option<AppError> = None;
    let mut verified = false;
    for url in urls {
        let target = match target_for(&state, &url).await {
            Ok(target) => target,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let client = match PeerClient::new(
            &url,
            &target.host,
            target.ip,
            target.port,
            Some(&request.token),
            state.settings().settings.federation.request_timeout_seconds,
        ) {
            Ok(client) => client,
            Err(error) => {
                last_error = Some(AppError::internal(error));
                continue;
            }
        };
        match client.grants().await {
            Ok(_) => {
                verified = true;
                break;
            }
            Err(error) => last_error = Some(error.into_app()),
        }
    }
    if !verified {
        return Err(last_error.unwrap_or(AppError::TokenRevoked));
    }
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
    crate::federation::presence::ensure(&state, id);
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
