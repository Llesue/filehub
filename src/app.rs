use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::state::{AppState, SlowRequest};
use crate::web;

pub fn router(state: Arc<AppState>) -> Router {
    let request_id_header = HeaderName::from_static("x-request-id");
    let csp = HeaderValue::from_static(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    );

    Router::new()
        .route("/healthz", get(healthz))
        .nest("/api/v1", crate::api::router(state.clone()))
        .fallback(web::serve_static)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            resolve_client_ip,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), slow_request))
        .layer(CatchPanicLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            csp,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .with_state(state)
}

async fn healthz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut response = axum::Json(json!({
        "status": "ok",
        "service": "filehub",
        "version": env!("CARGO_PKG_VERSION"),
        "jobs": state.jobs.summary(),
        "active_jobs": state.jobs.active_count(),
    }))
    .into_response();

    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn resolve_client_ip(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let header = HeaderName::from_static(crate::auth::CLIENT_IP_HEADER);
    let scheme_header = HeaderName::from_static("x-filehub-scheme");
    // Never trust client-supplied values for the internal headers.
    request.headers_mut().remove(&header);
    request.headers_mut().remove(&scheme_header);
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    if let Some(peer) = peer {
        let trusted = &state.settings().settings.security.trusted_proxies;
        let peer_ip = crate::auth::normalize_ip(peer.ip());
        let from_trusted_proxy = trusted
            .iter()
            .any(|rule| crate::auth::ip_matches_rule(peer_ip, rule));
        if let Some(ip) = crate::auth::resolve_client_ip(peer, request.headers(), trusted)
            && let Ok(value) = HeaderValue::from_str(&ip)
        {
            request.headers_mut().insert(header, value);
        }
        let forwarded = request
            .headers()
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .unwrap_or("http");
        let scheme = if from_trusted_proxy {
            forwarded
        } else {
            request.uri().scheme_str().unwrap_or("http")
        };
        if let Ok(value) = HeaderValue::from_str(scheme) {
            request.headers_mut().insert(scheme_header, value);
        }
    }
    next.run(request).await
}

async fn slow_request(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let started = Instant::now();
    let response = next.run(request).await;
    let duration_ms = started.elapsed().as_millis();
    if duration_ms >= 1_000 {
        let status = response.status().as_u16();
        if duration_ms >= 10_000 {
            tracing::error!(
                method = %method,
                path = %path,
                status,
                duration_ms,
                request_id = request_id.as_deref().unwrap_or(""),
                "very slow request"
            );
        } else {
            tracing::warn!(
                method = %method,
                path = %path,
                status,
                duration_ms,
                request_id = request_id.as_deref().unwrap_or(""),
                "slow request"
            );
        }
        state.events.publish_admin(
            "slow_request",
            json!({
                "method": method,
                "path": path,
                "status": status,
                "duration_ms": duration_ms,
            }),
        );
        state.push_slow_request(SlowRequest {
            method,
            path,
            status,
            duration_ms,
            request_id,
            at: crate::db::unix_now(),
        });
    }
    response
}

pub async fn run(state: Arc<AppState>) -> anyhow::Result<()> {
    run_until(state, shutdown_signal()).await
}

pub async fn run_until(
    state: Arc<AppState>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let settings = state.settings();
    let listen = settings.settings.listen;
    let listener = TcpListener::bind(listen)
        .await
        .with_context(|| format!("failed to bind {listen}"))?;

    tracing::info!(
        addr = %listen,
        version = env!("CARGO_PKG_VERSION"),
        "filehub starting"
    );
    state.start_background_tasks();
    let serve_state = Arc::clone(&state);
    axum::serve(
        listener,
        router(serve_state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .context("http server failed")?;
    state.flush_audit().await;
    match state.db.checkpoint().await {
        Ok(true) => tracing::warn!("shutdown WAL checkpoint was busy"),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "shutdown WAL checkpoint failed"),
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %err, "failed to install Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => tracing::warn!(error = %err, "failed to install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
