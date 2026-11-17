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
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let source = resolved.absolute.clone();
    let fit_short = query.fit.as_deref() == Some("short");
    let edge = query
        .edge
        .unwrap_or_else(|| query.w.unwrap_or(256))
        .clamp(16, 2048);
    thumbnail_response(
        &state,
        source,
        edge,
        edge,
        fit_short,
        current.user.id,
        &headers,
    )
    .await
}

pub(crate) async fn thumbnail_response(
    state: &Arc<AppState>,
    source: PathBuf,
    width: u32,
    edge: u32,
    fit_short: bool,
    user_id: i64,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    let settings = state.settings();
    if !settings.settings.thumbnails.enabled {
        return Err(AppError::NotFound);
    }
    let cache_dir = settings.settings.cache_dir.join("thumbs");
    let (cache_key, cache_path, failure_path) =
        thumb_paths(&cache_dir, &source, width, fit_short, edge)?;

    if matches_etag(headers, &cache_key) {
        return not_modified(&cache_key);
    }

    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        touch_cache_file_async(cache_path.clone()).await;
        return image_response(bytes, &cache_key);
    }
    if tokio::fs::metadata(&failure_path).await.is_ok() {
        return Err(AppError::NotFound);
    }

    let job = if let Some(job) = state.jobs.active_job_exact(JobKind::Thumbnail, &cache_key) {
        job
    } else {
        let max_source = settings.settings.thumbnails.max_source_bytes;
        let max_pixels = settings.settings.thumbnails.max_pixels;
        let jpeg_max_pixels = settings.settings.thumbnails.jpeg_max_pixels;
        let video_seek_seconds = settings.settings.thumbnails.video_seek_seconds;
        let video_seek_percent = settings.settings.thumbnails.video_seek_percent;
        let video_timeout_secs = settings.settings.thumbnails.video_timeout_secs.max(1);
        let job_state = Arc::clone(state);
        let job_source = source.clone();
        let job_cache_dir = cache_dir.clone();
        let job_cache_path = cache_path.clone();
        let job_failure_path = failure_path.clone();
        let job_ffmpeg = state.ffmpeg.clone();
        let job_key = cache_key.clone();
        let job_log_source = job_source.clone();
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
                    let scan_cancel = cancel.clone();
                    let started = Instant::now();
                    let result = tokio::task::spawn_blocking(move || {
                        generate_thumbnail(
                            &job_source,
                            width,
                            fit_short,
                            max_source,
                            max_pixels,
                            jpeg_max_pixels,
                            job_ffmpeg.as_deref(),
                            video_seek_seconds,
                            video_seek_percent,
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
                            tracing::debug!(
                                path = %job_log_source.display(),
                                width,
                                bytes = bytes.len(),
                                duration_ms = started.elapsed().as_millis() as u64,
                                "thumbnail generated"
                            );
                            Ok(JobOutcome::done(
                                json!({ "cache_key": job_key, "bytes": bytes.len() }),
                                1,
                                1,
                            ))
                        }
                        Err(ThumbError::Cancelled) => Err(AppError::Cancelled),
                        Err(ThumbError::Failed { reason, message }) => {
                            if let Some(parent) = job_failure_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let record = json!({
                                "reason": reason,
                                "message": message,
                                "version": 3,
                                "at": crate::db::unix_now(),
                            });
                            let _ = std::fs::write(&job_failure_path, record.to_string());
                            tracing::warn!(
                                path = %job_log_source.display(),
                                width,
                                %reason,
                                %message,
                                duration_ms = started.elapsed().as_millis() as u64,
                                "thumbnail generation failed"
                            );
                            Err(AppError::NotFound)
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

#[derive(Debug)]
enum ThumbError {
    Cancelled,
    Failed {
        reason: &'static str,
        message: String,
    },
}

impl ThumbError {
    fn failed(reason: &'static str, message: impl Into<String>) -> Self {
        Self::Failed {
            reason,
            message: message.into(),
        }
    }
}

impl From<ThumbError> for AppError {
    fn from(error: ThumbError) -> Self {
        match error {
            ThumbError::Cancelled => AppError::Cancelled,
            ThumbError::Failed { .. } => AppError::NotFound,
        }
    }
}

fn is_video_extension(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        name.rsplit_once('.').map(|(_, extension)| extension),
        Some("mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v")
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_thumbnail(
    source: &Path,
    width: u32,
    fit_short: bool,
    max_source: u64,
    max_pixels: u64,
    jpeg_max_pixels: u64,
    ffmpeg: Option<&Path>,
    video_seek_seconds: u64,
    video_seek_percent: u32,
    video_timeout_secs: u64,
    cancel: &CancellationToken,
    cache_dir: &Path,
    cache_path: &Path,
) -> Result<Vec<u8>, ThumbError> {
    if cancel.is_cancelled() {
        return Err(ThumbError::Cancelled);
    }
    let metadata = std::fs::metadata(source)
        .map_err(|error| ThumbError::failed("io", format!("failed to read metadata: {error}")))?;
    if !metadata.is_file() {
        return Err(ThumbError::failed("unsupported", "source is not a file"));
    }
    // Video files are handled by ffmpeg and can legitimately be several GB;
    // the source-size guard only applies to images decoded in memory.
    let video_like = is_video_extension(source);
    if !video_like && metadata.len() > max_source {
        return Err(ThumbError::failed(
            "max_source",
            format!(
                "source is {} bytes; limit is {max_source} bytes",
                metadata.len()
            ),
        ));
    }

    // Decide by content, not extension: downloads often carry a .jpeg name
    // while the payload is PNG/WebP and vice versa.
    let guessed = image::ImageReader::open(source)
        .ok()
        .and_then(|reader| reader.with_guessed_format().ok())
        .and_then(|reader| reader.format());
    let image = if guessed == Some(image::ImageFormat::Jpeg) {
        decode_jpeg(source, width, fit_short, jpeg_max_pixels, max_pixels)?
    } else if guessed.is_some() {
        decode_image(source, max_pixels)?
    } else {
        let Some(ffmpeg) = ffmpeg else {
            return Err(ThumbError::failed(
                "unsupported",
                "no decoder is available for this file",
            ));
        };
        let seek_seconds = video_seek_seconds_for(
            ffmpeg,
            source,
            video_seek_seconds,
            video_seek_percent,
            video_timeout_secs,
            cancel,
        );
        extract_video_frame_with_fallback(
            ffmpeg,
            source,
            width,
            seek_seconds,
            video_timeout_secs,
            cancel,
        )?
    };
    if cancel.is_cancelled() {
        return Err(ThumbError::Cancelled);
    }
    let image = apply_orientation(image, crate::properties::orientation(source));
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
        .map_err(|error| ThumbError::failed("encode", format!("JPEG encode failed: {error}")))?;

    std::fs::create_dir_all(cache_path.parent().unwrap_or(cache_dir)).map_err(|error| {
        ThumbError::failed("io", format!("failed to create cache dir: {error}"))
    })?;
    std::fs::write(cache_path, &encoded)
        .map_err(|error| ThumbError::failed("io", format!("failed to write cache: {error}")))?;
    Ok(encoded)
}

fn apply_orientation(image: image::DynamicImage, orientation: u32) -> image::DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

fn decode_image(source: &Path, max_pixels: u64) -> Result<image::DynamicImage, ThumbError> {
    let reader = image::ImageReader::open(source)
        .map_err(|error| ThumbError::failed("io", format!("failed to open image: {error}")))?
        .with_guessed_format()
        .map_err(|error| {
            ThumbError::failed("decode", format!("failed to detect image format: {error}"))
        })?;
    let (width, height) = reader.into_dimensions().map_err(|error| {
        ThumbError::failed("decode", format!("failed to read image header: {error}"))
    })?;
    if width as u64 * height as u64 > max_pixels {
        return Err(ThumbError::failed(
            "too_large",
            format!("image is {width}x{height} pixels; limit is {max_pixels}"),
        ));
    }

    let mut reader = image::ImageReader::open(source)
        .map_err(|error| ThumbError::failed("io", format!("failed to open image: {error}")))?
        .with_guessed_format()
        .map_err(|error| {
            ThumbError::failed("decode", format!("failed to detect image format: {error}"))
        })?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(65_535);
    limits.max_image_height = Some(65_535);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| ThumbError::failed("decode", format!("image decode failed: {error}")))
}

/// Decode JPEG with `jpeg-decoder` so large images can use DCT scaling instead
/// of allocating the full-resolution bitmap first.
fn decode_jpeg(
    source: &Path,
    width: u32,
    fit_short: bool,
    jpeg_max_pixels: u64,
    fallback_max_pixels: u64,
) -> Result<image::DynamicImage, ThumbError> {
    let file = std::fs::File::open(source)
        .map_err(|error| ThumbError::failed("io", format!("failed to open JPEG: {error}")))?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    decoder
        .read_info()
        .map_err(|error| ThumbError::failed("decode", format!("invalid JPEG: {error}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| ThumbError::failed("decode", "JPEG has no frame information"))?;
    if info.pixel_format == jpeg_decoder::PixelFormat::CMYK32 {
        // CMYK/YCCK needs colour conversion that the image crate already does
        // correctly; these files are rare and still bounded by max_pixels.
        return decode_image(source, fallback_max_pixels);
    }
    if info.pixel_format == jpeg_decoder::PixelFormat::L16 {
        return Err(ThumbError::failed(
            "unsupported",
            "16-bit JPEG is not supported",
        ));
    }
    let pixels = info.width as u64 * info.height as u64;
    if pixels > jpeg_max_pixels {
        return Err(ThumbError::failed(
            "too_large",
            format!(
                "image is {}x{} pixels; limit is {jpeg_max_pixels}",
                info.width, info.height
            ),
        ));
    }

    let original_width = info.width as f64;
    let original_height = info.height as f64;
    let short_edge = original_width.min(original_height);
    let scale = if fit_short {
        (width as f64 / short_edge).min(1.0)
    } else {
        (width as f64 / original_width).min(1.0)
    };
    let (output_width, output_height) = if scale < 1.0 {
        let requested_width =
            ((original_width * scale).ceil() as u64).clamp(1, u16::MAX as u64) as u16;
        let requested_height =
            ((original_height * scale).ceil() as u64).clamp(1, u16::MAX as u64) as u16;
        decoder
            .scale(requested_width, requested_height)
            .map_err(|error| {
                ThumbError::failed("decode", format!("JPEG scaled decode failed: {error}"))
            })?
    } else {
        (info.width, info.height)
    };
    let data = decoder
        .decode()
        .map_err(|error| ThumbError::failed("decode", format!("JPEG decode failed: {error}")))?;

    let image = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            image::RgbImage::from_raw(output_width as u32, output_height as u32, data)
                .map(image::DynamicImage::ImageRgb8)
        }
        jpeg_decoder::PixelFormat::L8 => {
            image::GrayImage::from_raw(output_width as u32, output_height as u32, data)
                .map(image::DynamicImage::ImageLuma8)
        }
        _ => None,
    };
    image.ok_or_else(|| {
        ThumbError::failed("decode", "JPEG decoder returned an unexpected buffer size")
    })
}

fn video_seek_seconds_for(
    ffmpeg: &Path,
    source: &Path,
    fallback_seconds: u64,
    percent: u32,
    timeout_secs: u64,
    cancel: &CancellationToken,
) -> f64 {
    if percent == 0 {
        return fallback_seconds as f64;
    }
    let Some(duration) = probe_duration(ffmpeg, source, timeout_secs.min(5), cancel) else {
        return fallback_seconds as f64;
    };
    let seek = duration * f64::from(percent) / 100.0;
    if seek.is_finite() && seek > 0.0 {
        seek
    } else {
        fallback_seconds as f64
    }
}

fn probe_duration(
    ffmpeg: &Path,
    source: &Path,
    timeout_secs: u64,
    cancel: &CancellationToken,
) -> Option<f64> {
    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(source)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stderr = child.stderr.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return None;
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                let bytes = reader.join().ok()?;
                return parse_duration(&String::from_utf8_lossy(&bytes));
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn parse_duration(text: &str) -> Option<f64> {
    let rest = text.split("Duration: ").nth(1)?;
    let stamp = rest.split(',').next()?.trim();
    if stamp == "N/A" {
        return None;
    }
    let mut parts = stamp.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn extract_video_frame_with_fallback(
    ffmpeg: &Path,
    source: &Path,
    width: u32,
    seek_seconds: f64,
    timeout_secs: u64,
    cancel: &CancellationToken,
) -> Result<image::DynamicImage, ThumbError> {
    match extract_video_frame(ffmpeg, source, width, seek_seconds, timeout_secs, cancel) {
        Err(ThumbError::Failed {
            reason: "ffmpeg", ..
        }) if seek_seconds > 0.0 => {
            extract_video_frame(ffmpeg, source, width, 0.0, timeout_secs, cancel)
        }
        other => other,
    }
}

fn extract_video_frame(
    ffmpeg: &Path,
    source: &Path,
    width: u32,
    seek_seconds: f64,
    timeout_secs: u64,
    cancel: &CancellationToken,
) -> Result<image::DynamicImage, ThumbError> {
    let seek = format!("{seek_seconds:.3}");
    let mut child = Command::new(ffmpeg)
        .args(["-nostdin", "-loglevel", "error", "-ss", &seek, "-i"])
        .arg(source)
        .args([
            "-frames:v",
            "1",
            "-an",
            "-vf",
            &format!("scale={width}:-2"),
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            ThumbError::failed("ffmpeg", format!("failed to start ffmpeg: {error}"))
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ThumbError::failed("ffmpeg", "ffmpeg stdout is unavailable"))?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.read_to_end(&mut bytes);
        }
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = stderr_reader.join();
            return Err(ThumbError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().unwrap_or_default();
                let stderr = stderr_reader.join().unwrap_or_default();
                if !status.success() || bytes.is_empty() {
                    tracing::warn!(
                        status = %status,
                        stderr = %String::from_utf8_lossy(&stderr[..stderr.len().min(2000)]).trim(),
                        "ffmpeg frame extraction failed"
                    );
                    return Err(ThumbError::failed(
                        "ffmpeg",
                        format!("ffmpeg exited with {status}"),
                    ));
                }
                return image::load_from_memory(&bytes).map_err(|error| {
                    ThumbError::failed(
                        "decode",
                        format!("ffmpeg produced an invalid frame: {error}"),
                    )
                });
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = stderr_reader.join();
                return Err(ThumbError::failed(
                    "ffmpeg",
                    format!("failed to poll ffmpeg: {error}"),
                ));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = stderr_reader.join();
            return Err(ThumbError::failed(
                "ffmpeg_timeout",
                format!("ffmpeg exceeded {timeout_secs}s"),
            ));
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
    hasher.update(b"thumb-v4");
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn thumb_paths(
    cache_dir: &Path,
    source: &Path,
    width: u32,
    fit_short: bool,
    edge: u32,
) -> Result<(String, PathBuf, PathBuf), AppError> {
    let key = cache_key(source, width, fit_short, edge)?;
    let cache_path = cache_dir
        .join(&key[0..2])
        .join(&key[2..4])
        .join(format!("{key}.jpg"));
    let failure_path = cache_path.with_extension("fail");
    Ok((key, cache_path, failure_path))
}

pub async fn status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ThumbQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
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
    let fit_short = query.fit.as_deref() == Some("short");
    let edge = query
        .edge
        .unwrap_or_else(|| query.w.unwrap_or(256))
        .clamp(16, 2048);
    let cache_dir = settings.settings.cache_dir.join("thumbs");
    let (_key, cache_path, failure_path) =
        thumb_paths(&cache_dir, &resolved.absolute, edge, fit_short, edge)?;
    if tokio::fs::metadata(&cache_path).await.is_ok() {
        return Ok(axum::Json(json!({ "state": "ready" })));
    }
    if let Ok(bytes) = tokio::fs::read(&failure_path).await {
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        return Ok(axum::Json(json!({
            "state": "failed",
            "reason": value.get("reason").and_then(serde_json::Value::as_str),
            "message": value.get("message").and_then(serde_json::Value::as_str),
        })));
    }
    Ok(axum::Json(json!({ "state": "missing" })))
}

#[derive(Debug, Deserialize)]
pub struct ThumbRetryRequest {
    root: i64,
    path: String,
    w: Option<u32>,
    edge: Option<u32>,
    fit: Option<String>,
}

pub async fn retry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ThumbRetryRequest>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let settings = state.settings();
    if !settings.settings.thumbnails.enabled {
        return Err(AppError::NotFound);
    }
    let root = state
        .db
        .find_root(request.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let resolved = authz::resolve(&state, current.user.id, root.id, &request.path, false).await?;
    let fit_short = request.fit.as_deref() == Some("short");
    let edge = request
        .edge
        .unwrap_or_else(|| request.w.unwrap_or(320))
        .clamp(16, 2048);
    let cache_dir = settings.settings.cache_dir.join("thumbs");
    let mut cleared = 0_usize;
    let mut widths = vec![edge];
    if edge != 2048 {
        widths.push(2048);
    }
    for width in widths {
        let (_key, cache_path, failure_path) =
            thumb_paths(&cache_dir, &resolved.absolute, width, fit_short, width)?;
        if tokio::fs::remove_file(&cache_path).await.is_ok() {
            cleared += 1;
        }
        if tokio::fs::remove_file(&failure_path).await.is_ok() {
            cleared += 1;
        }
    }
    audit(
        &state,
        current.user.id,
        "thumb_retry",
        &resolved.relative,
        &headers,
    )
    .await;
    Ok(axum::Json(json!({ "cleared": cleared })))
}

async fn audit(state: &Arc<AppState>, user_id: i64, action: &str, path: &str, headers: &HeaderMap) {
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: action.to_owned(),
            path: Some(path.to_owned()),
            detail: None,
            ip: auth::client_ip(headers),
        })
        .await;
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

#[cfg(test)]
mod duration_tests {
    use super::parse_duration;

    #[test]
    fn parses_ffmpeg_duration() {
        let stderr =
            "Input #0, mov,mp4,m4a:\n  Duration: 01:23:45.67, start: 0.000000, bitrate: 123 kb/s\n";
        assert!((parse_duration(stderr).unwrap() - 5025.67).abs() < 0.001);
    }

    #[test]
    fn rejects_unknown_duration() {
        assert!(parse_duration("Duration: N/A, start: 0.0").is_none());
        assert!(parse_duration("no duration here").is_none());
    }
}
