use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::Response;
use image::GenericImageView;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::auth;
use crate::authz;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ThumbQuery {
    root: i64,
    path: String,
    w: Option<u32>,
    edge: Option<u32>,
    fit: Option<String>,
}

pub async fn thumb(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(query): Query<ThumbQuery>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let root = state
        .db
        .find_root(query.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let settings = state.settings();
    if !settings.settings.thumbnails.enabled {
        return Err(AppError::NotFound);
    }

    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let source = resolved.absolute.clone();
    let fit_short = query.fit.as_deref() == Some("short");
    let edge = query
        .edge
        .unwrap_or_else(|| query.w.unwrap_or(256))
        .clamp(16, 2048);
    let width = edge;
    let cache_dir = settings.settings.cache_dir.join("thumbs");
    let cache_key = cache_key(&source, width, fit_short, edge)?;
    let cache_path = cache_dir
        .join(&cache_key[0..2])
        .join(&cache_key[2..4])
        .join(format!("{cache_key}.jpg"));

    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return image_response(bytes, &cache_key);
    }

    let job_lock = {
        let mut jobs = state.thumb_jobs.lock();
        jobs.entry(cache_key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _job_guard = job_lock.lock().await;
    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return image_response(bytes, &cache_key);
    }

    let _permit = state
        .thumb_semaphore
        .acquire()
        .await
        .map_err(|_| AppError::internal(anyhow::anyhow!("thumbnail worker closed")))?;
    let max_source = settings.settings.thumbnails.max_source_bytes;
    let max_pixels = settings.settings.thumbnails.max_pixels;
    let cache_dir_clone = cache_dir.clone();
    let cache_path_clone = cache_path.clone();
    let ffmpeg = state.ffmpeg.clone();
    let result = tokio::task::spawn_blocking(move || {
        generate_thumbnail(
            &source,
            width,
            fit_short,
            max_source,
            max_pixels,
            ffmpeg.as_deref(),
            &cache_dir_clone,
            &cache_path_clone,
        )
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("thumbnail task failed: {err}")))?;
    {
        let mut jobs = state.thumb_jobs.lock();
        if jobs
            .get(&cache_key)
            .is_some_and(|current| Arc::ptr_eq(current, &job_lock))
        {
            jobs.remove(&cache_key);
        }
    }
    let bytes = result?;

    let _ = prune_cache(&cache_dir, settings.settings.thumbnails.cache_max_bytes);
    image_response(bytes, &cache_key)
}

#[allow(clippy::too_many_arguments)]
fn generate_thumbnail(
    source: &Path,
    width: u32,
    fit_short: bool,
    max_source: u64,
    max_pixels: u64,
    ffmpeg: Option<&Path>,
    cache_dir: &Path,
    cache_path: &Path,
) -> Result<Vec<u8>, AppError> {
    let metadata = std::fs::metadata(source).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > max_source {
        return Err(AppError::NotFound);
    }

    let mut reader = image::ImageReader::open(source)
        .map_err(io_error)?
        .with_guessed_format()
        .map_err(io_error)?;

    let side = (max_pixels as f64).sqrt().max(1.0) as u32;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(side);
    limits.max_image_height = Some(side);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);

    let image = match reader.decode() {
        Ok(image) => image,
        Err(_) => {
            let Some(ffmpeg) = ffmpeg else {
                return Err(AppError::NotFound);
            };
            extract_video_frame(ffmpeg, source, width)?
        }
    };
    let (target_width, target_height) = if fit_short {
        let (source_width, source_height) = image.dimensions();
        let short = source_width.min(source_height).max(1);
        let scale = (width as f64 / short as f64).min(1.0);
        (
            ((source_width as f64 * scale).round() as u32).max(1),
            ((source_height as f64 * scale).round() as u32).max(1),
        )
    } else {
        (width, u32::MAX)
    };
    let thumbnail = image.thumbnail(target_width, target_height);
    let rgb = thumbnail.to_rgb8();

    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 82)
        .encode_image(&rgb)
        .map_err(|_| AppError::NotFound)?;

    std::fs::create_dir_all(cache_path.parent().unwrap_or(cache_dir)).map_err(io_error)?;
    std::fs::write(cache_path, &encoded).map_err(io_error)?;
    Ok(encoded)
}

fn extract_video_frame(
    ffmpeg: &Path,
    source: &Path,
    width: u32,
) -> Result<image::DynamicImage, AppError> {
    let output = Command::new(ffmpeg)
        .args(["-nostdin", "-loglevel", "error", "-ss", "1", "-i"])
        .arg(source)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={width}:-2"),
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "pipe:1",
        ])
        .output()
        .map_err(io_error)?;

    if !output.status.success() || output.stdout.is_empty() {
        return Err(AppError::NotFound);
    }

    image::load_from_memory(&output.stdout).map_err(|_| AppError::NotFound)
}

pub fn detect_ffmpeg() -> Option<PathBuf> {
    let output = Command::new("ffmpeg").arg("-version").output().ok()?;
    output.status.success().then(|| PathBuf::from("ffmpeg"))
}

fn cache_key(path: &Path, width: u32, fit_short: bool, edge: u32) -> Result<String, AppError> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(metadata.len().to_le_bytes());
    hasher.update(mtime.to_le_bytes());
    hasher.update(width.to_le_bytes());
    hasher.update(edge.to_le_bytes());
    hasher.update(if fit_short { b"short" } else { b"width" });
    hasher.update(b"thumb-v2");
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn image_response(bytes: Vec<u8>, key: &str) -> Result<Response, AppError> {
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{key}\""))
            .map_err(|err| AppError::internal(anyhow::anyhow!(err)))?,
    );
    Ok(response)
}

fn prune_cache(directory: &Path, max_bytes: u64) -> anyhow::Result<()> {
    let mut files = Vec::new();
    collect_cache_files(directory, &mut files);
    let mut total: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total <= max_bytes {
        return Ok(());
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

fn collect_cache_files(
    directory: &Path,
    files: &mut Vec<(PathBuf, u64, Option<std::time::SystemTime>)>,
) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_cache_files(&entry.path(), files);
        } else if metadata.is_file() {
            files.push((entry.path(), metadata.len(), metadata.modified().ok()));
        }
    }
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::internal(anyhow::Error::from(error))
}
