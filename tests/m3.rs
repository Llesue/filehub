use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    base: std::path::PathBuf,
    home: std::path::PathBuf,
    router: Router,
    cookie: String,
    csrf: String,
    admin_id: i64,
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
    let setup_body = json_body(setup).await;
    let csrf = setup_body["csrf_token"].as_str().unwrap().to_owned();
    let admin_id = setup_body["user"]["id"].as_i64().unwrap();

    let roots = send(&router, Method::GET, "/api/v1/roots", Some(&cookie), None).await;
    let root_id = json_body(roots).await["roots"][0]["id"].as_i64().unwrap();
    let home = dir.path().join("users/admin");

    TestApp {
        _dir: dir,
        base: home.parent().unwrap().join("shared"),
        home,
        router,
        cookie,
        csrf,
        admin_id,
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
async fn root_crud_overlap_permissions_and_quota() {
    let app = setup_app().await;
    std::fs::create_dir_all(app.base.join("sub")).unwrap();
    std::fs::write(app.base.join("sub/hello.txt"), b"hello").unwrap();

    let base_root = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/admin/users/{}/roots", app.admin_id),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"path": app.base.to_string_lossy(), "alias": "shared", "read_only": true}),
    )
    .await;
    assert_eq!(base_root.status(), StatusCode::OK);
    let base_root = json_body(base_root).await["root_id"].as_i64().unwrap();

    let sub_root = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/admin/users/{}/roots", app.admin_id),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"path": app.base.join("sub").to_string_lossy(), "alias": "shared-sub", "read_only": false}),
    )
    .await;
    assert_eq!(sub_root.status(), StatusCode::OK);

    let upload = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": base_root,
            "path": "sub",
            "filename": "write.txt",
            "total_size": 5,
            "chunk_size": 5
        }),
    )
    .await;
    assert_eq!(upload.status(), StatusCode::OK);
    let upload = json_body(upload).await;
    let upload_id = upload["upload_id"].as_str().unwrap().to_owned();
    let chunk = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/api/v1/uploads/{upload_id}/chunks/0"))
                .header(header::COOKIE, &app.cookie)
                .header("x-csrf-token", &app.csrf)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(b"write".to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(chunk.status(), StatusCode::OK);
    let complete = send(
        &app.router,
        Method::POST,
        &format!("/api/v1/uploads/{upload_id}/complete"),
        Some(&app.cookie),
        Some(&app.csrf),
    )
    .await;
    assert_eq!(complete.status(), StatusCode::OK);

    let forbidden = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": base_root,
            "path": "",
            "filename": "blocked.txt",
            "total_size": 5,
            "chunk_size": 5
        }),
    )
    .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let alice = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/users",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"username":"alice","password":"correct horse","role":"user","quota_bytes":5}),
    )
    .await;
    assert_eq!(alice.status(), StatusCode::OK);

    let login = send_json(
        &app.router,
        Method::POST,
        "/api/v1/login",
        None,
        None,
        json!({"username":"alice","password":"correct horse"}),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let alice_cookie = session_cookie(&login);
    let alice_csrf = json_body(login).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let forced = send_json(
        &app.router,
        Method::POST,
        "/api/v1/me/password/forced",
        Some(&alice_cookie),
        Some(&alice_csrf),
        json!({"new_password": "new password 9"}),
    )
    .await;
    assert_eq!(forced.status(), StatusCode::NO_CONTENT);
    let alice_roots = send(
        &app.router,
        Method::GET,
        "/api/v1/roots",
        Some(&alice_cookie),
        None,
    )
    .await;
    let alice_root = json_body(alice_roots).await["roots"][0]["id"]
        .as_i64()
        .unwrap();

    let quota = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&alice_cookie),
        Some(&alice_csrf),
        json!({
            "root": alice_root,
            "path": "",
            "filename": "too-big.txt",
            "total_size": 10,
            "chunk_size": 5
        }),
    )
    .await;
    assert_eq!(quota.status(), StatusCode::INSUFFICIENT_STORAGE);
}

#[cfg(unix)]
#[tokio::test]
async fn trusted_link_allows_registered_external_target() {
    let app = setup_app().await;
    let outside = app._dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("file.txt"), b"outside").unwrap();

    let link = app.home.join("outside-link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/trusted-links",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "link_path": link.to_string_lossy(),
            "target_path": outside.to_string_lossy(),
            "access": "read_only"
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    let link_id = created["id"].as_i64().unwrap();

    let list = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=outside-link", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(list.status(), StatusCode::OK);
    assert_eq!(json_body(list).await["entries"][0]["name"], "file.txt");

    let download = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/download?root={}&path=outside-link/file.txt",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(download.into_body(), 1024).await.unwrap()[..],
        b"outside"
    );

    let deleted = send(
        &app.router,
        Method::DELETE,
        &format!("/api/v1/admin/trusted-links/{link_id}"),
        Some(&app.cookie),
        Some(&app.csrf),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let blocked = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=outside-link", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn server_paths_require_existing_directory_and_support_crud() {
    let app = setup_app().await;
    let missing = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/server-paths",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"path": "/definitely/not/a/filehub/path", "alias": "missing"}),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let storage = app.base.join("storage");
    std::fs::create_dir_all(&storage).unwrap();
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/server-paths",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"path": storage.to_string_lossy(), "alias": "Red"}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    assert_eq!(
        created["real_path"],
        std::fs::canonicalize(&storage)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );

    let listed = send(
        &app.router,
        Method::GET,
        "/api/v1/admin/server-paths",
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = json_body(listed).await;
    assert_eq!(listed["paths"][0]["alias"], "Red");

    let deleted = send(
        &app.router,
        Method::DELETE,
        &format!(
            "/api/v1/admin/server-paths/{}",
            created["id"].as_i64().unwrap()
        ),
        Some(&app.cookie),
        Some(&app.csrf),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn zero_byte_upload_completes() {
    let app = setup_app().await;
    let upload = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "",
            "filename": "empty.txt",
            "total_size": 0,
            "chunk_size": 5
        }),
    )
    .await;
    assert_eq!(upload.status(), StatusCode::OK);
    let upload = json_body(upload).await;
    assert_eq!(upload["chunks"].as_i64(), Some(0));
    let upload_id = upload["upload_id"].as_str().unwrap().to_owned();

    let complete = send(
        &app.router,
        Method::POST,
        &format!("/api/v1/uploads/{upload_id}/complete"),
        Some(&app.cookie),
        Some(&app.csrf),
    )
    .await;
    assert_eq!(complete.status(), StatusCode::OK);

    let target = app.home.join("empty.txt");
    assert!(target.exists());
    assert_eq!(std::fs::metadata(target).unwrap().len(), 0);
}

#[tokio::test]
async fn jobs_list_returns_own_jobs() {
    let app = setup_app().await;
    let response = send(
        &app.router,
        Method::GET,
        "/api/v1/jobs?since=0",
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["jobs"].is_array());
}
