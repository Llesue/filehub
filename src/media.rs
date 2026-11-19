use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::auth;
use crate::authz;
use crate::error::AppError;
use crate::jobs::{JobContext, JobOutcome};
use crate::state::AppState;

const SUPPORTED: &[&str] = &[
    "aac",
    "mp3",
    "opus",
    "vorbis",
    "flac",
    "pcm_s16le",
    "pcm_s24le",
    "pcm_f32le",
];

fn info_cache() -> &'static Mutex<HashMap<String, (i64, Value)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (i64, Value)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn source_identity(path: &Path) -> Option<(i64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    Some((mtime, metadata.len()))
}

fn ffprobe_path(ffmpeg: &Path) -> PathBuf {
    let name = ffmpeg
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("ffmpeg");
    let probe = name.replace("ffmpeg", "ffprobe");
    ffmpeg.with_file_name(probe)
}

async fn probe_tracks(ffmpeg: &Path, source: &Path) -> Result<Vec<Value>, AppError> {
    let source = source.to_path_buf();
    let ffprobe = ffprobe_path(ffmpeg);
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "a",
                "-show_entries",
                "stream=index,codec_name,channels,channel_layout:stream_tags=language:stream_disposition=default",
                "-of",
                "json",
            ])
            .arg(&source)
            .output()
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("ffprobe task failed: {err}")))?
    .map_err(|err| AppError::UnsupportedMedia(format!("ffprobe is unavailable: {err}")))?;
    if !output.status.success() {
        return Err(AppError::UnsupportedMedia(
            "ffprobe could not read this file".to_owned(),
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| AppError::UnsupportedMedia("invalid ffprobe output".to_owned()))?;
    let streams = value
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(streams
        .into_iter()
        .enumerate()
        .map(|(position, stream)| {
            let codec = stream
                .get("codec_name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            let channels = stream.get("channels").and_then(Value::as_i64).unwrap_or(0);
            let layout = stream
                .get("channel_layout")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let language = stream
                .get("tags")
                .and_then(|tags| tags.get("language"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let default = stream
                .get("disposition")
                .and_then(|disposition| disposition.get("default"))
                .and_then(Value::as_i64)
                .unwrap_or(0)
                != 0;
            json!({
                "index": position,
                "codec": codec,
                "channels": channels,
                "layout": layout,
                "language": language,
                "default": default,
                "supported": SUPPORTED.contains(&codec.as_str()),
            })
        })
        .collect())
}

pub async fn audio_info(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<FileQuery>,
) -> Result<Json<Value>, AppError> {
    let Some(ffmpeg) = state.ffmpeg.clone() else {
        return Ok(Json(json!({
            "ffmpeg": false,
            "tracks": [],
            "default_track": null,
            "has_supported_track": true,
        })));
    };
    let current = auth::current_user(&state, &headers).await?;
    let resolved = authz::resolve(&state, current.user.id, query.root, &query.path, false).await?;
    let identity = source_identity(&resolved.absolute);
    let key = identity
        .map(|(mtime, size)| format!("{}:{}:{}", resolved.absolute.to_string_lossy(), mtime, size));
    if let Some(key) = key.as_deref()
        && let Some((cached_at, value)) = info_cache()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(key)
            .cloned()
        && crate::db::unix_now().saturating_sub(cached_at) < 3600
    {
        return Ok(Json(value));
    }
    let tracks = probe_tracks(&ffmpeg, &resolved.absolute).await?;
    let default_track = tracks
        .iter()
        .position(|track| track.get("default").and_then(Value::as_bool) == Some(true))
        .or(if tracks.is_empty() { None } else { Some(0) });
    let has_supported_track = tracks
        .iter()
        .any(|track| track.get("supported").and_then(Value::as_bool) == Some(true));
    let value = json!({
        "ffmpeg": true,
        "tracks": tracks,
        "default_track": default_track,
        "has_supported_track": has_supported_track,
    });
    if let Some(key) = key {
        let mut cache = info_cache()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(key, (crate::db::unix_now(), value.clone()));
    }
    Ok(Json(value))
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    root: i64,
    path: String,
}

#[derive(Debug, Deserialize)]
pub struct TranscodeRequest {
    root: i64,
    path: String,
    track: Option<usize>,
}

fn cache_key(source: &Path, track: usize) -> Option<String> {
    let (mtime, size) = source_identity(source)?;
    Some(auth::sha256_hex(&format!(
        "{}:{}:{}:{}:audio-v1",
        source.to_string_lossy(),
        mtime,
        size,
        track
    )))
}

fn media_cache_dir(state: &Arc<AppState>) -> PathBuf {
    state.settings().settings.cache_dir.join("media-audio")
}

pub async fn transcode(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TranscodeRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    if state.ffmpeg.is_none() {
        return Err(AppError::bad_request("ffmpeg is not available"));
    }
    let resolved =
        authz::resolve(&state, current.user.id, request.root, &request.path, false).await?;
    let track = request.track.unwrap_or(0);
    let key = cache_key(&resolved.absolute, track)
        .ok_or_else(|| AppError::bad_request("could not read the source file"))?;
    let target = media_cache_dir(&state).join(format!("{key}.mp4"));
    if target.is_file() {
        return Ok(Json(json!({
            "job_id": null,
            "ready": true,
            "path": request.path,
            "track": track,
        })));
    }
    let payload = json!({
        "root": request.root,
        "path": request.path,
        "track": track,
        "name": request.path,
    });
    let user_id = current.user.id;
    let job_state = Arc::clone(&state);
    let job_payload = payload.clone();
    let outcome = state
        .jobs
        .submit(
            crate::jobs::JobKind::MediaTranscode,
            key,
            user_id,
            payload,
            1,
            false,
            move |ctx| async move { run_transcode_job(job_state, user_id, job_payload, ctx).await },
        )
        .await?;
    Ok(Json(json!({
        "job_id": outcome.job.id,
        "ready": false,
        "path": request.path,
        "track": track,
    })))
}

pub async fn transcoded(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TranscodeQuery>,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let resolved = authz::resolve(&state, current.user.id, query.root, &query.path, false).await?;
    let key = cache_key(&resolved.absolute, query.track)
        .ok_or_else(|| AppError::bad_request("could not read the source file"))?;
    let target = media_cache_dir(&state).join(format!("{key}.mp4"));
    if !target.is_file() {
        return Err(AppError::NotFound);
    }
    crate::files::serve_file(&headers, &target, true).await
}

#[derive(Debug, Deserialize)]
pub struct TranscodeQuery {
    root: i64,
    path: String,
    #[serde(default)]
    track: usize,
}

async fn run_transcode_job(
    state: Arc<AppState>,
    user_id: i64,
    payload: Value,
    ctx: JobContext,
) -> Result<JobOutcome, AppError> {
    let ffmpeg = state
        .ffmpeg
        .clone()
        .ok_or_else(|| AppError::bad_request("ffmpeg is not available"))?;
    let root_id = payload
        .get("root")
        .and_then(Value::as_i64)
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("transcode payload missing root")))?;
    let path = payload
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let track = payload.get("track").and_then(Value::as_u64).unwrap_or(0) as usize;
    let root = crate::cmdrunner::load_root(&state, user_id, root_id).await?;
    let source = authz::resolve(&state, user_id, root.id, &path, false).await?;
    let key = cache_key(&source.absolute, track)
        .ok_or_else(|| AppError::bad_request("could not read the source file"))?;
    let dir = media_cache_dir(&state);
    crate::platform::create_private_dir_all(&dir).map_err(crate::fsops::io_error)?;
    let target = dir.join(format!("{key}.mp4"));
    if target.is_file() {
        return Ok(JobOutcome {
            result: json!({ "ready": true, "path": path }),
            progress: 1,
            total: 1,
            bytes_done: 0,
            bytes_total: 0,
        });
    }
    let temp = dir.join(format!(".{key}.part"));
    let _ = std::fs::remove_file(&temp);
    let source_arg = source.absolute.to_string_lossy().to_string();
    let audio_map = format!("0:a:{track}");
    let temp_arg = temp.to_string_lossy().to_string();
    let mut command = tokio::process::Command::new(&ffmpeg);
    command
        .args(["-nostdin", "-y", "-i"])
        .arg(&source_arg)
        .args(["-map", "0:v:0", "-map"])
        .arg(&audio_map)
        .args([
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ac",
            "2",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(&temp_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| AppError::UnsupportedMedia(format!("ffmpeg could not start: {err}")))?;
    let pid = child.id();
    let mut stderr = BufReader::new(
        child
            .stderr
            .take()
            .unwrap_or_else(|| unreachable!("stderr is piped")),
    );
    let mut tail = String::new();
    let mut line = String::new();
    let cancel = ctx.token();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                if let Some(pid) = pid
                    && let Some(pid) = rustix::process::Pid::from_raw(pid as i32)
                {
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
                }
                let _ = std::fs::remove_file(&temp);
                return Err(AppError::Cancelled);
            }
            result = child.wait() => {
                let status = result.map_err(|err| AppError::internal(anyhow::anyhow!("ffmpeg wait failed: {err}")))?;
                if !status.success() {
                    let _ = std::fs::remove_file(&temp);
                    let message = if tail.trim().is_empty() {
                        "ffmpeg failed to transcode this file".to_owned()
                    } else {
                        format!("ffmpeg failed: {}", tail.trim())
                    };
                    return Err(AppError::UnsupportedMedia(message));
                }
                break;
            }
            read = stderr.read_line(&mut line) => {
                let _ = read;
                if !line.trim().is_empty() {
                    tail = line.clone();
                    if tail.len() > 400 {
                        tail = tail[tail.len() - 400..].to_owned();
                    }
                }
                line.clear();
            }
        }
    }
    std::fs::rename(&temp, &target).map_err(crate::fsops::io_error)?;
    let size = std::fs::metadata(&target)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    Ok(JobOutcome {
        result: json!({
            "ready": true,
            "path": path,
            "size": size,
            "track": track,
        }),
        progress: 1,
        total: 1,
        bytes_done: size,
        bytes_total: size,
    })
}

/// Drop the oldest cached transcodes once the cache exceeds `max_bytes`.
pub fn prune_cache(dir: &Path, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    let total: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    let mut current = total;
    for (_, size, path) in files {
        if current <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            current = current.saturating_sub(size);
        }
        if current <= max_bytes.saturating_sub(max_bytes / 10) {
            break;
        }
    }
}

pub async fn warm_cache(state: &Arc<AppState>) {
    let dir = media_cache_dir(state);
    let max = state.settings().settings.media.audio_cache_max_bytes;
    let _ = tokio::task::spawn_blocking(move || prune_cache(&dir, max)).await;
}

#[allow(dead_code)]
fn _assert_send() {
    fn assert_send<T: Send>() {}
    assert_send::<AppState>();
}
