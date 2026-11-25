use std::time::{Duration, SystemTime};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use filehub::db::{UploadSession, unix_now};

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
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    if let Some(c) = csrf {
        builder = builder.header("x-csrf-token", c);
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

fn upload(
    id: &str,
    user_id: i64,
    root_id: i64,
    temp: &std::path::Path,
    expires_at: i64,
    now: i64,
) -> UploadSession {
    UploadSession {
        id: id.to_owned(),
        user_id,
        root_id,
        dir_path: String::new(),
        filename: format!("{id}.bin"),
        total_size: 1,
        chunk_size: 1,
        received: 1,
        chunk_bitmap: vec![1],
        temp_path: temp.to_string_lossy().to_string(),
        sha256: None,
        verify: "auto".to_owned(),
        hash_status: None,
        hash_error: None,
        verified_at: None,
        expires_at,
        created_at: now - 60,
        updated_at: now - 60,
        remote_upload_id: None,
        if_match: None,
        if_none_match: None,
        token_id: None,
        mtime: None,
    }
}

fn set_age(path: &std::path::Path, age: Duration) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

#[tokio::test]
async fn maintenance_cleans_expired_sessions_uploads_and_edit_temps() {
    let app = setup_app().await;
    let now = unix_now();

    app.state
        .db
        .insert_session("expired-session", "csrf", app.user_id, now - 10, None, None)
        .await
        .unwrap();
    app.state
        .db
        .insert_session("valid-session", "csrf", app.user_id, now + 3600, None, None)
        .await
        .unwrap();

    let expired_temp = app.home.join(".filehub-upload-up_expired.part");
    std::fs::write(&expired_temp, b"expired").unwrap();
    app.state
        .db
        .insert_upload(&upload(
            "up_expired",
            app.user_id,
            app.root_id,
            &expired_temp,
            now - 10,
            now,
        ))
        .await
        .unwrap();

    let valid_temp = app.home.join(".filehub-upload-up_valid.part");
    std::fs::write(&valid_temp, b"valid").unwrap();
    app.state
        .db
        .insert_upload(&upload(
            "up_valid",
            app.user_id,
            app.root_id,
            &valid_temp,
            now + 3600,
            now,
        ))
        .await
        .unwrap();

    let stale_edit = app.home.join(".filehub-edit-stale.tmp");
    std::fs::write(&stale_edit, b"stale").unwrap();
    set_age(&stale_edit, Duration::from_secs(3 * 86_400));
    let fresh_edit = app.home.join(".filehub-edit-fresh.tmp");
    std::fs::write(&fresh_edit, b"fresh").unwrap();
    let unrelated = app.home.join(".filehub-edit-note.txt");
    std::fs::write(&unrelated, b"keep").unwrap();

    app.state.run_maintenance().await;

    assert!(
        app.state
            .db
            .find_session("expired-session")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        app.state
            .db
            .find_session("valid-session")
            .await
            .unwrap()
            .is_some()
    );

    assert!(
        app.state
            .db
            .find_upload("up_expired", app.user_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !expired_temp.exists(),
        "expired upload temp was not removed"
    );
    assert!(
        app.state
            .db
            .find_upload("up_valid", app.user_id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(valid_temp.exists(), "valid upload temp must be kept");

    assert!(!stale_edit.exists(), "stale editor temp was not removed");
    assert!(fresh_edit.exists(), "fresh editor temp must be kept");
    assert!(unrelated.exists(), "only .filehub-edit-*.tmp is touched");
}

#[tokio::test]
async fn expired_upload_access_removes_its_temp_file() {
    let app = setup_app().await;
    let now = unix_now();
    let temp = app.home.join(".filehub-upload-up_stale.part");
    std::fs::write(&temp, b"stale").unwrap();
    app.state
        .db
        .insert_upload(&upload(
            "up_stale",
            app.user_id,
            app.root_id,
            &temp,
            now - 10,
            now,
        ))
        .await
        .unwrap();

    let response = send(
        &app.router,
        Method::GET,
        "/api/v1/uploads/up_stale",
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(!temp.exists(), "expired upload temp was not removed");
}

#[tokio::test]
async fn switching_dir_size_mode_clears_cached_sizes() {
    let app = setup_app().await;
    app.state
        .db
        .set_directory_size("/tmp/filehub-size-fixture", "ready", 512, 1, None)
        .await
        .unwrap();
    assert!(
        app.state
            .db
            .directory_size_state("/tmp/filehub-size-fixture")
            .await
            .unwrap()
            .is_some()
    );

    let response = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "jobs.dir_size_use_logical", "value": true}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut cleared = false;
    for _ in 0..100 {
        if app
            .state
            .db
            .directory_size_state("/tmp/filehub-size-fixture")
            .await
            .unwrap()
            .is_none()
        {
            cleared = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        cleared,
        "cached directory sizes must be dropped when the mode changes"
    );
}

#[tokio::test]
async fn wal_checkpoint_truncates_the_journal() {
    let app = setup_app().await;
    let busy = app.state.db.checkpoint().await.unwrap();
    assert!(
        !busy,
        "checkpoint should not be busy in a single-connection test"
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let stats = app.state.db.stats().await.unwrap();
    assert!(
        stats.wal_bytes < 64 * 1024,
        "WAL was not truncated: {} bytes",
        stats.wal_bytes
    );
}
