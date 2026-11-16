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
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = session_cookie(&setup);
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
        cookie,
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
    body: Value,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
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
async fn thumbnails_are_generated_cached_and_safe() {
    let app = setup_app().await;
    let uri = format!("/api/v1/thumb?root={}&path=photo.png&w=64", app.root_id);
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

    let heic = format!("/api/v1/thumb?root={}&path=fake.heic&w=64", app.root_id);
    let response = send(&app.router, Method::GET, &heic, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let broken = format!("/api/v1/thumb?root={}&path=broken.png&w=64", app.root_id);
    let response = send(&app.router, Method::GET, &broken, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let cache_root = app._dir.path().join("cache/thumbs");
    let failures = count_fail_markers(&cache_root);
    assert!(failures >= 1, "expected a negative thumbnail marker");
    let response = send(&app.router, Method::GET, &broken, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let traversal = format!("/api/v1/thumb?root={}&path=../photo.png&w=64", app.root_id);
    let response = send(&app.router, Method::GET, &traversal, Some(&app.cookie)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["error"]["code"], "INVALID_PATH");
    let _ = &app.home;
}
