use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use image::GenericImageView;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::auth;
use crate::authz;
use crate::error::AppError;
use crate::jobs::{JobKind, JobOutcome};
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
    headers: HeaderMap,
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
    let failure_path = cache_path.with_extension("fail");

    if matches_etag(&headers, &cache_key) {
        return not_modified(&cache_key);
    }

    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        touch_cache_file_async(cache_path.clone()).await;
        return image_response(bytes, &cache_key);
    }
    if tokio::fs::metadata(&failure_path).await.is_ok() {
        return Err(AppError::NotFound);
    }

    let user_id = current.user.id;
    let job = if let Some(job) = state.jobs.active_job_exact(JobKind::Thumbnail, &cache_key) {
        job
    } else {
        let max_source = settings.settings.thumbnails.max_source_bytes;
        let max_pixels = settings.settings.thumbnails.max_pixels;
        let video_seek_seconds = settings.settings.thumbnails.video_seek_seconds;
        let video_timeout_secs = settings.settings.thumbnails.video_timeout_secs.max(1);
        let job_state = Arc::clone(&state);
        let job_source = source.clone();
        let job_cache_dir = cache_dir.clone();
        let job_cache_path = cache_path.clone();
        let job_failure_path = failure_path.clone();
        let job_ffmpeg = state.ffmpeg.clone();
        let job_key = cache_key.clone();
        let job_is_image = is_image_extension(&source);
        let payload = json!({
            "source": source.to_string_lossy(),
            "width": width,
            "fit_short": fit_short,
            "edge": edge,
        });
        let outcome = state
            .jobs
            .submit(
                JobKind::Thumbnail,
                cache_key.clone(),
                user_id,
                payload,
                1,
                move |ctx| async move {
                    let cancel = ctx.token();
                    let file_size = std::fs::metadata(&job_source)
                        .map(|metadata| metadata.len())
                        .unwrap_or_default();
                    let semaphore = if file_size >= 8 * 1024 * 1024 {
                        job_state
                            .thumb_large_semaphore
                            .clone()
                            .unwrap_or_else(|| Arc::clone(&job_state.thumb_semaphore))
                    } else {
                        Arc::clone(&job_state.thumb_semaphore)
                    };
                    let _permit = semaphore.acquire().await.map_err(|_| {
                        AppError::internal(anyhow::anyhow!("thumbnail worker closed"))
                    })?;
                    let has_ffmpeg = job_ffmpeg.is_some();
                    let scan_cancel = cancel.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        generate_thumbnail(
                            &job_source,
                            width,
                            fit_short,
                            max_source,
                            max_pixels,
                            job_ffmpeg.as_deref(),
                            video_seek_seconds,
                            video_timeout_secs,
                            &scan_cancel,
                            &job_cache_dir,
                            &job_cache_path,
                        )
                    })
                    .await
                    .map_err(|err| {
                        AppError::internal(anyhow::anyhow!("thumbnail task failed: {err}"))
                    })?;
                    match result {
                        Ok(bytes) => {
                            let _ = std::fs::remove_file(&job_failure_path);
                            Ok(JobOutcome::done(
                                json!({ "cache_key": job_key, "bytes": bytes.len() }),
                                1,
                                1,
                            ))
                        }
                        Err(error) => {
                            if matches!(error, AppError::NotFound) && (job_is_image || has_ffmpeg) {
                                if let Some(parent) = job_failure_path.parent() {
                                    let _ = std::fs::create_dir_all(parent);
                                }
                                let _ = std::fs::write(&job_failure_path, b"1");
                            }
                            Err(error)
                        }
                    }
                },
            )
            .await?;
        outcome.job
    };

    let finished = if job.is_finished() {
        job
    } else {
        match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            state.jobs.wait_for(&job.id),
        )
        .await
        {
            Ok(Some(job)) => job,
            Ok(None) => {
                return Err(AppError::internal(anyhow::anyhow!(
                    "thumbnail job disappeared"
                )));
            }
            Err(_) => {
                return Err(AppError::internal(anyhow::anyhow!(
                    "thumbnail generation timed out"
                )));
            }
        }
    };

    match finished.status.as_str() {
        "completed" => {
            let bytes = tokio::fs::read(&cache_path)
                .await
                .map_err(|err| AppError::internal(anyhow::Error::from(err)))?;
            let _ = prune_cache(&cache_dir, settings.settings.thumbnails.cache_max_bytes);
            image_response(bytes, &cache_key)
        }
        "cancelled" => Err(AppError::NotFound),
        _ => {
            tracing::warn!(
                cache_key = %cache_key,
                error = finished.error.as_deref().unwrap_or("unknown error"),
                "thumbnail job failed"
            );
            Err(AppError::NotFound)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn can_preview(name: &str, video_enabled: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(extension) = lower.rsplit_once('.').map(|(_, extension)| extension) else {
        return false;
    };
    if matches!(
        extension,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "tiff" | "tif" | "bmp"
    ) {
        return true;
    }
    video_enabled && matches!(extension, "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v")
}

#[allow(clippy::too_many_arguments)]
fn is_image_extension(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        name.rsplit_once('.').map(|(_, extension)| extension),
        Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "tiff" | "tif" | "bmp")
    )
}

async fn touch_cache_file_async(path: PathBuf) {
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
    })
    .await;
}

fn matches_etag(headers: &HeaderMap, key: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|part| {
                let part = part.trim();
                part == "*" || part == format!("\"{key}\"")
            })
        })
}

fn not_modified(key: &str) -> Result<Response, AppError> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NOT_MODIFIED;
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

#[allow(clippy::too_many_arguments)]
fn generate_thumbnail(
    source: &Path,
    width: u32,
    fit_short: bool,
    max_source: u64,
    max_pixels: u64,
    ffmpeg: Option<&Path>,
    video_seek_seconds: u64,
    video_timeout_secs: u64,
    cancel: &CancellationToken,
    cache_dir: &Path,
    cache_path: &Path,
) -> Result<Vec<u8>, AppError> {
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
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
            extract_video_frame(
                ffmpeg,
                source,
                width,
                video_seek_seconds,
                video_timeout_secs,
                cancel,
            )?
        }
    };
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
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
    seek_seconds: u64,
    timeout_secs: u64,
    cancel: &CancellationToken,
) -> Result<image::DynamicImage, AppError> {
    let mut child = Command::new(ffmpeg)
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-ss",
            &seek_seconds.to_string(),
            "-i",
        ])
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
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io_error)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("ffmpeg stdout unavailable")))?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(AppError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().unwrap_or_default();
                if !status.success() || bytes.is_empty() {
                    return Err(AppError::NotFound);
                }
                return image::load_from_memory(&bytes).map_err(|_| AppError::NotFound);
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(io_error(error));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(AppError::NotFound);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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
    if max_bytes == 0 {
        return Ok(());
    }
    let mut files = Vec::new();
    collect_cache_files(directory, &mut files);
    let mut total: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total <= max_bytes {
        return Ok(());
    }
    let target = max_bytes.saturating_mul(90) / 100;
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= target {
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
