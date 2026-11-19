use std::io::Read;

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
    std::fs::create_dir_all(home.join("dir/sub")).unwrap();
    std::fs::write(home.join("alpha.txt"), b"alpha").unwrap();
    std::fs::write(home.join("dir/beta.txt"), b"bravo").unwrap();
    std::fs::write(home.join("dir/sub/gamma.txt"), b"gamma").unwrap();
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
    csrf: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some(value) = csrf {
        builder = builder.header("x-csrf-token", value);
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

async fn wait_job(app: &TestApp, job_id: &str) -> Value {
    for _ in 0..100 {
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/jobs/{job_id}"),
            Some(&app.cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let job = json_body(response).await;
        match job["status"].as_str().unwrap_or_default() {
            "completed" => return job,
            "failed" | "cancelled" => panic!("job did not complete: {job}"),
            _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    panic!("job timed out");
}

#[tokio::test]
async fn archive_zip_creates_file_with_selected_entries() {
    let app = setup_app().await;
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/archive/zip",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "dir": "",
            "paths": ["alpha.txt", "dir"],
            "name": "bundle",
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    assert_eq!(created["name"], "bundle.zip");
    let job = wait_job(&app, created["job_id"].as_str().unwrap()).await;
    assert_eq!(job["result"]["name"], "bundle.zip");
    assert_eq!(job["result"]["files"], 3);

    let archive = std::fs::File::open(app.home.join("bundle.zip")).unwrap();
    let mut zip = zip::ZipArchive::new(archive).unwrap();
    let mut names: Vec<String> = (0..zip.len())
        .map(|index| zip.by_index(index).unwrap().name().unwrap().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["alpha.txt", "dir/beta.txt", "dir/sub/gamma.txt"]
    );
    let mut contents = String::new();
    zip.by_name("dir/sub/gamma.txt")
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "gamma");

    let second = send_json(
        &app.router,
        Method::POST,
        "/api/v1/archive/zip",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "dir": "",
            "paths": ["alpha.txt"],
            "name": "bundle.zip",
        }),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let job = wait_job(&app, json_body(second).await["job_id"].as_str().unwrap()).await;
    assert_eq!(job["result"]["name"], "bundle (2).zip");
    assert!(app.home.join("bundle (2).zip").exists());
}

#[tokio::test]
async fn archive_zip_rejects_paths_outside_directory() {
    let app = setup_app().await;
    for path in [
        "../alpha.txt",
        "dir/../alpha.txt",
        "/etc/passwd",
        "dir\\\\beta.txt",
    ] {
        let response = send_json(
            &app.router,
            Method::POST,
            "/api/v1/archive/zip",
            Some(&app.cookie),
            Some(&app.csrf),
            json!({
                "root": app.root_id,
                "dir": "",
                "paths": [path],
                "name": "evil.zip",
            }),
        )
        .await;
        assert!(
            response.status().is_client_error(),
            "{path} should be rejected"
        );
    }
    assert!(!app.home.join("evil.zip").exists());
}

async fn run_cmd(app: &TestApp, line: &str, confirm_code: Option<&str>) -> (StatusCode, Value) {
    let mut payload = json!({
        "root": app.root_id,
        "cwd": "",
        "line": line,
        "confirm": true,
    });
    if let Some(code) = confirm_code {
        payload["confirm_code"] = json!(code);
    }
    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        payload,
    )
    .await;
    let status = response.status();
    (status, json_body(response).await)
}

#[tokio::test]
async fn terminal_help_zip_and_unzip() {
    let app = setup_app().await;
    let (status, body) = run_cmd(&app, "help zip", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["output"].as_str().unwrap().starts_with("zip [-r]"));

    let (status, body) = run_cmd(&app, "zip bundle.zip alpha.txt -y", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let output = body["output"].as_str().unwrap();
    let job_id = output
        .split_whitespace()
        .find(|value| value.starts_with("job_"))
        .unwrap()
        .trim_end_matches(':');
    wait_job(&app, job_id).await;
    assert!(app.home.join("bundle.zip").exists());

    let (status, body) = run_cmd(&app, "zip nodir.zip dir", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"].as_str().unwrap().contains("-r"));

    let archive_path = app.home.join("manual.zip");
    let file = std::fs::File::create(&archive_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("nested/hello.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, b"hello").unwrap();
    zip.finish().unwrap();

    let (status, body) = run_cmd(&app, "unzip manual.zip", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        std::fs::read(app.home.join("manual/nested/hello.txt")).unwrap(),
        b"hello"
    );
    let (status, body) = run_cmd(&app, "unzip manual.zip", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(app.home.join("manual (2)/nested/hello.txt").exists());
}

#[tokio::test]
async fn terminal_rm_requires_code_even_with_force() {
    let app = setup_app().await;
    std::fs::create_dir_all(app.home.join("victim/inner")).unwrap();
    std::fs::write(app.home.join("victim/inner/a.txt"), b"a").unwrap();

    let (status, body) = run_cmd(&app, "rm -r victim -y", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code = body["confirm"]["code"].as_str().unwrap().to_owned();
    assert!(
        app.home.join("victim").exists(),
        "nothing should be removed"
    );

    for _ in 0..3 {
        let (status, _) = run_cmd(&app, "rm -r victim -y", Some("0000")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _) = run_cmd(&app, "rm -r victim -y", Some(&code)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(app.home.join("victim").exists());

    let (status, body) = run_cmd(&app, "rm -r victim -y", None).await;
    assert_eq!(status, StatusCode::OK);
    let code = body["confirm"]["code"].as_str().unwrap().to_owned();
    let (status, body) = run_cmd(&app, "rm -r victim -y", Some(&code)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!app.home.join("victim").exists());
}

#[tokio::test]
async fn terminal_rm_small_file_uses_plain_confirmation() {
    let app = setup_app().await;
    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "cwd": "", "line": "rm alpha.txt"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(app.home.join("alpha.txt").exists());

    let (status, body) = run_cmd(&app, "rm alpha.txt -y", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!app.home.join("alpha.txt").exists());
}

fn duplicate_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    fn put16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    fn put32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        put32(&mut out, 0x0403_4b50);
        put16(&mut out, 20);
        put16(&mut out, 0);
        put16(&mut out, 0);
        put16(&mut out, 0);
        put16(&mut out, 0);
        put32(&mut out, 0);
        put32(&mut out, data.len() as u32);
        put32(&mut out, data.len() as u32);
        put16(&mut out, name.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        put32(&mut central, 0x0201_4b50);
        put16(&mut central, 20);
        put16(&mut central, 20);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put32(&mut central, 0);
        put32(&mut central, data.len() as u32);
        put32(&mut central, data.len() as u32);
        put16(&mut central, name.len() as u16);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put32(&mut central, 0);
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, 0x0605_4b50);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, entries.len() as u16);
    put16(&mut out, entries.len() as u16);
    put32(&mut out, central_size);
    put32(&mut out, central_offset);
    put16(&mut out, 0);
    out
}

fn write_zip(path: &std::path::Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        std::io::Write::write_all(&mut zip, data).unwrap();
    }
    zip.finish().unwrap();
}

#[tokio::test]
async fn unzip_rejects_traversal_symlinks_and_duplicates() {
    let app = setup_app().await;
    let outside = app._dir.path().join("escaped.txt");

    write_zip(&app.home.join("slip.zip"), &[("../escaped.txt", b"nope")]);
    let (status, _) = run_cmd(&app, "unzip slip.zip", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!outside.exists());

    let file = std::fs::File::create(app.home.join("link.zip")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.add_symlink(
        "link",
        "../escaped.txt",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.finish().unwrap();
    let (status, _) = run_cmd(&app, "unzip link.zip", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!app.home.join("link/link").exists());
    assert!(!outside.exists());

    std::fs::write(
        app.home.join("dup.zip"),
        duplicate_zip(&[("same.txt", b"one"), ("same.txt", b"two")]),
    )
    .unwrap();
    let (status, _) = run_cmd(&app, "unzip dup.zip", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!app.home.join("dup/same.txt").exists());
}

#[tokio::test]
async fn unzip_into_existing_directory_refuses_conflicts() {
    let app = setup_app().await;
    write_zip(&app.home.join("conflict.zip"), &[("a.txt", b"new")]);
    std::fs::create_dir_all(app.home.join("target")).unwrap();
    std::fs::write(app.home.join("target/a.txt"), b"old").unwrap();

    let (status, body) = run_cmd(&app, "unzip conflict.zip -d target", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        std::fs::read(app.home.join("target/a.txt")).unwrap(),
        b"old"
    );

    let (status, body) = run_cmd(&app, "unzip conflict.zip -d target/a.txt", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn archive_unzip_api_extracts_into_named_directory() {
    let app = setup_app().await;
    write_zip(&app.home.join("api.zip"), &[("docs/a.txt", b"hello")]);

    let response = send_json(
        &app.router,
        Method::POST,
        "/api/v1/archive/unzip",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "api.zip"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["name"], "api");
    wait_job(&app, body["job_id"].as_str().unwrap()).await;
    assert_eq!(
        std::fs::read(app.home.join("api/docs/a.txt")).unwrap(),
        b"hello"
    );

    let disabled = send_json(
        &app.router,
        Method::PUT,
        &format!("/api/v1/admin/users/{}", 1),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"feature_unzip": false}),
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::OK);
    write_zip(&app.home.join("second.zip"), &[("b.txt", b"b")]);
    let denied = send_json(
        &app.router,
        Method::POST,
        "/api/v1/archive/unzip",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "second.zip"}),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}
