use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use serde_json::{Value, json};

use crate::auth;
use crate::error::AppError;
use crate::fsops;
use crate::state::AppState;

pub async fn usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let roots = state
        .db
        .list_roots(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let home = PathBuf::from(current.user.home_dir.clone());
    let roots_for_task: Vec<_> = roots
        .into_iter()
        .filter(|root| !crate::platform::paths_equal(Path::new(&root.real_path), &home))
        .collect();
    let home_for_task = home.clone();
    let (used_bytes, disks) = tokio::task::spawn_blocking(move || {
        let used = fsops::path_size(&home_for_task)?;
        let disks = fsops::disk_usage(&roots_for_task)?;
        Ok::<_, AppError>((used, disks))
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("usage task failed: {err}")))??;

    Ok(Json(json!({
        "quota_bytes": current.user.quota_bytes,
        "used_bytes": used_bytes,
        "disks": disks,
    })))
}
