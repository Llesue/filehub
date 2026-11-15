use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;
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

async fn healthz() -> impl IntoResponse {
    let mut response = axum::Json(json!({
        "status": "ok",
        "service": "filehub",
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response();

    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
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

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
        .context("http server failed")?;

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
