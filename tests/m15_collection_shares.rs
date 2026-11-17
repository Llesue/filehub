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
    std::fs::write(home.join("one.txt"), b"first").unwrap();
    std::fs::write(home.join("two.txt"), b"second!").unwrap();
    image::RgbImage::from_fn(24, 16, |x, y| {
        image::Rgb([(x * 10) as u8, (y * 15) as u8, 80])
    })
    .save(home.join("pic.png"))
    .unwrap();
    std::fs::write(home.join(".secret"), b"hidden").unwrap();
    std::fs::create_dir(home.join("folder")).unwrap();
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
    ticket: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some(value) = ticket {
        builder = builder.header("x-share-ticket", value);
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

#[tokio::test]
async fn collection_share_exposes_selected_files_only() {
    let app = setup_app().await;
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["one.txt", "two.txt", "pic.png", ".secret"]}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK, "{created:?}");
    let created = json_body(created).await;
    assert_eq!(created["kind"], "collection");
    assert_eq!(created["item_count"], 4);
    let token = created["token"].as_str().unwrap().to_owned();

    let list = send(
        &app.router,
        Method::GET,
        "/api/v1/shares?limit=10",
        Some(&app.cookie),
        None,
    )
    .await;
    let list = json_body(list).await;
    let share = list["shares"]
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["token_hash"].is_null() || share["kind"] == "collection")
        .unwrap();
    assert_eq!(share["kind"], "collection");
    assert_eq!(share["item_count"], 4);

    let meta = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(meta.status(), StatusCode::OK);
    let meta = json_body(meta).await;
    assert_eq!(meta["collection"], true);
    assert_eq!(meta["read_only"], true);
    let entries = meta["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(|entry| entry["is_dir"] == false));
    let names: Vec<&str> = entries
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["one.txt", "two.txt", "pic.png", ".secret"]);
    let one = entries
        .iter()
        .find(|entry| entry["name"] == "one.txt")
        .unwrap();
    let item_id = one["item_id"].as_i64().unwrap();

    let download = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/download?item={item_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(download.into_body(), 1024).await.unwrap().as_ref(),
        b"first"
    );

    let pic = entries
        .iter()
        .find(|entry| entry["name"] == "pic.png")
        .unwrap();
    let pic_id = pic["item_id"].as_i64().unwrap();
    let thumb = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/thumb?item={pic_id}&w=64&wait=1"),
        None,
        None,
    )
    .await;
    assert_eq!(thumb.status(), StatusCode::OK);

    let preflight = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip/preflight"),
        None,
        None,
        json!({}),
    )
    .await;
    let preflight = json_body(preflight).await;
    assert_eq!(preflight["allowed"], true);
    assert_eq!(preflight["files"], 4);
    let expected_bytes: u64 = ["one.txt", "two.txt", "pic.png", ".secret"]
        .iter()
        .map(|name| std::fs::metadata(app.home.join(name)).unwrap().len())
        .sum();
    assert_eq!(preflight["bytes"].as_u64().unwrap(), expected_bytes);

    let zip = send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/public/shares/{token}/zip"),
        None,
        None,
        json!({"item_ids": [item_id, pic_id]}),
    )
    .await;
    assert_eq!(zip.status(), StatusCode::OK);
    let body = to_bytes(zip.into_body(), 1024 * 1024).await.unwrap();
    assert!(body.starts_with(b"PK"));
}

#[tokio::test]
async fn collection_share_rejects_directories_and_requires_item() {
    let app = setup_app().await;
    let rejected = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["one.txt", "folder"]}),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/shares",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "paths": ["one.txt", "two.txt"]}),
    )
    .await;
    let token = json_body(created).await["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let missing = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/public/shares/{token}/download"),
        None,
        None,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
