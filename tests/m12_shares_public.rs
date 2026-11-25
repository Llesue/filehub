use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    home: std::path::PathBuf,
    state: Arc<filehub::state::AppState>,
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
    let router = filehub::app::router(state.clone());
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
    std::fs::create_dir_all(home.join("pub/.git")).unwrap();
    std::fs::write(home.join("pub/a.txt"), b"alpha").unwrap();
    std::fs::write(home.join("pub/b.txt"), b"bravo!!").unwrap();
    std::fs::write(home.join("pub/.env"), b"SECRET=1").unwrap();
    std::fs::write(home.join("pub/.git/config"), b"[core]").unwrap();
    image::RgbImage::from_fn(32, 24, |x, y| {
        image::Rgb([(x * 8) as u8, (y * 10) as u8, 128])
    })
    .save(home.join("pub/img.png"))
    .unwrap();
    std::fs::write(home.join(".hidden-secret"), b"hidden root").unwrap();

    TestApp {
        _dir: dir,
        home,
        state,
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
    ticket: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some(value) = ticket {
        builder = builder.header("x-share-ticket", value);
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

async fn create_share(
    app: &TestApp,
    path: &str,
    read_only: bool,
    password: Option<&str>,
) -> String {
    let mut payload = json!({"root": app.root_id, "path": path, "read_only": read_only});
    if let Some(password) = password {
        payload["password"] = json!(password);
    }
    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        payload,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await["token"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn public_share_pagination_hidden_files_and_visit_count() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", true, None).await;

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}?limit=2"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|entry| !entry["name"].as_str().unwrap().starts_with('.'))
    );
    assert!(
        entries
            .iter()
            .all(|entry| entry["name"] != ".env" && entry["name"] != ".git")
    );
    let cursor = body["next_cursor"].as_str().unwrap().to_owned();

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}?limit=10&cursor={cursor}"),
        None,
        None,
    )
    .await;
    let body = json_body(response).await;
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);
    assert!(
        body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["name"] != ".env")
    );

    for hidden in ["path=.env", "path=.git/config"] {
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/public/shares/{token}/download?{hidden}"),
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{hidden}");
    }

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/thumb?path=img.png&w=64&wait=1"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );

    let thumb_hidden = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/thumb?path=.env&w=64"),
        None,
        None,
    )
    .await;
    assert_eq!(thumb_hidden.status(), StatusCode::NOT_FOUND);

    let list = send(
        &app.router,
        Method::GET,
        "/api/v1/shares?limit=10",
        Some(&app.cookie),
        None,
    )
    .await;
    let shares = json_body(list).await;
    let share = shares["shares"]
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["real_path"].as_str().unwrap().ends_with("/pub"))
        .unwrap();
    assert_eq!(
        share["visit_count"], 1,
        "only the root page counts as a visit"
    );
}

#[tokio::test]
async fn public_share_download_supports_range_and_zip() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", true, None).await;

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/download?path=a.txt"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        b"alpha"
    );

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/download?path=a.txt"),
        None,
        None,
    )
    .await;
    let etag = response
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        response.headers().get(header::ACCEPT_RANGES).unwrap(),
        "bytes"
    );

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/v1/public/shares/{token}/download?path=a.txt"))
                .header(header::RANGE, "bytes=1-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response.headers().get(header::CONTENT_RANGE).unwrap(),
        "bytes 1-3/5"
    );
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        b"lph"
    );

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/v1/public/shares/{token}/download?path=a.txt"))
                .header(header::IF_NONE_MATCH, etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);

    let preflight = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip/preflight"),
        None,
        None,
        json!({"path": ""}),
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::OK);
    let preflight = json_body(preflight).await;
    assert_eq!(preflight["allowed"], true);
    assert_eq!(preflight["files"], 3, "hidden files must not be counted");
    let bytes = preflight["bytes"].as_u64().unwrap();

    let zip = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip"),
        None,
        None,
        json!({"path": ""}),
    )
    .await;
    assert_eq!(zip.status(), StatusCode::OK);
    assert_eq!(
        zip.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/zip"
    );
    let body = to_bytes(zip.into_body(), 10 * 1024 * 1024).await.unwrap();
    assert!(body.starts_with(b"PK"));
    assert!(body.len() as u64 > bytes);
}

#[tokio::test]
async fn password_share_accepts_query_ticket() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", true, Some("let me in")).await;
    let unlock = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/unlock"),
        None,
        None,
        json!({"password": "let me in"}),
    )
    .await;
    assert_eq!(unlock.status(), StatusCode::OK);
    let ticket = json_body(unlock).await["ticket"]
        .as_str()
        .unwrap()
        .to_owned();

    let denied = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/thumb?path=img.png&w=64&wait=1"),
        None,
        None,
    )
    .await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/thumb?path=img.png&w=64&ticket={ticket}&wait=1"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let denied_events = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/events"),
        None,
        None,
    )
    .await;
    assert_eq!(denied_events.status(), StatusCode::UNAUTHORIZED);

    let events = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/events?ticket={ticket}"),
        None,
        None,
    )
    .await;
    assert_eq!(events.status(), StatusCode::OK);
    assert_eq!(
        events.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    assert_eq!(
        events
            .headers()
            .get("x-accel-buffering")
            .unwrap()
            .to_str()
            .unwrap(),
        "no"
    );
}

#[tokio::test]
async fn writable_share_supports_chunked_upload_and_cancel() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", false, None).await;

    let created = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/uploads"),
        None,
        None,
        json!({
            "path": "",
            "filename": "chunked.bin",
            "total_size": 5,
            "chunk_size": 2,
            "mtime": 1_700_000_002_i64
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    let upload_id = created["upload_id"].as_str().unwrap().to_owned();
    assert_eq!(created["chunks"], 3);

    let status = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/uploads/{upload_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(json_body(status).await["received"], 0);

    for (index, chunk) in [b"ab".as_ref(), b"cd".as_ref(), b"e".as_ref()]
        .into_iter()
        .enumerate()
    {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!(
                        "/api/v1/public/shares/{token}/uploads/{upload_id}/chunks/{index}"
                    ))
                    .body(Body::from(chunk.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "chunk {index}");
    }

    let status = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/uploads/{upload_id}"),
        None,
        None,
    )
    .await;
    let status = json_body(status).await;
    assert_eq!(status["received"], 5);
    assert_eq!(status["received_chunks"].as_array().unwrap().len(), 3);

    let complete = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/uploads/{upload_id}/complete"),
        None,
        None,
        json!({}),
    )
    .await;
    assert_eq!(complete.status(), StatusCode::OK);
    assert_eq!(json_body(complete).await["name"], "chunked.bin");
    let modified = std::fs::metadata(app.home.join("pub/chunked.bin"))
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(modified, 1_700_000_002);
    assert_eq!(
        std::fs::read(app.home.join("pub/chunked.bin")).unwrap(),
        b"abcde"
    );

    let created = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/uploads"),
        None,
        None,
        json!({"path": "", "filename": "cancelled.bin", "total_size": 4, "chunk_size": 2}),
    )
    .await;
    let upload_id = json_body(created).await["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let cancel = send(
        &app.router,
        Method::DELETE,
        &format!("/api/v1/public/shares/{token}/uploads/{upload_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(cancel.status(), StatusCode::NO_CONTENT);
    let gone = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/uploads/{upload_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hidden_share_root_can_still_be_shared() {
    let app = setup_app().await;
    let token = create_share(&app, ".hidden-secret", true, None).await;
    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["name"], ".hidden-secret");

    let download = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/download"),
        None,
        None,
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(download.into_body(), 1024).await.unwrap().as_ref(),
        b"hidden root"
    );
}

#[tokio::test]
async fn public_share_requests_are_rate_limited() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", true, None).await;
    let update = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "security.public_rate_limit_per_minute", "value": 2}),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    for attempt in 0..2 {
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/public/shares/{token}"),
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "attempt {attempt}");
    }
    let blocked = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn collection_share_lists_downloads_and_zips_selected_files() {
    let app = setup_app().await;
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["pub/a.txt", "pub/b.txt", "pub/img.png"]}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    assert_eq!(created["kind"], "collection");
    assert_eq!(created["item_count"], 3);
    let token = created["token"].as_str().unwrap().to_owned();

    let meta = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(meta.status(), StatusCode::OK);
    let meta = json_body(meta).await;
    assert_eq!(meta["collection"], true);
    assert_eq!(meta["read_only"], true);
    let entries = meta["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    let item_id = entries
        .iter()
        .find(|entry| entry["name"] == "a.txt")
        .unwrap()["item_id"]
        .as_i64()
        .unwrap();
    let image_id = entries
        .iter()
        .find(|entry| entry["name"] == "img.png")
        .unwrap()["item_id"]
        .as_i64()
        .unwrap();

    let download = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/download?item={item_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(download.into_body(), 1024).await.unwrap().as_ref(),
        b"alpha"
    );

    let thumb = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/thumb?item={image_id}&w=64&wait=1"),
        None,
        None,
    )
    .await;
    assert_eq!(thumb.status(), StatusCode::OK);

    let preflight = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip/preflight"),
        None,
        None,
        json!({"item_ids": [item_id]}),
    )
    .await;
    let preflight = json_body(preflight).await;
    assert_eq!(preflight["allowed"], true);
    assert_eq!(preflight["files"], 1);
    assert_eq!(preflight["bytes"], 5);

    let zip = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip"),
        None,
        None,
        json!({"item_ids": [item_id, image_id]}),
    )
    .await;
    assert_eq!(zip.status(), StatusCode::OK);
    let body = to_bytes(zip.into_body(), 10 * 1024 * 1024).await.unwrap();
    assert!(body.starts_with(b"PK"));

    let rejected = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["pub", "pub/a.txt"]}),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let list = send(
        &app.router,
        Method::GET,
        "/api/v1/shares?limit=20",
        Some(&app.cookie),
        None,
    )
    .await;
    let shares = json_body(list).await;
    let share = shares["shares"]
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["token_hash"].is_null() && share["kind"] == "collection")
        .or_else(|| {
            shares["shares"]
                .as_array()
                .unwrap()
                .iter()
                .find(|share| share["kind"] == "collection")
        })
        .unwrap();
    assert_eq!(share["item_count"], 3);
}

#[tokio::test]
async fn revoked_share_reports_gone_instead_of_not_found() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", true, None).await;
    let list = send(
        &app.router,
        Method::GET,
        "/api/v1/shares?limit=10",
        Some(&app.cookie),
        None,
    )
    .await;
    let shares = json_body(list).await;
    let id = shares["shares"]
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["real_path"].as_str().unwrap().ends_with("/pub"))
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let revoke = send_json(
        &app.router,
        Method::DELETE,
        &format!("/api/v1/shares/{id}"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({}),
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::GONE);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "SHARE_REVOKED");

    let events = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/events"),
        None,
        None,
    )
    .await;
    assert_eq!(events.status(), StatusCode::GONE);
    let body = json_body(events).await;
    assert_eq!(body["error"]["code"], "SHARE_REVOKED");

    let unknown = send(
        &app.router,
        Method::GET,
        "/api/v1/public/shares/sh_missing/events",
        None,
        None,
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

#[cfg(unix)]
#[tokio::test]
async fn writable_share_never_writes_through_symlink_leaf() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", false, None).await;
    let outside = app._dir.path().join("outside.txt");
    std::os::unix::fs::symlink(&outside, app.home.join("pub/dangling.txt")).unwrap();

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/v1/public/shares/{token}/upload?path=&filename=dangling.txt"
                ))
                .body(Body::from("pwned".as_bytes().to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["name"], "dangling (2).txt");
    assert!(!outside.exists(), "upload escaped through dangling symlink");
    assert!(
        std::fs::symlink_metadata(app.home.join("pub/dangling.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read(app.home.join("pub/dangling (2).txt")).unwrap(),
        b"pwned"
    );

    let outside_chunked = app._dir.path().join("outside-chunked.bin");
    std::os::unix::fs::symlink(&outside_chunked, app.home.join("pub/chunked.txt")).unwrap();
    let created = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/uploads"),
        None,
        None,
        json!({"path": "", "filename": "chunked.txt", "total_size": 3, "chunk_size": 3}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let upload_id = json_body(created).await["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let chunk = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!(
                    "/api/v1/public/shares/{token}/uploads/{upload_id}/chunks/0"
                ))
                .body(Body::from(b"abc".to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(chunk.status(), StatusCode::OK);
    let complete = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/uploads/{upload_id}/complete"),
        None,
        None,
        json!({}),
    )
    .await;
    assert_eq!(complete.status(), StatusCode::OK);
    assert_eq!(json_body(complete).await["name"], "chunked (2).txt");
    assert!(
        !outside_chunked.exists(),
        "chunked upload escaped through dangling symlink"
    );
    assert_eq!(
        std::fs::read(app.home.join("pub/chunked (2).txt")).unwrap(),
        b"abc"
    );
}

#[tokio::test]
async fn public_share_zip_respects_owner_feature_toggle() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", true, None).await;

    let preflight = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip/preflight"),
        None,
        None,
        json!({"path": ""}),
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::OK);
    assert_eq!(json_body(preflight).await["allowed"], true);

    let disabled = send_json(
        &app.router,
        Method::PUT,
        &format!("/api/v1/admin/users/{}", 1),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"feature_zip": false}),
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::OK);

    let preflight = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip/preflight"),
        None,
        None,
        json!({"path": ""}),
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::OK);
    let body = json_body(preflight).await;
    assert_eq!(body["allowed"], false);
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("disabled")
    );

    let zip = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip"),
        None,
        None,
        json!({"path": ""}),
    )
    .await;
    assert_eq!(zip.status(), StatusCode::FORBIDDEN);

    let meta = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}?path="),
        None,
        None,
    )
    .await;
    assert_eq!(json_body(meta).await["zip_allowed"], false);
}

#[tokio::test]
async fn public_share_zip_form_accepts_repeated_paths() {
    let app = setup_app().await;
    let token = create_share(&app, "pub", true, None).await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/public/shares/{token}/zip/form"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("path=&paths=a.txt&paths=b.txt"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/zip"
    );
    let body = to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    assert!(body.starts_with(b"PK"));
}

#[tokio::test]
async fn expired_share_is_persisted_as_revoked() {
    let app = setup_app().await;
    let expires_at = filehub::db::unix_now() + 1;
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "pub/a.txt", "expires_at": expires_at}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    let id = created["id"].as_i64().unwrap();
    let token = created["token"].as_str().unwrap().to_owned();

    let meta = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(meta.status(), StatusCode::OK);

    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(app.state.expire_shares().await >= 1);

    let list = send(
        &app.router,
        Method::GET,
        "/api/v1/shares?limit=20",
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(list.status(), StatusCode::OK);
    let body = json_body(list).await;
    let share = body["shares"]
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["id"].as_i64() == Some(id))
        .unwrap();
    assert_eq!(share["revoked"], json!(true));
    assert_eq!(share["expired"], json!(true));

    let meta = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(meta.status(), StatusCode::GONE);
    assert_eq!(json_body(meta).await["error"]["code"], "SHARE_REVOKED");

    let revived = send_json(
        &app.router,
        Method::PUT,
        &format!("/api/v1/shares/{id}"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"revoked": false, "expires_at": filehub::db::unix_now() + 3600}),
    )
    .await;
    assert_eq!(revived.status(), StatusCode::OK);
    let meta = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(meta.status(), StatusCode::OK);
}
