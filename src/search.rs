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
    let real_path = std::path::Path::new(&target.root.real_path)
        .join(&target.relative)
        .to_string_lossy()
        .to_string();
    let payload = json!({
        "root_id": target.root.id,
        "path": target.relative,
        "real_path": real_path,
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
    if root.is_remote() {
        return crate::federation::proxy::search(
            &state,
            current.user.id,
            &root,
            query_text,
            query.limit.unwrap_or(100).clamp(1, 200) as usize,
        )
        .await;
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
    let started = std::time::Instant::now();
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

    if started.elapsed().as_millis() >= 500 {
        tracing::warn!(
            root_id = root.id,
            query_len = query_text.chars().count(),
            duration_ms = started.elapsed().as_millis() as u64,
            "slow search"
        );
    }
    Ok(Json(json!({ "entries": results })))
}

#[derive(Debug, Deserialize)]
pub struct SearchAllQuery {
    q: String,
    limit: Option<i64>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    modified_after: Option<i64>,
    modified_before: Option<i64>,
    kind: Option<String>,
    cursor: Option<String>,
}

fn is_cjk(character: char) -> bool {
    let code = character as u32;
    (0x3400..=0x4DBF).contains(&code)
        || (0x4E00..=0x9FFF).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
}

fn root_path_bounds(path: &str) -> Vec<(String, String)> {
    let mut bounds = Vec::new();
    for (separator, upper) in [("/", "0"), ("\\", "]")] {
        let base = path.trim_end_matches(['/', '\\']);
        let start = format!("{base}{separator}");
        let end = format!("{base}{upper}");
        if start < end {
            bounds.push((start, end));
        }
    }
    bounds
}

pub async fn search_all(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<SearchAllQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let query_text = query.q.trim();
    let query_len = query_text.chars().count();
    let has_cjk = query_text.chars().any(is_cjk);
    if query_len < 3 && !(query_len == 2 && has_cjk) {
        return Err(AppError::bad_request(
            "search query must be at least 3 characters, or 2 CJK characters",
        ));
    }
    if let (Some(min), Some(max)) = (query.min_size, query.max_size)
        && min > max
    {
        return Err(AppError::bad_request("min_size must not exceed max_size"));
    }
    let mut is_dir = None;
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
    let all_roots = state
        .db
        .list_roots(current.user.id)
        .await
        .map_err(AppError::internal)?;
    // A root inside another root would search the same files twice and can
    // force an extra full FTS scan for a namespace with few matches.
    let roots: Vec<Root> = all_roots
        .iter()
        .filter(|root| {
            !all_roots.iter().any(|other| {
                other.id != root.id
                    && other.real_path.len() < root.real_path.len()
                    && crate::platform::path_within(
                        std::path::Path::new(&root.real_path),
                        std::path::Path::new(&other.real_path),
                    )
            })
        })
        .cloned()
        .collect();
    let ranges: Vec<(String, String)> = roots
        .iter()
        .flat_map(|root| root_path_bounds(&root.real_path))
        .collect();
    let started = std::time::Instant::now();
    let cursor = query
        .cursor
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| AppError::bad_request("invalid cursor"))
        })
        .transpose()?;
    let filters = crate::db::IndexSearchFilters {
        min_size: query.min_size,
        max_size: query.max_size,
        modified_after: query.modified_after,
        modified_before: query.modified_before,
        is_dir,
        mime_prefix,
    };
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let (entries, next_cursor) = if query_len == 2 {
        state
            .db
            .search_entries_bigram(
                query_text,
                ranges,
                preferences.show_hidden,
                limit,
                cursor,
                filters,
            )
            .await
            .map_err(AppError::internal)?
    } else {
        state
            .db
            .search_entries_all(
                query_text,
                ranges,
                preferences.show_hidden,
                limit,
                cursor,
                filters,
            )
            .await
            .map_err(AppError::internal)?
    };
    if started.elapsed().as_millis() >= 500 {
        tracing::warn!(
            roots = roots.len(),
            query_len = query_text.chars().count(),
            duration_ms = started.elapsed().as_millis() as u64,
            "slow unified search"
        );
    }
    let dir_paths: Vec<String> = entries
        .iter()
        .filter(|entry| entry.is_dir)
        .map(|entry| entry.real_path.clone())
        .collect();
    let dir_stats = if dir_paths.is_empty() {
        std::collections::HashMap::new()
    } else {
        state
            .db
            .dir_index_stats(dir_paths)
            .await
            .map_err(AppError::internal)?
    };
    let results: Vec<Value> = entries
        .into_iter()
        .filter_map(|entry| {
            let (root, relative) = roots
                .iter()
                .filter_map(|root| {
                    entry
                        .real_path
                        .strip_prefix(root.real_path.trim_end_matches(['/', '\\']))
                        .map(|relative| (root, relative))
                })
                .min_by_key(|(_, relative)| relative.len())?;
            let path = relative.trim_start_matches(['/', '\\']).replace('\\', "/");
            if path.is_empty() {
                return None;
            }
            let stat = if entry.is_dir {
                dir_stats.get(&entry.real_path)
            } else {
                None
            };
            Some(json!({
                "root_id": root.id,
                "name": entry.name,
                "path": path,
                "is_dir": entry.is_dir,
                "size": entry.size,
                "modified": entry.mtime,
                "created": entry.created_at,
                "mime_type": entry.mime_type,
                "has_preview": entry.has_preview,
                "dir_size": stat.and_then(|value| value.size),
                "dir_size_source": stat.map(|value| value.source.clone()),
                "dir_size_stale": stat.is_some_and(|value| value.stale),
                "entry_count": stat.and_then(|value| value.entry_count),
            }))
        })
        .collect();
    Ok(Json(json!({
        "entries": results,
        "next_cursor": next_cursor.map(|value| value.to_string()),
    })))
}

async fn load_root(state: &Arc<AppState>, user_id: i64, root_id: i64) -> Result<Root, AppError> {
    state
        .db
        .find_root(root_id, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)
}
