use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
use crate::fsops;
use crate::jobs::{JobKind, JobOutcome};
use crate::state::AppState;

/// Identifies the source image in `thumbnail.ready` / `thumbnail.failed`
/// events so the browser can reload only the affected thumbnail.
#[derive(Debug, Clone)]
pub(crate) struct ThumbScope {
    pub root_id: Option<i64>,
    pub share_id: Option<i64>,
    pub item_id: Option<i64>,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ThumbQuery {
    root: i64,
    path: String,
    w: Option<u32>,
    edge: Option<u32>,
    fit: Option<String>,
    wait: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fit {
    Width,
    Short,
    Long,
}

impl Fit {
    pub(crate) fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("short") => Self::Short,
            Some("long") => Self::Long,
            _ => Self::Width,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Width => "width",
            Self::Short => "short",
            Self::Long => "long",
        }
    }
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
    if root.is_remote() {
        let edge = query
            .edge
            .unwrap_or_else(|| query.w.unwrap_or(256))
            .clamp(16, settings_edge(&state));
        return crate::federation::proxy::thumb(
            &state,
            current.user.id,
            &root,
            &query.path,
            query.w.unwrap_or(256),
            edge,
            Fit::from_query(query.fit.as_deref()).as_str(),
        )
        .await;
    }
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let source = resolved.absolute.clone();
    let fit = Fit::from_query(query.fit.as_deref());
    let preview_edge = settings_edge(&state);
    let edge = query
        .edge
        .unwrap_or_else(|| query.w.unwrap_or(256))
        .clamp(16, preview_edge);
    thumbnail_response(
        &state,
        source,
        edge,
        edge,
        fit,
        current.user.id,
        &headers,
        Some(ThumbScope {
            root_id: Some(root.id),
            share_id: None,
            item_id: None,
            path: resolved.relative,
        }),
        wants_wait(query.wait.as_deref()),
    )
    .await
}

pub(crate) fn wants_wait(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value, "1" | "true" | "yes" | "on"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn thumbnail_response(
    state: &Arc<AppState>,
    source: PathBuf,
    width: u32,
    edge: u32,
    fit: Fit,
    user_id: i64,
    headers: &HeaderMap,
    scope: Option<ThumbScope>,
    wait: bool,
) -> Result<Response, AppError> {
    let settings = state.settings();
    if !settings.settings.thumbnails.enabled {
        return Err(AppError::NotFound);
    }
    let cache_dir = settings.settings.cache_dir.join("thumbs");
    let (cache_key, cache_path, failure_path) = thumb_paths(&cache_dir, &source, width, fit, edge)?;

    if matches_etag(headers, &cache_key) {
        return not_modified(&cache_key);
    }

    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        touch_cache_file_async(cache_path.clone()).await;
        return image_response(bytes, &cache_key);
    }
    if tokio::fs::metadata(&failure_path).await.is_ok() {
        let stale = tokio::fs::read(&failure_path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("version").and_then(serde_json::Value::as_u64))
            .map(|version| version < 4)
            .unwrap_or(true);
        if stale {
            let _ = tokio::fs::remove_file(&failure_path).await;
        } else {
            return Err(AppError::NotFound);
        }
    }

    // Grid thumbnails are only a visual aid. Huge camera/design files would
    // burn minutes of CPU for a 320px image, so skip them and let the grid use
    // the type icon. Full-size previews are generated on demand separately.
    if width < 1024
        && !is_video_extension(&source)
        && let Some(header) = inspect_image(&source)
        && header.width as u64 * header.height as u64 > 100_000_000
    {
        write_failure_record(
            &failure_path,
            "too_large_thumb",
            &format!(
                "source is {}x{}; grid thumbnails are skipped above 100MP",
                header.width, header.height
            ),
        )
        .await;
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
        let job_cache_max_bytes = settings.settings.thumbnails.cache_max_bytes;
        let job_prune_dir = job_cache_dir.clone();
        let job_ffmpeg = state.ffmpeg.clone();
        let job_key = cache_key.clone();
        let job_log_source = job_source.clone();
        let cancel_scope = scope.as_ref().and_then(|scope| {
            let root_id = scope.root_id?;
            let path = scope.path.replace('\\', "/");
            if width >= 1024 {
                Some(format!("file:{root_id}:{path}"))
            } else {
                let dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
                Some(format!("dir:{root_id}:{dir}"))
            }
        });
        let payload = json!({
            "source": source.to_string_lossy(),
            "width": width,
            "fit": fit.as_str(),
            "edge": edge,
            "cancel_scope": cancel_scope,
        });
        let outcome = state
            .jobs
            .submit(
                JobKind::Thumbnail,
                cache_key.clone(),
                user_id,
                payload,
                1,
                width >= 1024,
                move |ctx| async move {
                    let cancel = ctx.token();
                    // Select the lane by output size: a 256px list thumbnail must
                    // never occupy the reserved full-size preview lane, even for
                    // a 50MP source image.
                    let semaphore = if width >= 1024 {
                        Arc::clone(&job_state.thumb_preview_semaphore)
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
                            fit,
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
                            maybe_prune_cache(&job_prune_dir, job_cache_max_bytes);
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
                                let _ = crate::platform::create_private_dir_all(parent);
                            }
                            let record = json!({
                                "reason": reason,
                                "message": message,
                                "version": 4,
                                "at": crate::db::unix_now(),
                            });
                            let _ = crate::platform::write_private_file(
                                &job_failure_path,
                                record.to_string().as_bytes(),
                            );
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

    if !wait {
        spawn_thumb_notifier(
            Arc::clone(state),
            job.id.clone(),
            user_id,
            scope.clone(),
            edge,
        );
        return pending_response();
    }

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
            maybe_prune_cache(&cache_dir, settings.settings.thumbnails.cache_max_bytes);
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

fn spawn_thumb_notifier(
    state: Arc<AppState>,
    job_id: String,
    user_id: i64,
    scope: Option<ThumbScope>,
    width: u32,
) {
    let Some(scope) = scope else {
        return;
    };
    tokio::spawn(async move {
        let Some(finished) = state.jobs.wait_for(&job_id).await else {
            return;
        };
        let event = match finished.status.as_str() {
            "completed" => "thumbnail.ready",
            "failed" | "cancelled" => "thumbnail.failed",
            _ => return,
        };
        publish_thumb_event(&state, event, user_id, &Some(scope), width);
    });
}

fn publish_thumb_event(
    state: &Arc<AppState>,
    event: &str,
    user_id: i64,
    scope: &Option<ThumbScope>,
    width: u32,
) {
    let Some(scope) = scope else {
        return;
    };
    let data = json!({
        "root": scope.root_id,
        "share": scope.share_id,
        "item": scope.item_id,
        "path": scope.path,
        "width": width,
    });
    if let Some(share_id) = scope.share_id {
        state.events.publish_share(event, share_id, data);
    } else {
        state.events.publish(event, Some(user_id), data);
    }
}

/// 1x1 transparent PNG shown while a thumbnail is being generated. The file
/// icon underneath remains visible, so a pending thumbnail never looks broken.
const THUMB_PLACEHOLDER: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60, 0x60, 0x60,
    0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0xa5, 0xf6, 0x45, 0x40, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn pending_response() -> Result<Response, AppError> {
    let mut response = Response::new(Body::from(THUMB_PLACEHOLDER));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
        .headers_mut()
        .insert("x-filehub-thumb-state", HeaderValue::from_static("pending"));
    Ok(response)
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
    fit: Fit,
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
        decode_jpeg(source, width, fit, jpeg_max_pixels, max_pixels)?
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
    // Cap the long edge as well: `fit=short` alone would turn a panorama into
    // an absurdly large intermediate image (e.g. 10240x2048 for 5:1).
    const MAX_EDGE: f64 = 4096.0;
    let (source_width, source_height) = image.dimensions();
    let base = match fit {
        Fit::Short => width as f64 / source_width.min(source_height).max(1) as f64,
        Fit::Long => width as f64 / source_width.max(source_height).max(1) as f64,
        Fit::Width => width as f64 / source_width.max(1) as f64,
    };
    let scale = base
        .min(1.0)
        .min(MAX_EDGE / source_width.max(1) as f64)
        .min(MAX_EDGE / source_height.max(1) as f64);
    let target_width = ((source_width as f64 * scale).round() as u32).max(1);
    let target_height = ((source_height as f64 * scale).round() as u32).max(1);
    let thumbnail = image.thumbnail(target_width, target_height);
    let rgb = thumbnail.to_rgb8();

    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 82)
        .encode_image(&rgb)
        .map_err(|error| ThumbError::failed("encode", format!("JPEG encode failed: {error}")))?;

    crate::platform::create_private_dir_all(cache_path.parent().unwrap_or(cache_dir)).map_err(
        |error| ThumbError::failed("io", format!("failed to create cache dir: {error}")),
    )?;
    crate::platform::write_private_file(cache_path, &encoded)
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
    fit: Fit,
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
    let long_edge = original_width.max(original_height);
    let scale = match fit {
        Fit::Short => (width as f64 / short_edge).min(1.0),
        Fit::Long => (width as f64 / long_edge).min(1.0),
        Fit::Width => (width as f64 / original_width).min(1.0),
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

pub(crate) struct ImageHeader {
    pub width: u32,
    pub height: u32,
    pub jpeg: bool,
    pub cmyk: bool,
}

/// Reads only the image header. Used by grid-thumbnail gating and the
/// pre-open preview check; never decodes pixel data.
pub(crate) fn inspect_image(path: &Path) -> Option<ImageHeader> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let format = reader.format()?;
    if format == image::ImageFormat::Jpeg {
        let file = std::fs::File::open(path).ok()?;
        let mut decoder = jpeg_decoder::Decoder::new(file);
        decoder.read_info().ok()?;
        let info = decoder.info()?;
        return Some(ImageHeader {
            width: info.width as u32,
            height: info.height as u32,
            jpeg: true,
            cmyk: info.pixel_format == jpeg_decoder::PixelFormat::CMYK32,
        });
    }
    // Read dimensions from the reader that already sniffed the content.
    // `image::image_dimensions(path)` reopens the file and picks the decoder
    // from the extension, so a PNG downloaded as `.jpg` would fail here.
    let (width, height) = reader.into_dimensions().ok()?;
    Some(ImageHeader {
        width,
        height,
        jpeg: false,
        cmyk: false,
    })
}

/// Drops cached failure records so a raised pixel/byte limit takes effect
/// immediately instead of being blocked by an old `too_large` record.
pub(crate) async fn clear_failure_records(cache_dir: &Path) {
    let root = cache_dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(file_type) if file_type.is_dir() => stack.push(path),
                    Ok(file_type)
                        if file_type.is_file()
                            && path.extension().and_then(|value| value.to_str())
                                == Some("fail") =>
                    {
                        let _ = std::fs::remove_file(path);
                    }
                    _ => {}
                }
            }
        }
    })
    .await;
}

async fn write_failure_record(path: &Path, reason: &str, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = crate::platform::create_private_dir_all(parent);
    }
    let record = json!({
        "reason": reason,
        "message": message,
        "version": 4,
        "at": crate::db::unix_now(),
    });
    let _ = crate::platform::write_private_file(path, record.to_string().as_bytes());
}

fn settings_edge(state: &AppState) -> u32 {
    state
        .settings()
        .settings
        .thumbnails
        .preview_edge
        .clamp(512, 4096)
}

fn cache_key(path: &Path, width: u32, fit: Fit, edge: u32) -> Result<String, AppError> {
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
    hasher.update(fit.as_str().as_bytes());
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
    fit: Fit,
    edge: u32,
) -> Result<(String, PathBuf, PathBuf), AppError> {
    let key = cache_key(source, width, fit, edge)?;
    let cache_path = cache_dir
        .join(&key[0..2])
        .join(&key[2..4])
        .join(format!("{key}.jpg"));
    let failure_path = cache_path.with_extension("fail");
    Ok((key, cache_path, failure_path))
}

#[derive(Debug, Deserialize)]
pub struct ImageInfoQuery {
    root: i64,
    path: String,
}

/// Header-only preview feasibility check. The lightbox calls this before it
/// opens so an over-limit image never reaches the browser decoder.
pub async fn image_info(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ImageInfoQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
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
        let size = stat
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let settings = state.settings();
        let previewable = stat.get("is_dir").and_then(serde_json::Value::as_bool) != Some(true)
            && size <= settings.settings.thumbnails.max_source_bytes;
        return Ok(axum::Json(json!({
            "width": null,
            "height": null,
            "pixels": null,
            "limit": null,
            "previewable": previewable,
            "reason": if previewable { serde_json::Value::Null } else { json!("max_source") },
        })));
    }
    let resolved = authz::resolve(&state, current.user.id, root.id, &query.path, false).await?;
    let metadata = tokio::fs::metadata(&resolved.absolute)
        .await
        .map_err(fsops::io_error)?;
    if !metadata.is_file() {
        return Err(AppError::NotFound);
    }
    let settings = state.settings();
    let source = resolved.absolute.clone();
    let header = tokio::task::spawn_blocking(move || inspect_image(&source))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("image header task failed: {err}")))?;
    let Some(header) = header else {
        return Ok(axum::Json(json!({
            "width": null,
            "height": null,
            "pixels": null,
            "limit": null,
            "previewable": false,
            "reason": "unsupported",
        })));
    };
    let pixels = header.width as u64 * header.height as u64;
    let limit = if header.jpeg && !header.cmyk {
        settings.settings.thumbnails.jpeg_max_pixels
    } else {
        settings.settings.thumbnails.max_pixels
    };
    let (previewable, reason) = if metadata.len() > settings.settings.thumbnails.max_source_bytes {
        (false, Some("max_source"))
    } else if pixels > limit {
        (false, Some("too_large"))
    } else {
        (true, None)
    };
    Ok(axum::Json(json!({
        "width": header.width,
        "height": header.height,
        "pixels": pixels,
        "limit": limit,
        "previewable": previewable,
        "reason": reason,
    })))
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
    let fit = Fit::from_query(query.fit.as_deref());
    let edge = query
        .edge
        .unwrap_or_else(|| query.w.unwrap_or(256))
        .clamp(16, settings_edge(&state));
    let cache_dir = settings.settings.cache_dir.join("thumbs");
    let (_key, cache_path, failure_path) =
        thumb_paths(&cache_dir, &resolved.absolute, edge, fit, edge)?;
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

#[derive(Debug, Deserialize)]
pub struct ThumbCancelRequest {
    root: i64,
    path: String,
    scope: String,
}

/// Drops queued list-thumbnail jobs when the browser leaves a directory, or
/// cancels the running preview job when the lightbox closes. Copy/move/upload
/// and index jobs are never touched here.
pub async fn cancel_scope(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ThumbCancelRequest>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let root = state
        .db
        .find_root(request.root, current.user.id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let path = request
        .path
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    if path.len() > 4096 || path.contains('\0') {
        return Err(AppError::invalid_path("invalid thumbnail scope"));
    }
    let scope = match request.scope.as_str() {
        "dir" => format!("dir:{}:{path}", root.id),
        "file" if !path.is_empty() => format!("file:{}:{path}", root.id),
        "file" => return Err(AppError::invalid_path("thumbnail file path is empty")),
        _ => return Err(AppError::bad_request("scope must be dir or file")),
    };
    let include_running = request.scope == "file";
    let cancelled = state.jobs.cancel_thumbnail_scope(&scope, include_running);
    Ok(axum::Json(json!({ "cancelled": cancelled })))
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
    let fit = Fit::from_query(request.fit.as_deref());
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
            thumb_paths(&cache_dir, &resolved.absolute, width, fit, width)?;
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

static LAST_CACHE_PRUNE: AtomicU64 = AtomicU64::new(0);

/// `prune_cache` stats every cached thumbnail. Running it after every
/// generated image turns thumbnail generation quadratic on large caches.
fn maybe_prune_cache(directory: &Path, max_bytes: u64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let last = LAST_CACHE_PRUNE.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 60 {
        return;
    }
    if LAST_CACHE_PRUNE
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let _ = prune_cache(directory, max_bytes);
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
mod image_header_tests {
    use super::inspect_image;

    #[test]
    fn reads_png_dimensions_when_extension_says_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        image::RgbImage::from_pixel(8, 6, image::Rgb([10, 20, 30]))
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();

        let header = inspect_image(&path).expect("header");
        assert_eq!((header.width, header.height), (8, 6));
        assert!(!header.jpeg);
    }
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
