use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::db::unix_now;
use crate::error::AppError;
use crate::state::AppState;

fn require_share(state: &Arc<AppState>, user: &crate::db::User) -> Result<(), AppError> {
    if !state.settings().settings.federation.enabled_inbound || !user.feature_federation_share {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<(), AppError> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 64 || label.chars().any(char::is_control) {
        return Err(AppError::bad_request("invalid label"));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateGrantRequest {
    root: i64,
    path: Option<String>,
    label: String,
    mode: Option<String>,
}

pub async fn create_grant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateGrantRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_share(&state, &current.user)?;
    validate_label(&request.label)?;
    let mode = request.mode.as_deref().unwrap_or("ro");
    if !matches!(mode, "ro" | "rw") {
        return Err(AppError::bad_request("mode must be ro or rw"));
    }
    let limit = state.settings().settings.federation.max_grants_per_user as i64;
    if state
        .db
        .federation_count_grants(current.user.id)
        .await
        .map_err(AppError::internal)?
        >= limit
    {
        return Err(AppError::TooManyRequests);
    }
    let path = request.path.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, request.root, &path, false).await?;
    if !resolved.absolute.is_dir() {
        return Err(AppError::bad_request("grant must be a directory"));
    }
    if mode == "rw" && resolved.access_read_only {
        return Err(AppError::bad_request("directory is read-only"));
    }
    let real_path = resolved.absolute.to_string_lossy().into_owned();
    let id = state
        .db
        .federation_create_grant(
            current.user.id,
            request.root,
            &real_path,
            request.label.trim(),
            mode,
        )
        .await
        .map_err(AppError::internal)?;
    state.events.publish(
        "roots.changed",
        Some(current.user.id),
        json!({ "user_id": current.user.id }),
    );
    Ok(Json(json!({
        "id": id,
        "label": request.label.trim(),
        "mode": mode,
        "created_at": unix_now(),
    })))
}

pub async fn list_grants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    require_share(&state, &current.user)?;
    let grants = state
        .db
        .federation_list_grants(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let mut items = Vec::new();
    for grant in grants {
        let tokens = state
            .db
            .federation_grant_token_count(grant.id)
            .await
            .map_err(AppError::internal)?;
        items.push(json!({
            "id": grant.id,
            "label": grant.label,
            "path": grant.real_path,
            "mode": grant.mode,
            "revoked": grant.revoked,
            "available": std::path::Path::new(&grant.real_path).is_dir(),
            "token_count": tokens,
            "created_at": grant.created_at,
        }));
    }
    Ok(Json(json!({ "grants": items })))
}

pub async fn revoke_grant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_share(&state, &current.user)?;
    let changed = state
        .db
        .federation_revoke_grant(id, current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !changed {
        return Err(AppError::NotFound);
    }
    crate::federation::presence::notify_grant(
        &state,
        current.user.id,
        id,
        "grant.revoked",
        json!({ "grant": id }),
    );
    state.events.publish(
        "roots.changed",
        Some(current.user.id),
        json!({ "user_id": current.user.id }),
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    name: String,
    grant_ids: Vec<i64>,
    expires_at: Option<i64>,
}

pub async fn create_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateTokenRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_share(&state, &current.user)?;
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::bad_request("invalid token name"));
    }
    if request.grant_ids.is_empty() || request.grant_ids.len() > 100 {
        return Err(AppError::bad_request("select 1-100 grants"));
    }
    if request
        .expires_at
        .is_some_and(|expires| expires <= unix_now())
    {
        return Err(AppError::bad_request("expires_at must be in the future"));
    }
    let owned = state
        .db
        .federation_list_grants(current.user.id)
        .await
        .map_err(AppError::internal)?;
    for grant_id in &request.grant_ids {
        if !owned
            .iter()
            .any(|grant| grant.id == *grant_id && !grant.revoked)
        {
            return Err(AppError::Forbidden);
        }
    }
    let limit = state.settings().settings.federation.max_grants_per_user as i64;
    if state
        .db
        .federation_count_tokens(current.user.id)
        .await
        .map_err(AppError::internal)?
        >= limit
    {
        return Err(AppError::TooManyRequests);
    }
    let raw = format!("fhfed_{}", auth::random_token(32));
    let hash = auth::sha256_hex(&raw);
    let prefix: String = raw.chars().take(10).collect();
    let id = state
        .db
        .federation_create_token(
            current.user.id,
            name,
            &hash,
            &prefix,
            request.expires_at,
            &request.grant_ids,
        )
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "id": id,
        "name": name,
        "token": raw,
        "prefix": prefix,
        "expires_at": request.expires_at,
        "grant_ids": request.grant_ids,
    })))
}

pub async fn list_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    require_share(&state, &current.user)?;
    let tokens = state
        .db
        .federation_list_tokens(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let mut items = Vec::new();
    for token in tokens {
        let grants = state
            .db
            .federation_token_grant_ids(token.id)
            .await
            .map_err(AppError::internal)?;
        items.push(json!({
            "id": token.id,
            "name": token.name,
            "prefix": token.prefix,
            "created_at": token.created_at,
            "expires_at": token.expires_at,
            "revoked": token.revoked,
            "last_used_at": token.last_used_at,
            "grant_ids": grants,
        }));
    }
    Ok(Json(json!({ "tokens": items })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateTokenRequest {
    grant_ids: Vec<i64>,
}

pub async fn update_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(request): Json<UpdateTokenRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_share(&state, &current.user)?;
    if request.grant_ids.is_empty() || request.grant_ids.len() > 100 {
        return Err(AppError::bad_request("select 1-100 grants"));
    }
    let changed = state
        .db
        .federation_update_token_grants(id, current.user.id, &request.grant_ids)
        .await
        .map_err(AppError::internal)?;
    if !changed {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "id": id, "grant_ids": request.grant_ids })))
}

pub async fn revoke_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_share(&state, &current.user)?;
    let changed = state
        .db
        .federation_revoke_token(id, current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !changed {
        return Err(AppError::NotFound);
    }
    crate::federation::presence::notify_token(&state, id, "token.revoked", json!({ "token": id }));
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn rotate_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    require_share(&state, &current.user)?;
    let raw = format!("fhfed_{}", auth::random_token(32));
    let hash = auth::sha256_hex(&raw);
    let prefix: String = raw.chars().take(10).collect();
    let changed = state
        .db
        .federation_update_token_hash(id, current.user.id, &hash, &prefix)
        .await
        .map_err(AppError::internal)?;
    if !changed {
        return Err(AppError::NotFound);
    }
    // The stored ciphertext on every mount still holds the old token. Push
    // the change and close those sockets so clients do not keep using it.
    crate::federation::presence::notify_token(&state, id, "token.rotated", json!({ "token": id }));
    Ok(Json(json!({ "id": id, "token": raw, "prefix": prefix })))
}
