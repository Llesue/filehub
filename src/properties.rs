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
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    // Inspect the path as requested so a symlink is reported as a link, not
    // as its canonical target (authz has already validated the target).
    let absolute = Path::new(&resolved.root.real_path).join(&resolved.relative);
    let relative = resolved.relative.clone();
    let value = tokio::task::spawn_blocking(move || describe(&absolute, &relative))
        .await
        .map_err(|error| {
            AppError::internal(anyhow::anyhow!("properties task failed: {error}"))
        })??;
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

fn exif_info(path: &Path) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
    let mut info = serde_json::Map::new();
    let fields = [
        ("make", exif::Tag::Make),
        ("model", exif::Tag::Model),
        ("lens", exif::Tag::LensModel),
        ("taken_at", exif::Tag::DateTimeOriginal),
        ("exposure", exif::Tag::ExposureTime),
        ("f_number", exif::Tag::FNumber),
        ("iso", exif::Tag::PhotographicSensitivity),
        ("focal_length", exif::Tag::FocalLength),
    ];
    for (key, tag) in fields {
        if let Some(field) = exif.get_field(tag, exif::In::PRIMARY) {
            info.insert(
                key.to_owned(),
                Value::String(field.display_value().with_unit(&exif).to_string()),
            );
        }
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
