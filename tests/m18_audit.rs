use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    home: std::path::PathBuf,
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
    let roots = send(&router, Method::GET, "/api/v1/roots", Some(&cookie), None).await;
    let root_id = json_body(roots).await["roots"][0]["id"].as_i64().unwrap();
    let home = dir.path().join("users/admin");
    std::fs::write(home.join("hello.txt"), b"hello").unwrap();
    TestApp {
        _dir: dir,
        home,
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
    extra: Option<(&str, &str)>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some((name, value)) = extra {
        builder = builder.header(name, value);
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

async fn send_json_if_match(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: &str,
    csrf: &str,
    etag: &str,
    body: Value,
) -> Response<Body> {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header("x-csrf-token", csrf)
        .header(header::IF_MATCH, etag)
        .body(Body::from(body.to_string()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
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
async fn share_links_stay_stable_and_rotate_invalidates_old_links() {
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
    let id = created["id"].as_i64().unwrap();
    let token = created["token"].as_str().unwrap().to_owned();

    let first = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/shares/{id}/url"),
        Some(&app.cookie),
        None,
    )
    .await;
    let second = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/shares/{id}/url"),
        Some(&app.cookie),
        None,
    )
    .await;
    let first = json_body(first).await["token"].as_str().unwrap().to_owned();
    let second = json_body(second).await["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(token, first);
    assert_eq!(first, second);

    let page = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(page.status(), StatusCode::OK);

    let rotated = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/shares/{id}/rotate"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({}),
    )
    .await;
    assert_eq!(rotated.status(), StatusCode::OK);
    let fresh = json_body(rotated).await["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(fresh, token);

    let old = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(old.status(), StatusCode::NOT_FOUND);
    let new = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{fresh}"),
        None,
        None,
    )
    .await;
    assert_eq!(new.status(), StatusCode::OK);
}

#[tokio::test]
async fn dir_version_reports_directories_only() {
    let app = setup_app().await;
    let dir = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/fs/dir-version?root={}&path=", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(dir.status(), StatusCode::OK);
    assert!(json_body(dir).await["mtime"].as_i64().unwrap() > 0);

    let file = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/fs/dir-version?root={}&path=hello.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(file.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gbk_file_round_trips_through_the_editor() {
    let app = setup_app().await;
    let (encoded, _, _) = encoding_rs::GBK.encode("中文");
    std::fs::write(app.home.join("legacy.txt"), encoded.as_ref()).unwrap();

    let probe = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/file/probe?root={}&path=legacy.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(probe.status(), StatusCode::OK);
    let probe = json_body(probe).await;
    assert_eq!(probe["is_text"], true);
    assert_eq!(probe["reason"], "text");
    assert_eq!(probe["encoding"], "gbk");
    assert_eq!(probe["bom"], false);
    assert_eq!(probe["lossy"], false);
    assert_eq!(probe["size"], 4);

    let read = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/file?root={}&path=legacy.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let body = json_body(read).await;
    assert_eq!(body["content"], "中文");
    assert_eq!(body["encoding"], "gbk");
    assert_eq!(body["lossy"], false);
    let etag = body["etag"].as_str().unwrap().to_owned();

    let saved = send_json_if_match(
        &app.router,
        Method::PUT,
        &format!("/api/v1/file?root={}&path=legacy.txt", app.root_id),
        &app.cookie,
        &app.csrf,
        &etag,
        json!({"content":"中文改", "encoding":"gbk", "bom": false}),
    )
    .await;
    let saved = json_body(saved).await;
    assert!(saved.get("etag").is_some(), "save failed: {saved}");
    let _ = etag;
    let bytes = std::fs::read(app.home.join("legacy.txt")).unwrap();
    let (decoded, _, errors) = encoding_rs::GBK.decode(&bytes);
    assert!(!errors);
    assert_eq!(&*decoded, "中文改");
}

#[tokio::test]
async fn utf16_bom_file_round_trips_through_the_editor() {
    let app = setup_app().await;
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(
        "hello 中文\r\nsecond"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes()),
    );
    std::fs::write(app.home.join("utf16.txt"), &bytes).unwrap();

    let read = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/file?root={}&path=utf16.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let body = json_body(read).await;
    assert_eq!(body["encoding"], "utf-16le");
    assert_eq!(body["bom"], true);
    let etag = body["etag"].as_str().unwrap().to_owned();

    let saved = send_json_if_match(
        &app.router,
        Method::PUT,
        &format!("/api/v1/file?root={}&path=utf16.txt", app.root_id),
        &app.cookie,
        &app.csrf,
        &etag,
        json!({"content":"hello 中文改\r\nsecond", "encoding":"utf-16le", "bom": true}),
    )
    .await;
    assert_eq!(
        saved.status(),
        StatusCode::OK,
        "{:?}",
        json_body(saved).await
    );

    let bytes = std::fs::read(app.home.join("utf16.txt")).unwrap();
    assert!(bytes.starts_with(&[0xFF, 0xFE]));
    let (decoded, errors) = encoding_rs::UTF_16LE.decode_without_bom_handling(&bytes[2..]);
    assert!(!errors);
    assert_eq!(&*decoded, "hello 中文改\r\nsecond");
}

#[tokio::test]
async fn lossy_utf16_file_is_probed_and_read() {
    let app = setup_app().await;
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend("hello ".encode_utf16().flat_map(|unit| unit.to_le_bytes()));
    bytes.extend([0x00, 0xD8]);
    std::fs::write(app.home.join("lossy.txt"), &bytes).unwrap();

    let probe = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/file/probe?root={}&path=lossy.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(probe.status(), StatusCode::OK);
    let probe = json_body(probe).await;
    assert_eq!(probe["is_text"], true);
    assert_eq!(probe["reason"], "lossy");
    assert_eq!(probe["encoding"], "utf-16le");
    assert_eq!(probe["bom"], true);
    assert_eq!(probe["lossy"], true);

    let read = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/file?root={}&path=lossy.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let read = json_body(read).await;
    assert_eq!(read["encoding"], "utf-16le");
    assert_eq!(read["bom"], true);
    assert_eq!(read["lossy"], true);
    assert!(
        read["content"]
            .as_str()
            .is_some_and(|content| content.contains('\u{FFFD}'))
    );
}

#[tokio::test]
async fn binary_signatures_are_not_editable() {
    let app = setup_app().await;
    std::fs::write(
        app.home.join("fake.txt"),
        b"\x89PNG\r\n\x1a\nnot really text",
    )
    .unwrap();
    let probe = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/file/probe?root={}&path=fake.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(probe.status(), StatusCode::OK);
    let body = json_body(probe).await;
    assert_eq!(body["is_text"], false);
    assert_eq!(body["reason"], "binary");
    assert!(body["encoding"].is_null());
    assert_eq!(body["bom"], false);
    assert_eq!(body["lossy"], false);
}

#[tokio::test]
async fn cookie_secure_is_rejected_without_https() {
    let app = setup_app().await;
    let response = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "security.cookie_secure", "value": true}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "security.cookie_secure", "value": false}),
    )
    .await;
    let body = json_body(response).await;
    assert!(
        body.get("error").is_some() || body.get("settings").is_some(),
        "{body}"
    );
}

#[tokio::test]
async fn explicit_iso_8859_1_keeps_the_requested_label() {
    let app = setup_app().await;
    std::fs::write(app.home.join("latin1.txt"), b"caf\xe9 na\xefve").unwrap();

    let read = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/file?root={}&path=latin1.txt&encoding=iso-8859-1",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let body = json_body(read).await;
    assert_eq!(body["encoding"], "iso-8859-1");
    assert_eq!(body["content"], "café naïve");
    assert_eq!(body["bom"], false);

    // An explicit Windows-1252 request must keep its own label.
    let windows = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/file?root={}&path=latin1.txt&encoding=windows-1252",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(windows.status(), StatusCode::OK);
    assert_eq!(json_body(windows).await["encoding"], "windows-1252");
}
