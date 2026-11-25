use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::db::{IndexPolicyRecord, User};
use crate::error::AppError;
use crate::jobs::JobKind;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPolicy {
    #[default]
    Normal,
    Background,
    OnDemand,
    Ignore,
}

impl IndexPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Background => "background",
            Self::OnDemand => "on_demand",
            Self::Ignore => "ignore",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "background" => Some(Self::Background),
            "on_demand" => Some(Self::OnDemand),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }

    pub fn visible_by_default(self) -> bool {
        matches!(self, Self::Normal | Self::Background)
    }

    pub fn scheduled(self) -> bool {
        matches!(self, Self::Normal | Self::Background)
    }

    pub fn allows_browse(self) -> bool {
        matches!(self, Self::Normal | Self::Background | Self::OnDemand)
    }

    pub fn is_ignore(self) -> bool {
        matches!(self, Self::Ignore)
    }
}

pub async fn all_policies(state: &Arc<AppState>) -> Result<Vec<IndexPolicyRecord>, AppError> {
    {
        let cache = state.policy_cache.lock();
        if let Some(records) = cache.as_ref() {
            return Ok(records.clone());
        }
    }
    let records = state
        .db
        .index_policies()
        .await
        .map_err(AppError::internal)?;
    *state.policy_cache.lock() = Some(records.clone());
    Ok(records)
}

pub fn invalidate_cache(state: &Arc<AppState>) {
    *state.policy_cache.lock() = None;
}

pub async fn resolve(state: &Arc<AppState>, path: &Path) -> Result<IndexPolicy, AppError> {
    let records = all_policies(state).await?;
    let mut best: Option<(&IndexPolicyRecord, usize)> = None;
    for record in &records {
        let policy_path = Path::new(&record.real_path);
        if crate::platform::path_within(path, policy_path) {
            let depth = policy_path.components().count();
            if best.is_none_or(|(_, current)| depth >= current) {
                best = Some((record, depth));
            }
        }
    }
    Ok(best
        .and_then(|(record, _)| IndexPolicy::parse(&record.policy))
        .unwrap_or_default())
}

pub async fn resolve_prefix(
    state: &Arc<AppState>,
    real_path: &str,
) -> Result<IndexPolicy, AppError> {
    resolve(state, Path::new(real_path)).await
}

#[derive(Debug, Deserialize)]
pub struct PolicyQuery {
    root: i64,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PolicyRequest {
    root: i64,
    path: String,
    policy: String,
}

#[derive(Debug, Deserialize)]
pub struct DeletePolicyRequest {
    root: i64,
    path: String,
}

pub async fn list_policies(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PolicyQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let path = query.path.unwrap_or_default();
    let resolved = authz::resolve(&state, current.user.id, query.root, &path, false).await?;
    let effective = resolve(&state, &resolved.absolute).await?;
    let records = all_policies(&state).await?;
    let explicit: Vec<Value> = records
        .iter()
        .filter(|record| {
            crate::platform::path_within(Path::new(&record.real_path), &resolved.absolute)
                || crate::platform::path_within(&resolved.absolute, Path::new(&record.real_path))
        })
        .map(|record| {
            json!({
                "path": record.real_path,
                "policy": record.policy,
                "updated_by": record.updated_by,
                "updated_at": record.updated_at,
            })
        })
        .collect();
    Ok(Json(json!({
        "path": resolved.relative,
        "effective": effective.as_str(),
        "explicit": explicit,
    })))
}

pub async fn put_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PolicyRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let policy = IndexPolicy::parse(&request.policy).ok_or_else(|| {
        AppError::bad_request("policy must be normal, background, on_demand or ignore")
    })?;
    let resolved =
        authz::resolve(&state, current.user.id, request.root, &request.path, false).await?;
    let metadata = tokio::fs::metadata(&resolved.absolute)
        .await
        .map_err(|_| AppError::NotFound)?;
    if !metadata.is_dir() {
        return Err(AppError::bad_request("policy path must be a directory"));
    }
    let real_path = resolved.absolute.to_string_lossy().to_string();
    state
        .db
        .set_index_policy(&real_path, policy.as_str(), Some(current.user.id))
        .await
        .map_err(AppError::internal)?;
    invalidate_cache(&state);
    state
        .db
        .set_index_visible_subtree(&real_path, policy.visible_by_default())
        .await
        .map_err(AppError::internal)?;
    state
        .db
        .set_indexed_dir_policy_subtree(&real_path, policy.as_str())
        .await
        .map_err(AppError::internal)?;
    if !policy.scheduled() {
        let _ = state
            .jobs
            .cancel_descendants(JobKind::Index, &real_path)
            .await;
        let _ = state
            .jobs
            .cancel_descendants(JobKind::DirSize, &real_path)
            .await;
    } else {
        state.note_index_change(
            std::path::Path::new(&real_path),
            crate::gate::ChangeReason::Manual,
        );
    }
    crate::api::audit(
        &state,
        Some(current.user.id),
        "index_policy",
        Some(&resolved.relative),
        Some(policy.as_str()),
        &headers,
    )
    .await;
    Ok(Json(json!({
        "path": resolved.relative,
        "policy": policy.as_str(),
        "source": "explicit",
    })))
}

pub async fn delete_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<DeletePolicyRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let resolved =
        authz::resolve(&state, current.user.id, request.root, &request.path, false).await?;
    let real_path = resolved.absolute.to_string_lossy().to_string();
    let removed = state
        .db
        .delete_index_policy(&real_path)
        .await
        .map_err(AppError::internal)?;
    invalidate_cache(&state);
    let effective = resolve(&state, &resolved.absolute).await?;
    state
        .db
        .set_index_visible_subtree(&real_path, effective.visible_by_default())
        .await
        .map_err(AppError::internal)?;
    state
        .db
        .set_indexed_dir_policy_subtree(&real_path, effective.as_str())
        .await
        .map_err(AppError::internal)?;
    if effective.scheduled() {
        state.note_index_change(
            std::path::Path::new(&real_path),
            crate::gate::ChangeReason::Manual,
        );
    }
    crate::api::audit(
        &state,
        Some(current.user.id),
        "index_policy_reset",
        Some(&resolved.relative),
        if removed {
            Some("explicit removed")
        } else {
            Some("already inherited")
        },
        &headers,
    )
    .await;
    Ok(Json(json!({
        "path": resolved.relative,
        "policy": effective.as_str(),
        "source": "inherited",
    })))
}

pub fn policy_paths_for_user(
    policies: &[IndexPolicyRecord],
    user: &User,
) -> Vec<IndexPolicyRecord> {
    let roots = [PathBuf::from(&user.home_dir)];
    policies
        .iter()
        .filter(|record| {
            roots
                .iter()
                .any(|root| crate::platform::path_within(Path::new(&record.real_path), root))
        })
        .cloned()
        .collect()
}
