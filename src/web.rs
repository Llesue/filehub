use std::path::Path;

use axum::body::Body;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::{EmbeddedFile, RustEmbed};

use crate::error::AppError;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Assets;

pub async fn serve_static(uri: Uri) -> Response {
    let raw = uri.path().trim_start_matches('/');

    if raw.starts_with("api/") {
        return AppError::NotFound.into_response();
    }

    if raw.contains("..") || raw.contains('\\') {
        return AppError::NotFound.into_response();
    }

    if raw.is_empty() {
        return index_response();
    }

    if let Some(file) = Assets::get(raw) {
        return asset_response(raw, file);
    }

    if Path::new(raw).extension().is_some() {
        return AppError::NotFound.into_response();
    }

    index_response()
}

fn index_response() -> Response {
    match Assets::get("index.html") {
        Some(file) => asset_response("index.html", file),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "frontend not built; run `make web-build`",
        )
            .into_response(),
    }
}

fn asset_response(path: &str, file: EmbeddedFile) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let content_type = if path.ends_with(".webmanifest") {
        "application/manifest+json"
    } else {
        mime.as_ref()
    };
    let cache_control = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };

    let mut response = Response::new(Body::from(file.data.into_owned()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(cache_control),
    );
    response
}
