use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Path as AxumPath, Request, State, rejection::JsonRejection};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::{self, AuthKind};
use crate::config;
use crate::db::{ApiToken, User, unix_now};
use crate::error::AppError;
use crate::state::AppState;

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let public = Router::new()
        .route("/setup/status", get(setup_status))
        .route("/setup", post(setup));

    let protected = Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/version", get(crate::updates::version))
        .route("/admin/updates/check", post(crate::updates::admin_check))
        .route(
            "/remote/grants",
            get(crate::federation::admin::list_grants).post(crate::federation::admin::create_grant),
        )
        .route(
            "/remote/grants/{id}",
            axum::routing::delete(crate::federation::admin::revoke_grant),
        )
        .route(
            "/remote/tokens",
            get(crate::federation::admin::list_tokens).post(crate::federation::admin::create_token),
        )
        .route(
            "/remote/tokens/{id}",
            axum::routing::delete(crate::federation::admin::revoke_token)
                .patch(crate::federation::admin::update_token),
        )
        .route(
            "/remote/tokens/{id}/rotate",
            post(crate::federation::admin::rotate_token),
        )
        .route("/remote/activity", get(crate::federation::mounts::activity))
        .route(
            "/remote/thumb-warm",
            post(crate::federation::mounts::thumb_warm),
        )
        .route("/remote/probe", post(crate::federation::mounts::probe))
        .route(
            "/remote/grants/preview",
            post(crate::federation::mounts::preview),
        )
        .route(
            "/remote/mounts",
            get(crate::federation::mounts::list).post(crate::federation::mounts::create),
        )
        .route(
            "/remote/mounts/{id}",
            axum::routing::delete(crate::federation::mounts::delete),
        )
        .route(
            "/remote/mounts/{id}",
            axum::routing::patch(crate::federation::mounts::rename),
        )
        .route(
            "/remote/mounts/{id}/test",
            post(crate::federation::mounts::test),
        )
        .route(
            "/remote/mounts/{id}/reauth",
            post(crate::federation::mounts::reauth),
        )
        .route(
            "/remote/mounts/{id}/grants",
            get(crate::federation::mounts::mount_grants),
        )
        .route(
            "/remote/mounts/{id}/grants/{grant_id}",
            post(crate::federation::mounts::attach_grant),
        )
        .route("/me/password", post(change_password))
        .route("/events", get(events_stream))
        .route(
            "/me/preferences",
            get(crate::preferences::get_preferences).patch(crate::preferences::update_preferences),
        )
        .route(
            "/me/avatar",
            get(crate::preferences::get_avatar)
                .post(crate::preferences::upload_avatar)
                .delete(crate::preferences::delete_avatar),
        )
        .route("/auth/tokens", get(list_tokens).post(create_token))
        .route("/auth/tokens/{id}", delete(delete_token))
        .route("/admin/users", get(list_users).post(create_user))
        .route("/admin/users/{id}", put(update_user).delete(delete_user))
        .route("/admin/users/usage", get(crate::usage::admin_batch))
        .route("/admin/users/{id}/usage", get(user_usage))
        .route("/admin/status", get(crate::admin_fs::status))
        .route("/admin/roots", get(crate::admin_fs::list_all_roots))
        .route(
            "/admin/server-paths",
            get(crate::admin_fs::list_server_paths).post(crate::admin_fs::add_server_path),
        )
        .route(
            "/admin/server-paths/{id}",
            put(crate::admin_fs::update_server_path).delete(crate::admin_fs::delete_server_path),
        )
        .route("/admin/audit", get(crate::admin_fs::list_audit))
        .route(
            "/admin/settings",
            get(get_settings).put(put_setting).patch(patch_settings),
        )
        .route("/admin/settings/{key}", delete(delete_setting))
        .route("/admin/settings/reset", post(reset_settings))
        .route("/admin/verify-password", post(verify_admin_password))
        .route("/admin/restart", post(restart_server))
        .route(
            "/admin/branding/icon",
            post(crate::branding::upload_icon).delete(crate::branding::delete_icon),
        )
        .route(
            "/admin/custom-css",
            put(crate::branding::put_custom_css).delete(crate::branding::delete_custom_css),
        )
        .route("/roots", get(crate::files::roots))
        .route("/list", get(crate::files::list))
        .route("/fs/dir-size", get(crate::files::dir_size))
        .route("/properties", get(crate::properties::properties))
        .route("/download", get(crate::files::download))
        .route("/download/zip", post(crate::files::zip))
        .route("/media/audio-info", get(crate::media::audio_info))
        .route("/media/transcode", post(crate::media::transcode))
        .route("/media/transcoded", get(crate::media::transcoded))
        .route("/archive/zip", post(crate::files::archive_zip))
        .route("/archive/unzip", post(crate::unzip::archive_unzip))
        .route("/index", post(crate::search::index))
        .route("/index/status", get(crate::search::index_status))
        .route("/search", get(crate::search::search))
        .route("/search/all", get(crate::search::search_all))
        .route("/usage", get(crate::usage::usage))
        .route("/file/probe", get(crate::editor::probe))
        .route("/file", get(crate::editor::read).put(crate::editor::save))
        .route("/edit/presence", post(crate::presence::update))
        .route("/thumb", get(crate::thumbs::thumb))
        .route("/fs/image-info", get(crate::thumbs::image_info))
        .route("/thumb/status", get(crate::thumbs::status))
        .route("/thumb/retry", post(crate::thumbs::retry))
        .route("/thumb/cancel", post(crate::thumbs::cancel_scope))
        .route("/cmd", post(crate::cmdrunner::run))
        .route("/terminal/ws", get(crate::terminal_ws::websocket))
        .route(
            "/shares",
            get(crate::shares::list).post(crate::shares::create),
        )
        .route(
            "/shares/{id}",
            put(crate::shares::update).delete(crate::shares::revoke),
        )
        .route("/shares/{id}/purge", delete(crate::shares::purge))
        .route("/shares/{id}/url", get(crate::shares::url))
        .route("/shares/{id}/rotate", post(crate::shares::rotate))
        .route("/public/shares/{token}", get(crate::shares::public_meta))
        .route("/public/shares/{token}/unlock", post(crate::shares::unlock))
        .route(
            "/public/shares/{token}/download",
            get(crate::shares::download),
        )
        .route("/public/shares/{token}/upload", post(crate::shares::upload))
        .route("/public/shares/{token}/thumb", get(crate::shares::thumb))
        .route("/public/shares/{token}/events", get(crate::shares::events))
        .route(
            "/public/shares/{token}/uploads",
            post(crate::shares::create_upload),
        )
        .route(
            "/public/shares/{token}/uploads/{id}",
            get(crate::shares::upload_status).delete(crate::shares::cancel_upload),
        )
        .route(
            "/public/shares/{token}/uploads/{id}/chunks/{index}",
            put(crate::shares::upload_chunk),
        )
        .route(
            "/public/shares/{token}/uploads/{id}/complete",
            post(crate::shares::complete_upload),
        )
        .route("/public/shares/{token}/zip", post(crate::shares::zip))
        .route(
            "/public/shares/{token}/zip/form",
            post(crate::shares::zip_form),
        )
        .route(
            "/public/shares/{token}/zip/preflight",
            post(crate::shares::zip_preflight),
        )
        .route("/uploads", post(crate::files::create_upload))
        .route(
            "/uploads/{id}",
            get(crate::files::upload_status).delete(crate::files::cancel_upload),
        )
        .route(
            "/uploads/{id}/chunks/{index}",
            put(crate::files::upload_chunk),
        )
        .route(
            "/uploads/{id}/complete",
            post(crate::files::complete_upload),
        )
        .route("/fs/mkdir", post(crate::files::mkdir))
        .route("/fs/touch", post(crate::files::touch))
        .route("/fs/rename", post(crate::files::rename))
        .route("/fs/copy", post(crate::files::copy))
        .route("/fs/move", post(crate::files::move_items))
        .route("/fs/transfer-jobs", post(crate::files::transfer_job))
        .route(
            "/fs/transfer-jobs/{id}",
            get(crate::files::job_status).delete(crate::files::job_cancel),
        )
        .route("/jobs", get(crate::files::list_jobs))
        .route(
            "/jobs/{id}",
            get(crate::files::job_status).delete(crate::files::job_cancel),
        )
        .route("/fs/delete/preview", post(crate::files::delete_preview))
        .route("/fs/dir-version", get(crate::files::dir_version))
        .route("/fs/delete", post(crate::files::delete_confirm))
        .route(
            "/admin/users/{id}/roots",
            get(crate::admin_fs::list_roots).post(crate::admin_fs::add_root),
        )
        .route(
            "/admin/users/{id}/roots/{root_id}",
            put(crate::admin_fs::update_root).delete(crate::admin_fs::delete_root),
        )
        .route(
            "/admin/trusted-links",
            get(crate::admin_fs::list_trusted_links).post(crate::admin_fs::create_trusted_link),
        )
        .route(
            "/admin/trusted-links/{id}",
            delete(crate::admin_fs::delete_trusted_link),
        )
        .layer(middleware::from_fn_with_state(state, require_initialized));

    public
        .merge(protected)
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .merge(crate::branding::public_router())
}

async fn events_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let user_id = current.user.id;
    let admin = current.user.role == "admin";
    let receiver = state.events.subscribe();
    let stream = futures_util::stream::unfold(receiver, move |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if !event.visible_to_user(user_id, admin) {
                        continue;
                    }
                    let data = serde_json::to_string(&event.data).unwrap_or_default();
                    let out = SseEvent::default()
                        .id(event.id.to_string())
                        .event(event.event)
                        .data(data);
                    return Some((Ok::<_, Infallible>(out), receiver));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    let out = SseEvent::default()
                        .event("resync")
                        .data(json!({ "missed": missed }).to_string());
                    return Some((Ok::<_, Infallible>(out), receiver));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(response)
}

async fn require_initialized(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if state.is_initialized() {
        next.run(request).await
    } else {
        AppError::SetupRequired.into_response()
    }
}

#[derive(Debug, Deserialize)]
struct SetupRequest {
    token: String,
    username: String,
    password: String,
    language: Option<String>,
}

async fn setup_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "initialized": state.is_initialized() }))
}

async fn setup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<SetupRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let Json(request) = payload.map_err(json_rejection)?;

    if state.is_initialized() {
        return Err(AppError::conflict("setup is already complete"));
    }

    if !auth::constant_time_eq(&state.setup.token, &request.token) {
        return Err(AppError::Unauthorized);
    }

    validate_username(&request.username)?;
    validate_password(&request.password)?;
    if let Some(language) = request.language.as_deref()
        && !matches!(language, "zh-CN" | "en")
    {
        return Err(AppError::bad_request("language must be zh-CN or en"));
    }

    let password_hash = auth::hash_password_async(request.password)
        .await
        .map_err(AppError::internal)?;
    let settings = state.settings();
    let home_dir = settings
        .settings
        .users_root
        .join(&request.username)
        .to_string_lossy()
        .to_string();

    let user_id = state
        .db
        .insert_user(&request.username, &password_hash, "admin", &home_dir, None)
        .await
        .map_err(map_db_error)?;

    create_home_root(&state, user_id, &home_dir).await?;

    if let Some(language) = request.language {
        let mut preferences = state
            .db
            .get_user_preferences(user_id)
            .await
            .map_err(AppError::internal)?;
        preferences.language = language;
        state
            .db
            .save_user_preferences(user_id, &preferences)
            .await
            .map_err(AppError::internal)?;
    }

    state
        .setup
        .initialized
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let user = state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("created user is missing")))?;

    audit(
        &state,
        Some(user.id),
        "setup",
        None,
        Some("admin account created"),
        &headers,
    )
    .await;

    session_response(&state, &user, &headers).await
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn verify_password_with_limit(
    state: &Arc<AppState>,
    password: String,
    hash: String,
) -> Result<bool, AppError> {
    let permit = state
        .password_semaphore
        .acquire()
        .await
        .map_err(|_| AppError::internal(anyhow::anyhow!("password semaphore closed")))?;
    let result = auth::verify_password_async(password, hash)
        .await
        .map_err(AppError::internal);
    drop(permit);
    result
}

async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let Json(request) = payload.map_err(json_rejection)?;

    let client_ip = auth::client_ip(&headers).unwrap_or_else(|| "unknown".to_owned());
    let ip_key = format!("login-ip:{client_ip}");
    if state.ip_limiter.is_blocked(&ip_key) {
        return Err(AppError::TooManyRequests);
    }
    if state.rate_limiter.is_blocked(&request.username) {
        return Err(AppError::TooManyRequests);
    }

    let Some(user) = state
        .db
        .find_user_by_username(&request.username)
        .await
        .map_err(AppError::internal)?
    else {
        // Burn the same Argon2 cost as a real account so timing does not
        // reveal whether the username exists.
        verify_password_with_limit(
            &state,
            request.password.clone(),
            auth::DUMMY_PASSWORD_HASH.to_owned(),
        )
        .await?;
        state.rate_limiter.record_failure(&request.username);
        state.ip_limiter.record_failure(&ip_key);
        audit(
            &state,
            None,
            "login_failed",
            None,
            Some("unknown username"),
            &headers,
        )
        .await;
        return Err(AppError::Unauthorized);
    };

    if user.disabled {
        verify_password_with_limit(&state, request.password.clone(), user.password_hash.clone())
            .await?;
        state.rate_limiter.record_failure(&request.username);
        state.ip_limiter.record_failure(&ip_key);
        return Err(AppError::Unauthorized);
    }

    let valid =
        verify_password_with_limit(&state, request.password, user.password_hash.clone()).await?;

    if !valid {
        state.rate_limiter.record_failure(&request.username);
        state.ip_limiter.record_failure(&ip_key);
        audit(
            &state,
            Some(user.id),
            "login_failed",
            None,
            Some("invalid password"),
            &headers,
        )
        .await;
        return Err(AppError::Unauthorized);
    }

    state.rate_limiter.clear(&request.username);
    state
        .db
        .touch_last_login(user.id)
        .await
        .map_err(AppError::internal)?;
    audit(&state, Some(user.id), "login", None, None, &headers).await;

    session_response(&state, &user, &headers).await
}

async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;

    if current.auth_kind == AuthKind::Session
        && let Some(token) = auth::session_token(&headers)
    {
        state
            .db
            .delete_session(&auth::sha256_hex(&token))
            .await
            .map_err(AppError::internal)?;
    }

    audit(
        &state,
        Some(current.user.id),
        "logout",
        None,
        None,
        &headers,
    )
    .await;

    let secure = state.settings().settings.security.cookie_secure;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, auth::clear_session_cookie(secure)?);
    Ok(response)
}

async fn me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let auth_kind = match current.auth_kind {
        AuthKind::Session => "session",
        AuthKind::Token => "token",
    };

    Ok(Json(json!({
        "user": user_json(&current.user),
        "remote": remote_capabilities(&state, &current.user),
        "csrf_token": current.csrf_token,
        "auth": auth_kind,
    })))
}

#[derive(Debug, Deserialize)]
struct CreateTokenRequest {
    name: String,
    scope: String,
    expires_at: Option<i64>,
}

async fn list_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let tokens = state
        .db
        .list_tokens(current.user.id)
        .await
        .map_err(AppError::internal)?;

    Ok(Json(json!({
        "tokens": tokens.iter().map(token_json).collect::<Vec<_>>()
    })))
}

async fn create_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<CreateTokenRequest>, JsonRejection>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;

    let Json(request) = payload.map_err(json_rejection)?;
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::bad_request(
            "token name must be between 1 and 64 characters",
        ));
    }
    if request.scope != "read" && request.scope != "write" {
        return Err(AppError::bad_request("scope must be read or write"));
    }
    if request
        .expires_at
        .is_some_and(|expires_at| expires_at <= unix_now())
    {
        return Err(AppError::bad_request("expires_at must be in the future"));
    }

    let raw_token = format!("fh_{}", auth::random_token(32));
    let token_hash = auth::sha256_hex(&raw_token);
    let prefix: String = raw_token.chars().take(8).collect();
    let id = state
        .db
        .insert_token(
            current.user.id,
            name,
            &token_hash,
            &prefix,
            &request.scope,
            request.expires_at,
        )
        .await
        .map_err(AppError::internal)?;

    audit(
        &state,
        Some(current.user.id),
        "token_created",
        None,
        Some(name),
        &headers,
    )
    .await;

    Ok(Json(json!({
        "id": id,
        "token": raw_token,
        "name": name,
        "prefix": prefix,
        "scope": request.scope,
        "expires_at": request.expires_at,
    })))
}

async fn delete_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;

    let deleted = state
        .db
        .delete_token(id, current.user.id)
        .await
        .map_err(AppError::internal)?;

    if !deleted {
        return Err(AppError::NotFound);
    }

    audit(
        &state,
        Some(current.user.id),
        "token_deleted",
        None,
        Some(&format!("token_id={id}")),
        &headers,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    role: Option<String>,
    quota_bytes: Option<i64>,
}

async fn list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;

    let users = state.db.list_users().await.map_err(AppError::internal)?;
    let now = unix_now();
    let online: std::collections::HashSet<i64> = state
        .db
        .online_user_ids(now - 300, now)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .collect();
    let users_json: Vec<Value> = users
        .iter()
        .map(|user| {
            let mut value = user_json(user);
            if let Some(object) = value.as_object_mut() {
                object.insert("online".to_owned(), json!(online.contains(&user.id)));
            }
            value
        })
        .collect();
    Ok(Json(json!({ "users": users_json })))
}

async fn create_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<CreateUserRequest>, JsonRejection>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    let Json(request) = payload.map_err(json_rejection)?;
    validate_username(&request.username)?;
    validate_password(&request.password)?;

    let role = request.role.as_deref().unwrap_or("user");
    validate_role(role)?;
    if request.quota_bytes.is_some_and(|quota| quota < 0) {
        return Err(AppError::bad_request("quota_bytes cannot be negative"));
    }

    let password_hash = auth::hash_password_async(request.password)
        .await
        .map_err(AppError::internal)?;
    let settings = state.settings();
    let home_dir = settings
        .settings
        .users_root
        .join(&request.username)
        .to_string_lossy()
        .to_string();

    let settings_snapshot = state.settings();
    let quota_bytes = request
        .quota_bytes
        .unwrap_or(settings_snapshot.settings.default_quota_bytes);
    let id = state
        .db
        .insert_user(
            &request.username,
            &password_hash,
            role,
            &home_dir,
            Some(quota_bytes),
        )
        .await
        .map_err(map_db_error)?;

    create_home_root(&state, id, &home_dir).await?;
    let mut preferences = state
        .db
        .get_user_preferences(id)
        .await
        .map_err(AppError::internal)?;
    preferences.theme = settings_snapshot.settings.ui.default_theme.clone();
    preferences.language = settings_snapshot.settings.ui.default_locale.clone();
    preferences.show_hidden = settings_snapshot.settings.show_hidden_default;
    preferences.show_thumbnails = settings_snapshot.settings.thumbnails.enabled;
    preferences.upload_conflict = settings_snapshot.settings.upload_conflict_default.clone();
    preferences.terminal_enabled = settings_snapshot.settings.terminal_enabled;
    preferences.editor_enabled = settings_snapshot.settings.editor.enabled;
    state
        .db
        .save_user_preferences(id, &preferences)
        .await
        .map_err(AppError::internal)?;

    let user = state
        .db
        .find_user_by_id(id)
        .await
        .map_err(AppError::internal)?
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("created user is missing")))?;

    audit(
        &state,
        Some(current.user.id),
        "user_created",
        None,
        Some(&request.username),
        &headers,
    )
    .await;
    state
        .events
        .publish_admin("user.updated", json!({ "id": id, "action": "created" }));

    Ok(Json(json!({ "user": user_json(&user) })))
}

#[derive(Debug, Deserialize)]
struct UpdateUserRequest {
    password: Option<String>,
    role: Option<String>,
    quota_bytes: Option<i64>,
    clear_quota: Option<bool>,
    disabled: Option<bool>,
    feature_terminal: Option<bool>,
    feature_zip: Option<bool>,
    feature_unzip: Option<bool>,
    feature_federation_mount: Option<bool>,
    feature_federation_share: Option<bool>,
}

async fn update_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    payload: Result<Json<UpdateUserRequest>, JsonRejection>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    let Json(request) = payload.map_err(json_rejection)?;
    let target = state
        .db
        .find_user_by_id(id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;

    if request.role.is_some() {
        validate_role(request.role.as_deref().unwrap_or_default())?;
    }
    if request.quota_bytes.is_some_and(|quota| quota < 0) {
        return Err(AppError::bad_request("quota_bytes cannot be negative"));
    }
    if request.password.is_some() {
        validate_password(request.password.as_deref().unwrap_or_default())?;
    }

    let disables_target = request.disabled == Some(true);
    let demotes_target = request
        .role
        .as_deref()
        .is_some_and(|role| role != "admin" && target.role == "admin");

    if id == current.user.id && (disables_target || demotes_target) {
        return Err(AppError::bad_request(
            "you cannot disable or demote your own account",
        ));
    }

    if (disables_target || demotes_target) && target.role == "admin" {
        let admins = state
            .db
            .count_active_admins()
            .await
            .map_err(AppError::internal)?;
        if admins <= 1 {
            return Err(AppError::conflict(
                "the last admin cannot be disabled or demoted",
            ));
        }
    }

    let quota = if request.clear_quota == Some(true) {
        Some(None)
    } else if request.quota_bytes.is_some() {
        Some(request.quota_bytes)
    } else {
        None
    };

    state
        .db
        .update_user(id, request.role.as_deref(), quota, request.disabled)
        .await
        .map_err(AppError::internal)?;
    state.invalidate_usage(id);

    let features_changed = request.feature_terminal.is_some()
        || request.feature_zip.is_some()
        || request.feature_unzip.is_some()
        || request.feature_federation_mount.is_some()
        || request.feature_federation_share.is_some();
    if features_changed {
        state
            .db
            .update_user_features(
                id,
                request.feature_terminal,
                request.feature_zip,
                request.feature_unzip,
                request.feature_federation_mount,
                request.feature_federation_share,
            )
            .await
            .map_err(AppError::internal)?;
        state
            .events
            .publish("user.features_changed", Some(id), json!({ "user_id": id }));
        audit(
            &state,
            Some(current.user.id),
            "user_features_changed",
            None,
            Some(&format!("user_id={id}")),
            &headers,
        )
        .await;
    }

    if let Some(password) = request.password {
        let password_hash = auth::hash_password_async(password)
            .await
            .map_err(AppError::internal)?;
        state
            .db
            .update_user_password(id, &password_hash)
            .await
            .map_err(AppError::internal)?;
    }

    let user = state
        .db
        .find_user_by_id(id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;

    audit(
        &state,
        Some(current.user.id),
        "user_updated",
        None,
        Some(&format!("user_id={id}")),
        &headers,
    )
    .await;
    state
        .events
        .publish_admin("user.updated", json!({ "id": id, "action": "updated" }));

    Ok(Json(json!({ "user": user_json(&user) })))
}

async fn user_usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    let user = state
        .db
        .find_user_by_id(id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;
    let ttl = state.settings().settings.jobs.usage_cache_seconds as i64;
    if let Some(snapshot) = state.usage_cache.lock().get(&user.id).cloned() {
        let stale = unix_now().saturating_sub(snapshot.computed_at) > ttl.max(1);
        if stale {
            crate::usage::spawn_refresh(Arc::clone(&state), user.clone());
        }
        return Ok(Json(json!({
            "used_bytes": snapshot.used_bytes,
            "quota_bytes": snapshot.quota_bytes,
            "computed_at": snapshot.computed_at,
            "computing": stale,
        })));
    }
    crate::usage::spawn_refresh(Arc::clone(&state), user.clone());
    Ok(Json(json!({
        "used_bytes": null,
        "quota_bytes": user.quota_bytes,
        "computed_at": null,
        "computing": true,
    })))
}

async fn delete_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    if id == current.user.id {
        return Err(AppError::bad_request("you cannot disable your own account"));
    }

    let target = state
        .db
        .find_user_by_id(id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)?;

    if target.role == "admin" {
        let admins = state
            .db
            .count_active_admins()
            .await
            .map_err(AppError::internal)?;
        if admins <= 1 {
            return Err(AppError::conflict("the last admin cannot be disabled"));
        }
    }

    if !state
        .db
        .soft_delete_user(id)
        .await
        .map_err(AppError::internal)?
    {
        return Err(AppError::NotFound);
    }

    audit(
        &state,
        Some(current.user.id),
        "user_deleted",
        None,
        Some(&format!("user_id={id}")),
        &headers,
    )
    .await;
    state
        .events
        .publish_admin("user.updated", json!({ "id": id, "action": "deleted" }));

    Ok(StatusCode::NO_CONTENT)
}

async fn get_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    Ok(Json(settings_json(&state)))
}

#[derive(Debug, Deserialize)]
struct PutSettingRequest {
    key: String,
    value: Value,
}

fn notify_settings_changed(state: &Arc<AppState>, keys: &[String]) {
    state.events.publish_all(
        "settings.changed",
        json!({ "keys": keys, "at": crate::db::unix_now() }),
    );
}

fn request_is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-filehub-scheme")
        .and_then(|value| value.to_str().ok())
        == Some("https")
}

fn cookie_secure_blocked(headers: &HeaderMap, value: &Value) -> bool {
    value.as_bool() == Some(true) && !request_is_https(headers)
}

async fn put_setting(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<PutSettingRequest>, JsonRejection>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    let Json(request) = payload.map_err(json_rejection)?;
    if !config::is_known_key(&request.key) {
        return Err(AppError::bad_request("unknown settings key"));
    }
    if config::is_bootstrap_key(&request.key) {
        return Err(AppError::bad_request(
            "this setting is bootstrap-only and cannot be changed from the database",
        ));
    }
    if request.key == "security.cookie_secure" && cookie_secure_blocked(&headers, &request.value) {
        return Err(AppError::bad_request(
            "security.cookie_secure requires HTTPS; enable it only behind TLS",
        ));
    }

    let encoded = serde_json::to_string(&request.value)
        .map_err(|err| AppError::bad_request(format!("invalid value: {err}")))?;

    let mut overrides = state
        .db
        .settings_overrides()
        .await
        .map_err(AppError::internal)?;
    if let Some(existing) = overrides.iter_mut().find(|(key, _)| key == &request.key) {
        existing.1 = encoded.clone();
    } else {
        overrides.push((request.key.clone(), encoded.clone()));
    }

    let ensure_paths = is_path_setting(&request.key);
    let candidate = config::build_snapshot(&state.config, &overrides)
        .map_err(|err| AppError::bad_request(err.to_string()))?;
    if ensure_paths {
        config::ensure_directories(&candidate)
            .map_err(|err| AppError::bad_request(err.to_string()))?;
    }

    let started = Instant::now();
    state
        .db
        .upsert_setting(&request.key, &encoded)
        .await
        .map_err(AppError::internal)?;
    state
        .reload_settings(ensure_paths)
        .await
        .map_err(AppError::internal)?;
    clear_dir_size_cache_if_needed(&state, &request.key).await;
    if started.elapsed().as_millis() >= 500 {
        tracing::warn!(
            key = %request.key,
            duration_ms = started.elapsed().as_millis() as u64,
            "slow settings write"
        );
    }

    audit(
        &state,
        Some(current.user.id),
        "settings_updated",
        None,
        Some(&request.key),
        &headers,
    )
    .await;
    notify_settings_changed(&state, std::slice::from_ref(&request.key));

    Ok(Json(settings_json(&state)))
}

#[derive(Debug, Deserialize)]
struct PatchSettingsRequest {
    values: serde_json::Map<String, Value>,
}

async fn patch_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<PatchSettingsRequest>, JsonRejection>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let Json(request) = payload.map_err(json_rejection)?;
    if request.values.is_empty() {
        return Err(AppError::bad_request("values cannot be empty"));
    }
    if request.values.len() > 32 {
        return Err(AppError::bad_request("at most 32 settings per request"));
    }
    if let Some(value) = request.values.get("security.cookie_secure")
        && cookie_secure_blocked(&headers, value)
    {
        return Err(AppError::bad_request(
            "security.cookie_secure requires HTTPS; enable it only behind TLS",
        ));
    }

    let mut encoded_values = Vec::with_capacity(request.values.len());
    let mut ensure_paths = false;
    for (key, value) in &request.values {
        if !config::is_known_key(key) {
            return Err(AppError::bad_request(format!(
                "unknown settings key: {key}"
            )));
        }
        if config::is_bootstrap_key(key) {
            return Err(AppError::bad_request(
                "this setting is bootstrap-only and cannot be changed from the database",
            ));
        }
        ensure_paths |= is_path_setting(key);
        let encoded = serde_json::to_string(value)
            .map_err(|err| AppError::bad_request(format!("invalid value: {err}")))?;
        encoded_values.push((key.clone(), encoded));
    }

    let mut overrides = state
        .db
        .settings_overrides()
        .await
        .map_err(AppError::internal)?;
    for (key, encoded) in &encoded_values {
        if let Some(existing) = overrides.iter_mut().find(|(name, _)| name == key) {
            existing.1 = encoded.clone();
        } else {
            overrides.push((key.clone(), encoded.clone()));
        }
    }

    let candidate = config::build_snapshot(&state.config, &overrides)
        .map_err(|err| AppError::bad_request(err.to_string()))?;
    if ensure_paths {
        config::ensure_directories(&candidate)
            .map_err(|err| AppError::bad_request(err.to_string()))?;
    }

    let started = Instant::now();
    state
        .db
        .upsert_settings_batch(encoded_values)
        .await
        .map_err(AppError::internal)?;
    state
        .reload_settings(ensure_paths)
        .await
        .map_err(AppError::internal)?;

    let keys: Vec<String> = request.values.keys().cloned().collect();
    if keys.iter().any(|key| key == "jobs.dir_size_use_logical") {
        let cleanup = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                match cleanup.db.clear_directory_sizes_batch(500).await {
                    Ok(0) => break,
                    Ok(_) => tokio::time::sleep(Duration::from_millis(5)).await,
                    Err(error) => {
                        tracing::warn!(%error, "failed to clear directory size cache");
                        break;
                    }
                }
            }
        });
    }
    if keys.iter().any(|key| {
        matches!(
            key.as_str(),
            "thumbnails.jpeg_max_pixels" | "thumbnails.max_pixels" | "thumbnails.max_source_bytes"
        )
    }) {
        let cache_dir = state.settings().settings.cache_dir.join("thumbs");
        tokio::spawn(async move {
            crate::thumbs::clear_failure_records(&cache_dir).await;
        });
    }
    if started.elapsed().as_millis() >= 500 {
        tracing::warn!(
            count = keys.len(),
            duration_ms = started.elapsed().as_millis() as u64,
            "slow settings batch write"
        );
    }

    audit(
        &state,
        Some(current.user.id),
        "settings_updated",
        None,
        Some(&keys.join(",")),
        &headers,
    )
    .await;
    notify_settings_changed(&state, &keys);

    Ok(Json(settings_json(&state)))
}

#[derive(Debug, Deserialize)]
struct ResetSettingsRequest {
    keys: Vec<String>,
}

async fn reset_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ResetSettingsRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    if request.keys.is_empty() || request.keys.len() > 200 {
        return Err(AppError::bad_request("keys must contain 1-200 items"));
    }
    for key in &request.keys {
        if !config::is_known_key(key) || config::is_bootstrap_key(key) {
            return Err(AppError::bad_request("unknown or bootstrap settings key"));
        }
    }
    for key in &request.keys {
        state
            .db
            .delete_setting(key)
            .await
            .map_err(AppError::internal)?;
    }
    state
        .reload_settings(false)
        .await
        .map_err(AppError::internal)?;
    audit(
        &state,
        Some(current.user.id),
        "settings_reset",
        None,
        Some(&request.keys.join(",")),
        &headers,
    )
    .await;
    notify_settings_changed(&state, &request.keys);
    Ok(Json(json!({ "reset": request.keys.len() })))
}

#[derive(Debug, Deserialize)]
struct VerifyPasswordRequest {
    password: String,
}

async fn verify_admin_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<VerifyPasswordRequest>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    let valid = auth::verify_password_async(request.password, current.user.password_hash.clone())
        .await
        .map_err(AppError::internal)?;
    if valid {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::Forbidden)
    }
}

async fn restart_server(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;
    audit(
        &state,
        Some(current.user.id),
        "server_restart",
        None,
        None,
        &headers,
    )
    .await;
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        // Exit non-zero so systemd's Restart=on-failure brings the service back.
        std::process::exit(1);
    });
    Ok(Json(json!({ "restarting": true })))
}

async fn delete_setting(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(key): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_admin(&current)?;
    auth::require_csrf(&current, &headers)?;

    if !config::is_known_key(&key) {
        return Err(AppError::NotFound);
    }

    let ensure_paths = is_path_setting(&key);
    state
        .db
        .delete_setting(&key)
        .await
        .map_err(AppError::internal)?;
    state
        .reload_settings(ensure_paths)
        .await
        .map_err(AppError::internal)?;
    clear_dir_size_cache_if_needed(&state, &key).await;

    audit(
        &state,
        Some(current.user.id),
        "settings_reset",
        None,
        Some(&key),
        &headers,
    )
    .await;
    notify_settings_changed(&state, std::slice::from_ref(&key));

    Ok(Json(settings_json(&state)))
}

fn is_path_setting(key: &str) -> bool {
    matches!(key, "db_path" | "users_root" | "cache_dir")
}

async fn clear_dir_size_cache_if_needed(state: &Arc<AppState>, key: &str) {
    if key == "jobs.dir_size_use_logical" {
        let cleanup = Arc::clone(state);
        tokio::spawn(async move {
            loop {
                match cleanup.db.clear_directory_sizes_batch(500).await {
                    Ok(0) => break,
                    Ok(_) => tokio::time::sleep(Duration::from_millis(5)).await,
                    Err(error) => {
                        tracing::warn!(%error, "failed to clear directory size cache");
                        break;
                    }
                }
            }
        });
    }
    if matches!(
        key,
        "thumbnails.jpeg_max_pixels" | "thumbnails.max_pixels" | "thumbnails.max_source_bytes"
    ) {
        let cache_dir = state.settings().settings.cache_dir.join("thumbs");
        tokio::spawn(async move {
            crate::thumbs::clear_failure_records(&cache_dir).await;
        });
    }
}

async fn session_response(
    state: &Arc<AppState>,
    user: &User,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    let (token, csrf_token, expires_at) = auth::create_session(state, user, headers).await?;
    let secure = state.settings().settings.security.cookie_secure;
    let max_age = (expires_at - unix_now()).max(0);

    let mut response = Json(json!({
        "user": user_json(user),
        "remote": remote_capabilities(state, user),
        "csrf_token": csrf_token,
        "auth": "session",
    }))
    .into_response();

    response.headers_mut().insert(
        header::SET_COOKIE,
        auth::session_cookie(&token, max_age, secure)?,
    );
    Ok(response)
}

fn settings_json(state: &Arc<AppState>) -> Value {
    let snapshot = state.settings();
    json!({
        "settings": snapshot.effective,
        "sources": snapshot.sources,
        "restart_required": snapshot.restart_required,
        "platform": std::env::consts::OS,
    })
}

fn remote_capabilities(state: &Arc<AppState>, user: &User) -> Value {
    let settings = state.settings();
    json!({
        "mount": settings.settings.federation.enabled_outbound && user.feature_federation_mount,
        "share": settings.settings.federation.enabled_inbound && user.feature_federation_share,
    })
}

fn user_json(user: &User) -> Value {
    json!({
        "id": user.id,
        "username": user.username,
        "role": user.role,
        "quota_bytes": user.quota_bytes,
        "disabled": user.disabled,
        "features": {
            "terminal": user.feature_terminal,
            "zip": user.feature_zip,
            "unzip": user.feature_unzip,
            "federation_mount": user.feature_federation_mount,
            "federation_share": user.feature_federation_share,
        },
        "created_at": user.created_at,
        "updated_at": user.updated_at,
        "last_login_at": user.last_login_at,
    })
}

fn token_json(token: &ApiToken) -> Value {
    json!({
        "id": token.id,
        "name": token.name,
        "prefix": token.prefix,
        "scope": token.scope,
        "last_used_at": token.last_used_at,
        "expires_at": token.expires_at,
        "created_at": token.created_at,
    })
}

fn validate_username(username: &str) -> Result<(), AppError> {
    if !(3..=32).contains(&username.chars().count()) {
        return Err(AppError::bad_request(
            "username must be between 3 and 32 characters",
        ));
    }

    if !username
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err(AppError::bad_request(
            "username may only contain letters, numbers, dot, underscore, and hyphen",
        ));
    }

    Ok(())
}

fn validate_password(password: &str) -> Result<(), AppError> {
    if password.is_empty() {
        return Err(AppError::bad_request("password cannot be empty"));
    }
    if password.chars().count() > 1024 {
        return Err(AppError::bad_request("password is too long"));
    }
    Ok(())
}

fn validate_role(role: &str) -> Result<(), AppError> {
    if role == "admin" || role == "user" {
        Ok(())
    } else {
        Err(AppError::bad_request("role must be admin or user"))
    }
}

#[derive(Debug, Deserialize)]
struct PasswordChangeRequest {
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<PasswordChangeRequest>, JsonRejection>,
) -> Result<StatusCode, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let Json(request) = payload.map_err(json_rejection)?;
    validate_password(&request.new_password)?;

    let valid =
        auth::verify_password_async(request.current_password, current.user.password_hash.clone())
            .await
            .map_err(AppError::internal)?;
    if !valid {
        return Err(AppError::Unauthorized);
    }

    let hash = auth::hash_password_async(request.new_password)
        .await
        .map_err(AppError::internal)?;
    state
        .db
        .update_user_password(current.user.id, &hash)
        .await
        .map_err(AppError::internal)?;
    audit(
        &state,
        Some(current.user.id),
        "password_changed",
        None,
        None,
        &headers,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn json_rejection(rejection: JsonRejection) -> AppError {
    AppError::bad_request(rejection.body_text())
}

async fn create_home_root(
    state: &Arc<AppState>,
    user_id: i64,
    home_dir: &str,
) -> Result<(), AppError> {
    let home = std::path::PathBuf::from(home_dir);
    let canonical = tokio::task::spawn_blocking(move || crate::fsops::ensure_directory(&home))
        .await
        .map_err(|err| {
            AppError::internal(anyhow::anyhow!("home directory task failed: {err}"))
        })??;
    let real_path = canonical
        .to_str()
        .ok_or_else(|| AppError::internal(anyhow::anyhow!("home directory is not valid UTF-8")))?;

    state
        .db
        .ensure_root(user_id, real_path, "My Files", false)
        .await
        .map_err(AppError::internal)?;
    Ok(())
}

fn map_db_error(err: anyhow::Error) -> AppError {
    let detail = format!("{err:#}");
    if detail.contains("UNIQUE constraint failed") {
        AppError::conflict("a record with that unique value already exists")
    } else {
        AppError::internal(err)
    }
}

async fn audit(
    state: &Arc<AppState>,
    user_id: Option<i64>,
    action: &str,
    path: Option<&str>,
    detail: Option<&str>,
    headers: &HeaderMap,
) {
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id,
            action: action.to_owned(),
            path: path.map(str::to_owned),
            detail: detail.map(str::to_owned),
            ip: auth::client_ip(headers),
        })
        .await;
}
