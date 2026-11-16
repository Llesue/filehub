use std::net::SocketAddr;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
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
        None,
        json!({"token":"test-token","username":"admin","password":"correct horse"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = session_cookie(&setup);
    let body = json_body(setup).await;
    let csrf = body["csrf_token"].as_str().unwrap().to_owned();
    let roots = send(
        &router,
        Method::GET,
        "/api/v1/roots",
        Some(&cookie),
        None,
        None,
    )
    .await;
    let root_id = json_body(roots).await["roots"][0]["id"].as_i64().unwrap();

    TestApp {
        _dir: dir,
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
    peer: Option<SocketAddr>,
) -> Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    if let Some(value) = csrf {
        builder = builder.header("x-csrf-token", value);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    if let Some(peer) = peer {
        request.extensions_mut().insert(ConnectInfo(peer));
    }
    app.clone().oneshot(request).await.unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn send_json(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    peer: Option<SocketAddr>,
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
    let mut request = builder.body(Body::from(body.to_string())).unwrap();
    if let Some(peer) = peer {
        request.extensions_mut().insert(ConnectInfo(peer));
    }
    app.clone().oneshot(request).await.unwrap()
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

async fn mkdir(app: &TestApp, name: &str, peer: SocketAddr, forwarded: Option<&str>) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/fs/mkdir")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &app.cookie)
        .header("x-csrf-token", &app.csrf);
    if let Some(value) = forwarded {
        builder = builder.header("x-forwarded-for", value);
    }
    let request = builder
        .body(Body::from(
            json!({"root": app.root_id, "path": "", "name": name}).to_string(),
        ))
        .unwrap();
    let mut request = request;
    request.extensions_mut().insert(ConnectInfo(peer));
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "mkdir {name} failed");
}

fn peer(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse().unwrap(), port)
}

async fn audit_ip(app: &TestApp, path: &str) -> Option<String> {
    app.state.flush_audit().await;
    let response = send(
        &app.router,
        Method::GET,
        "/api/v1/admin/audit?limit=100",
        Some(&app.cookie),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["action"] == "mkdir" && entry["path"] == path)
        .and_then(|entry| entry["ip"].as_str())
        .map(str::to_owned)
}

#[tokio::test]
async fn audit_ip_uses_peer_unless_trusted_and_ignores_spoofed_headers() {
    let app = setup_app().await;

    mkdir(&app, "direct-ip", peer("10.0.0.9", 50000), Some("1.2.3.4")).await;
    assert_eq!(
        audit_ip(&app, "direct-ip").await.as_deref(),
        Some("10.0.0.9")
    );

    let update = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        None,
        json!({"key": "security.trusted_proxies", "value": ["10.0.0.0/24", "::1"]}),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    mkdir(
        &app,
        "trusted-proxy",
        peer("10.0.0.9", 50001),
        Some("1.2.3.4"),
    )
    .await;
    assert_eq!(
        audit_ip(&app, "trusted-proxy").await.as_deref(),
        Some("1.2.3.4")
    );

    mkdir(&app, "trusted-v6", peer("::1", 50002), Some("2001:db8::7")).await;
    assert_eq!(
        audit_ip(&app, "trusted-v6").await.as_deref(),
        Some("2001:db8::7")
    );

    mkdir(
        &app,
        "mapped-v4",
        peer("::ffff:10.0.0.9", 50003),
        Some("5.6.7.8"),
    )
    .await;
    assert_eq!(
        audit_ip(&app, "mapped-v4").await.as_deref(),
        Some("5.6.7.8")
    );

    mkdir(
        &app,
        "garbage-forwarded",
        peer("10.0.0.9", 50004),
        Some("not-an-ip"),
    )
    .await;
    assert_eq!(
        audit_ip(&app, "garbage-forwarded").await.as_deref(),
        Some("10.0.0.9")
    );

    mkdir(
        &app,
        "untrusted-peer",
        peer("192.0.2.10", 50005),
        Some("1.2.3.4"),
    )
    .await;
    assert_eq!(
        audit_ip(&app, "untrusted-peer").await.as_deref(),
        Some("192.0.2.10")
    );

    let no_peer = send_json(
        &app.router,
        Method::POST,
        "/api/v1/fs/mkdir",
        Some(&app.cookie),
        Some(&app.csrf),
        None,
        json!({"root": app.root_id, "path": "", "name": "no-peer"}),
    )
    .await;
    assert_eq!(no_peer.status(), StatusCode::OK);
    assert_eq!(audit_ip(&app, "no-peer").await, None);
}

#[tokio::test]
async fn invalid_trusted_proxy_configuration_is_rejected() {
    let app = setup_app().await;
    let response = send_json(
        &app.router,
        Method::PUT,
        "/api/v1/admin/settings",
        Some(&app.cookie),
        Some(&app.csrf),
        None,
        json!({"key": "security.trusted_proxies", "value": ["10.0.0.0/99"]}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
