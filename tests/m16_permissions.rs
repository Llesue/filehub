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
    user_id: i64,
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
    let user_id = body["user"]["id"].as_i64().unwrap();
    let roots = send(&router, Method::GET, "/api/v1/roots", Some(&cookie), None).await;
    let root_id = json_body(roots).await["roots"][0]["id"].as_i64().unwrap();
    TestApp {
        home: dir.path().join("users/admin"),
        _dir: dir,
        router,
        state,
        cookie,
        csrf,
        user_id,
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

fn is_root() -> bool {
    #[cfg(unix)]
    {
        rustix::process::geteuid().as_raw() == 0
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[tokio::test]
async fn policy_read_only_is_reported_separately() {
    let app = setup_app().await;
    let update = send_json(
        &app.router,
        Method::PUT,
        &format!("/api/v1/admin/users/{}/roots/{}", app.user_id, app.root_id),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"alias":"我的文件","read_only":true,"sort_order":0}),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    let list = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(list.status(), StatusCode::OK);
    let body = json_body(list).await;
    assert_eq!(body["read_only"], true);
    assert_eq!(body["read_only_reason"], "policy");

    let mkdir = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/mkdir",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root":app.root_id,"path":"","name":"blocked"}),
    )
    .await;
    assert_eq!(mkdir.status(), StatusCode::FORBIDDEN);
    let body = json_body(mkdir).await;
    assert_eq!(body["error"]["code"], "FORBIDDEN");
}

#[tokio::test]
async fn filesystem_read_only_and_file_writability() {
    if is_root() {
        // Root bypasses mode bits, so this assertion only makes sense for the
        // unprivileged service account used in production.
        return;
    }
    let app = setup_app().await;
    let locked = app.home.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("a.txt"), "hello").unwrap();
    let read_only_file = locked.join("readonly.txt");
    std::fs::write(&read_only_file, "hello").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        std::fs::set_permissions(&read_only_file, std::fs::Permissions::from_mode(0o444)).unwrap();
    }

    let list = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=locked", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(list.status(), StatusCode::OK);
    let body = json_body(list).await;
    assert_eq!(body["read_only"], true);
    assert_eq!(body["read_only_reason"], "filesystem");

    let mkdir = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/mkdir",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root":app.root_id,"path":"locked","name":"x"}),
    )
    .await;
    assert_eq!(mkdir.status(), StatusCode::FORBIDDEN);
    let body = json_body(mkdir).await;
    assert_eq!(body["error"]["code"], "PERMISSION_DENIED");

    let file = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/file?root={}&path=locked/readonly.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(file.status(), StatusCode::OK);
    let body = json_body(file).await;
    assert_eq!(body["writable"], false);
    assert_eq!(body["read_only_reason"], "file");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[tokio::test]
async fn svg_is_never_served_inline() {
    let app = setup_app().await;
    std::fs::write(
        app.home.join("icon.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
    )
    .unwrap();
    std::fs::write(
        app.home.join("pixel.png"),
        [
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    )
    .unwrap();

    let svg = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/download?root={}&path=icon.svg&inline=true",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(svg.status(), StatusCode::OK);
    let disposition = svg
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(disposition.starts_with("attachment"));

    let png = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/download?root={}&path=pixel.png&inline=true",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(png.status(), StatusCode::OK);
    let disposition = png
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(disposition.starts_with("inline"));
}

#[tokio::test]
async fn setup_language_becomes_admin_preference() {
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
        json!({"token":"test-token","username":"admin","password":"correct horse","language":"en"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = session_cookie(&setup);
    let preferences = send(
        &router,
        Method::GET,
        "/api/v1/me/preferences",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(preferences.status(), StatusCode::OK);
    let body = json_body(preferences).await;
    assert_eq!(body["preferences"]["language"], "en");

    let invalid_dir = tempfile::tempdir().unwrap();
    let invalid_state = filehub::state::AppState::new_test(invalid_dir.path(), "test-token")
        .await
        .unwrap();
    let invalid_router = filehub::app::router(invalid_state);
    let invalid = send_json(
        &invalid_router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse","language":"fr"}),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_returns_before_subtree_cache_cleanup() {
    let app = setup_app().await;
    let victim = app.home.join("victim");
    std::fs::create_dir_all(victim.join("child")).unwrap();
    std::fs::write(victim.join("child").join("a.txt"), "hello").unwrap();

    let victim_path = victim.to_string_lossy().to_string();
    let child_path = victim.join("child").to_string_lossy().to_string();
    let deep_path = victim
        .join("child")
        .join("deep")
        .to_string_lossy()
        .to_string();
    let mut rows = Vec::new();
    rows.push((victim_path.clone(), 5, 0, None));
    rows.push((child_path.clone(), 5, 0, None));
    rows.push((deep_path.clone(), 5, 0, None));
    for index in 0..5000 {
        rows.push((
            victim
                .join(format!("child/cache-{index}"))
                .to_string_lossy()
                .to_string(),
            1,
            0,
            None,
        ));
    }
    app.state.db.set_directory_sizes_batch(rows).await.unwrap();

    let preview = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/delete/preview",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root":app.root_id,"paths":["victim"]}),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    let body = json_body(preview).await;
    let token = body["token"].as_str().unwrap().to_owned();
    let code = body["confirm_code"].as_str().unwrap().to_owned();

    let started = std::time::Instant::now();
    let deleted = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/delete",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"token":token,"confirm_code":code}),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "delete must not wait for subtree cache cleanup"
    );
    assert!(!victim.exists());

    let mut cleaned = false;
    for _ in 0..200 {
        if app
            .state
            .db
            .directory_size_state(&child_path)
            .await
            .unwrap()
            .is_none()
        {
            cleaned = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(cleaned, "subtree cache rows must be cleaned in background");
}

#[tokio::test]
async fn batch_settings_update_is_atomic_and_validated() {
    let app = setup_app().await;
    let response = send_json(
        &app.router,
        Method::PATCH,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"values":{"thumbnails.enabled":false,"editor.enabled":false}}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["settings"]["thumbnails"]["enabled"], false);
    assert_eq!(body["settings"]["editor"]["enabled"], false);

    let invalid = send_json(
        &app.router,
        Method::PATCH,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"values":{"not.a.key":1}}),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_feature_toggles_are_enforced_server_side() {
    let app = setup_app().await;
    seed_file(app.home.join("a.txt"));

    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/users",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"username":"alice","password":"correct horse","role":"user"}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    let alice_id = created["user"]["id"].as_i64().unwrap();
    assert_eq!(created["user"]["features"]["terminal"], true);
    assert_eq!(created["user"]["features"]["zip"], true);
    assert_eq!(created["user"]["features"]["unzip"], true);

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
    let me = send(
        &app.router,
        Method::GET,
        "/api/v1/me",
        Some(&alice_cookie),
        None,
    )
    .await;
    let alice_csrf = json_body(me).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let alice_root = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/admin/users/{alice_id}/roots"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"path": app.home.to_string_lossy(), "alias": "shared", "read_only": false}),
    )
    .await;
    assert_eq!(alice_root.status(), StatusCode::OK);
    let alice_root_id = json_body(alice_root).await["root_id"].as_i64().unwrap();

    let preferences = send(
        &app.router,
        Method::GET,
        "/api/v1/me/preferences",
        Some(&alice_cookie),
        None,
    )
    .await;
    let preferences = json_body(preferences).await;
    assert_eq!(preferences["capabilities"]["terminal"], true);
    assert_eq!(preferences["capabilities"]["zip"], true);
    assert_eq!(preferences["capabilities"]["unzip"], true);

    let updated = send_json(
        &app.router,
        Method::PUT,
        &format!("/api/v1/admin/users/{alice_id}"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "feature_terminal": false,
            "feature_zip": false,
            "feature_unzip": false,
        }),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated = json_body(updated).await;
    assert_eq!(updated["user"]["features"]["terminal"], false);
    assert_eq!(updated["user"]["features"]["zip"], false);
    assert_eq!(updated["user"]["features"]["unzip"], false);

    let preferences = send(
        &app.router,
        Method::GET,
        "/api/v1/me/preferences",
        Some(&alice_cookie),
        None,
    )
    .await;
    let preferences = json_body(preferences).await;
    assert_eq!(preferences["capabilities"]["terminal"], false);
    assert_eq!(preferences["capabilities"]["zip"], false);
    assert_eq!(preferences["capabilities"]["unzip"], false);

    let cmd = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&alice_cookie),
        Some(&alice_csrf),
        json!({"root": alice_root_id, "cwd": "", "line": "pwd"}),
    )
    .await;
    assert_eq!(cmd.status(), StatusCode::FORBIDDEN);

    let zip = send_json(
        &app.router,
        Method::POST,
        "/api/v1/download/zip",
        Some(&alice_cookie),
        Some(&alice_csrf),
        json!({"root": alice_root_id, "paths": ["a.txt"]}),
    )
    .await;
    assert_eq!(zip.status(), StatusCode::FORBIDDEN);

    // User preference cannot re-enable what the admin disabled.
    let enabled = send_json(
        &app.router,
        Method::PATCH,
        "/api/v1/me/preferences",
        Some(&alice_cookie),
        Some(&alice_csrf),
        json!({"terminal_enabled": true}),
    )
    .await;
    assert_eq!(enabled.status(), StatusCode::OK);
    let preferences = send(
        &app.router,
        Method::GET,
        "/api/v1/me/preferences",
        Some(&alice_cookie),
        None,
    )
    .await;
    assert_eq!(
        json_body(preferences).await["capabilities"]["terminal"],
        false,
        "user preference must not override the admin policy"
    );

    let restored = send_json(
        &app.router,
        Method::PUT,
        &format!("/api/v1/admin/users/{alice_id}"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"feature_terminal": true, "feature_zip": true, "feature_unzip": true}),
    )
    .await;
    assert_eq!(restored.status(), StatusCode::OK);

    let cmd = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&alice_cookie),
        Some(&alice_csrf),
        json!({"root": alice_root_id, "cwd": "", "line": "pwd"}),
    )
    .await;
    assert_eq!(cmd.status(), StatusCode::OK);
}

fn seed_file(path: std::path::PathBuf) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, b"data").unwrap();
}

#[tokio::test]
async fn entry_limit_blocks_creating_more_files() {
    let app = setup_app().await;
    std::fs::create_dir_all(&app.home).unwrap();

    let setting = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key":"max_entries_per_user","value":2}),
    )
    .await;
    assert_eq!(setting.status(), StatusCode::OK);

    for name in ["a.txt", "b.txt"] {
        let response = send_json(
            &app.router,
            Method::POST,
            "/api/v1/fs/touch",
            Some(&app.cookie),
            Some(&app.csrf),
            json!({"root": app.root_id, "path": "", "name": name}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{name}");
    }

    let denied = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/touch",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "", "name": "c.txt"}),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::INSUFFICIENT_STORAGE);
    let body = json_body(denied).await;
    assert_eq!(body["error"]["code"], "ENTRY_LIMIT_EXCEEDED");
    assert!(!app.home.join("c.txt").exists());

    let mkdir = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/mkdir",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "", "name": "dir"}),
    )
    .await;
    assert_eq!(mkdir.status(), StatusCode::INSUFFICIENT_STORAGE);

    let usage = send(
        &app.router,
        Method::GET,
        "/api/v1/usage",
        Some(&app.cookie),
        None,
    )
    .await;
    let usage = json_body(usage).await;
    assert_eq!(usage["entry_count"], 2);
    assert_eq!(usage["max_entries"], 2);
}

#[tokio::test]
async fn admin_can_reset_settings_and_verify_password() {
    let app = setup_app().await;

    let wrong = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/verify-password",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"password": "wrong"}),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

    let valid = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/verify-password",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"password": "correct horse"}),
    )
    .await;
    assert_eq!(valid.status(), StatusCode::NO_CONTENT);

    let updated = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "max_entries_per_user", "value": 5}),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    assert_eq!(
        json_body(updated).await["settings"]["max_entries_per_user"],
        5
    );

    let reset = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/settings/reset",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"keys": ["max_entries_per_user"]}),
    )
    .await;
    assert_eq!(reset.status(), StatusCode::OK);
    let settings = send(
        &app.router,
        Method::GET,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(
        json_body(settings).await["settings"]["max_entries_per_user"],
        1_000_000
    );
}
