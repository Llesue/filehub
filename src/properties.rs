use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PropertiesQuery {
    root: i64,
    path: String,
}

pub async fn properties(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PropertiesQuery>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
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
        let name = query
            .path
            .rsplit_once('/')
            .map(|(_, name)| name)
            .unwrap_or(query.path.as_str());
        return Ok(Json(json!({
            "name": name,
            "path": query.path,
            "is_dir": stat.get("is_dir").cloned().unwrap_or(json!(false)),
            "is_symlink": false,
            "mime_type": stat.get("mime_type").cloned().unwrap_or(json!("")),
            "size": stat.get("size").cloned().unwrap_or(json!(0)),
            "created": serde_json::Value::Null,
            "modified": stat.get("modified").cloned().unwrap_or(json!(0)),
            "accessed": serde_json::Value::Null,
        })));
    }
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    // Inspect the path as requested so a symlink is reported as a link, not
    // as its canonical target (authz has already validated the target).
    let absolute = Path::new(&resolved.root.real_path).join(&resolved.relative);
    let hash_path = resolved.absolute.clone();
    let relative = resolved.relative.clone();
    let mut value = tokio::task::spawn_blocking(move || describe(&absolute, &relative))
        .await
        .map_err(|error| {
            AppError::internal(anyhow::anyhow!("properties task failed: {error}"))
        })??;
    if value.get("is_dir").and_then(Value::as_bool) != Some(true)
        && !value
            .get("is_symlink")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && let Ok(metadata) = tokio::fs::metadata(&hash_path).await
        && metadata.is_file()
    {
        let mtime_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos() as i64)
            .unwrap_or_default();
        if let Some(hash) = crate::hash::cached_property_hash(
            &state,
            &hash_path.to_string_lossy(),
            metadata.len() as i64,
            mtime_ns,
        )
        .await?
        {
            for (key, item) in hash.as_object().into_iter().flatten() {
                value[key] = item.clone();
            }
        }
    }
    Ok(Json(value))
}

fn describe(path: &Path, relative: &str) -> Result<Value, AppError> {
    let symlink_metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    let is_symlink = symlink_metadata.file_type().is_symlink();
    let metadata = if is_symlink {
        std::fs::metadata(path).unwrap_or_else(|_| symlink_metadata.clone())
    } else {
        symlink_metadata.clone()
    };
    let is_dir = metadata.is_dir();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(relative)
        .to_owned();

    let mut value = json!({
        "name": name,
        "path": relative,
        "is_dir": is_dir,
        "is_symlink": is_symlink,
        "mime_type": mime_guess::from_path(path).first_or_octet_stream().to_string(),
        "size": metadata.len(),
        "created": timestamp(metadata.created().ok()),
        "modified": timestamp(metadata.modified().ok()),
        "accessed": timestamp(metadata.accessed().ok()),
    });

    if is_symlink {
        let (state, target) = symlink_state(path);
        value["symlink_state"] = json!(state);
        value["symlink_target"] = json!(target);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        value["mode"] = json!(format!("{:04o}", metadata.mode() & 0o7777));
    }

    if !is_dir
        && !is_symlink
        && let Some(image) = image_info(path)
    {
        value["image"] = image;
    }

    Ok(value)
}

fn symlink_state(path: &Path) -> (&'static str, Option<String>) {
    let target = std::fs::read_link(path).ok();
    match std::fs::metadata(path) {
        Ok(_) => {
            let display = target.and_then(|target| {
                // Only expose relative targets; absolute ones may point at
                // paths the caller cannot browse.
                if target.is_absolute() {
                    None
                } else {
                    target.to_str().map(str::to_owned)
                }
            });
            ("ok", display)
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => ("denied", None),
        Err(_) => ("broken", None),
    }
}

fn timestamp(time: Option<std::time::SystemTime>) -> Option<i64> {
    time.and_then(|time| {
        time.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs() as i64)
    })
}

fn image_info(path: &Path) -> Option<Value> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let format = reader.format()?;
    let (width, height) = reader.into_dimensions().ok()?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut image = json!({
        "width": width,
        "height": height,
        "format": format.extensions_str().first().copied().unwrap_or("unknown"),
    });
    if matches!(extension.as_str(), "jpg" | "jpeg" | "tif" | "tiff")
        && let Some(exif) = exif_info(path)
    {
        image["exif"] = exif;
    }
    Some(image)
}

fn clean_ascii(bytes: &[u8]) -> String {
    // Camera firmware often pads ASCII fields with NUL or 0xFF. The exif
    // crate prints those padding bytes, which surfaced as trailing garbage.
    let end = bytes
        .iter()
        .position(|byte| *byte == 0 || *byte == 0xFF)
        .unwrap_or(bytes.len());
    let mut slice = &bytes[..end];
    while matches!(slice.last(), Some(b' ') | Some(b'\t')) {
        slice = &slice[..slice.len() - 1];
    }
    String::from_utf8_lossy(slice).trim().to_owned()
}

fn ascii_field(field: &exif::Field) -> Option<String> {
    match &field.value {
        exif::Value::Ascii(parts) => {
            let text = clean_ascii(parts.first()?);
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn rational_field(field: &exif::Field) -> Option<f64> {
    match &field.value {
        exif::Value::Rational(values) => values
            .first()
            .filter(|value| value.denom != 0)
            .map(|value| f64::from(value.num) / f64::from(value.denom)),
        _ => None,
    }
}

fn trim_decimal(value: f64, digits: usize) -> String {
    let text = format!("{value:.digits$}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn format_exposure(seconds: f64) -> String {
    if seconds <= 0.0 {
        return String::new();
    }
    if seconds < 1.0 {
        format!("1/{} s", (1.0 / seconds).round() as u64)
    } else {
        format!("{} s", trim_decimal(seconds, 2))
    }
}

fn exif_info(path: &Path) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
    let mut info = serde_json::Map::new();

    for (key, tag) in [
        ("make", exif::Tag::Make),
        ("model", exif::Tag::Model),
        ("lens", exif::Tag::LensModel),
        ("taken_at", exif::Tag::DateTimeOriginal),
    ] {
        if let Some(field) = exif.get_field(tag, exif::In::PRIMARY)
            && let Some(text) = ascii_field(field)
        {
            info.insert(key.to_owned(), Value::String(text));
        }
    }
    if let Some(field) = exif.get_field(exif::Tag::ExposureTime, exif::In::PRIMARY)
        && let Some(seconds) = rational_field(field)
    {
        info.insert(
            "exposure".to_owned(),
            Value::String(format_exposure(seconds)),
        );
    }
    if let Some(field) = exif.get_field(exif::Tag::FNumber, exif::In::PRIMARY)
        && let Some(value) = rational_field(field)
    {
        info.insert(
            "f_number".to_owned(),
            Value::String(format!("f/{}", trim_decimal(value, 1))),
        );
    }
    if let Some(field) = exif.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY) {
        let value = field
            .value
            .get_uint(0)
            .map(|value| Value::String(value.to_string()))
            .or_else(|| ascii_field(field).map(Value::String));
        if let Some(value) = value {
            info.insert("iso".to_owned(), value);
        }
    }
    if let Some(field) = exif.get_field(exif::Tag::FocalLength, exif::In::PRIMARY)
        && let Some(value) = rational_field(field)
    {
        info.insert(
            "focal_length".to_owned(),
            Value::String(format!("{} mm", trim_decimal(value, 1))),
        );
    }
    if let Some(field) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        && let Some(value) = field.value.get_uint(0)
    {
        info.insert("orientation".to_owned(), json!(value));
    }
    (!info.is_empty()).then_some(Value::Object(info))
}

/// EXIF orientation for thumbnails. Returns 1 when absent or unreadable.
pub fn orientation(path: &Path) -> u32 {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "jpg" | "jpeg" | "tif" | "tiff") {
        return 1;
    }
    exif_info(path)
        .and_then(|info| info.get("orientation").and_then(Value::as_u64))
        .map(|value| value as u32)
        .filter(|value| (1..=8).contains(value))
        .unwrap_or(1)
}

fn io_error(error: std::io::Error) -> AppError {
    match error.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        std::io::ErrorKind::PermissionDenied => AppError::Forbidden,
        _ => AppError::internal(anyhow::Error::from(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_fields_stop_at_padding_bytes() {
        assert_eq!(clean_ascii(b"Xiaomi\xff\xff\xff"), "Xiaomi");
        assert_eq!(clean_ascii(b"24094RAD4C\0\0"), "24094RAD4C");
        assert_eq!(clean_ascii(b"  Pixel 9  \0"), "Pixel 9");
        assert_eq!(clean_ascii(b"\0\0\0"), "");
    }

    #[test]
    fn exposure_is_printed_as_a_handheld_shutter_speed() {
        assert_eq!(format_exposure(1.0 / 101.0101), "1/101 s");
        assert_eq!(format_exposure(0.5), "1/2 s");
        assert_eq!(format_exposure(2.0), "2 s");
        assert_eq!(format_exposure(1.6), "1.6 s");
    }

    #[test]
    fn decimals_do_not_keep_trailing_zeros() {
        assert_eq!(trim_decimal(1.590000, 1), "1.6");
        assert_eq!(trim_decimal(4.0, 1), "4");
        assert_eq!(trim_decimal(4.94, 1), "4.9");
    }
}
