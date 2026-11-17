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
use crate::jobs::{JobKind, JobOutcome};
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
    auth::require_csrf(&current, &headers)?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let path = request.path.unwrap_or_default();
    let recursive = request.recursive.unwrap_or(false);
    let target = indexer::resolve_index_target(&state, current.user.id, root, &path).await?;
    let job_key = format!(
        "{}|{}|{}",
        target.root.real_path,
        target.relative,
        if recursive { "tree" } else { "dir" }
    );
    let payload = json!({
        "root_id": target.root.id,
        "path": target.relative,
        "recursive": recursive,
        "requested_path": path,
        "covered_by": target.covered_by,
    });
    let user_id = current.user.id;
    let covered_by = target.covered_by.clone();
    let job_state = Arc::clone(&state);
    let job_root = target.root;
    let job_relative = target.relative;
    let outcome = state
        .jobs
        .submit(
            JobKind::Index,
            job_key,
            user_id,
            payload,
            1,
            false,
            move |ctx| async move {
                let cancel = ctx.token();
                let mut last_reported = 0_usize;
                let mut progress = |directories: usize, _entries: usize| {
                    if directories.saturating_sub(last_reported) >= 8 {
                        last_reported = directories;
                        ctx.report(directories, 0, 0, 0);
                    }
                };
                let stats = indexer::index_path(
                    &job_state,
                    user_id,
                    job_root,
                    &job_relative,
                    recursive,
                    &cancel,
                    &mut progress,
                )
                .await?;
                let directories = stats.directories;
                Ok(JobOutcome::done(
                    json!(stats),
                    directories,
                    directories.max(1),
                ))
            },
        )
        .await?;
    let status = if outcome.job.status == "running" {
        "running"
    } else {
        "queued"
    };
    Ok(Json(json!({
        "job_id": outcome.job.id,
        "status": status,
        "indexing": true,
        "covered_by": covered_by,
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
    let job = state.jobs.active_job_for_path(JobKind::Index, &real_path);

    match status {
        Some((status, entry_count, indexed_at, total_size_bytes, partial)) => Ok(Json(json!({
            "status": status,
            "entry_count": entry_count,
            "indexed_at": indexed_at,
            "total_size_bytes": total_size_bytes,
            "partial": partial,
            "job": job,
        }))),
        None => Ok(Json(json!({
            "status": "missing",
            "entry_count": 0,
            "indexed_at": null,
            "total_size_bytes": 0,
            "partial": false,
            "job": job,
        }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    q: String,
    root: i64,
    limit: Option<i64>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    modified_after: Option<i64>,
    modified_before: Option<i64>,
    is_dir: Option<bool>,
    kind: Option<String>,
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

    if let (Some(min), Some(max)) = (query.min_size, query.max_size)
        && min > max
    {
        return Err(AppError::bad_request("min_size must not exceed max_size"));
    }
    let mut is_dir = query.is_dir;
    let mime_prefix = match query.kind.as_deref() {
        Some("image") => Some("image/%".to_owned()),
        Some("video") => Some("video/%".to_owned()),
        Some("audio") => Some("audio/%".to_owned()),
        Some("text") => Some("text/%".to_owned()),
        Some("pdf") => Some("application/pdf".to_owned()),
        Some("dir") => {
            is_dir = Some(true);
            None
        }
        Some("file") => {
            is_dir = Some(false);
            None
        }
        _ => None,
    };
    let entries = state
        .db
        .search_entries(
            query_text,
            &root.real_path,
            preferences.show_hidden,
            query.limit.unwrap_or(100),
            crate::db::IndexSearchFilters {
                min_size: query.min_size,
                max_size: query.max_size,
                modified_after: query.modified_after,
                modified_before: query.modified_before,
                is_dir,
                mime_prefix,
            },
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
                "created": entry.created_at,
                "mime_type": entry.mime_type,
                "has_preview": entry.has_preview,
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
