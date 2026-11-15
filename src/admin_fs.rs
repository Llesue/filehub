use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::db::{ServerPath, TrustedLink};
use crate::error::AppError;
use crate::fsops;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    limit: Option<i64>,
    cursor: Option<String>,
}

pub async fn list_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let offset = query
        .cursor
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let mut entries = state
        .db
        .list_audit(limit + 1, offset)
        .await
        .map_err(AppError::internal)?;
    let has_more = entries.len() as i64 > limit;
    entries.truncate(limit as usize);
    Ok(Json(json!({
        "next_cursor": has_more.then(|| (offset + limit).to_string()),
        "entries": entries.iter().map(|entry| json!({
            "id": entry.id,
            "username": entry.username,
            "action": entry.action,
            "path": entry.path,
            "detail": entry.detail,
            "ip": entry.ip,
            "created_at": entry.created_at,
        })).collect::<Vec<_>>()
    })))
}

pub async fn list_roots(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(user_id): AxumPath<i64>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;

    if state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .is_none()
    {
        return Err(AppError::NotFound);
    }

    let roots = state
        .db
        .list_roots(user_id)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "roots": roots.iter().map(|root| json!({
            "id": root.id,
            "alias": root.alias,
            "real_path": root.real_path,
            "read_only": root.read_only,
            "sort_order": root.sort_order,
        })).collect::<Vec<_>>()
    })))
}

#[derive(Debug, Deserialize)]
pub struct AddRootRequest {
    path: String,
    alias: String,
    read_only: Option<bool>,
    sort_order: Option<i64>,
}

pub async fn add_root(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(user_id): AxumPath<i64>,
    Json(request): Json<AddRootRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    if state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .is_none()
    {
        return Err(AppError::NotFound);
    }
    if request.alias.trim().is_empty() {
        return Err(AppError::bad_request("alias is required"));
    }

    let canonical = fsops::canonical_directory(&request.path)?;
    let real_path = canonical
        .to_str()
        .ok_or_else(|| AppError::invalid_path("root path is not valid UTF-8"))?;
    let id = state
        .db
        .add_root(
            user_id,
            real_path,
            request.alias.trim(),
            request.read_only.unwrap_or(false),
            request.sort_order.unwrap_or(0),
        )
        .await
        .map_err(map_db_error)?;

    audit(&state, current.user.id, "root_added", real_path, &headers).await;

    Ok(Json(json!({ "root_id": id, "real_path": real_path })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateRootRequest {
    alias: String,
    read_only: Option<bool>,
    sort_order: Option<i64>,
}

pub async fn update_root(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((user_id, root_id)): AxumPath<(i64, i64)>,
    Json(request): Json<UpdateRootRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    if request.alias.trim().is_empty() {
        return Err(AppError::bad_request("alias is required"));
    }

    let updated = state
        .db
        .update_root(
            root_id,
            user_id,
            request.alias.trim(),
            request.read_only.unwrap_or(false),
            request.sort_order.unwrap_or(0),
        )
        .await
        .map_err(AppError::internal)?;

    if !updated {
        return Err(AppError::NotFound);
    }

    audit(
        &state,
        current.user.id,
        "root_updated",
        &root_id.to_string(),
        &headers,
    )
    .await;

    Ok(Json(json!({ "updated": true })))
}

pub async fn delete_root(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((user_id, root_id)): AxumPath<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    let deleted = state
        .db
        .delete_root(root_id, user_id)
        .await
        .map_err(AppError::internal)?;
    if !deleted {
        return Err(AppError::NotFound);
    }

    audit(
        &state,
        current.user.id,
        "root_deleted",
        &root_id.to_string(),
        &headers,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_trusted_links(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;

    let links = state
        .db
        .list_trusted_links()
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "links": links.iter().map(link_json).collect::<Vec<_>>()
    })))
}

#[derive(Debug, Deserialize)]
pub struct CreateTrustedLinkRequest {
    link_path: String,
    target_path: String,
    access: Option<String>,
}

pub async fn create_trusted_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateTrustedLinkRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    let access = request.access.as_deref().unwrap_or("inherit");
    let link = fsops::prepare_trusted_link(&request.link_path, &request.target_path, access)?;
    let id = state
        .db
        .insert_trusted_link(&link)
        .await
        .map_err(map_db_error)?;

    audit(
        &state,
        current.user.id,
        "trusted_link_added",
        &link.link_path,
        &headers,
    )
    .await;

    Ok(Json(json!({ "id": id, "link": link_json(&link) })))
}

pub async fn delete_trusted_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    let deleted = state
        .db
        .delete_trusted_link(id)
        .await
        .map_err(AppError::internal)?;
    if !deleted {
        return Err(AppError::NotFound);
    }

    audit(
        &state,
        current.user.id,
        "trusted_link_deleted",
        &id.to_string(),
        &headers,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_server_paths(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    let paths = state
        .db
        .list_server_paths()
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "paths": paths.iter().map(server_path_json).collect::<Vec<_>>()
    })))
}

#[derive(Debug, Deserialize)]
pub struct AddServerPathRequest {
    path: String,
    alias: String,
}

pub async fn add_server_path(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<AddServerPathRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    if request.alias.trim().is_empty() {
        return Err(AppError::bad_request("alias is required"));
    }
    let canonical = fsops::canonical_directory(&request.path)?;
    let real_path = canonical
        .to_str()
        .ok_or_else(|| AppError::invalid_path("server path is not valid UTF-8"))?;
    let id = state
        .db
        .insert_server_path(request.alias.trim(), real_path)
        .await
        .map_err(map_db_error)?;
    audit(
        &state,
        current.user.id,
        "server_path_added",
        real_path,
        &headers,
    )
    .await;
    Ok(Json(json!({ "id": id, "real_path": real_path })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateServerPathRequest {
    alias: String,
}

pub async fn update_server_path(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(request): Json<UpdateServerPathRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let alias = request.alias.trim();
    if alias.is_empty() {
        return Err(AppError::bad_request("alias is required"));
    }
    let changed = state
        .db
        .update_server_path_alias(id, alias)
        .await
        .map_err(AppError::internal)?;
    if !changed {
        return Err(AppError::NotFound);
    }
    audit(
        &state,
        current.user.id,
        "server_path_updated",
        alias,
        &headers,
    )
    .await;
    Ok(Json(json!({ "alias": alias })))
}

pub async fn delete_server_path(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let paths = state
        .db
        .list_server_paths()
        .await
        .map_err(AppError::internal)?;
    let Some(path) = paths.into_iter().find(|path| path.id == id) else {
        return Err(AppError::NotFound);
    };
    if state
        .db
        .count_roots_for_path(&path.real_path)
        .await
        .map_err(AppError::internal)?
        > 0
    {
        return Err(AppError::conflict(
            "server path is still assigned to users; remove those roots first",
        ));
    }
    let deleted = state
        .db
        .delete_server_path(id)
        .await
        .map_err(AppError::internal)?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    audit(
        &state,
        current.user.id,
        "server_path_deleted",
        &path.real_path,
        &headers,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn server_path_json(path: &ServerPath) -> Value {
    json!({
        "id": path.id,
        "alias": path.alias,
        "real_path": path.real_path,
        "created_at": path.created_at,
    })
}

fn link_json(link: &TrustedLink) -> Value {
    json!({
        "id": link.id,
        "link_path": link.link_path,
        "target_path": link.target_path,
        "access": link.access,
        "created_at": link.created_at,
    })
}

fn map_db_error(err: anyhow::Error) -> AppError {
    let detail = format!("{err:#}");
    if detail.contains("UNIQUE constraint failed") {
        AppError::conflict("record already exists")
    } else {
        AppError::internal(err)
    }
}

async fn audit(state: &Arc<AppState>, user_id: i64, action: &str, path: &str, headers: &HeaderMap) {
    let ip = auth::client_ip(headers);
    if let Err(err) = state
        .db
        .audit(Some(user_id), action, Some(path), None, ip.as_deref())
        .await
    {
        tracing::warn!(error = %err, action, "failed to write audit log");
    }
}
