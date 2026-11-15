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
    std::fs::write(home.join("hello.txt"), b"hello share").unwrap();
    std::fs::create_dir(home.join("folder")).unwrap();
    std::fs::write(home.join("folder/inside.txt"), b"inside").unwrap();
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
    ticket: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    if let Some(c) = csrf {
        builder = builder.header("x-csrf-token", c);
    }
    if let Some(c) = ticket {
        builder = builder.header("x-share-ticket", c);
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
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    if let Some(c) = csrf {
        builder = builder.header("x-csrf-token", c);
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
async fn share_crud_password_and_download() {
    let app = setup_app().await;
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "hello.txt"}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    let token = created["token"].as_str().unwrap().to_owned();
    let id = created["id"].as_i64().unwrap();

    let meta_uri = format!("/api/v1/public/shares/{token}");
    let meta = send(&app.router, Method::GET, &meta_uri, None, None, None).await;
    assert_eq!(meta.status(), StatusCode::OK);
    assert_eq!(json_body(meta).await["name"], "hello.txt");

    let download_uri = format!("/api/v1/public/shares/{token}/download");
    let download = send(&app.router, Method::GET, &download_uri, None, None, None).await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(download.into_body(), 1024).await.unwrap()[..],
        b"hello share"
    );

    let revoke = send(
        &app.router,
        Method::DELETE,
        &format!("/api/v1/shares/{id}"),
        Some(&app.cookie),
        Some(&app.csrf),
        None,
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
    let revoked = send(&app.router, Method::GET, &meta_uri, None, None, None).await;
    assert_eq!(revoked.status(), StatusCode::NOT_FOUND);

    let protected = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "folder", "password": "let me in"}),
    )
    .await;
    assert_eq!(protected.status(), StatusCode::OK);
    let protected = json_body(protected).await;
    let token = protected["token"].as_str().unwrap().to_owned();
    let meta_uri = format!("/api/v1/public/shares/{token}");

    let denied = send(&app.router, Method::GET, &meta_uri, None, None, None).await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let wrong = send_json(
        &app.router,
        Method::POST,
        &format!("{meta_uri}/unlock"),
        None,
        None,
        json!({"password": "wrong"}),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let unlocked = send_json(
        &app.router,
        Method::POST,
        &format!("{meta_uri}/unlock"),
        None,
        None,
        json!({"password": "let me in"}),
    )
    .await;
    assert_eq!(unlocked.status(), StatusCode::OK);
    let ticket = json_body(unlocked).await["ticket"]
        .as_str()
        .unwrap()
        .to_owned();

    let meta = send(
        &app.router,
        Method::GET,
        &meta_uri,
        None,
        None,
        Some(&ticket),
    )
    .await;
    assert_eq!(meta.status(), StatusCode::OK);
    assert_eq!(json_body(meta).await["entries"][0]["name"], "inside.txt");

    let traversal = send(
        &app.router,
        Method::GET,
        &format!("{meta_uri}/download?path=../../etc/passwd"),
        None,
        None,
        Some(&ticket),
    )
    .await;
    assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);
}
