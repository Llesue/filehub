use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};

use crate::db::Root;
use crate::error::AppError;
use crate::federation::mounts::client_for_root;
use crate::state::AppState;

#[allow(clippy::too_many_arguments)]
pub async fn list(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: Option<&str>,
    cursor: Option<&str>,
    limit: Option<usize>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<Json<Value>, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let value = client
        .list(
            mount.grant_id,
            path.unwrap_or(""),
            cursor,
            limit.unwrap_or(200),
            sort.unwrap_or("name"),
            order.unwrap_or("asc"),
        )
        .await
        .map_err(|error| error.into_app())?;
    let mut entries = value.get("entries").cloned().unwrap_or(json!([]));
    if let Some(array) = entries.as_array_mut() {
        for entry in array {
            if let Some(object) = entry.as_object_mut() {
                object.entry("is_symlink").or_insert(json!(false));
                object.entry("created_at").or_insert(json!(0));
                object.entry("symlink_state").or_insert(Value::Null);
                object.entry("symlink_target").or_insert(Value::Null);
            }
        }
    }
    let total = entries.as_array().map(Vec::len).unwrap_or(0);
    Ok(Json(json!({
        "path": value.get("path").cloned().unwrap_or_else(|| json!(path.unwrap_or(""))),
        "entries": entries,
        "next_cursor": value.get("next_cursor").cloned().unwrap_or(Value::Null),
        "total": total,
        "read_only": mount.mode != "rw",
        "read_only_reason": if mount.mode != "rw" { json!("policy") } else { Value::Null },
        "root_id": root.id,
        "root_alias": root.alias,
    })))
}

pub async fn download(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    path: &str,
    inline: bool,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    let (client, mount) = client_for_root(state, user_id, root.id).await?;
    let response = client
        .download(mount.grant_id, path, inline, headers)
        .await
        .map_err(|error| error.into_app())?;
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ETAG,
        header::ACCEPT_RANGES,
        header::CONTENT_DISPOSITION,
        header::LAST_MODIFIED,
        header::CACHE_CONTROL,
    ] {
        if let Some(value) = response.headers().get(&name) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .map_err(|error| AppError::internal(anyhow::anyhow!(error)))
}
