use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    home: std::path::PathBuf,
    router: Router,
    state: Arc<filehub::state::AppState>,
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
    std::fs::create_dir_all(home.join("d1/d2")).unwrap();
    std::fs::create_dir_all(home.join("dst")).unwrap();
    std::fs::create_dir_all(home.join("src")).unwrap();
    std::fs::write(home.join("d1/a.txt"), b"alpha").unwrap();
    std::fs::write(home.join("d1/d2/b.txt"), b"1234567").unwrap();
    std::fs::write(home.join("src/one.txt"), b"one").unwrap();
    std::fs::write(home.join("src/two.txt"), b"two-two").unwrap();
    std::fs::write(home.join("photo.png"), b"not a real png").unwrap();

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

async fn wait_job(app: &TestApp, id: &str) -> Value {
    for _ in 0..200 {
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/jobs/{id}"),
            Some(&app.cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let job = json_body(response).await;
        match job["status"].as_str() {
            Some("completed") | Some("failed") | Some("cancelled") => return job,
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("job {id} did not finish");
}

async fn wait_dir_size(app: &TestApp, path: &str) -> Value {
    for _ in 0..200 {
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/fs/dir-size?root={}&path={path}", app.root_id),
            Some(&app.cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_body(response).await;
        match value["status"].as_str() {
            Some("ready") | Some("error") => return value,
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("dir size for {path} did not finish");
}

#[tokio::test]
async fn dir_size_caches_nested_directories_and_survives_reload() {
    let app = setup_app().await;

    let first = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/fs/dir-size?root={}&path=d1", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    assert!(matches!(
        first["status"].as_str(),
        Some("queued") | Some("running") | Some("ready")
    ));

    let d1 = wait_dir_size(&app, "d1").await;
    assert_eq!(d1["status"], "ready");
    assert_eq!(d1["size"].as_u64().unwrap(), 12);

    // The parent scan persisted every nested directory, so entering d2 must
    // not enqueue another scan.
    let d2 = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/fs/dir-size?root={}&path=d1/d2", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    let d2 = json_body(d2).await;
    assert_eq!(d2["status"], "ready");
    assert_eq!(d2["size"].as_u64().unwrap(), 7);

    let again = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/fs/dir-size?root={}&path=d1", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    let again = json_body(again).await;
    assert_eq!(again["status"], "ready");
    assert_eq!(again["size"].as_u64().unwrap(), 12);

    let rows = app.state.db.directory_size_counts().await.unwrap();
    assert!(
        rows.iter()
            .any(|(status, count)| status == "ready" && *count >= 2)
    );

    let TestApp {
        _dir: dir,
        home,
        router: _,
        state: _,
        cookie,
        csrf,
        root_id,
    } = app;
    let state = filehub::state::AppState::new_test(dir.path(), "test-token")
        .await
        .unwrap();
    let router = filehub::app::router(state.clone());
    let restarted = TestApp {
        _dir: dir,
        home,
        router,
        state,
        cookie,
        csrf,
        root_id,
    };
    let after_restart = send(
        &restarted.router,
        Method::GET,
        &format!("/api/v1/fs/dir-size?root={}&path=d1", restarted.root_id),
        Some(&restarted.cookie),
        None,
    )
    .await;
    assert_eq!(after_restart.status(), StatusCode::OK);
    let after_restart = json_body(after_restart).await;
    assert_eq!(after_restart["status"], "ready");
    assert_eq!(after_restart["size"].as_u64().unwrap(), 12);
}

#[tokio::test]
async fn transfer_job_copies_in_background_and_updates_caches() {
    let app = setup_app().await;

    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/transfer-jobs",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "source_root": app.root_id,
            "paths": ["src"],
            "dest_root": app.root_id,
            "dest_path": "dst",
            "move_items": false,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let job_id = body["job_id"].as_str().unwrap().to_owned();

    let job = wait_job(&app, &job_id).await;
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["result"]["failed"].as_u64().unwrap(), 0);
    assert!(app.home.join("dst/src/one.txt").is_file());
    assert!(app.home.join("dst/src/two.txt").is_file());
    assert!(app.home.join("src/one.txt").is_file());

    let size = wait_dir_size(&app, "dst/src").await;
    assert_eq!(size["status"], "ready");
    assert_eq!(size["size"].as_u64().unwrap(), 10);
}

#[tokio::test]
async fn audit_pagination_uses_keyset_cursor() {
    let app = setup_app().await;
    for index in 0..5 {
        app.state
            .db
            .audit(None, "test", Some(&format!("path-{index}")), None, None)
            .await
            .unwrap();
    }

    let first = send(
        &app.router,
        Method::GET,
        "/api/v1/admin/audit?limit=2",
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    let entries = first["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();

    let second = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/admin/audit?limit=2&cursor={cursor}"),
        Some(&app.cookie),
        None,
    )
    .await;
    let second = json_body(second).await;
    let entries = second["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0]["id"].as_i64().unwrap() < first["entries"][1]["id"].as_i64().unwrap());
}

#[tokio::test]
async fn index_aggregate_size_is_used_by_dir_size() {
    let app = setup_app().await;
    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/index",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "", "recursive": true}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let job_id = json_body(response).await["job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let job = wait_job(&app, &job_id).await;
    assert_eq!(job["status"], "completed", "{job}");

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/fs/dir-size?root={}&path=d1", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let value = json_body(response).await;
    assert_eq!(value["status"], "ready");
    assert_eq!(value["indexed"], true);
    assert_eq!(value["size"].as_u64().unwrap(), 12);
}

async fn run_index(app: &TestApp) -> Value {
    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/index",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({ "root": app.root_id, "path": "", "recursive": true }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let job_id = json_body(response).await["job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    wait_job(app, &job_id).await
}

#[tokio::test]
async fn quick_scan_refreshes_changed_directories() {
    let app = setup_app().await;
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");

    std::fs::write(app.home.join("d1/c.txt"), b"new").unwrap();
    std::fs::create_dir_all(app.home.join("d1/newdir")).unwrap();
    std::fs::write(app.home.join("d1/newdir/deep.txt"), b"deep").unwrap();
    let cancel = CancellationToken::new();
    let mut progress = |_checked: usize, _total: usize| {};
    let stats = filehub::indexer::quick_scan(&app.state, &cancel, &mut progress)
        .await
        .unwrap();
    assert!(stats.changed >= 1, "{stats:?}");
    assert!(stats.new_directories >= 1, "{stats:?}");

    let new_dir = app.home.join("d1/newdir");
    let new_dir = new_dir.to_string_lossy().to_string();
    let mut indexed = false;
    for _ in 0..100 {
        if let Some((status, count, _, _, _)) = app.state.db.index_status(&new_dir).await.unwrap()
            && status == "ready"
            && count >= 1
        {
            indexed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(indexed, "quick scan did not index the new child directory");

    let root = app
        .state
        .db
        .list_roots(1)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let entries = app
        .state
        .db
        .search_entries("c.txt", &root.real_path, true, 10, Default::default())
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "c.txt");
}

#[tokio::test]
async fn search_filters_size_kind_and_directory() {
    let app = setup_app().await;
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");

    let search = |query: &str| {
        let uri = format!("{query}&root={}", app.root_id);
        let router = app.router.clone();
        let cookie = app.cookie.clone();
        async move {
            let response = send(&router, Method::GET, &uri, Some(&cookie), None).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            body["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };

    let large = search("/api/v1/search?q=txt&min_size=6").await;
    assert!(large.contains(&"b.txt".to_owned()));
    assert!(large.contains(&"two.txt".to_owned()));
    assert!(!large.contains(&"a.txt".to_owned()));

    let small = search("/api/v1/search?q=txt&max_size=5").await;
    assert!(small.contains(&"a.txt".to_owned()));
    assert!(!small.contains(&"b.txt".to_owned()));

    let directories = search("/api/v1/search?q=d1&kind=dir").await;
    assert!(directories.contains(&"d1".to_owned()));
    assert!(!directories.contains(&"a.txt".to_owned()));

    let future = search("/api/v1/search?q=txt&modified_after=9999999999").await;
    assert!(future.is_empty(), "{future:?}");
}

#[tokio::test]
async fn list_exposes_index_metadata() {
    let app = setup_app().await;
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let photo = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "photo.png")
        .unwrap();
    assert_eq!(photo["mime_type"], "image/png");
    assert_eq!(photo["has_preview"], true);
    assert!(photo["created_at"].as_i64().is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn dir_size_disk_mode_deduplicates_hardlinks() {
    use std::os::unix::fs::MetadataExt;

    let app = setup_app().await;
    let hard_dir = app.home.join("hard");
    std::fs::create_dir_all(&hard_dir).unwrap();
    let file = hard_dir.join("source.bin");
    std::fs::write(&file, vec![7_u8; 5000]).unwrap();
    std::fs::hard_link(&file, hard_dir.join("link.bin")).unwrap();
    let expected_blocks = std::fs::metadata(&file).unwrap().blocks() * 512;

    let logical = wait_dir_size(&app, "hard").await;
    assert_eq!(logical["size"].as_u64().unwrap(), 10_000);

    app.state
        .db
        .upsert_setting("jobs.dir_size_use_logical", "false")
        .await
        .unwrap();
    app.state.reload_settings().await.unwrap();
    app.state
        .invalidate_path_caches(&hard_dir, Some(1), true)
        .await;

    let disk = wait_dir_size(&app, "hard").await;
    assert_eq!(disk["size"].as_u64().unwrap(), expected_blocks.max(4096));
}

#[tokio::test]
async fn browsing_an_unindexed_directory_queues_a_background_index() {
    let app = setup_app().await;
    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=d1", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let target = app.home.join("d1");
    let target = target.to_string_lossy().to_string();
    let mut indexed = false;
    for _ in 0..100 {
        if let Some((status, count, _, _, _)) = app.state.db.index_status(&target).await.unwrap()
            && status == "ready"
            && count >= 2
        {
            indexed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(indexed, "browse did not queue a background index");
}

#[tokio::test]
async fn async_audit_writer_flushes_batches() {
    let app = setup_app().await;
    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/mkdir",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({ "root": app.root_id, "path": "", "name": "audit-dir" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    app.state.flush_audit().await;
    let entries = app.state.db.list_audit(20, None).await.unwrap();
    assert!(entries.iter().any(|entry| entry.action == "mkdir"));
}
