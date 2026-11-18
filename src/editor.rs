use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::error::AppError;
use crate::fsops;
use crate::state::{AppState, UsageInvalidation};

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    root: i64,
    path: String,
}

pub async fn probe(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let settings = state.settings();
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !settings.settings.editor.enabled || !preferences.editor_enabled {
        return Err(AppError::NotFound);
    }
    let resolved = authz::resolve(&state, current.user.id, query.root, &query.path, false).await?;
    let info = tokio::task::spawn_blocking({
        let path = resolved.absolute.clone();
        move || fsops::file_info(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("editor metadata task failed: {err}")))??;
    if info.size > settings.settings.editor.max_file_bytes {
        return Ok(Json(json!({
            "is_text": false,
            "reason": "too_large",
            "size": info.size,
        })));
    }
    let bytes = tokio::fs::read(&resolved.absolute)
        .await
        .map_err(fsops::io_error)?;
    let (is_text, reason) = classify_text(&bytes);
    Ok(Json(json!({
        "is_text": is_text,
        "reason": reason,
        "size": info.size,
    })))
}

pub async fn read(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let settings = state.settings();
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !settings.settings.editor.enabled || !preferences.editor_enabled {
        return Err(AppError::NotFound);
    }
    let resolved = authz::resolve(&state, current.user.id, query.root, &query.path, false).await?;
    let info = tokio::task::spawn_blocking({
        let path = resolved.absolute.clone();
        move || fsops::file_info(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("editor metadata task failed: {err}")))??;

    if info.size > settings.settings.editor.max_file_bytes {
        return Err(AppError::too_large("file exceeds the editor size limit"));
    }

    let bytes = tokio::fs::read(&resolved.absolute)
        .await
        .map_err(fsops::io_error)?;
    if !classify_text(&bytes).0 {
        return Err(AppError::UnsupportedMedia(
            "file is not UTF-8 text".to_owned(),
        ));
    }
    let line_ending = detect_line_ending(&bytes);
    let content = String::from_utf8(bytes)
        .map_err(|_| AppError::UnsupportedMedia("file is not UTF-8".to_owned()))?;
    let (writable, read_only_reason) = writability(&resolved.absolute);

    Ok(Json(json!({
        "content": content,
        "etag": info.etag,
        "size": info.size,
        "modified": info.modified,
        "line_ending": line_ending,
        "writable": writable,
        "read_only_reason": read_only_reason,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SaveRequest {
    content: String,
}

pub async fn save(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
    Json(request): Json<SaveRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let settings = state.settings();
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !settings.settings.editor.enabled || !preferences.editor_enabled {
        return Err(AppError::NotFound);
    }
    if request.content.len() as u64 > settings.settings.editor.max_file_bytes {
        return Err(AppError::too_large("content exceeds the editor size limit"));
    }

    let resolved = authz::resolve(&state, current.user.id, query.root, &query.path, true).await?;
    let info = tokio::task::spawn_blocking({
        let path = resolved.absolute.clone();
        move || fsops::file_info(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("editor metadata task failed: {err}")))??;

    let expected = headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::PreconditionFailed)?;
    if expected != info.etag {
        return Err(AppError::PreconditionFailed);
    }
    if !writability(&resolved.absolute).0 {
        return Err(AppError::PermissionDenied);
    }

    let bytes = request.content.into_bytes();
    let path = resolved.absolute.clone();
    let write_path = path.clone();
    tokio::task::spawn_blocking(move || fsops::write_text_atomic(&write_path, &bytes))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("editor save task failed: {err}")))??;

    let new_info = tokio::task::spawn_blocking({
        let path = resolved.absolute.clone();
        move || fsops::file_info(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("editor metadata task failed: {err}")))??;
    let delta = new_info.size as i64 - info.size as i64;
    let usage = if crate::platform::path_within(&path, std::path::Path::new(&current.user.home_dir))
    {
        UsageInvalidation::Delta(delta)
    } else {
        UsageInvalidation::None
    };
    state
        .invalidate_path_caches_with_usage(&path, Some(current.user.id), false, usage)
        .await;

    audit(&state, current.user.id, &resolved.relative, &headers).await;

    Ok(Json(json!({
        "etag": new_info.etag,
        "size": new_info.size,
    })))
}

fn detect_line_ending(bytes: &[u8]) -> &'static str {
    let mut lf = 0usize;
    let mut crlf = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
    }
    match (lf, crlf) {
        (0, 0) | (_, 0) => "lf",
        (0, _) => "crlf",
        _ => "mixed",
    }
}

fn writability(path: &std::path::Path) -> (bool, Option<&'static str>) {
    if fsops::file_writable(path) == Some(false) {
        return (false, Some("file"));
    }
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    if fsops::directory_writable(parent) == Some(false) {
        return (false, Some("directory"));
    }
    (true, None)
}

fn classify_text(bytes: &[u8]) -> (bool, &'static str) {
    if bytes.is_empty() {
        return (true, "empty");
    }
    if bytes.contains(&0) {
        return (false, "binary");
    }
    let mut end = bytes.len();
    let text = loop {
        match std::str::from_utf8(&bytes[..end]) {
            Ok(text) => break text,
            Err(_) if end > 0 && bytes.len().saturating_sub(end) < 4 => end -= 1,
            Err(_) => return (false, "not_utf8"),
        }
    };
    if end == 0 {
        return (false, "not_utf8");
    }
    let control = text
        .chars()
        .filter(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t' | '\u{0c}'))
        .count();
    if control > (text.chars().count() / 100).max(1) {
        return (false, "binary");
    }
    (true, "text")
}

async fn audit(state: &Arc<AppState>, user_id: i64, path: &str, headers: &HeaderMap) {
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: "edit".to_owned(),
            path: Some(path.to_owned()),
            detail: None,
            ip: auth::client_ip(headers),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::{classify_text, detect_line_ending};

    #[test]
    fn detects_utf8_text() {
        assert_eq!(classify_text(b"hello\nworld\n"), (true, "text"));
        assert_eq!(classify_text("中文\n".as_bytes()), (true, "text"));
    }

    #[test]
    fn rejects_binary_and_non_utf8() {
        assert_eq!(classify_text(b"hello\0world"), (false, "binary"));
        assert_eq!(classify_text(&[0xff, 0xfe, 0x00]), (false, "binary"));
        assert_eq!(classify_text(&[0x80, 0x81, 0x82]), (false, "not_utf8"));
    }

    #[test]
    fn detects_line_endings() {
        assert_eq!(detect_line_ending(b"a\nb\n"), "lf");
        assert_eq!(detect_line_ending(b"a\r\nb\r\n"), "crlf");
        assert_eq!(detect_line_ending(b"a\r\nb\nc\n"), "mixed");
        assert_eq!(detect_line_ending(b"no newline"), "lf");
    }

    #[test]
    fn rejects_control_heavy_content() {
        let mut bytes = Vec::new();
        for _ in 0..20 {
            bytes.extend_from_slice(&[1, 2, 3, 4]);
        }
        assert_eq!(classify_text(&bytes), (false, "binary"));
    }
}
