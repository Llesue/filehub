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
    std::fs::create_dir_all(home.join("团队")).unwrap();
    std::fs::write(home.join("alpha.txt"), b"alpha").unwrap();
    std::fs::write(home.join("报告-2026.pdf"), b"report").unwrap();
    std::fs::write(home.join("团队/会议记录.txt"), b"notes").unwrap();

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

async fn search(app: &TestApp, query: &str) -> Vec<String> {
    let uri = format!(
        "/api/v1/search?q={}&root={}",
        percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC),
        app.root_id
    );
    let response = send(&app.router, Method::GET, &uri, Some(&app.cookie), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn index_and_search_finds_english_and_chinese() {
    let app = setup_app().await;

    let index = send_json(
        &app.router,
        Method::POST,
        "/api/v1/index",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "path": "", "recursive": true}),
    )
    .await;
    assert_eq!(index.status(), StatusCode::OK);
    assert!(json_body(index).await["entries"].as_u64().unwrap() >= 3);

    let status = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/index/status?root={}&path=", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(json_body(status).await["status"], "ready");

    assert!(
        search(&app, "alpha")
            .await
            .contains(&"alpha.txt".to_owned())
    );
    assert!(
        search(&app, "会议记录")
            .await
            .contains(&"会议记录.txt".to_owned())
    );
    assert!(
        search(&app, "报告")
            .await
            .contains(&"报告-2026.pdf".to_owned())
    );

    let create = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({
            "root": app.root_id,
            "path": "",
            "filename": "new.txt",
            "total_size": 3,
            "chunk_size": 3
        }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::OK);
    let upload_id = json_body(create).await["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
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
                .body(Body::from(b"new".to_vec()))
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

    let status = send(
        &app.router,
        Method::GET,
        &format!("/api/v1/index/status?root={}&path=", app.root_id),
        Some(&app.cookie),
        None,
    )
    .await;
    assert_eq!(json_body(status).await["status"], "stale");
    assert!(
        search(&app, "alpha")
            .await
            .iter()
            .any(|name| name == "alpha.txt")
    );
    assert!(app.home.join("new.txt").exists());
}
