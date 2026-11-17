use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use image::ImageFormat;
use serde_json::{Value, json};

use crate::auth;
use crate::error::AppError;
use crate::state::AppState;

const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;
const MAX_ICON_EDGE: u32 = 2048;

pub fn public_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/branding", get(branding))
        .route("/branding/icon", get(icon))
        .route("/custom.css", get(custom_css))
}

fn branding_json(state: &AppState, version: i64, has_icon: bool, custom_css_version: i64) -> Value {
    let name = state.settings().settings.ui.brand_name.trim().to_owned();
    json!({
        "name": name,
        "version": version,
        "icon_url": has_icon.then(|| format!("/api/v1/branding/icon?v={version}")),
        "custom_css_version": custom_css_version,
    })
}

pub async fn branding(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let icon = state.db.branding_icon().await.map_err(AppError::internal)?;
    let version = icon.as_ref().map(|icon| icon.version).unwrap_or(0);
    let custom_css_version = state
        .db
        .custom_css()
        .await
        .map_err(AppError::internal)?
        .filter(|(content, _)| !content.is_empty())
        .map(|(_, version)| version)
        .unwrap_or(0);
    Ok(Json(branding_json(
        &state,
        version,
        icon.is_some(),
        custom_css_version,
    )))
}

pub async fn icon(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(icon) = state.db.branding_icon().await.map_err(AppError::internal)? else {
        return Err(AppError::NotFound);
    };
    let etag = format!("\"brand-{}\"", icon.version);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str())
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, icon.mime_type)
        .header(header::CONTENT_LENGTH, icon.icon.len().to_string())
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(axum::body::Body::from(icon.icon))
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))
}

/// Admin upload: accepts PNG/JPEG/WebP, validates and re-encodes to PNG so
/// metadata and any polyglot payload are stripped before serving.
pub async fn upload_icon(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    if body.is_empty() || body.len() > MAX_ICON_BYTES {
        return Err(AppError::bad_request(
            "icon must be between 1 byte and 2 MiB",
        ));
    }
    let normalized = tokio::task::spawn_blocking(move || normalize_icon(&body))
        .await
        .map_err(|error| AppError::internal(anyhow::anyhow!("icon task failed: {error}")))??;
    let version = state
        .db
        .set_branding_icon(normalized, "image/png")
        .await
        .map_err(AppError::internal)?;
    audit(&state, current.user.id, "branding_updated", &headers).await;
    Ok(Json(json!({
        "version": version,
        "icon_url": format!("/api/v1/branding/icon?v={version}"),
    })))
}

pub async fn delete_icon(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let removed = state
        .db
        .delete_branding_icon()
        .await
        .map_err(AppError::internal)?;
    audit(&state, current.user.id, "branding_reset", &headers).await;
    Ok(Json(json!({ "deleted": removed })))
}

pub async fn custom_css(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let Some((content, version)) = state.db.custom_css().await.map_err(AppError::internal)? else {
        return Err(AppError::NotFound);
    };
    if content.is_empty() {
        return Err(AppError::NotFound);
    }
    let etag = format!("\"css-{version}\"");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(axum::body::Body::from(content))
        .map_err(|error| AppError::internal(anyhow::Error::from(error)))
}

#[derive(Debug, serde::Deserialize)]
pub struct CustomCssRequest {
    content: String,
}

pub async fn put_custom_css(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CustomCssRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    if request.content.trim().is_empty() {
        return Err(AppError::bad_request("custom CSS must not be empty"));
    }
    if request.content.len() > 256 * 1024 {
        return Err(AppError::too_large("custom CSS must be at most 256 KiB"));
    }
    let version = state
        .db
        .set_custom_css(&request.content)
        .await
        .map_err(AppError::internal)?;
    audit(&state, current.user.id, "custom_css_updated", &headers).await;
    Ok(Json(json!({ "version": version })))
}

pub async fn delete_custom_css(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let version = state
        .db
        .clear_custom_css()
        .await
        .map_err(AppError::internal)?;
    audit(&state, current.user.id, "custom_css_cleared", &headers).await;
    Ok(Json(json!({ "version": version })))
}

fn normalize_icon(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| AppError::bad_request("invalid image data"))?;
    let format = reader
        .format()
        .ok_or_else(|| AppError::bad_request("unknown image format"))?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(AppError::bad_request(
            "only PNG, JPEG and WebP icons are supported",
        ));
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| AppError::bad_request("invalid image dimensions"))?;
    if width == 0 || height == 0 || width > MAX_ICON_EDGE || height > MAX_ICON_EDGE {
        return Err(AppError::bad_request(format!(
            "icon dimensions must be between 1 and {MAX_ICON_EDGE} pixels"
        )));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| AppError::bad_request("invalid image data"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_ICON_EDGE);
    limits.max_image_height = Some(MAX_ICON_EDGE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| AppError::bad_request("failed to decode icon"))?;
    let mut output = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut output), ImageFormat::Png)
        .map_err(|_| AppError::bad_request("failed to encode icon"))?;
    Ok(output)
}

async fn audit(state: &Arc<AppState>, user_id: i64, action: &str, headers: &HeaderMap) {
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: action.to_owned(),
            path: None,
            detail: None,
            ip: auth::client_ip(headers),
        })
        .await;
}
