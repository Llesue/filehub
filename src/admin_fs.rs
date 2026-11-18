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
    before_id: Option<i64>,
}

pub async fn list_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let before_id = query.before_id.or_else(|| {
        query
            .cursor
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
    });
    let mut entries = state
        .db
        .list_audit(limit + 1, before_id)
        .await
        .map_err(AppError::internal)?;
    let has_more = entries.len() as i64 > limit;
    entries.truncate(limit as usize);
    let next_cursor = has_more.then(|| entries.last().map(|entry| entry.id.to_string()));
    Ok(Json(json!({
        "next_cursor": next_cursor.flatten(),
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
        "roots": roots.iter().map(|root| {
            let covered = covering_root(&roots, root);
            json!({
                "id": root.id,
                "alias": root.alias,
                "real_path": root.real_path,
                "read_only": root.read_only,
                "sort_order": root.sort_order,
                "covered_by": covered.map(|other| other.alias.clone()),
                "covered_by_id": covered.map(|other| other.id),
            })
        }).collect::<Vec<_>>()
    })))
}

fn covering_root<'a>(
    roots: &'a [crate::db::Root],
    root: &crate::db::Root,
) -> Option<&'a crate::db::Root> {
    let path = std::path::Path::new(&root.real_path);
    roots
        .iter()
        .filter(|other| {
            other.id != root.id
                && other.real_path.len() < root.real_path.len()
                && crate::platform::path_within(path, std::path::Path::new(&other.real_path))
        })
        .min_by_key(|other| other.real_path.len())
}

pub async fn list_all_roots(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    let all = state
        .db
        .list_all_roots()
        .await
        .map_err(AppError::internal)?;
    let mut grouped: std::collections::BTreeMap<i64, Vec<crate::db::Root>> =
        std::collections::BTreeMap::new();
    for root in all {
        grouped.entry(root.user_id).or_default().push(root);
    }
    let mut result = serde_json::Map::new();
    for (user_id, roots) in grouped {
        let values: Vec<Value> = roots
            .iter()
            .map(|root| {
                let covered = covering_root(&roots, root);
                json!({
                    "id": root.id,
                    "alias": root.alias,
                    "real_path": root.real_path,
                    "read_only": root.read_only,
                    "sort_order": root.sort_order,
                    "covered_by": covered.map(|other| other.alias.clone()),
                    "covered_by_id": covered.map(|other| other.id),
                })
            })
            .collect();
        result.insert(user_id.to_string(), Value::Array(values));
    }
    Ok(Json(json!({ "roots": result })))
}

pub async fn status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    let jobs = state.jobs.summary();
    let status_started = std::time::Instant::now();
    let started = std::time::Instant::now();
    let job_counts = state.db.job_counts().await.map_err(AppError::internal)?;
    let jobs_ms = started.elapsed().as_millis();
    let directory_sizes = state.cached_directory_size_counts();
    state.schedule_directory_size_counts_refresh();
    let sizes_ms = 0_u128;
    let started = std::time::Instant::now();
    let stats = state.db.stats().await.map_err(AppError::internal)?;
    let stats_ms = started.elapsed().as_millis();
    let slow_requests = state.recent_slow_requests();
    let settings = state.settings();
    let last_quick_scan = state
        .db
        .runtime_state_get("index.last_quick_scan")
        .await
        .ok()
        .flatten();
    let last_full_scan = state
        .db
        .runtime_state_get("index.last_full_scan")
        .await
        .ok()
        .flatten();
    let total_ms = status_started.elapsed().as_millis();
    if total_ms >= 500 {
        tracing::warn!(jobs_ms, sizes_ms, stats_ms, "slow admin status queries");
    }
    Ok(Json(json!({
        "jobs": jobs,
        "job_counts": job_counts.iter().map(|(kind, status, count)| json!({
            "kind": kind,
            "status": status,
            "count": count,
        })).collect::<Vec<_>>(),
        "directory_sizes": directory_sizes.iter().map(|(status, count)| json!({
            "status": status,
            "count": count,
        })).collect::<Vec<_>>(),
        "database": {
            "page_count": stats.page_count,
            "page_size": stats.page_size,
            "freelist_count": stats.freelist_count,
            "wal_bytes": stats.wal_bytes,
        },
        "slow_requests": slow_requests,
        "index": {
            "scheduled": settings.settings.index.scheduled_scans,
            "quick_scan_minutes": settings.settings.index.quick_scan_minutes,
            "full_scan_hours": settings.settings.index.full_scan_hours,
            "last_quick_scan": last_quick_scan,
            "last_full_scan": last_full_scan,
        },
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
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: action.to_owned(),
            path: Some(path.to_owned()),
            detail: None,
            ip: auth::client_ip(headers),
        })
        .await;
}
