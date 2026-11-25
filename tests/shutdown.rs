use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct TestApp {
    _dir: tempfile::TempDir,
    router: Router,
    state: Arc<filehub::state::AppState>,
    cookie: String,
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
    let _ = to_bytes(setup.into_body(), 1024 * 1024).await.unwrap();
    TestApp {
        _dir: dir,
        router,
        state,
        cookie,
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

async fn read_headers(stream: &mut TcpStream) -> Vec<u8> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "connection closed before response headers");
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return buffer;
        }
    }
}

#[tokio::test]
async fn sse_connections_survive_graceful_shutdown() {
    let app = setup_app().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let server_shutdown = shutdown.clone();
    let state_shutdown = app.state.shutdown.clone();
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.router
                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            server_shutdown.cancelled().await;
            state_shutdown.cancel();
        })
        .await;
    });

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET /api/v1/events HTTP/1.1\r\nHost: {addr}\r\nCookie: {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
        app.cookie
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let headers = read_headers(&mut stream).await;
    let headers = String::from_utf8_lossy(&headers);
    assert!(
        headers.starts_with("HTTP/1.1 200"),
        "unexpected SSE response: {headers}"
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("graceful shutdown waited on the SSE connection")
        .unwrap();
}

#[tokio::test]
async fn shutdown_cancels_background_jobs() {
    let app = setup_app().await;
    let submitted = app
        .state
        .jobs
        .submit(
            filehub::jobs::JobKind::Index,
            "shutdown-test".to_owned(),
            1,
            json!({ "mode": "test" }),
            1,
            false,
            |ctx| async move {
                ctx.token().cancelled().await;
                Err(filehub::error::AppError::Cancelled)
            },
        )
        .await
        .unwrap();
    let job_id = submitted.job.id;
    assert_eq!(app.state.jobs.cancel_all(), 1);
    app.state.jobs.wait_idle().await;
    let job = app.state.jobs.get(&job_id).await.unwrap().unwrap();
    assert_eq!(job.status, "cancelled");
}
