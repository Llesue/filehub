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
async fn size_sort_uses_directory_sizes_and_refreshes_unknown_directories() {
    let app = setup_app().await;
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");

    async fn list_names(app: &TestApp, order: &str) -> Vec<String> {
        let response = send(
            &app.router,
            Method::GET,
            &format!(
                "/api/v1/list?root={}&path=&sort=size&order={order}",
                app.root_id
            ),
            Some(&app.cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap().to_owned())
            .collect()
    }

    let names = list_names(&app, "desc").await;
    let position = |name: &str| names.iter().position(|value| value == name).unwrap();
    assert_eq!(names.first().map(String::as_str), Some("d1"), "{names:?}");
    assert!(position("src") < position("dst"), "{names:?}");
    assert!(position("dst") < position("photo.png"), "{names:?}");

    let nested = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/list?root={}&path=d1&sort=size&order=desc",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    let nested = json_body(nested).await;
    let nested_names: Vec<&str> = nested["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(nested_names, vec!["d2", "a.txt"], "{nested_names:?}");

    let ascending = list_names(&app, "asc").await;
    let ascending_position = |name: &str| ascending.iter().position(|value| value == name).unwrap();
    assert!(
        ascending_position("src") < ascending_position("d1"),
        "{ascending:?}"
    );
    assert!(
        ascending_position("d1") < ascending_position("dst"),
        "{ascending:?}"
    );

    std::fs::create_dir(app.home.join("fresh")).unwrap();
    std::fs::write(app.home.join("fresh/big.bin"), vec![7_u8; 12_345]).unwrap();
    let pending = list_names(&app, "desc").await;
    let pending_fresh = pending
        .iter()
        .position(|value| value == "fresh")
        .expect("fresh directory is missing");
    assert!(
        pending_fresh
            < pending
                .iter()
                .position(|value| value == "photo.png")
                .unwrap(),
        "directories must sort before files: {pending:?}"
    );

    let mut refreshed = pending;
    for _ in 0..120 {
        refreshed = list_names(&app, "desc").await;
        if refreshed.first().map(String::as_str) == Some("fresh") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        refreshed.first().map(String::as_str),
        Some("fresh"),
        "background dir-size refresh did not reorder: {refreshed:?}"
    );
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

#[tokio::test]
async fn quick_scan_persists_new_directories_when_the_batch_limit_is_hit() {
    let app = setup_app().await;
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");

    for index in 0..520 {
        std::fs::create_dir_all(app.home.join(format!("d1/n{index}"))).unwrap();
    }

    let cancel = CancellationToken::new();
    let mut progress = |_checked: usize, _total: usize| {};
    filehub::indexer::quick_scan(&app.state, &cancel, &mut progress)
        .await
        .unwrap();

    let pending = app.state.db.take_pending_index_dirs(1000).await.unwrap();
    assert!(
        pending.len() >= 20,
        "overflow candidates were dropped: {} pending",
        pending.len()
    );
    assert!(
        pending.iter().all(|path| path.contains("d1/n")),
        "{pending:?}"
    );
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
    std::fs::write(app.home.join("doc.pdf"), b"%PDF-1.4").unwrap();
    std::fs::write(app.home.join("song.mp3"), b"ID3fake").unwrap();
    std::fs::write(app.home.join("clip.mp4"), b"fake").unwrap();
    std::fs::write(app.home.join("note.md"), b"# note").unwrap();
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

    let images = search("/api/v1/search?q=photo&kind=image").await;
    assert_eq!(images, vec!["photo.png".to_owned()]);
    let audio = search("/api/v1/search?q=song&kind=audio").await;
    assert_eq!(audio, vec!["song.mp3".to_owned()]);
    let videos = search("/api/v1/search?q=clip&kind=video").await;
    assert_eq!(videos, vec!["clip.mp4".to_owned()]);
    let pdfs = search("/api/v1/search?q=doc&kind=pdf").await;
    assert_eq!(pdfs, vec!["doc.pdf".to_owned()]);
    let text = search("/api/v1/search?q=note&kind=text").await;
    assert_eq!(text, vec!["note.md".to_owned()]);
    let mismatched = search("/api/v1/search?q=photo&kind=video").await;
    assert!(mismatched.is_empty(), "{mismatched:?}");

    let files = search("/api/v1/search?q=txt&kind=file").await;
    assert!(files.contains(&"a.txt".to_owned()));
    assert!(!files.contains(&"d1".to_owned()));
    let small_files = search("/api/v1/search?q=txt&kind=file&max_size=5").await;
    assert!(small_files.contains(&"a.txt".to_owned()));
    assert!(!small_files.contains(&"b.txt".to_owned()));
    assert!(!small_files.contains(&"two.txt".to_owned()));
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
    app.state.reload_settings(false).await.unwrap();
    app.state
        .invalidate_path_caches_with_usage(
            &hard_dir,
            Some(1),
            false,
            filehub::state::UsageInvalidation::None,
        )
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

#[tokio::test]
async fn dir_size_serves_stale_value_while_refreshing() {
    let app = setup_app().await;
    let first = wait_dir_size(&app, "d1").await;
    assert_eq!(first["status"], "ready");
    assert_eq!(first["size"].as_u64().unwrap(), 12);

    // Force the cached row to look outdated. The endpoint must keep returning
    // the old value while the refresh job runs.
    let real_path = app.home.join("d1").to_string_lossy().to_string();
    app.state
        .db
        .set_directory_size(&real_path, "ready", 12, 1, None)
        .await
        .unwrap();

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/fs/dir-size?root={}&path=d1", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let stale = json_body(response).await;
    assert_eq!(stale["status"], "ready");
    assert_eq!(stale["stale"], true);
    assert_eq!(stale["refreshing"], true);
    assert_eq!(stale["size"].as_u64().unwrap(), 12);

    for _ in 0..200 {
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/fs/dir-size?root={}&path=d1", app.root_id),
            Some(&app.cookie),
            None,
        )
        .await;
        let value = json_body(response).await;
        if value["status"] == "ready" && value["stale"] == false {
            assert_eq!(value["size"].as_u64().unwrap(), 12);
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("refreshed directory size did not become fresh");
}

#[tokio::test]
async fn directory_size_failures_keep_the_last_value() {
    let app = setup_app().await;
    let path = "/tmp/filehub-size-stale-fixture";
    app.state
        .db
        .set_directory_size(path, "ready", 42, 1, None)
        .await
        .unwrap();

    app.state.db.begin_directory_size(path, 2).await.unwrap();
    let state = app
        .state
        .db
        .directory_size_state(path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.0, "running");
    assert_eq!(state.1, 42);
    assert_eq!(state.2, 2);
    assert!(state.5, "running refresh must keep has_value");

    app.state
        .db
        .fail_directory_size(path, "error", 2, Some("boom"))
        .await
        .unwrap();
    let state = app
        .state
        .db
        .directory_size_state(path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.0, "error");
    assert_eq!(state.1, 42);
    assert!(state.5, "failed refresh must keep has_value");
}

#[tokio::test]
async fn soft_deleted_user_disappears_and_cannot_login() {
    let app = setup_app().await;
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/users",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"username":"qa-soft-delete","password":"correct horse 9","role":"user"}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let id = json_body(created).await["user"]["id"].as_i64().unwrap();

    let deleted = send_json(
        &app.router,
        Method::DELETE,
        &format!("/api/v1/admin/users/{id}"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({}),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let users = send(
        &app.router,
        Method::GET,
        "/api/v1/admin/users",
        Some(&app.cookie),
        None,
    )
    .await;
    let users = json_body(users).await;
    assert!(
        !users["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|user| user["username"] == "qa-soft-delete"),
        "soft-deleted user is still listed"
    );

    let login = send_json(
        &app.router,
        Method::POST,
        "/api/v1/login",
        None,
        None,
        json!({"username":"qa-soft-delete","password":"correct horse 9"}),
    )
    .await;
    assert_eq!(login.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn custom_css_is_admin_controlled_and_public() {
    let app = setup_app().await;
    let css = ".demo { color: red; }";
    let saved = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/custom-css",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"content": css}),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK);
    let body = json_body(saved).await;
    let version = body["version"].as_i64().unwrap();
    assert!(version > 0);

    let response = send(&app.router, Method::GET, "/api/v1/custom.css", None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), 1024).await.unwrap()[..],
        css.as_bytes()
    );

    let branding = send(&app.router, Method::GET, "/api/v1/branding", None, None).await;
    assert_eq!(json_body(branding).await["custom_css_version"], version);

    let cleared = send_json(
        &app.router,
        Method::DELETE,
        "/api/v1/admin/custom-css",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({}),
    )
    .await;
    assert_eq!(cleared.status(), StatusCode::OK);
    let response = send(&app.router, Method::GET, "/api/v1/custom.css", None, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn usage_delta_updates_cached_snapshot() {
    let app = setup_app().await;
    app.state.usage_cache.lock().insert(
        1,
        filehub::state::UsageSnapshot {
            used_bytes: 100,
            entry_count: 0,
            quota_bytes: None,
            disks: Vec::new(),
            computed_at: 42,
        },
    );
    app.state.adjust_usage(1, 25);
    app.state.adjust_usage(1, -10);
    let snapshot = app.state.usage_cache.lock().get(&1).cloned().unwrap();
    assert_eq!(snapshot.used_bytes, 115);
    assert_eq!(snapshot.computed_at, 42);
    app.state.adjust_usage(1, -1000);
    assert_eq!(app.state.usage_cache.lock().get(&1).unwrap().used_bytes, 0);
}

#[tokio::test]
async fn unified_search_unions_roots_without_duplicates() {
    let app = setup_app().await;
    std::fs::write(app.home.join("alpha-note.txt"), b"alpha").unwrap();
    std::fs::write(app.home.join("beta-note.txt"), b"beta").unwrap();
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");
    let root = app
        .state
        .db
        .list_roots(1)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let base = root.real_path.trim_end_matches(['/', '\\']);
    let ranges = vec![
        (format!("{base}/"), format!("{base}0")),
        // Overlapping sub-root must not duplicate results.
        (format!("{base}/"), format!("{base}0")),
    ];
    let (first_page, cursor) = app
        .state
        .db
        .search_entries_all("note", ranges.clone(), true, 1, None, Default::default())
        .await
        .unwrap();
    assert_eq!(first_page.len(), 1);
    let cursor = cursor.expect("a second page must exist");
    let (second_page, _) = app
        .state
        .db
        .search_entries_all("note", ranges, true, 1, Some(cursor), Default::default())
        .await
        .unwrap();
    assert_eq!(second_page.len(), 1);
    assert_ne!(first_page[0].name, second_page[0].name);
    let mut entries = first_page;
    entries.extend(second_page);
    let mut names: Vec<_> = entries.into_iter().map(|entry| entry.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names, vec!["alpha-note.txt", "beta-note.txt"]);
}

#[tokio::test]
async fn two_character_chinese_search_uses_bigrams() {
    let app = setup_app().await;
    std::fs::write(app.home.join("山西旅游-测试.txt"), b"x").unwrap();
    std::fs::write(app.home.join("手机照片-测试.txt"), b"x").unwrap();
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");
    let root = app
        .state
        .db
        .list_roots(1)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let base = root.real_path.trim_end_matches(['/', '\\']);
    let ranges = vec![(format!("{base}/"), format!("{base}0"))];
    let (entries, _) = app
        .state
        .db
        .search_entries_bigram("山西", ranges.clone(), true, 10, None, Default::default())
        .await
        .unwrap();
    assert!(entries.iter().any(|entry| entry.name.contains("山西旅游")));
    let (entries, _) = app
        .state
        .db
        .search_entries_bigram("手机", ranges, true, 10, None, Default::default())
        .await
        .unwrap();
    assert!(entries.iter().any(|entry| entry.name.contains("手机照片")));
}

#[tokio::test]
async fn list_focus_returns_the_page_containing_the_file() {
    let app = setup_app().await;
    let big = app.home.join("big");
    std::fs::create_dir_all(big.join("aaa-dir")).unwrap();
    for index in 0..450 {
        std::fs::write(big.join(format!("file-{index:04}.txt")), b"x").unwrap();
    }
    let job = run_index(&app).await;
    assert_eq!(job["status"], "completed", "{job}");

    let response = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/list?root={}&path=big&sort=name&focus=file-0325.txt&limit=100",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let focus_index = body["focus_index"].as_u64().unwrap() as usize;
    assert_eq!(body["focus_offset"], 326);
    assert_eq!(body["total"], 451);
    assert_eq!(body["entries"][focus_index]["name"], "file-0325.txt");
    assert!(focus_index < 100);

    let response = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/list?root={}&path=big&sort=name&order=desc&focus=file-0325.txt&limit=100",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let focus_index = body["focus_index"].as_u64().unwrap() as usize;
    assert_eq!(body["focus_offset"], 125);
    assert_eq!(body["entries"][focus_index]["name"], "file-0325.txt");

    // Size sorting bypasses the index and must locate on the filesystem.
    let response = send(
        &app.router,
        Method::GET,
        &format!(
            "/api/v1/list?root={}&path=big&sort=size&focus=file-0325.txt&limit=100",
            app.root_id
        ),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let focus_index = body["focus_index"].as_u64().unwrap() as usize;
    assert_eq!(body["entries"][focus_index]["name"], "file-0325.txt");
}

#[tokio::test]
async fn list_focus_missing_file_returns_404() {
    let app = setup_app().await;
    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=&focus=missing.txt", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn inline_aac_skips_wechat_primer_frame_and_maps_ranges() {
    let app = setup_app().await;
    let mut data = vec![0xFF, 0xF9, 0x60, 0x80, 0x01, 0x3F, 0xFC, 0x00, 0x00];
    data.extend_from_slice(&[0xFF, 0xF1, 0x60, 0x40, 0x0D, 0x1F, 0xFC]);
    data.extend_from_slice(b"abcdefghij");
    std::fs::write(app.home.join("wechat.aac"), &data).unwrap();
    let uri = format!(
        "/api/v1/download?root={}&path=wechat.aac&inline=true",
        app.root_id
    );

    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/aac");
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "17");
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(&body[..], &data[9..]);

    let request = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::COOKIE, &app.cookie)
        .header(header::RANGE, "bytes=0-3")
        .body(Body::empty())
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 0-3/17");
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(&body[..], &data[9..13]);

    let plain = format!("/api/v1/download?root={}&path=wechat.aac", app.root_id);
    let response = send(&app.router, Method::GET, &plain, Some(&app.cookie), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(&body[..], &data[..]);
}

#[tokio::test]
async fn terminal_rejects_root_deletion_and_self_copy() {
    let app = setup_app().await;
    std::fs::create_dir_all(app.home.join("work/sub")).unwrap();
    std::fs::write(app.home.join("work/a.txt"), b"a").unwrap();

    let run = |line: &str| {
        let app = &app;
        let line = line.to_owned();
        async move {
            send_json(
                &app.router,
                Method::POST,
                "/api/v1/cmd",
                Some(&app.cookie),
                Some(&app.csrf),
                json!({
                    "root": app.root_id,
                    "cwd": "",
                    "line": line,
                    "confirm": true,
                }),
            )
            .await
        }
    };

    for line in ["rm -r .", "rm -r /", "cp -r . work/sub", "mv . work/sub"] {
        let response = run(line).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{line} should be rejected"
        );
    }
    assert!(app.home.exists());
    assert!(app.home.join("work/a.txt").exists());

    let response = run("cp -r work work/sub").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = run("rm -r work -y").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let code = body["confirm"]["code"].as_str().unwrap().to_owned();
    assert!(app.home.join("work").exists());

    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "cwd": "",
            "line": "rm -r work -y",
            "confirm": true,
            "confirm_code": code,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!app.home.join("work").exists());
}

#[tokio::test]
async fn terminal_lists_roots_and_transfers_across_roots() {
    let app = setup_app().await;
    std::fs::create_dir_all(app.home.join("backup")).unwrap();
    std::fs::write(app.home.join("a.txt"), b"alpha").unwrap();

    let me = send_json(
        &app.router,
        Method::GET,
        "/api/v1/me",
        Some(&app.cookie),
        None,
        serde_json::json!({}),
    )
    .await;
    let user_id = json_body(me).await["user"]["id"].as_i64().unwrap();
    let created = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/admin/users/{user_id}/roots"),
        Some(&app.cookie),
        Some(&app.csrf),
        serde_json::json!({
            "path": app.home.join("backup").to_string_lossy(),
            "alias": "backup",
            "read_only": false,
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let backup_root = json_body(created).await["root_id"].as_i64().unwrap();

    let roots = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        serde_json::json!({"root": app.root_id, "cwd": "", "line": "roots"}),
    )
    .await;
    let output = json_body(roots).await["output"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(output.contains("backup"));
    assert!(output.contains(&format!("@{backup_root}")));

    let copy = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        serde_json::json!({
            "root": app.root_id,
            "cwd": "",
            "line": format!("cp a.txt @{backup_root}:/"),
        }),
    )
    .await;
    assert_eq!(copy.status(), StatusCode::OK);
    let output = json_body(copy).await["output"].as_str().unwrap().to_owned();
    assert!(output.contains("queued copy job"), "{output}");
    for _ in 0..100 {
        if app.home.join("backup/a.txt").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        std::fs::read(app.home.join("backup/a.txt")).unwrap(),
        b"alpha"
    );

    let ls = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        serde_json::json!({"root": app.root_id, "cwd": "", "line": "ls"}),
    )
    .await;
    let output = json_body(ls).await["output"].as_str().unwrap().to_owned();
    assert!(output.contains("\u{1b}[34m"), "ls should color directories");
    assert!(output.contains("\u{1b}[90m"), "ls should color the summary");

    let jobs = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        serde_json::json!({"root": app.root_id, "cwd": "", "line": "job"}),
    )
    .await;
    let output = json_body(jobs).await["output"].as_str().unwrap().to_owned();
    assert!(
        output.contains("\u{1b}["),
        "job output should use color codes"
    );
}

#[tokio::test]
async fn transfer_job_copies_many_files_with_parallel_workers() {
    let app = setup_app().await;
    std::fs::create_dir_all(app.home.join("copy")).unwrap();
    for dir in 0..4 {
        let path = app.home.join(format!("many/d{dir}"));
        std::fs::create_dir_all(&path).unwrap();
        for file in 0..10 {
            std::fs::write(path.join(format!("f{file}.txt")), format!("{dir}-{file}")).unwrap();
        }
    }
    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/transfer-jobs",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "source_root": app.root_id,
            "paths": ["many"],
            "dest_root": app.root_id,
            "dest_path": "copy",
            "move_items": false,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let job = wait_job(&app, body["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed", "{job}");
    for dir in 0..4 {
        for file in 0..10 {
            assert_eq!(
                std::fs::read(app.home.join(format!("copy/many/d{dir}/f{file}.txt"))).unwrap(),
                format!("{dir}-{file}").as_bytes()
            );
        }
    }
}

#[tokio::test]
async fn external_path_alias_rename_updates_assigned_roots() {
    let app = setup_app().await;
    std::fs::create_dir_all(app.home.join("external")).unwrap();
    let me = send_json(
        &app.router,
        Method::GET,
        "/api/v1/me",
        Some(&app.cookie),
        None,
        json!({}),
    )
    .await;
    let user_id = json_body(me).await["user"]["id"].as_i64().unwrap();

    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/server-paths",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "path": app.home.join("external").to_string_lossy(),
            "alias": "Alpha",
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let path_id = json_body(created).await["id"].as_i64().unwrap();

    let assigned = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/admin/users/{user_id}/roots"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "path": app.home.join("external").to_string_lossy(),
            "alias": "Alpha",
            "read_only": false,
        }),
    )
    .await;
    assert_eq!(assigned.status(), StatusCode::OK);
    let root_id = json_body(assigned).await["root_id"].as_i64().unwrap();

    let renamed = send_json(
        &app.router,
        Method::PUT,
        &format!("/api/v1/admin/server-paths/{path_id}"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"alias": "Beta"}),
    )
    .await;
    assert_eq!(renamed.status(), StatusCode::OK);

    let roots = send_json(
        &app.router,
        Method::GET,
        &format!("/api/v1/admin/users/{user_id}/roots"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({}),
    )
    .await;
    let roots = json_body(roots).await;
    let root = roots["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|root| root["id"].as_i64() == Some(root_id))
        .unwrap();
    assert_eq!(root["alias"], "Beta");
}
