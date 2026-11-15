use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::db::UserPreferences;
use crate::error::AppError;
use crate::state::AppState;

fn system_json(state: &Arc<AppState>) -> Value {
    let settings = state.settings();
    json!({
        "thumbnails": settings.settings.thumbnails.enabled,
        "editor": settings.settings.editor.enabled,
        "terminal": settings.settings.terminal_enabled,
    })
}

fn preferences_json(preferences: &UserPreferences) -> Value {
    json!({
        "theme": preferences.theme,
        "language": preferences.language,
        "show_hidden": preferences.show_hidden,
        "show_thumbnails": preferences.show_thumbnails,
        "upload_conflict": preferences.upload_conflict,
        "terminal_enabled": preferences.terminal_enabled,
        "editor_enabled": preferences.editor_enabled,
        "custom_css_root_id": preferences.custom_css_root_id,
        "custom_css_path": preferences.custom_css_path,
        "custom_css_version": preferences.custom_css_version,
        "avatar_version": preferences.avatar_version,
    })
}

pub async fn get_preferences(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "preferences": preferences_json(&preferences),
        "system": system_json(&state),
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePreferencesRequest {
    theme: Option<String>,
    language: Option<String>,
    show_hidden: Option<bool>,
    show_thumbnails: Option<bool>,
    upload_conflict: Option<String>,
    terminal_enabled: Option<bool>,
    editor_enabled: Option<bool>,
    clear_custom_css: Option<bool>,
    custom_css_root_id: Option<i64>,
    custom_css_path: Option<String>,
}

pub async fn update_preferences(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpdatePreferencesRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let mut preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;

    if let Some(theme) = request.theme {
        if !matches!(theme.as_str(), "light" | "dark") {
            return Err(AppError::bad_request("theme must be light or dark"));
        }
        preferences.theme = theme;
    }
    if let Some(language) = request.language {
        if !matches!(language.as_str(), "zh-CN" | "en") {
            return Err(AppError::bad_request("language must be zh-CN or en"));
        }
        preferences.language = language;
    }
    if let Some(show_hidden) = request.show_hidden {
        preferences.show_hidden = show_hidden;
    }
    if let Some(show_thumbnails) = request.show_thumbnails {
        preferences.show_thumbnails = show_thumbnails;
    }
    if let Some(terminal_enabled) = request.terminal_enabled {
        preferences.terminal_enabled = terminal_enabled;
    }
    if let Some(editor_enabled) = request.editor_enabled {
        preferences.editor_enabled = editor_enabled;
    }
    if let Some(conflict) = request.upload_conflict {
        if !matches!(conflict.as_str(), "rename" | "overwrite" | "skip") {
            return Err(AppError::bad_request("invalid upload conflict policy"));
        }
        preferences.upload_conflict = conflict;
    }

    if request.clear_custom_css.unwrap_or(false) {
        preferences.custom_css_root_id = None;
        preferences.custom_css_path = None;
        preferences.custom_css_version += 1;
    } else if let (Some(root_id), Some(path)) =
        (request.custom_css_root_id, request.custom_css_path)
    {
        let resolved = authz::resolve(&state, current.user.id, root_id, &path, false).await?;
        if resolved.relative.is_empty()
            || !resolved.absolute.is_file()
            || !path.to_ascii_lowercase().ends_with(".css")
        {
            return Err(AppError::bad_request("custom CSS must be a .css file"));
        }
        let metadata = tokio::fs::metadata(&resolved.absolute)
            .await
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        if metadata.len() > 1024 * 1024 {
            return Err(AppError::too_large("custom CSS file is too large"));
        }
        let bytes = tokio::fs::read(&resolved.absolute)
            .await
            .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
        if std::str::from_utf8(&bytes).is_err() {
            return Err(AppError::invalid_path("custom CSS must be UTF-8"));
        }
        preferences.custom_css_root_id = Some(root_id);
        preferences.custom_css_path = Some(resolved.relative);
        preferences.custom_css_version += 1;
    }

    state
        .db
        .save_user_preferences(current.user.id, &preferences)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "preferences": preferences_json(&preferences),
        "system": system_json(&state),
    })))
}

pub async fn get_custom_css(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let (Some(root_id), Some(path)) = (preferences.custom_css_root_id, preferences.custom_css_path)
    else {
        return Err(AppError::NotFound);
    };
    let resolved = authz::resolve(&state, current.user.id, root_id, &path, false).await?;
    let bytes = tokio::fs::read(&resolved.absolute)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    Response::builder()
        .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
        .header(header::CACHE_CONTROL, "private, max-age=0, must-revalidate")
        .body(axum::body::Body::from(bytes))
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))
}

pub async fn get_avatar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    let Some(path) = preferences.avatar_path else {
        return Err(AppError::NotFound);
    };
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    Response::builder()
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "private, max-age=0, must-revalidate")
        .body(axum::body::Body::from(bytes))
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))
}

pub async fn upload_avatar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    if body.len() > 2 * 1024 * 1024 {
        return Err(AppError::too_large("avatar must be at most 2 MiB"));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !matches!(content_type, "image/jpeg" | "image/png" | "image/webp") {
        return Err(AppError::UnsupportedMedia(
            "avatar must be JPEG, PNG or WebP".to_owned(),
        ));
    }
    let image = image::load_from_memory(&body)
        .map_err(|_| AppError::UnsupportedMedia("invalid avatar image".to_owned()))?;
    let image = image.thumbnail(256, 256);
    let mut encoded = Vec::new();
    image
        .to_rgb8()
        .write_to(&mut Cursor::new(&mut encoded), image::ImageFormat::Jpeg)
        .map_err(|_| AppError::UnsupportedMedia("could not encode avatar".to_owned()))?;

    let avatars = avatar_directory(&state);
    tokio::fs::create_dir_all(&avatars)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
    let path = avatars.join(format!("{}.jpg", current.user.id));
    tokio::fs::write(&path, encoded)
        .await
        .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;

    let mut preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    preferences.avatar_path = Some(path.to_string_lossy().to_string());
    preferences.avatar_version += 1;
    state
        .db
        .save_user_preferences(current.user.id, &preferences)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({
        "preferences": preferences_json(&preferences),
        "system": system_json(&state),
    })))
}

fn avatar_directory(state: &Arc<AppState>) -> PathBuf {
    state
        .settings()
        .settings
        .db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("avatars")
}

pub async fn delete_avatar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let mut preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    if let Some(path) = preferences.avatar_path.take() {
        let _ = tokio::fs::remove_file(path).await;
    }
    preferences.avatar_version += 1;
    state
        .db
        .save_user_preferences(current.user.id, &preferences)
        .await
        .map_err(AppError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}
