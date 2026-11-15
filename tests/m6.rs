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
    std::fs::write(home.join("a.txt"), b"a").unwrap();
    std::fs::write(home.join("b.txt"), b"b").unwrap();
    std::fs::create_dir(home.join("sub")).unwrap();
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

async fn cmd(app: &TestApp, cwd: &str, line: &str, confirm: bool) -> Response<Body> {
    send_json(
        &app.router,
        Method::POST,
        "/api/v1/cmd",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": app.root_id, "cwd": cwd, "line": line, "confirm": confirm}),
    )
    .await
}

#[tokio::test]
async fn safe_command_runner_handles_wildcards_and_confirmation() {
    let app = setup_app().await;

    let pwd = cmd(&app, "", "pwd", false).await;
    assert_eq!(pwd.status(), StatusCode::OK);
    assert_eq!(json_body(pwd).await["output"], "/");

    let ls = cmd(&app, "", "ls", false).await;
    assert_eq!(ls.status(), StatusCode::OK);
    assert!(
        json_body(ls).await["output"]
            .as_str()
            .unwrap()
            .contains("a.txt")
    );

    let cd = cmd(&app, "", "cd sub", false).await;
    assert_eq!(cd.status(), StatusCode::OK);
    assert_eq!(json_body(cd).await["output"], "sub");

    let ls_after_cd = cmd(&app, "sub", "ls", false).await;
    assert_eq!(ls_after_cd.status(), StatusCode::OK);

    let tree = cmd(&app, "", "tree", false).await;
    assert_eq!(tree.status(), StatusCode::OK);
    assert!(
        json_body(tree).await["output"]
            .as_str()
            .unwrap()
            .contains("a.txt")
    );

    let mkdir = cmd(&app, "", "mkdir dest", false).await;
    assert_eq!(mkdir.status(), StatusCode::OK);
    let cp = cmd(&app, "", "cp *.txt dest", false).await;
    assert_eq!(cp.status(), StatusCode::OK);
    assert!(app.home.join("dest/a.txt").exists());
    assert!(app.home.join("dest/b.txt").exists());

    let mv = cmd(&app, "", "mv dest/a.txt dest/c.txt", false).await;
    assert_eq!(mv.status(), StatusCode::OK);
    assert!(app.home.join("dest/c.txt").exists());

    let denied = cmd(&app, "", "rm dest/c.txt", false).await;
    assert_eq!(denied.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(denied).await["error"]["code"], "CONFIRM_REQUIRED");

    let removed = cmd(&app, "", "rm -y dest/c.txt", false).await;
    assert_eq!(removed.status(), StatusCode::OK);
    assert!(!app.home.join("dest/c.txt").exists());

    let inject = cmd(&app, "", "ls; cat /etc/passwd", false).await;
    assert_eq!(inject.status(), StatusCode::BAD_REQUEST);

    let ln = cmd(&app, "", "ln -s a.txt link.txt", false).await;
    assert_eq!(ln.status(), StatusCode::OK);
    assert!(app.home.join("link.txt").is_symlink());
}
