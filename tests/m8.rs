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
    root_id: i64,
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
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = session_cookie(&setup);
    let body = json_body(setup).await;
    let csrf = body["csrf_token"].as_str().unwrap().to_owned();
    let roots = send(
        &router,
        Method::GET,
        "/api/v1/roots",
        Some(&cookie),
        None,
        None,
    )
    .await;
    let root_id = json_body(roots).await["roots"][0]["id"].as_i64().unwrap();
    let home = dir.path().join("users/admin");
    std::fs::write(home.join("note.txt"), b"hello").unwrap();
    std::fs::write(home.join("image.bin"), [0_u8, 1, 2]).unwrap();
    TestApp {
        _dir: dir,
        router,
        cookie,
        csrf,
        root_id,
    }
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    if_match: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    if let Some(c) = csrf {
        builder = builder.header("x-csrf-token", c);
    }
    if let Some(c) = if_match {
        builder = builder.header(header::IF_MATCH, c);
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
    if_match: Option<&str>,
    body: Value,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    if let Some(c) = csrf {
        builder = builder.header("x-csrf-token", c);
    }
    if let Some(c) = if_match {
        builder = builder.header(header::IF_MATCH, c);
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
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

#[tokio::test]
async fn editor_read_save_etag_and_binary_rejection() {
    let app = setup_app().await;
    let uri = format!("/api/v1/file?root={}&path=note.txt", app.root_id);
    let read = send(
        &app.router,
        Method::GET,
        &uri,
        Some(&app.cookie),
        None,
        None,
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let read = json_body(read).await;
    assert_eq!(read["content"], "hello");
    let etag = read["etag"].as_str().unwrap().to_owned();

    let saved = send_json(
        &app.router,
        Method::PUT,
        &uri,
        Some(&app.cookie),
        Some(&app.csrf),
        Some(&etag),
        json!({"content":"hello world"}),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK);
    let saved = json_body(saved).await;
    assert_ne!(saved["etag"], etag);

    let conflict = send_json(
        &app.router,
        Method::PUT,
        &uri,
        Some(&app.cookie),
        Some(&app.csrf),
        Some(&etag),
        json!({"content":"stale"}),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::PRECONDITION_FAILED);

    let binary_uri = format!("/api/v1/file?root={}&path=image.bin", app.root_id);
    let binary = send(
        &app.router,
        Method::GET,
        &binary_uri,
        Some(&app.cookie),
        None,
        None,
    )
    .await;
    assert_eq!(binary.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let traversal = format!("/api/v1/file?root={}&path=../note.txt", app.root_id);
    let traversal = send(
        &app.router,
        Method::GET,
        &traversal,
        Some(&app.cookie),
        None,
        None,
    )
    .await;
    assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);
}
