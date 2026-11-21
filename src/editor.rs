use std::borrow::Cow;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::error::AppError;
use crate::fsops;
use crate::state::{AppState, UsageInvalidation};

const SUPPORTED_ENCODINGS: &[&str] = &[
    "utf-8",
    "utf-16le",
    "utf-16be",
    "gbk",
    "big5",
    "shift_jis",
    "euc-kr",
    "windows-1252",
];

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    root: i64,
    path: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    root: i64,
    path: String,
    encoding: Option<String>,
}

struct DecodedText<'a> {
    text: Cow<'a, str>,
    encoding: &'static Encoding,
    bom: bool,
    lossy: bool,
}

fn bom_for(encoding: &'static Encoding) -> Option<&'static [u8]> {
    if encoding == UTF_8 {
        Some(&[0xEF, 0xBB, 0xBF])
    } else if encoding == UTF_16LE {
        Some(&[0xFF, 0xFE])
    } else if encoding == UTF_16BE {
        Some(&[0xFE, 0xFF])
    } else {
        None
    }
}

fn has_binary_signature(bytes: &[u8]) -> bool {
    const SIGNATURES: &[&[u8]] = &[
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        &[0xFF, 0xD8, 0xFF],
        b"GIF87a",
        b"GIF89a",
        b"%PDF-",
        b"PK\x03\x04",
        b"PK\x05\x06",
        &[0x1F, 0x8B],
        &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C],
        b"Rar!\x1A\x07",
        &[0x7F, b'E', b'L', b'F'],
        b"MZ",
        b"SQLite format 3\0",
        &[0xD0, 0xCF, 0x11, 0xE0],
        &[0x00, b'a', b's', b'm'],
        &[0x49, 0x49, 0x2A, 0x00],
        &[0x4D, 0x4D, 0x00, 0x2A],
        b"BM",
        b"RIFF",
    ];
    if SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature))
    {
        return true;
    }
    bytes.len() > 12 && &bytes[4..8] == b"ftyp"
}

fn is_utf32_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) || bytes.starts_with(&[0xFF, 0xFE, 0x00, 0x00])
}

fn parse_encoding(label: &str) -> Option<&'static Encoding> {
    let encoding = Encoding::for_label(label.trim().as_bytes())?;
    let name = encoding.name().to_ascii_lowercase();
    SUPPORTED_ENCODINGS
        .contains(&name.as_str())
        .then_some(encoding)
}

fn decode_with<'a>(bytes: &'a [u8], encoding: &'static Encoding) -> DecodedText<'a> {
    let (start, bom) = match bom_for(encoding) {
        Some(mark) if bytes.starts_with(mark) => (mark.len(), true),
        _ => (0, false),
    };
    let (text, _, had_errors) = encoding.decode(&bytes[start..]);
    DecodedText {
        text,
        encoding,
        bom,
        lossy: had_errors,
    }
}

fn decode_text(bytes: &[u8]) -> Option<DecodedText<'_>> {
    if has_binary_signature(bytes) || is_utf32_bom(bytes) {
        return None;
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let text = std::str::from_utf8(rest).ok()?;
        return Some(DecodedText {
            text: Cow::Borrowed(text),
            encoding: UTF_8,
            bom: true,
            lossy: false,
        });
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let (text, _, had_errors) = UTF_16LE.decode(rest);
        return Some(DecodedText {
            text,
            encoding: UTF_16LE,
            bom: true,
            lossy: had_errors,
        });
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (text, _, had_errors) = UTF_16BE.decode(rest);
        return Some(DecodedText {
            text,
            encoding: UTF_16BE,
            bom: true,
            lossy: had_errors,
        });
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(DecodedText {
            text: Cow::Borrowed(text),
            encoding: UTF_8,
            bom: false,
            lossy: false,
        });
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Deny);
    let (text, _, had_errors) = encoding.decode(bytes);
    Some(DecodedText {
        text,
        encoding,
        bom: false,
        lossy: had_errors,
    })
}

fn encode_text(content: &str, encoding: &'static Encoding, bom: bool) -> Option<Vec<u8>> {
    let (encoded, _, unmappable) = encoding.encode(content);
    if unmappable {
        return None;
    }
    let mut bytes = Vec::with_capacity(encoded.len() + 3);
    if bom && let Some(mark) = bom_for(encoding) {
        bytes.extend_from_slice(mark);
    }
    bytes.extend_from_slice(&encoded);
    Some(bytes)
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
    let root = state
        .db
        .find_root(query.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if root.is_remote() {
        let (client, mount) =
            crate::federation::mounts::client_for_root(&state, current.user.id, root.id).await?;
        let stat = client
            .stat(mount.grant_id, &query.path)
            .await
            .map_err(|error| error.into_app())?;
        let size = stat.get("size").and_then(Value::as_u64).unwrap_or(0);
        if size > settings.settings.editor.max_file_bytes {
            return Ok(Json(json!({
                "is_text": false,
                "reason": "too_large",
                "size": size,
            })));
        }
        let (bytes, _) = client
            .read_bytes(
                mount.grant_id,
                &query.path,
                settings.settings.editor.max_file_bytes,
            )
            .await
            .map_err(|error| error.into_app())?;
        let (is_text, reason) = classify_text(&bytes);
        return Ok(Json(json!({
            "is_text": is_text,
            "reason": reason,
            "size": size,
        })));
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
    Query(query): Query<ReadQuery>,
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
    let root = state
        .db
        .find_root(query.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if root.is_remote() {
        let (client, mount) =
            crate::federation::mounts::client_for_root(&state, current.user.id, root.id).await?;
        let stat = client
            .stat(mount.grant_id, &query.path)
            .await
            .map_err(|error| error.into_app())?;
        let size = stat.get("size").and_then(Value::as_u64).unwrap_or(0);
        if size > settings.settings.editor.max_file_bytes {
            return Err(AppError::too_large("file exceeds the editor size limit"));
        }
        let (bytes, etag) = client
            .read_bytes(
                mount.grant_id,
                &query.path,
                settings.settings.editor.max_file_bytes,
            )
            .await
            .map_err(|error| error.into_app())?;
        let decoded = if let Some(label) = query.encoding.as_deref() {
            let encoding = parse_encoding(label)
                .ok_or_else(|| AppError::bad_request("unknown or unsupported encoding"))?;
            decode_with(&bytes, encoding)
        } else {
            decode_text(&bytes)
                .ok_or_else(|| AppError::UnsupportedMedia("file is not text".to_owned()))?
        };
        if looks_binary(&decoded.text) {
            return Err(AppError::UnsupportedMedia("file is not text".to_owned()));
        }
        let line_ending = detect_line_ending(&decoded.text);
        return Ok(Json(json!({
            "content": decoded.text,
            "etag": etag,
            "size": size,
            "modified": stat.get("modified").cloned().unwrap_or(Value::Null),
            "line_ending": line_ending,
            "writable": mount.mode == "rw",
            "read_only_reason": if mount.mode == "rw" { Value::Null } else { json!("directory") },
            "encoding": decoded.encoding.name().to_ascii_lowercase(),
            "bom": decoded.bom,
            "lossy": decoded.lossy,
        })));
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
    let decoded = if let Some(label) = query.encoding.as_deref() {
        let encoding = parse_encoding(label)
            .ok_or_else(|| AppError::bad_request("unknown or unsupported encoding"))?;
        decode_with(&bytes, encoding)
    } else {
        decode_text(&bytes)
            .ok_or_else(|| AppError::UnsupportedMedia("file is not text".to_owned()))?
    };
    if looks_binary(&decoded.text) {
        return Err(AppError::UnsupportedMedia("file is not text".to_owned()));
    }
    let line_ending = detect_line_ending(&decoded.text);
    let (writable, read_only_reason) = writability(&resolved.absolute);

    Ok(Json(json!({
        "content": decoded.text,
        "etag": info.etag,
        "size": info.size,
        "modified": info.modified,
        "line_ending": line_ending,
        "writable": writable,
        "read_only_reason": read_only_reason,
        "encoding": decoded.encoding.name().to_ascii_lowercase(),
        "bom": decoded.bom,
        "lossy": decoded.lossy,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SaveRequest {
    content: String,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    bom: bool,
    /// Explicit user confirmation to overwrite a file that changed since the
    /// editor loaded it. Normal saves still require If-Match.
    #[serde(default)]
    force: bool,
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

    let encoding = match request.encoding.as_deref() {
        Some(label) => parse_encoding(label)
            .ok_or_else(|| AppError::bad_request("unknown or unsupported encoding"))?,
        None => UTF_8,
    };
    let bytes = encode_text(&request.content, encoding, request.bom)
        .ok_or(AppError::EncodingUnrepresentable)?;
    if bytes.len() as u64 > settings.settings.editor.max_file_bytes {
        return Err(AppError::too_large("content exceeds the editor size limit"));
    }

    let root = state
        .db
        .find_root(query.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    if root.is_remote() {
        let (client, mount) =
            crate::federation::mounts::client_for_root(&state, current.user.id, root.id).await?;
        if mount.mode != "rw" {
            return Err(AppError::Forbidden);
        }
        let stat = client
            .stat(mount.grant_id, &query.path)
            .await
            .map_err(|error| error.into_app())?;
        let expected = if request.force {
            None
        } else {
            let expected = headers
                .get(header::IF_MATCH)
                .and_then(|value| value.to_str().ok())
                .ok_or(AppError::PreconditionFailed)?;
            let current_etag = stat.get("etag").and_then(Value::as_str).unwrap_or("");
            if expected != current_etag {
                return Err(AppError::PreconditionFailed);
            }
            Some(expected.to_owned())
        };
        let (parent, filename) = query
            .path
            .rsplit_once('/')
            .unwrap_or(("", query.path.as_str()));
        let total = bytes.len() as i64;
        let chunk_size = total.max(1);
        let created = client
            .create_upload(
                mount.grant_id,
                parent,
                filename,
                total,
                chunk_size,
                expected.as_deref(),
                None,
            )
            .await
            .map_err(|error| error.into_app())?;
        let peer_id = created.get("id").and_then(Value::as_str).ok_or_else(|| {
            AppError::internal(anyhow::anyhow!("peer did not return an upload id"))
        })?;
        if total > 0 {
            client
                .put_chunk(peer_id, 0, bytes)
                .await
                .map_err(|error| error.into_app())?;
        }
        let completed = client
            .complete_upload(peer_id)
            .await
            .map_err(|error| error.into_app())?;
        crate::federation::proxy::invalidate_root(&state, root.id);
        return Ok(Json(json!({
            "etag": completed.get("etag").cloned().unwrap_or(Value::Null),
            "size": completed.get("size").cloned().unwrap_or(Value::Null),
        })));
    }
    let resolved = authz::resolve(&state, current.user.id, query.root, &query.path, true).await?;
    let info = tokio::task::spawn_blocking({
        let path = resolved.absolute.clone();
        move || fsops::file_info(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("editor metadata task failed: {err}")))??;

    if !request.force {
        let expected = headers
            .get(header::IF_MATCH)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppError::PreconditionFailed)?;
        if expected != info.etag {
            return Err(AppError::PreconditionFailed);
        }
    }
    if !writability(&resolved.absolute).0 {
        return Err(AppError::PermissionDenied);
    }

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

fn detect_line_ending(text: &str) -> &'static str {
    let mut lf = 0usize;
    let mut crlf = 0usize;
    let bytes = text.as_bytes();
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

fn looks_binary(text: &str) -> bool {
    let mut controls = 0usize;
    let mut total = 0usize;
    for value in text.chars() {
        total += 1;
        if value == '\u{FFFD}' {
            continue;
        }
        if value.is_control() && !matches!(value, '\n' | '\r' | '\t' | '\u{0c}') {
            controls += 1;
        }
    }
    total > 0 && controls > (total / 100).max(1)
}

fn classify_text(bytes: &[u8]) -> (bool, &'static str) {
    if bytes.is_empty() {
        return (true, "empty");
    }
    let Some(decoded) = decode_text(bytes) else {
        return (false, "binary");
    };
    if decoded.lossy {
        return (false, "not_utf8");
    }
    let utf16 = decoded.encoding == UTF_16LE || decoded.encoding == UTF_16BE;
    if !utf16 && bytes.contains(&0) {
        return (false, "binary");
    }
    if decoded.encoding != UTF_8
        && decoded
            .text
            .chars()
            .any(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t' | '\u{0c}'))
    {
        return (false, "not_utf8");
    }
    if looks_binary(&decoded.text) {
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
    use super::{classify_text, decode_text, detect_line_ending, encode_text};

    fn derive_test_encoding(label: &str) -> &'static encoding_rs::Encoding {
        encoding_rs::Encoding::for_label(label.as_bytes()).unwrap()
    }

    #[test]
    fn detects_utf8_text() {
        assert_eq!(classify_text(b"hello\nworld\n"), (true, "text"));
        assert_eq!(classify_text("中文\n".as_bytes()), (true, "text"));
    }

    #[test]
    fn rejects_known_binary_signatures() {
        assert_eq!(classify_text(b"\x89PNG\r\n\x1a\nrest"), (false, "binary"));
        assert_eq!(classify_text(b"%PDF-1.7\n"), (false, "binary"));
        assert_eq!(classify_text(b"PK\x03\x04rest"), (false, "binary"));
        assert_eq!(classify_text(b"\x7fELFrest"), (false, "binary"));
    }

    #[test]
    fn detects_gbk_and_utf16() {
        let gbk = encode_text("中文", derive_test_encoding("gbk"), false).unwrap();
        assert_eq!(classify_text(&gbk), (true, "text"));
        let decoded = decode_text(&gbk).unwrap();
        assert_eq!(&*decoded.text, "中文");

        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend("中文".encode_utf16().flat_map(|unit| unit.to_le_bytes()));
        assert_eq!(classify_text(&utf16), (true, "text"));
        assert_eq!(&*decode_text(&utf16).unwrap().text, "中文");
    }

    #[test]
    fn keeps_bom_on_round_trip() {
        let bytes = encode_text("hello", derive_test_encoding("utf-8"), true).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        let decoded = decode_text(&bytes).unwrap();
        assert!(decoded.bom);
        assert_eq!(&*decoded.text, "hello");
        assert_eq!(
            encode_text(&decoded.text, decoded.encoding, decoded.bom).unwrap(),
            bytes
        );
    }

    #[test]
    fn reports_unrepresentable_characters() {
        let encoding = derive_test_encoding("windows-1252");
        assert!(encode_text("中文", encoding, false).is_none());
    }

    #[test]
    fn rejects_binary_and_non_utf8() {
        assert_eq!(classify_text(b"hello\0world"), (false, "binary"));
        assert_eq!(classify_text(&[0xff, 0xfe, 0x00]), (false, "not_utf8"));
        // Valid GBK bytes; legacy text must stay editable.
        assert_eq!(classify_text(&[0x80, 0x81, 0x82]), (true, "text"));
    }

    #[test]
    fn detects_line_endings() {
        assert_eq!(detect_line_ending("a\nb\n"), "lf");
        assert_eq!(detect_line_ending("a\r\nb\r\n"), "crlf");
        assert_eq!(detect_line_ending("a\r\nb\nc\n"), "mixed");
        assert_eq!(detect_line_ending("no newline"), "lf");
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
