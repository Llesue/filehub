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
    let roots = send(&router, Method::GET, "/api/v1/roots", Some(&cookie)).await;
    let root_id = json_body(roots).await["roots"][0]["id"].as_i64().unwrap();
    let home = dir.path().join("users/admin");
    image::RgbImage::from_fn(120, 80, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, 128])
    })
    .save(home.join("photo.png"))
    .unwrap();
    std::fs::write(home.join("fake.heic"), b"not a real heic").unwrap();
    std::fs::write(home.join("broken.png"), b"not a real png").unwrap();
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

async fn send(app: &Router, method: Method, uri: &str, cookie: Option<&str>) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
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

fn count_fail_markers(directory: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                count_fail_markers(&path)
            } else if path.extension().and_then(|value| value.to_str()) == Some("fail") {
                1
            } else {
                0
            }
        })
        .sum()
}

async fn json_body(response: Response<Body>) -> Value {
    let body = to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn async_thumbnails_return_a_placeholder_and_publish_events() {
    let app = setup_app().await;
    let mut events = app.state.events.subscribe();
    let uri = format!("/api/v1/thumb?root={}&path=photo.png&w=64", app.root_id);
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-filehub-thumb-state")
            .unwrap()
            .to_str()
            .unwrap(),
        "pending"
    );
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(body.starts_with(&[0x89, 0x50, 0x4e, 0x47]));

    let ready = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(event) if event.event == "thumbnail.ready" => break event,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("event bus closed")
                }
            }
        }
    })
    .await
    .expect("thumbnail.ready event");
    assert_eq!(ready.data["root"], app.root_id);
    assert_eq!(ready.data["path"], "photo.png");
    assert_eq!(ready.data["width"], 64);

    let sync_uri = format!(
        "/api/v1/thumb?root={}&path=photo.png&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &sync_uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );

    let broken = format!("/api/v1/thumb?root={}&path=broken.png&w=64", app.root_id);
    let response = send(&app.router, Method::GET, &broken, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let failed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(event) if event.event == "thumbnail.failed" => break event,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("event bus closed")
                }
            }
        }
    })
    .await
    .expect("thumbnail.failed event");
    assert_eq!(failed.data["path"], "broken.png");
    let response = send(&app.router, Method::GET, &broken, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn thumbnails_are_generated_cached_and_safe() {
    let app = setup_app().await;
    let uri = format!(
        "/api/v1/thumb?root={}&path=photo.png&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );
    let etag = response
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(body.starts_with(&[0xff, 0xd8]));

    let builder = Request::builder()
        .method(Method::GET)
        .uri(&uri)
        .header(header::COOKIE, &app.cookie)
        .header(header::IF_NONE_MATCH, &etag);
    let response = app
        .router
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);

    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);

    let heic = format!(
        "/api/v1/thumb?root={}&path=fake.heic&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &heic, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let broken = format!(
        "/api/v1/thumb?root={}&path=broken.png&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &broken, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let cache_root = app._dir.path().join("cache/thumbs");
    let failures = count_fail_markers(&cache_root);
    assert!(failures >= 1, "expected a negative thumbnail marker");
    let response = send(&app.router, Method::GET, &broken, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let traversal = format!(
        "/api/v1/thumb?root={}&path=../photo.png&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &traversal, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["error"]["code"], "INVALID_PATH");
    let _ = &app.home;
}

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[tokio::test]
async fn video_thumbnails_fall_back_to_first_frame_and_reject_broken_files() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg is not installed; skipping video thumbnail regression");
        return;
    }
    let app = setup_app().await;
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=10",
            "-t",
            "0.4",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(app.home.join("short.mp4"))
        .status()
        .unwrap();
    assert!(status.success(), "failed to generate the test clip");

    let uri = format!(
        "/api/v1/thumb?root={}&path=short.mp4&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a clip shorter than the seek offset should still return the first frame"
    );
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );

    std::fs::write(app.home.join("broken.mp4"), b"not a video").unwrap();
    let uri = format!(
        "/api/v1/thumb?root={}&path=broken.mp4&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.4",
            "-c:a",
            "aac",
        ])
        .arg(app.home.join("audio-only.mp4"))
        .status()
        .unwrap();
    if status.success() {
        let uri = format!(
            "/api/v1/thumb?root={}&path=audio-only.mp4&w=64&wait=1",
            app.root_id
        );
        let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "audio-only files must not produce a video thumbnail"
        );
    }
}

fn tool_available(command: &str, args: &[&str]) -> bool {
    std::process::Command::new(command)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[tokio::test]
async fn large_and_variant_jpegs_generate_thumbnails() {
    if !tool_available("magick", &["-version"]) {
        eprintln!("ImageMagick is not installed; skipping JPEG variant regression");
        return;
    }
    let app = setup_app().await;
    let variants = [
        ("big.jpg", vec!["-size", "10000x8000", "xc:#3366cc"]),
        (
            "gray.jpg",
            vec![
                "-size",
                "800x600",
                "gradient:black-white",
                "-colorspace",
                "Gray",
            ],
        ),
        (
            "progressive.jpg",
            vec![
                "-size",
                "800x600",
                "gradient:red-blue",
                "-interlace",
                "Plane",
            ],
        ),
        (
            "cmyk.jpg",
            vec![
                "-size",
                "800x600",
                "gradient:red-blue",
                "-colorspace",
                "CMYK",
            ],
        ),
    ];
    for (name, arguments) in variants {
        let status = std::process::Command::new("magick")
            .args(&arguments)
            .arg(app.home.join(name))
            .status()
            .unwrap();
        assert!(status.success(), "failed to generate {name}");
        let uri = format!("/api/v1/thumb?root={}&path={name}&w=64&wait=1", app.root_id);
        let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
        assert_eq!(response.status(), StatusCode::OK, "{name} thumbnail failed");
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
    }
}

#[tokio::test]
async fn thumbnail_failures_report_reason_and_can_be_retried() {
    let app = setup_app().await;
    let uri = format!(
        "/api/v1/thumb?root={}&path=broken.png&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let status_uri = format!(
        "/api/v1/thumb/status?root={}&path=broken.png&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &status_uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let status = json_body(response).await;
    assert_eq!(status["state"], "failed");
    assert_eq!(status["reason"], "decode");

    let retry = send_json(
        &app.router,
        Method::POST,
        "/api/v1/thumb/retry",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "broken.png", "w": 64}),
    )
    .await;
    assert_eq!(retry.status(), StatusCode::OK);
    assert!(
        json_body(retry).await["cleared"].as_u64().unwrap() >= 1,
        "retry should clear the failure marker"
    );

    let response = send(&app.router, Method::GET, &status_uri, Some(&app.cookie)).await;
    assert_eq!(json_body(response).await["state"], "missing");
}

#[tokio::test]
async fn oversized_source_reports_max_source_reason() {
    let app = setup_app().await;
    std::fs::write(app.home.join("too-big.png"), vec![0_u8; 4096]).unwrap();
    let update = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "thumbnails.max_source_bytes", "value": 1024}),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    let uri = format!(
        "/api/v1/thumb?root={}&path=too-big.png&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let status_uri = format!(
        "/api/v1/thumb/status?root={}&path=too-big.png&w=64",
        app.root_id
    );
    let status =
        json_body(send(&app.router, Method::GET, &status_uri, Some(&app.cookie)).await).await;
    assert_eq!(status["state"], "failed");
    assert_eq!(status["reason"], "max_source");
}

#[tokio::test]
async fn thumbnails_sniff_image_format_and_skip_size_limit_for_video() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg is not installed; skipping video size regression");
        return;
    }
    let app = setup_app().await;
    // A PNG saved with a .jpeg name must still decode.
    std::fs::copy(app.home.join("photo.png"), app.home.join("mislabeled.jpeg")).unwrap();
    let uri = format!(
        "/api/v1/thumb?root={}&path=mislabeled.jpeg&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::OK);

    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=10",
            "-t",
            "0.5",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(app.home.join("large.mp4"))
        .status()
        .unwrap();
    assert!(status.success());
    let update = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"key": "thumbnails.max_source_bytes", "value": 1024}),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    let uri = format!(
        "/api/v1/thumb?root={}&path=large.mp4&w=64&wait=1",
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie)).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "videos must not be rejected by the image source-size limit"
    );
}

#[tokio::test]
async fn preview_thumbnails_use_a_reserved_lane() {
    let app = setup_app().await;
    let mut held = Vec::new();
    while let Ok(permit) = app.state.thumb_semaphore.clone().try_acquire_owned() {
        held.push(permit);
    }
    assert!(!held.is_empty(), "list thumbnail lane should have permits");

    let uri = format!(
        "/api/v1/thumb?root={}&path=photo.png&w=2048&wait=1",
        app.root_id
    );
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        send(&app.router, Method::GET, &uri, Some(&app.cookie)),
    )
    .await
    .expect("preview thumbnail must not wait for the list lane");
    assert_eq!(response.status(), StatusCode::OK);
    drop(held);

    let mut preview_held = Vec::new();
    while let Ok(permit) = app
        .state
        .thumb_preview_semaphore
        .clone()
        .try_acquire_owned()
    {
        preview_held.push(permit);
    }
    assert!(!preview_held.is_empty(), "preview lane should have permits");
    let list_uri = format!(
        "/api/v1/thumb?root={}&path=photo.png&w=64&wait=1&v=2",
        app.root_id
    );
    let list_response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        send(&app.router, Method::GET, &list_uri, Some(&app.cookie)),
    )
    .await
    .expect("list thumbnail must not wait for the preview lane");
    assert_eq!(list_response.status(), StatusCode::OK);
    drop(preview_held);
}
