use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    config_path: std::path::PathBuf,
    router: Router,
}

async fn test_app() -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    let state = filehub::state::AppState::new_test(dir.path(), "test-token")
        .await
        .unwrap();
    TestApp {
        _dir: dir,
        config_path,
        router: filehub::app::router(state),
    }
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
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn setup_login_admin_and_settings_flow() {
    let app = test_app().await;

    let status = send(&app.router, Method::GET, "/api/v1/setup/status", None, None).await;
    assert_eq!(status.status(), StatusCode::OK);
    assert_eq!(json_body(status).await["initialized"], false);

    let wrong = send_json(
        &app.router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"wrong","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let setup = send_json(
        &app.router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let admin_cookie = session_cookie(&setup);
    let setup_body = json_body(setup).await;
    assert_eq!(setup_body["user"]["role"], "admin");
    let admin_csrf = setup_body["csrf_token"].as_str().unwrap().to_owned();

    let setup_again = send_json(
        &app.router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"test-token","username":"admin2","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup_again.status(), StatusCode::CONFLICT);

    let me = send(
        &app.router,
        Method::GET,
        "/api/v1/me",
        Some(&admin_cookie),
        None,
    )
    .await;
    assert_eq!(me.status(), StatusCode::OK);
    let me_body = json_body(me).await;
    assert_eq!(me_body["user"]["username"], "admin");
    assert_eq!(me_body["auth"], "session");

    let settings = send(
        &app.router,
        Method::GET,
        "/api/v1/admin/settings",
        Some(&admin_cookie),
        None,
    )
    .await;
    assert_eq!(settings.status(), StatusCode::OK);
    let settings_body = json_body(settings).await;
    assert_eq!(settings_body["settings"]["thumbnails"]["enabled"], true);
    assert_eq!(settings_body["sources"]["thumbnails.enabled"], "default");

    let updated = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&admin_cookie),
        Some(&admin_csrf),
        json!({"key":"thumbnails.enabled","value":false}),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated_body = json_body(updated).await;
    assert_eq!(updated_body["settings"]["thumbnails"]["enabled"], false);
    assert_eq!(updated_body["sources"]["thumbnails.enabled"], "db");

    let reset = send(
        &app.router,
        Method::DELETE,
        "/api/v1/admin/settings/thumbnails.enabled",
        Some(&admin_cookie),
        Some(&admin_csrf),
    )
    .await;
    assert_eq!(reset.status(), StatusCode::OK);
    let reset_body = json_body(reset).await;
    assert_eq!(reset_body["settings"]["thumbnails"]["enabled"], true);
    assert_eq!(reset_body["sources"]["thumbnails.enabled"], "default");

    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/admin/users",
        Some(&admin_cookie),
        Some(&admin_csrf),
        json!({"username":"alice","password":"correct horse","role":"user"}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);

    let user_login = send_json(
        &app.router,
        Method::POST,
        "/api/v1/login",
        None,
        None,
        json!({"username":"alice","password":"correct horse"}),
    )
    .await;
    assert_eq!(user_login.status(), StatusCode::OK);
    let user_cookie = session_cookie(&user_login);

    let forbidden = send(
        &app.router,
        Method::GET,
        "/api/v1/admin/users",
        Some(&user_cookie),
        None,
    )
    .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let token_created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/auth/tokens",
        Some(&admin_cookie),
        Some(&admin_csrf),
        json!({"name":"test","scope":"read"}),
    )
    .await;
    assert_eq!(token_created.status(), StatusCode::OK);
    let token_body = json_body(token_created).await;
    let api_token = token_body["token"].as_str().unwrap().to_owned();

    let token_me = send(&app.router, Method::GET, "/api/v1/me", None, None).await;
    assert_eq!(token_me.status(), StatusCode::UNAUTHORIZED);

    let token_me = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/me")
                .header(header::AUTHORIZATION, format!("Bearer {api_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(token_me.status(), StatusCode::OK);
    assert_eq!(json_body(token_me).await["auth"], "token");

    let read_token_write = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/v1/admin/settings")
                .header(header::AUTHORIZATION, format!("Bearer {api_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"key":"thumbnails.enabled","value":true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_token_write.status(), StatusCode::FORBIDDEN);

    let logout = send(
        &app.router,
        Method::POST,
        "/api/v1/logout",
        Some(&admin_cookie),
        Some(&admin_csrf),
    )
    .await;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);

    let after_logout = send(
        &app.router,
        Method::GET,
        "/api/v1/me",
        Some(&admin_cookie),
        None,
    )
    .await;
    assert_eq!(after_logout.status(), StatusCode::UNAUTHORIZED);

    let persisted = filehub::state::AppState::new(Some(&app.config_path))
        .await
        .unwrap();
    let persisted_router = filehub::app::router(persisted);
    let login = send_json(
        &persisted_router,
        Method::POST,
        "/api/v1/login",
        None,
        None,
        json!({"username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let persisted_cookie = session_cookie(&login);
    let persisted_settings = send(
        &persisted_router,
        Method::GET,
        "/api/v1/admin/settings",
        Some(&persisted_cookie),
        None,
    )
    .await;
    assert_eq!(persisted_settings.status(), StatusCode::OK);
    assert_eq!(
        json_body(persisted_settings).await["settings"]["thumbnails"]["enabled"],
        true
    );
}

#[tokio::test]
async fn csrf_is_required_for_cookie_mutations() {
    let app = test_app().await;

    let setup = send_json(
        &app.router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = session_cookie(&setup);

    let missing = send(
        &app.router,
        Method::POST,
        "/api/v1/logout",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn user_preferences_default_and_patch_flow() {
    let app = test_app().await;
    let setup = send_json(
        &app.router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = session_cookie(&setup);
    let csrf = json_body(setup).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();

    let preferences = send(
        &app.router,
        Method::GET,
        "/api/v1/me/preferences",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(preferences.status(), StatusCode::OK);
    let preferences = json_body(preferences).await;
    assert_eq!(preferences["preferences"]["theme"], "light");
    assert_eq!(preferences["preferences"]["show_hidden"], true);
    assert_eq!(preferences["preferences"]["upload_conflict"], "rename");
    assert_eq!(preferences["preferences"]["terminal_enabled"], true);

    let updated = send_json(
        &app.router,
        Method::PATCH,
        "/api/v1/me/preferences",
        Some(&cookie),
        Some(&csrf),
        json!({
            "theme": "dark",
            "show_hidden": false,
            "show_thumbnails": false,
            "upload_conflict": "skip",
            "terminal_enabled": false
        }),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated = json_body(updated).await;
    assert_eq!(updated["preferences"]["theme"], "dark");
    assert_eq!(updated["preferences"]["show_hidden"], false);
    assert_eq!(updated["preferences"]["show_thumbnails"], false);
    assert_eq!(updated["preferences"]["upload_conflict"], "skip");
    assert_eq!(updated["preferences"]["terminal_enabled"], false);
}

#[tokio::test]
async fn login_unknown_user_and_wrong_password_look_the_same() {
    let app = test_app().await;
    let setup = send_json(
        &app.router,
        Method::POST,
        "/api/v1/setup",
        None,
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);

    let unknown = send_json(
        &app.router,
        Method::POST,
        "/api/v1/login",
        None,
        None,
        json!({"username":"ghost","password":"whatever"}),
    )
    .await;
    let wrong = send_json(
        &app.router,
        Method::POST,
        "/api/v1/login",
        None,
        None,
        json!({"username":"admin","password":"whatever"}),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(unknown).await, json_body(wrong).await);

    assert!(!filehub::auth::verify_password(
        "whatever",
        filehub::auth::DUMMY_PASSWORD_HASH
    ));
}
