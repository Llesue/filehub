use std::io::{Cursor, Read};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    home: std::path::PathBuf,
    router: Router,
    state: std::sync::Arc<filehub::state::AppState>,
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
    assert_eq!(roots.status(), StatusCode::OK);
    let roots = json_body(roots).await;
    let root_id = roots["roots"][0]["id"].as_i64().unwrap();

    let home = dir.path().join("users/admin");
    std::fs::create_dir_all(home.join("sub")).unwrap();
    std::fs::write(home.join("hello.txt"), b"hello world").unwrap();
    std::fs::write(home.join("sub/nested.txt"), b"nested").unwrap();

    TestApp {
        _dir: dir,
        home,
        router,
        state,
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
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
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
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
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
    body: Vec<u8>,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/octet-stream");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
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

#[tokio::test]
async fn list_download_range_and_etag() {
    let app = setup_app().await;
    let list_uri = format!("/api/v1/list?root={}&path=&sort=name", app.root_id);

    let response = send(&app.router, Method::GET, &list_uri, Some(&app.cookie), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let names: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert!(names.contains(&"hello.txt"));
    assert!(names.contains(&"sub"));

    let download_uri = format!("/api/v1/download?root={}&path=hello.txt", app.root_id);
    let response = send(
        &app.router,
        Method::GET,
        &download_uri,
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::ETAG).is_some());
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(&body[..], b"hello world");

    let mut request = Request::builder()
        .method(Method::GET)
        .uri(&download_uri)
        .header(header::COOKIE, &app.cookie)
        .header(header::RANGE, "bytes=0-4")
        .body(Body::empty())
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(&body[..], b"hello");

    request = Request::builder()
        .method(Method::GET)
        .uri(&download_uri)
        .header(header::COOKIE, &app.cookie)
        .header(header::IF_NONE_MATCH, "\"not-a-match\"")
        .body(Body::empty())
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let traversal = format!("/api/v1/download?root={}&path=../secret", app.root_id);
    let response = send(
        &app.router,
        Method::GET,
        &traversal,
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["error"]["code"], "INVALID_PATH");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc", app.home.join("escape")).unwrap();
        let symlink_uri = format!("/api/v1/download?root={}&path=escape/passwd", app.root_id);
        let response = send(
            &app.router,
            Method::GET,
            &symlink_uri,
            Some(&app.cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn upload_resume_delete_and_zip() {
    let app = setup_app().await;

    let create = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "",
            "filename": "upload.txt",
            "total_size": 11,
            "chunk_size": 5
        }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::OK);
    let create = json_body(create).await;
    let upload_id = create["upload_id"].as_str().unwrap().to_owned();

    let chunk_uri = format!("/api/v1/uploads/{upload_id}/chunks/0");
    let response = send_bytes(
        &app.router,
        Method::PUT,
        &chunk_uri,
        Some(&app.cookie),
        Some(&app.csrf),
        b"hello".to_vec(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let status_uri = format!("/api/v1/uploads/{upload_id}");
    let response = send(
        &app.router,
        Method::GET,
        &status_uri,
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let status = json_body(response).await;
    assert_eq!(status["received_chunks"], json!([0]));

    let response = send_bytes(
        &app.router,
        Method::PUT,
        &format!("/api/v1/uploads/{upload_id}/chunks/1"),
        Some(&app.cookie),
        Some(&app.csrf),
        b" worl".to_vec(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = send_bytes(
        &app.router,
        Method::PUT,
        &format!("/api/v1/uploads/{upload_id}/chunks/2"),
        Some(&app.cookie),
        Some(&app.csrf),
        b"d".to_vec(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = send(
        &app.router,
        Method::POST,
        &format!("/api/v1/uploads/{upload_id}/complete"),
        Some(&app.cookie),
        Some(&app.csrf),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read(app.home.join("upload.txt")).unwrap(),
        b"hello world"
    );

    std::fs::write(app.home.join("delete-me.txt"), b"bye").unwrap();
    let preview = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/delete/preview",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["delete-me.txt"]}),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    let preview = json_body(preview).await;
    let token = preview["token"].as_str().unwrap().to_owned();

    let deleted = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/delete",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"token": token}),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(!app.home.join("delete-me.txt").exists());

    let delete_dir = app.home.join("delete-dir");
    std::fs::create_dir_all(delete_dir.join("nested")).unwrap();
    let preview = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/delete/preview",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["delete-dir"]}),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    let preview = json_body(preview).await;
    let token = preview["token"].as_str().unwrap().to_owned();
    let code = preview["confirm_code"].as_str().unwrap().to_owned();

    let denied = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/delete",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"token": token}),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    let deleted = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/delete",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"token": token, "confirm_code": code}),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(!delete_dir.exists());

    let zip = send_json(
        &app.router,
        Method::POST,
        "/api/v1/download/zip",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["hello.txt"]}),
    )
    .await;
    assert_eq!(zip.status(), StatusCode::OK);
    assert_eq!(
        zip.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/zip"
    );
    let body = to_bytes(zip.into_body(), 10 * 1024 * 1024).await.unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(body)).unwrap();
    let mut file = archive.by_name("hello.txt").unwrap();
    let mut content = String::new();
    file.read_to_string(&mut content).unwrap();
    assert_eq!(content, "hello world");
}

#[tokio::test]
async fn edit_presence_is_scoped_to_the_same_user_and_file() {
    let app = setup_app().await;
    let mut events = app.state.events.subscribe();
    let first = send_json(
        &app.router,
        Method::POST,
        "/api/v1/edit/presence",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "hello.txt",
            "state": "open",
            "client": "tab-one",
        }),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(json_body(first).await["others"], 0);
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.expect("event bus");
            if event.event == "edit.presence" {
                break event;
            }
        }
    })
    .await
    .expect("presence event");
    assert_eq!(event.event, "edit.presence");
    assert_eq!(event.user_id, Some(1));
    assert_eq!(event.data["path"], "hello.txt");
    assert_eq!(event.data["client"], "tab-one");

    let second = send_json(
        &app.router,
        Method::POST,
        "/api/v1/edit/presence",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "hello.txt",
            "state": "open",
            "client": "tab-two",
        }),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = json_body(second).await;
    assert_eq!(second_body["others"], 1);
    assert_eq!(second_body["peers"][0]["client"], "tab-one");

    let other_file = send_json(
        &app.router,
        Method::POST,
        "/api/v1/edit/presence",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "sub/nested.txt",
            "state": "open",
            "client": "tab-two",
        }),
    )
    .await;
    assert_eq!(other_file.status(), StatusCode::OK);
    assert_eq!(json_body(other_file).await["others"], 0);

    let close = send_json(
        &app.router,
        Method::POST,
        "/api/v1/edit/presence",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "hello.txt",
            "state": "close",
            "client": "tab-two",
        }),
    )
    .await;
    assert_eq!(close.status(), StatusCode::OK);
    assert_eq!(json_body(close).await["others"], 1);

    let invalid = send_json(
        &app.router,
        Method::POST,
        "/api/v1/edit/presence",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "hello.txt",
            "state": "open",
            "client": "bad client!",
        }),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn user_updates_are_published_to_the_admin_list_and_the_affected_user() {
    let app = setup_app().await;
    let mut events = app.state.events.subscribe();
    let response = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/users/1",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({ "feature_terminal": true }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let mut admin_event = false;
    let mut scoped_event = false;
    for _ in 0..10 {
        let Ok(Ok(event)) =
            tokio::time::timeout(std::time::Duration::from_secs(3), events.recv()).await
        else {
            break;
        };
        if event.event != "user.updated" || event.data["id"] != 1 {
            continue;
        }
        admin_event |= event.admin_only;
        scoped_event |= event.user_id == Some(1);
        if admin_event && scoped_event {
            break;
        }
    }
    assert!(admin_event, "administrators need a user.updated event");
    assert!(
        scoped_event,
        "the affected user needs a scoped user.updated event"
    );
}

#[tokio::test]
async fn upload_preserves_client_mtime() {
    let app = setup_app().await;
    let mtime = 1_700_000_000_i64;
    let create = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "",
            "filename": "photo.jpg",
            "total_size": 3,
            "chunk_size": 3,
            "mtime": mtime
        }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::OK);
    let upload_id = json_body(create).await["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let response = send_bytes(
        &app.router,
        Method::PUT,
        &format!("/api/v1/uploads/{upload_id}/chunks/0"),
        Some(&app.cookie),
        Some(&app.csrf),
        b"abc".to_vec(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = send(
        &app.router,
        Method::POST,
        &format!("/api/v1/uploads/{upload_id}/complete"),
        Some(&app.cookie),
        Some(&app.csrf),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let modified = std::fs::metadata(app.home.join("photo.jpg"))
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(modified, mtime as u64);
}
