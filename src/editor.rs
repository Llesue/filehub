use std::borrow::Cow;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use encoding_rs::{
    BIG5, EUC_KR, Encoding, GB18030, GBK, ISO_8859_15, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE,
    WINDOWS_1250, WINDOWS_1251, WINDOWS_1252,
};
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
    "gb18030",
    "big5",
    "shift_jis",
    "euc-kr",
    "windows-1250",
    "windows-1251",
    "windows-1252",
    "iso-8859-1",
    "iso-8859-15",
];

/// Candidate encodings for legacy text detection. Order is only a final
/// tie-breaker: chardetng provides a hint, then replacement/control counts and
/// a small character-plausibility penalty decide the winner.
const DETECTION_ENCODINGS: &[&Encoding] = &[
    GBK,
    GB18030,
    BIG5,
    SHIFT_JIS,
    EUC_KR,
    WINDOWS_1252,
    WINDOWS_1251,
    WINDOWS_1250,
    ISO_8859_15,
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
    /// ISO-8859-1 and Windows-1252 are byte-identical outside 0x80-0x9F.
    /// Keep the precise label for the editor while still using the superset
    /// Windows-1252 codec for encoding.
    iso_8859_1: bool,
}

impl DecodedText<'_> {
    fn encoding_label(&self) -> String {
        if self.iso_8859_1 {
            "iso-8859-1".to_owned()
        } else {
            self.encoding.name().to_ascii_lowercase()
        }
    }
}

struct Classification<'a> {
    is_text: bool,
    reason: &'static str,
    decoded: Option<DecodedText<'a>>,
}

fn binary_classification() -> Classification<'static> {
    Classification {
        is_text: false,
        reason: "binary",
        decoded: None,
    }
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

/// encoding_rs aliases the ISO-8859-1 label to Windows-1252 because that is
/// what the WHATWG Encoding Standard specifies. When the user explicitly asks
/// for ISO-8859-1, reject code points outside Latin-1 instead of silently
/// writing Windows-1252 punctuation bytes.
fn is_iso_8859_1_label(label: &str) -> bool {
    matches!(
        label.trim().to_ascii_lowercase().as_str(),
        "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1"
    )
}

fn iso_8859_1_representable(content: &str) -> bool {
    content.chars().all(|value| u32::from(value) <= 0xFF)
}

fn decode_with<'a>(bytes: &'a [u8], encoding: &'static Encoding) -> DecodedText<'a> {
    let (start, bom) = match bom_for(encoding) {
        Some(mark) if bytes.starts_with(mark) => (mark.len(), true),
        _ => (0, false),
    };
    let (text, had_errors) = encoding.decode_without_bom_handling(&bytes[start..]);
    DecodedText {
        text,
        encoding,
        bom,
        lossy: had_errors,
        iso_8859_1: false,
    }
}

/// UTF-16 without a BOM usually has a very lopsided NUL distribution: ASCII
/// code units put one zero byte in every pair. CJK-only UTF-16 is inherently
/// ambiguous without a BOM and falls through to the legacy candidates.
fn utf16_without_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let pairs = bytes.len() / 2;
    let mut even_zeros = 0usize;
    let mut odd_zeros = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == 0 {
            if index % 2 == 0 {
                even_zeros += 1;
            } else {
                odd_zeros += 1;
            }
        }
    }
    let (encoding, high, low) = if odd_zeros >= even_zeros {
        (UTF_16LE, odd_zeros, even_zeros)
    } else {
        (UTF_16BE, even_zeros, odd_zeros)
    };
    // At least a quarter of code units must look like ASCII and almost none
    // may violate the expected zero parity.
    if high.saturating_mul(4) < pairs || low.saturating_mul(20) > pairs {
        return None;
    }
    Some(encoding)
}

fn has_c1_bytes(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| (0x80..=0x9F).contains(byte))
}

fn detection_hint(bytes: &[u8]) -> Option<&'static Encoding> {
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Deny);
    if encoding == UTF_8 || encoding == UTF_16LE || encoding == UTF_16BE {
        None
    } else {
        Some(encoding)
    }
}

#[derive(Clone, Copy)]
struct TextStats {
    replacements: usize,
    controls: usize,
    suspicious: usize,
    latin_letters: usize,
    other_script: usize,
    cjk_chars: usize,
    total: usize,
}

fn text_stats(text: &str) -> TextStats {
    let mut stats = TextStats {
        replacements: 0,
        controls: 0,
        suspicious: 0,
        latin_letters: 0,
        other_script: 0,
        cjk_chars: 0,
        total: 0,
    };
    for value in text.chars() {
        stats.total += 1;
        if value == '\u{FFFD}' {
            stats.replacements += 1;
            continue;
        }
        if value.is_control() && !matches!(value, '\n' | '\r' | '\t' | '\u{0c}') {
            stats.controls += 1;
        }
        if value.is_alphabetic() {
            if value.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&value) {
                stats.latin_letters += 1;
            } else {
                stats.other_script += 1;
            }
        }
        if ('\u{FF61}'..='\u{FF9F}').contains(&value) {
            stats.other_script += 1;
        }
        if ('\u{3040}'..='\u{30FF}').contains(&value)
            || ('\u{3400}'..='\u{4DBF}').contains(&value)
            || ('\u{4E00}'..='\u{9FFF}').contains(&value)
            || ('\u{AC00}'..='\u{D7AF}').contains(&value)
            || ('\u{F900}'..='\u{FAFF}').contains(&value)
            || ('\u{FF66}'..='\u{FF9D}').contains(&value)
        {
            stats.cjk_chars += 1;
        }
        // These are the ISO-8859-1 interpretations of the bytes ISO-8859-15
        // uses for €, Š, š, Ž, ž, Œ, œ and Ÿ, plus spacing diacritics that
        // Windows-1250 produces for bytes Windows-1252 renders as fractions.
        if matches!(
            value,
            '¤' | '¦'
                | '¨'
                | '´'
                | '¸'
                | '¼'
                | '¾'
                | '\u{02C7}'
                | '\u{02D8}'
                | '\u{02D9}'
                | '\u{02DB}'
                | '\u{02DD}'
        ) {
            stats.suspicious += 1;
        }
    }
    stats
}

/// A few Cyrillic, kana or CJK characters embedded in otherwise Latin text
/// almost always mean the wrong legacy encoding was chosen (for example
/// Windows-1252 bytes read as Windows-1251).
fn mixed_script(stats: &TextStats) -> bool {
    stats.latin_letters >= 8
        && stats.other_script > 0
        && stats.other_script.saturating_mul(3) < stats.latin_letters
}

fn controls_excessive(total: usize, controls: usize) -> bool {
    if total == 0 {
        return false;
    }
    if total < 8 {
        return controls > 0;
    }
    controls > (total / 100).max(1)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct CandidateRank {
    replacements: usize,
    controls: usize,
    suspicious: usize,
    script_penalty: u8,
    hint_penalty: u8,
    order: usize,
}

/// Single-byte Western candidates can decode CJK or Cyrillic bytes into
/// plausible-looking accented Latin letters (for example GBK "中文" becomes
/// "ÖÐÎÄ" in Windows-1252). Mark text that is almost entirely made of
/// U+00C0..U+00FF letters as an unlikely Western result.
fn latin1_heavy(text: &str) -> bool {
    let mut letters = 0usize;
    let mut accented = 0usize;
    for value in text.chars() {
        if value.is_alphabetic() {
            letters += 1;
            if ('\u{00C0}'..='\u{00FF}').contains(&value) {
                accented += 1;
            }
        }
    }
    letters >= 4 && accented * 10 > letters * 7
}

fn has_gb18030_four_byte(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|window| {
        (0x81..=0xFE).contains(&window[0])
            && (0x30..=0x39).contains(&window[1])
            && (0x81..=0xFE).contains(&window[2])
            && (0x30..=0x39).contains(&window[3])
    })
}

fn decode_legacy(bytes: &[u8]) -> Option<DecodedText<'_>> {
    let four_byte = has_gb18030_four_byte(bytes);
    let hint = if four_byte {
        Some(GB18030)
    } else {
        detection_hint(bytes)
    };
    let mut best: Option<(CandidateRank, DecodedText<'_>)> = None;
    for (order, encoding) in DETECTION_ENCODINGS.iter().enumerate() {
        let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
        if text.is_empty() || text.contains('\0') {
            continue;
        }
        let stats = text_stats(&text);
        if controls_excessive(stats.total, stats.controls) {
            continue;
        }
        // chardetng is only a tie-breaker. GB18030 and GBK decode the same
        // GBK text; keep the narrower label when both are valid.
        let hinted = hint.is_some_and(|value| {
            value == *encoding || (!four_byte && value == GB18030 && *encoding == GBK)
        });
        let western = *encoding == WINDOWS_1252
            || *encoding == WINDOWS_1251
            || *encoding == WINDOWS_1250
            || *encoding == ISO_8859_15;
        let cjk_encoding = *encoding == GBK
            || *encoding == GB18030
            || *encoding == BIG5
            || *encoding == SHIFT_JIS
            || *encoding == EUC_KR;
        let rank = CandidateRank {
            replacements: stats.replacements,
            controls: stats.controls,
            suspicious: stats.suspicious,
            script_penalty: u8::from(
                mixed_script(&stats)
                    || (western && latin1_heavy(&text))
                    || (cjk_encoding && stats.cjk_chars == 0),
            ),
            hint_penalty: u8::from(!hinted),
            order,
        };
        let decoded = DecodedText {
            text,
            encoding,
            bom: false,
            lossy: had_errors,
            iso_8859_1: false,
        };
        if best.as_ref().is_none_or(|(best_rank, _)| rank < *best_rank) {
            best = Some((rank, decoded));
        }
    }
    best.map(|(_, mut decoded)| {
        if decoded.encoding == WINDOWS_1252 && !has_c1_bytes(bytes) {
            decoded.iso_8859_1 = true;
        }
        decoded
    })
}

fn decoded_is_binary(decoded: &DecodedText<'_>) -> bool {
    decoded.text.contains('\0') || looks_binary(&decoded.text)
}

fn classify_decoded(decoded: DecodedText<'_>) -> Classification<'_> {
    if decoded.text.is_empty() {
        return Classification {
            is_text: true,
            reason: "empty",
            decoded: Some(decoded),
        };
    }
    if decoded_is_binary(&decoded) {
        return binary_classification();
    }
    Classification {
        is_text: true,
        reason: if decoded.lossy { "lossy" } else { "text" },
        decoded: Some(decoded),
    }
}

fn classify_text(bytes: &[u8]) -> Classification<'_> {
    if bytes.is_empty() {
        return Classification {
            is_text: true,
            reason: "empty",
            decoded: Some(DecodedText {
                text: Cow::Borrowed(""),
                encoding: UTF_8,
                bom: false,
                lossy: false,
                iso_8859_1: false,
            }),
        };
    }
    if has_binary_signature(bytes) || is_utf32_bom(bytes) {
        return binary_classification();
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return classify_decoded(decode_with(bytes, UTF_8));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return classify_decoded(decode_with(bytes, UTF_16LE));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return classify_decoded(decode_with(bytes, UTF_16BE));
    }
    if let Some(encoding) = utf16_without_bom(bytes) {
        let decoded = decode_with(bytes, encoding);
        if !decoded_is_binary(&decoded) {
            return classify_decoded(decoded);
        }
    }
    if bytes.contains(&0) {
        return binary_classification();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return classify_decoded(DecodedText {
            text: Cow::Borrowed(text),
            encoding: UTF_8,
            bom: false,
            lossy: false,
            iso_8859_1: false,
        });
    }
    if let Some(decoded) = decode_legacy(bytes) {
        return classify_decoded(decoded);
    }
    binary_classification()
}

fn decode_text(bytes: &[u8]) -> Option<DecodedText<'_>> {
    classify_text(bytes).decoded
}

fn encode_text(content: &str, encoding: &'static Encoding, bom: bool) -> Option<Vec<u8>> {
    // encoding_rs only decodes UTF-16; its encoder always produces UTF-8.
    // Write UTF-16 code units ourselves so saving preserves the encoding.
    if encoding == UTF_16LE || encoding == UTF_16BE {
        let mut bytes = Vec::with_capacity(content.len() * 2 + 2);
        if bom && let Some(mark) = bom_for(encoding) {
            bytes.extend_from_slice(mark);
        }
        for unit in content.encode_utf16() {
            if encoding == UTF_16LE {
                bytes.extend_from_slice(&unit.to_le_bytes());
            } else {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
        }
        return Some(bytes);
    }
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

fn too_large_json(size: u64) -> Value {
    json!({
        "is_text": false,
        "reason": "too_large",
        "size": size,
        "encoding": Value::Null,
        "bom": false,
        "lossy": false,
    })
}

fn probe_json(size: u64, classification: &Classification<'_>) -> Value {
    let decoded = classification.decoded.as_ref();
    json!({
        "is_text": classification.is_text,
        "reason": classification.reason,
        "size": size,
        "encoding": decoded.map(DecodedText::encoding_label),
        "bom": decoded.is_some_and(|value| value.bom),
        "lossy": decoded.is_some_and(|value| value.lossy),
    })
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
            return Ok(Json(too_large_json(size)));
        }
        let (bytes, _) = client
            .read_bytes(
                mount.grant_id,
                &query.path,
                settings.settings.editor.max_file_bytes,
            )
            .await
            .map_err(|error| error.into_app())?;
        let classification = classify_text(&bytes);
        return Ok(Json(probe_json(size, &classification)));
    }
    let resolved = authz::resolve(&state, current.user.id, query.root, &query.path, false).await?;
    let info = tokio::task::spawn_blocking({
        let path = resolved.absolute.clone();
        move || fsops::file_info(&path)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("editor metadata task failed: {err}")))??;
    if info.size > settings.settings.editor.max_file_bytes {
        return Ok(Json(too_large_json(info.size)));
    }
    let bytes = tokio::fs::read(&resolved.absolute)
        .await
        .map_err(fsops::io_error)?;
    let classification = classify_text(&bytes);
    Ok(Json(probe_json(info.size, &classification)))
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
            "encoding": decoded.encoding_label(),
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
        "encoding": decoded.encoding_label(),
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
        Some(label) => {
            let encoding = parse_encoding(label)
                .ok_or_else(|| AppError::bad_request("unknown or unsupported encoding"))?;
            if is_iso_8859_1_label(label) && !iso_8859_1_representable(&request.content) {
                return Err(AppError::EncodingUnrepresentable);
            }
            encoding
        }
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
    if text.contains('\0') {
        return true;
    }
    let stats = text_stats(text);
    controls_excessive(stats.total, stats.controls)
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
    use super::{
        DETECTION_ENCODINGS, SUPPORTED_ENCODINGS, classify_text, decode_text, detect_line_ending,
        encode_text, is_iso_8859_1_label, iso_8859_1_representable, parse_encoding, too_large_json,
    };
    use encoding_rs::{
        BIG5, EUC_KR, Encoding, GB18030, GBK, ISO_8859_15, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE,
        WINDOWS_1250, WINDOWS_1251, WINDOWS_1252,
    };

    fn encoded(text: &str, encoding: &'static Encoding) -> Vec<u8> {
        encode_text(text, encoding, false).expect("fixture is not representable")
    }

    fn assert_decoded(bytes: &[u8], expected: &str, encoding: &'static Encoding, bom: bool) {
        let classification = classify_text(bytes);
        assert!(classification.is_text, "not text");
        assert_eq!(classification.reason, "text");
        let decoded = classification.decoded.expect("missing decoded text");
        assert_eq!(&*decoded.text, expected);
        assert_eq!(decoded.encoding, encoding);
        assert_eq!(decoded.bom, bom);
        assert!(!decoded.lossy);
    }

    #[test]
    fn too_large_probe_keeps_the_metadata_shape() {
        let value = too_large_json(42);
        assert_eq!(value["is_text"], false);
        assert_eq!(value["reason"], "too_large");
        assert_eq!(value["size"], 42);
        assert!(value["encoding"].is_null());
        assert_eq!(value["bom"], false);
        assert_eq!(value["lossy"], false);
    }

    #[test]
    fn reports_empty_content() {
        let classification = classify_text(b"");
        assert!(classification.is_text);
        assert_eq!(classification.reason, "empty");
        assert_eq!(&*classification.decoded.unwrap().text, "");

        let classification = classify_text(&[0xFF, 0xFE]);
        assert!(classification.is_text);
        assert_eq!(classification.reason, "empty");
        let decoded = classification.decoded.unwrap();
        assert_eq!(decoded.encoding, UTF_16LE);
        assert!(decoded.bom);
    }

    #[test]
    fn detects_utf8_text() {
        let classification = classify_text(b"hello\nworld\n");
        assert!(classification.is_text);
        assert_eq!(classification.reason, "text");
        let decoded = classification.decoded.expect("missing text");
        assert_eq!(&*decoded.text, "hello\nworld\n");
        assert_eq!(decoded.encoding, UTF_8);
        assert!(!decoded.bom);

        let classification = classify_text("中文\n".as_bytes());
        assert!(classification.is_text);
        assert_eq!(classification.reason, "text");
        assert_eq!(&*decode_text("中文\n".as_bytes()).unwrap().text, "中文\n");
    }

    #[test]
    fn detects_utf8_bom() {
        let bytes = encode_text("hello", UTF_8, true).unwrap();
        assert_decoded(&bytes, "hello", UTF_8, true);
    }

    #[test]
    fn detects_utf16_with_bom() {
        let text = "hello 中文\n";
        for encoding in [UTF_16LE, UTF_16BE] {
            let bytes = encode_text(text, encoding, true).unwrap();
            assert_decoded(&bytes, text, encoding, true);
        }
    }

    #[test]
    fn detects_utf16_without_bom() {
        let text = "hello world\nsecond line\n";
        for encoding in [UTF_16LE, UTF_16BE] {
            let bytes = encoded(text, encoding);
            assert_decoded(&bytes, text, encoding, false);
        }
    }

    #[test]
    fn detects_gbk_and_gb18030() {
        let text = "简体中文测试，你好世界。";
        assert_decoded(&encoded(text, GBK), text, GBK, false);

        let text = "𠀀中文";
        assert_decoded(&encoded(text, GB18030), text, GB18030, false);
    }

    #[test]
    fn detects_big5() {
        let text = "繁體中文測試，你好世界。";
        assert_decoded(&encoded(text, BIG5), text, BIG5, false);
    }

    #[test]
    fn detects_shift_jis() {
        let text = "日本語のテキストです。こんにちは。";
        assert_decoded(&encoded(text, SHIFT_JIS), text, SHIFT_JIS, false);
    }

    #[test]
    fn detects_euc_kr() {
        let text = "한국어 텍스트입니다. 안녕하세요.";
        assert_decoded(&encoded(text, EUC_KR), text, EUC_KR, false);
    }

    #[test]
    fn detects_windows_codepages() {
        let text = "Příliš žluťoučký kůň úpěl ďábelské ódy.";
        assert_decoded(&encoded(text, WINDOWS_1250), text, WINDOWS_1250, false);

        let text = "Съешь ещё этих мягких французских булок, да выпей чаю.";
        assert_decoded(&encoded(text, WINDOWS_1251), text, WINDOWS_1251, false);

        let text = "“Smart quotes” — café €5 and ½ of it.";
        assert_decoded(&encoded(text, WINDOWS_1252), text, WINDOWS_1252, false);
    }

    #[test]
    fn detects_iso_8859_1_and_15() {
        // ISO-8859-1 and Windows-1252 are byte-identical outside 0x80-0x9F.
        let text = "café señor niño";
        let bytes = encoded(text, WINDOWS_1252);
        let classification = classify_text(&bytes);
        assert!(classification.is_text);
        assert_eq!(classification.reason, "text");
        let decoded = classification.decoded.expect("missing text");
        assert_eq!(&*decoded.text, text);
        assert_eq!(decoded.encoding, WINDOWS_1252);
        assert_eq!(decoded.encoding_label(), "iso-8859-1");

        let text = "Le prix est 20 €, l'œuvre de Škoda se déguste.";
        assert_decoded(&encoded(text, ISO_8859_15), text, ISO_8859_15, false);
    }

    #[test]
    fn lossy_utf16_is_still_editable() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("hello ".encode_utf16().flat_map(|unit| unit.to_le_bytes()));
        // Unpaired high surrogate: decodes to U+FFFD but is clearly text.
        bytes.extend([0x00, 0xD8]);
        let classification = classify_text(&bytes);
        assert!(classification.is_text);
        assert_eq!(classification.reason, "lossy");
        let decoded = classification.decoded.expect("missing text");
        assert_eq!(decoded.encoding, UTF_16LE);
        assert!(decoded.bom);
        assert!(decoded.lossy);
        assert!(decoded.text.starts_with("hello "));
        assert!(decoded.text.contains('\u{FFFD}'));
    }

    #[test]
    fn round_trips_gbk_and_utf16_bom() {
        for (text, encoding, bom) in [
            ("简体中文测试\r\n第二行", GBK, false),
            ("简体中文测试\r\n第二行", UTF_16LE, true),
        ] {
            let bytes = encode_text(text, encoding, bom).unwrap();
            let decoded = decode_text(&bytes).expect("missing text");
            assert_eq!(&*decoded.text, text);
            assert_eq!(decoded.encoding, encoding);
            assert_eq!(decoded.bom, bom);
            assert_eq!(
                encode_text(&decoded.text, decoded.encoding, decoded.bom).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn keeps_bom_on_round_trip() {
        let bytes = encode_text("hello", UTF_8, true).unwrap();
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
        assert!(encode_text("中文", WINDOWS_1252, false).is_none());
        assert!(!iso_8859_1_representable("€"));
        assert!(iso_8859_1_representable("café señor"));
        assert!(is_iso_8859_1_label("ISO-8859-1"));
        assert!(is_iso_8859_1_label("latin1"));
        assert!(!is_iso_8859_1_label("windows-1252"));
    }

    #[test]
    fn rejects_known_binary_signatures() {
        for bytes in [
            b"\x89PNG\r\n\x1a\nrest".as_slice(),
            b"%PDF-1.7\n",
            b"PK\x03\x04rest",
            b"\x7fELFrest",
            b"MZ\x90\x00",
            b"GIF89a......",
        ] {
            let classification = classify_text(bytes);
            assert!(!classification.is_text, "{bytes:?}");
            assert_eq!(classification.reason, "binary");
            assert!(classification.decoded.is_none());
        }
    }

    #[test]
    fn rejects_binary_and_control_heavy_content() {
        assert!(!classify_text(b"hello\0world").is_text);
        let mut bytes = Vec::new();
        for _ in 0..20 {
            bytes.extend_from_slice(&[1, 2, 3, 4]);
        }
        let classification = classify_text(&bytes);
        assert!(!classification.is_text);
        assert_eq!(classification.reason, "binary");
    }

    #[test]
    fn valid_legacy_bytes_stay_editable() {
        // Valid GBK bytes; legacy text must stay editable.
        assert!(classify_text(&[0x80, 0x81, 0x82]).is_text);
    }

    #[test]
    fn supports_every_detected_encoding() {
        for encoding in [UTF_8, UTF_16LE, UTF_16BE] {
            assert!(
                SUPPORTED_ENCODINGS.contains(&encoding.name().to_ascii_lowercase().as_str()),
                "{}",
                encoding.name()
            );
        }
        for encoding in DETECTION_ENCODINGS {
            assert!(
                SUPPORTED_ENCODINGS.contains(&encoding.name().to_ascii_lowercase().as_str()),
                "{}",
                encoding.name()
            );
        }
        assert!(SUPPORTED_ENCODINGS.contains(&"iso-8859-1"));
        for label in [
            "utf-8",
            "utf-16le",
            "utf-16be",
            "gbk",
            "gb18030",
            "big5",
            "shift_jis",
            "euc-kr",
            "windows-1250",
            "windows-1251",
            "windows-1252",
            "iso-8859-1",
            "iso-8859-15",
        ] {
            assert!(parse_encoding(label).is_some(), "{label}");
        }
    }

    #[test]
    fn detects_line_endings() {
        assert_eq!(detect_line_ending("a\nb\n"), "lf");
        assert_eq!(detect_line_ending("a\r\nb\r\n"), "crlf");
        assert_eq!(detect_line_ending("a\r\nb\nc\n"), "mixed");
        assert_eq!(detect_line_ending("no newline"), "lf");
    }
}
