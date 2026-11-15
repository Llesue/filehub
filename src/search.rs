use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::db::Root;
use crate::error::AppError;
use crate::indexer;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct IndexRequest {
    root: i64,
    path: Option<String>,
    recursive: Option<bool>,
}

pub async fn index(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<IndexRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let path = request.path.unwrap_or_default();
    let recursive = request.recursive.unwrap_or(false);
    let resolved = authz::resolve(&state, current.user.id, root.id, &path, false).await?;
    let job_key = format!(
        "{}|{}",
        resolved.absolute.to_string_lossy(),
        if recursive { "tree" } else { "dir" }
    );
    if !state.index_jobs.lock().insert(job_key.clone()) {
        return Ok(Json(json!({
            "directories": 0,
            "entries": 0,
            "indexing": true,
        })));
    }
    let result = indexer::index_path(&state, current.user.id, root, &path, recursive).await;
    state.index_jobs.lock().remove(&job_key);
    let stats = result?;

    Ok(Json(json!({
        "directories": stats.directories,
        "entries": stats.entries,
        "indexing": false,
    })))
}

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    root: i64,
    path: Option<String>,
}

pub async fn index_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StatusQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = load_root(&state, current.user.id, query.root).await?;
    let path = query.path.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, root.id, &path, false).await?;
    let real_path = resolved.absolute.to_string_lossy().to_string();
    let status = state
        .db
        .index_status(&real_path)
        .await
        .map_err(AppError::internal)?;

    match status {
        Some((status, entry_count, indexed_at)) => Ok(Json(json!({
            "status": status,
            "entry_count": entry_count,
            "indexed_at": indexed_at,
        }))),
        None => Ok(Json(json!({
            "status": "missing",
            "entry_count": 0,
            "indexed_at": null,
        }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    q: String,
    root: i64,
    limit: Option<i64>,
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let root = load_root(&state, current.user.id, query.root).await?;
    let query_text = query.q.trim();
    if query_text.is_empty() {
        return Err(AppError::bad_request("search query is empty"));
    }

    let entries = state
        .db
        .search_entries(
            query_text,
            &root.real_path,
            preferences.show_hidden,
            query.limit.unwrap_or(100),
        )
        .await
        .map_err(AppError::internal)?;
    let root_prefix = root.real_path.trim_end_matches(['/', '\\']);

    let results: Vec<Value> = entries
        .into_iter()
        .filter_map(|entry| {
            let relative = entry.real_path.strip_prefix(root_prefix)?;
            let path = relative.trim_start_matches(['/', '\\']).replace('\\', "/");
            if path.is_empty() {
                return None;
            }
            Some(json!({
                "name": entry.name,
                "path": path,
                "is_dir": entry.is_dir,
                "size": entry.size,
                "modified": entry.mtime,
            }))
        })
        .take(query.limit.unwrap_or(100).clamp(1, 500) as usize)
        .collect();

    Ok(Json(json!({ "entries": results })))
}

async fn load_root(state: &Arc<AppState>, user_id: i64, root_id: i64) -> Result<Root, AppError> {
    state
        .db
        .find_root(root_id, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)
}
