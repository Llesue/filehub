use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    router: Router,
    cookie: String,
    root_id: i64,
}

#[cfg(unix)]
impl TestApp {
    fn home(&self) -> std::path::PathBuf {
        self._dir.path().join("users/admin")
    }
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
    let roots = send(&router, Method::GET, "/api/v1/roots", Some(&cookie)).await;
    let root_id = json_body(roots).await["roots"][0]["id"].as_i64().unwrap();
    let home = dir.path().join("users/admin");
    image::RgbImage::from_fn(32, 24, |x, y| {
        image::Rgb([(x * 7) as u8, (y * 9) as u8, 100])
    })
    .save(home.join("img.png"))
    .unwrap();
    std::fs::write(home.join("broken.jpg"), b"not a real jpeg").unwrap();
    std::fs::create_dir(home.join("sub")).unwrap();
    TestApp {
        _dir: dir,
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
async fn properties_describe_files_directories_and_images() {
    let app = setup_app().await;
    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/properties?root={}&path=img.png", app.root_id),
        Some(&app.cookie),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["name"], "img.png");
    assert_eq!(body["is_dir"], false);
    assert_eq!(body["mime_type"], "image/png");
    assert_eq!(body["image"]["width"], 32);
    assert_eq!(body["image"]["height"], 24);
    assert_eq!(body["image"]["format"], "png");
    assert!(body["modified"].as_i64().unwrap() > 0);
    #[cfg(unix)]
    assert!(body["mode"].as_str().unwrap().len() == 4);

    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/properties?root={}&path=sub", app.root_id),
        Some(&app.cookie),
    )
    .await;
    let body = json_body(response).await;
    assert_eq!(body["is_dir"], true);
    assert!(body["image"].is_null());
}

#[tokio::test]
async fn properties_handle_damaged_images_and_symlinks() {
    let app = setup_app().await;
    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/properties?root={}&path=broken.jpg", app.root_id),
        Some(&app.cookie),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["name"], "broken.jpg");
    assert!(body["image"].is_null());

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("img.png", app.home().join("link.png")).unwrap();
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/properties?root={}&path=link.png", app.root_id),
            Some(&app.cookie),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["is_symlink"], true);
        assert_eq!(body["symlink_state"], "ok");
    }
}

#[tokio::test]
#[cfg(unix)]
async fn list_reports_symlink_state() {
    let app = setup_app().await;
    std::os::unix::fs::symlink("img.png", app.home().join("good-link.png")).unwrap();
    std::os::unix::fs::symlink("missing-target", app.home().join("bad-link")).unwrap();
    let response = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=", app.root_id),
        Some(&app.cookie),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let entries = body["entries"].as_array().unwrap();
    let good = entries
        .iter()
        .find(|entry| entry["name"] == "good-link.png")
        .unwrap();
    assert_eq!(good["is_symlink"], true);
    assert_eq!(good["symlink_state"], "ok");
    assert_eq!(good["symlink_target"], "img.png");
    assert_eq!(good["is_dir"], false);
    let bad = entries
        .iter()
        .find(|entry| entry["name"] == "bad-link")
        .unwrap();
    assert_eq!(bad["is_symlink"], true);
    assert_eq!(bad["symlink_state"], "broken");
}
