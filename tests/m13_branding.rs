use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    router: Router,
    cookie: String,
    csrf: String,
}

async fn setup_app() -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let state = filehub::state::AppState::new_test(dir.path(), "test-token")
        .await
        .unwrap();
    let router = filehub::app::router(state);
    let setup = send_json(
        &router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = session_cookie(&setup);
    let body = json_body(setup).await;
    let csrf = body["csrf_token"].as_str().unwrap().to_owned();
    TestApp {
        _dir: dir,
        router,
        cookie,
        csrf,
    }
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some(value) = csrf {
        builder = builder.header("x-csrf-token", value);
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn send_json(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    body: Value,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some(value) = csrf {
        builder = builder.header("x-csrf-token", value);
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn send_bytes(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    content_type: &str,
    body: Vec<u8>,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some(value) = csrf {
        builder = builder.header("x-csrf-token", value);
    }
    app.clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

fn session_cookie(response: &Response<Body>) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn json_body(response: Response<Body>) -> Value {
    let body = to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbImage::from_fn(64, 64, |x, y| {
        image::Rgb([(x * 4) as u8, (y * 4) as u8, 200])
    })
    .write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .unwrap();
    bytes
}

#[tokio::test]
async fn branding_name_and_icon_lifecycle() {
    let app = setup_app().await;

    let response = send(&app.router, Method::GET, "/api/v1/branding", None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["name"], "FileHub");
    assert_eq!(body["version"], 0);
    assert!(body["icon_url"].is_null());

    let rename = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "ui.brand_name", "value": "Acme Files"}),
    )
    .await;
    assert_eq!(rename.status(), StatusCode::OK);
    let body =
        json_body(send(&app.router, Method::GET, "/api/v1/branding", None, None).await).await;
    assert_eq!(body["name"], "Acme Files");

    let invalid = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "ui.brand_name", "value": "   "}),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>".to_vec();
    let rejected = send_bytes(
        &app.router,
        Method::POST,
        "/api/v1/admin/branding/icon",
        Some(&app.cookie),
        Some(&app.csrf),
        "image/svg+xml",
        svg,
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let upload = send_bytes(
        &app.router,
        Method::POST,
        "/api/v1/admin/branding/icon",
        Some(&app.cookie),
        Some(&app.csrf),
        "image/png",
        png_bytes(),
    )
    .await;
    assert_eq!(upload.status(), StatusCode::OK);
    let upload = json_body(upload).await;
    assert_eq!(upload["version"], 1);
    assert_eq!(upload["icon_url"], "/api/v1/branding/icon?v=1");

    let body =
        json_body(send(&app.router, Method::GET, "/api/v1/branding", None, None).await).await;
    assert_eq!(body["version"], 1);
    assert_eq!(body["icon_url"], "/api/v1/branding/icon?v=1");

    let icon = send(
        &app.router,
        Method::GET,
        "/api/v1/branding/icon?v=1",
        None,
        None,
    )
    .await;
    assert_eq!(icon.status(), StatusCode::OK);
    assert_eq!(
        icon.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    let etag = icon
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        to_bytes(icon.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .starts_with(b"\x89PNG")
    );

    let cached = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/branding/icon")
                .header(header::IF_NONE_MATCH, etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);

    let reset = send(
        &app.router,
        Method::DELETE,
        "/api/v1/admin/branding/icon",
        Some(&app.cookie),
        Some(&app.csrf),
    )
    .await;
    assert_eq!(reset.status(), StatusCode::OK);
    let body =
        json_body(send(&app.router, Method::GET, "/api/v1/branding", None, None).await).await;
    assert_eq!(body["version"], 0);
    assert!(body["icon_url"].is_null());
}
