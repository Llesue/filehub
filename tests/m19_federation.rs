use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Instance {
    _dir: tempfile::TempDir,
    base: String,
    router: Router,
    cookie: String,
    csrf: String,
    root_id: i64,
}

async fn start() -> Instance {
    let dir = tempfile::tempdir().unwrap();
    let state = filehub::state::AppState::new_test(dir.path(), "test-token")
        .await
        .unwrap();
    let router = filehub::app::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = router.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, served).await;
    });

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

    let features = send_json(
        &router,
        Method::PUT,
        "/api/v1/admin/users/1",
        Some(&cookie),
        Some(&csrf),
        json!({
            "feature_federation_mount": true,
            "feature_federation_share": true,
        }),
    )
    .await;
    assert_eq!(
        features.status(),
        StatusCode::OK,
        "enabling federation failed"
    );

    state
        .db
        .upsert_settings_batch(vec![
            ("federation.enabled_inbound".to_owned(), "true".to_owned()),
            ("federation.enabled_outbound".to_owned(), "true".to_owned()),
            ("federation.allow_loopback".to_owned(), "true".to_owned()),
            (
                "federation.allow_plaintext_lan".to_owned(),
                "true".to_owned(),
            ),
        ])
        .await
        .unwrap();
    state.reload_settings(true).await.unwrap();

    let roots = send(&router, Method::GET, "/api/v1/roots", Some(&cookie), None).await;
    assert_eq!(roots.status(), StatusCode::OK);
    let roots = json_body(roots).await;
    let root_id = roots["roots"][0]["id"].as_i64().unwrap();

    std::fs::create_dir_all(dir.path().join("users/admin/shared")).unwrap();
    std::fs::write(dir.path().join("users/admin/shared/hello.txt"), b"hello").unwrap();
    std::fs::write(dir.path().join("users/admin/shared/nested.txt"), b"nested").unwrap();

    Instance {
        _dir: dir,
        base: format!("http://{addr}"),
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

async fn send_bytes(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    data: Vec<u8>,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/octet-stream");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
    }
    app.clone()
        .oneshot(builder.body(Body::from(data)).unwrap())
        .await
        .unwrap()
}

fn session_cookie(response: &Response<Body>) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap()
        .to_owned()
}

async fn json_body(response: Response<Body>) -> Value {
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn upload(app: &Instance, root: i64, dir: &str, name: &str, data: &[u8]) -> Response<Body> {
    let created = send_json(
        &app.router,
        Method::POST,
        "/api/v1/uploads",
        Some(&app.cookie),
        Some(&app.csrf),
        json!({"root": root, "path": dir, "filename": name, "total_size": data.len()}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK, "create upload failed");
    let body = json_body(created).await;
    let id = body["upload_id"].as_str().unwrap().to_owned();
    let put = send_bytes(
        &app.router,
        Method::PUT,
        &format!("/api/v1/uploads/{id}/chunks/0"),
        Some(&app.cookie),
        Some(&app.csrf),
        data.to_vec(),
    )
    .await;
    assert!(
        put.status().is_success(),
        "chunk upload failed: {}",
        put.status()
    );
    send_json(
        &app.router,
        Method::POST,
        &format!("/api/v1/uploads/{id}/complete"),
        Some(&app.cookie),
        Some(&app.csrf),
        json!({}),
    )
    .await
}

async fn peer(
    client: &reqwest::Client,
    base: &str,
    method: Method,
    path: &str,
    token: Option<&str>,
    key: Option<&str>,
    body: Option<Value>,
) -> reqwest::Response {
    let mut request = client
        .request(method, format!("{base}/api/federation/v1{path}"))
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    if let Some(body) = body {
        request = request.body(body.to_string());
    }
    request.send().await.unwrap()
}

async fn wait_job(app: &Instance, id: &str) -> Value {
    for _ in 0..100 {
        let response = send(
            &app.router,
            Method::GET,
            &format!("/api/v1/jobs/{id}"),
            Some(&app.cookie),
            None,
        )
        .await;
        let body = json_body(response).await;
        match body["status"].as_str() {
            Some("completed") => return body,
            Some("failed") => panic!("job failed: {body}"),
            Some("cancelled") => panic!("job cancelled: {body}"),
            _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    panic!("job did not finish");
}

async fn peer_json(response: reqwest::Response) -> Value {
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert!(
        status.is_success() || status == StatusCode::MULTI_STATUS,
        "peer returned {status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn federation_contract_round_trip() {
    let peer_instance = start().await;
    let mount_instance = start().await;

    let grant = send_json(
        &peer_instance.router,
        Method::POST,
        "/api/v1/remote/grants",
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"root": peer_instance.root_id, "path": "shared", "label": "Shared", "mode": "rw"}),
    )
    .await;
    assert_eq!(grant.status(), StatusCode::OK);
    let grant = json_body(grant).await;
    let grant_id = grant["id"].as_i64().unwrap();

    let token = send_json(
        &peer_instance.router,
        Method::POST,
        "/api/v1/remote/tokens",
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"name": "contract", "grant_ids": [grant_id]}),
    )
    .await;
    assert_eq!(token.status(), StatusCode::OK);
    let token = json_body(token).await;
    let token_id = token["id"].as_i64().unwrap();
    let token = token["token"].as_str().unwrap().to_owned();

    let mount = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/remote/mounts",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({
            "url": peer_instance.base,
            "token": token.clone(),
            "grant_id": grant_id,
            "alias": "peer",
        }),
    )
    .await;
    assert_eq!(mount.status(), StatusCode::OK, "mount failed");
    let mount = json_body(mount).await;
    let remote_root = mount["root_id"].as_i64().unwrap();

    std::fs::create_dir_all(peer_instance._dir.path().join("users/admin/shared/sub")).unwrap();
    std::fs::write(
        peer_instance
            ._dir
            .path()
            .join("users/admin/shared/sub/attached.txt"),
        b"attached",
    )
    .unwrap();
    let second = send_json(
        &peer_instance.router,
        Method::POST,
        "/api/v1/remote/grants",
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"root": peer_instance.root_id, "path": "shared/sub", "label": "Sub", "mode": "rw"}),
    )
    .await;
    let second_id = json_body(second).await["id"].as_i64().unwrap();
    let update = send_json(
        &peer_instance.router,
        Method::PATCH,
        &format!("/api/v1/remote/tokens/{token_id}"),
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"grant_ids": [grant_id, second_id]}),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);
    let available = send(
        &mount_instance.router,
        Method::GET,
        "/api/v1/remote/mounts/1/grants",
        Some(&mount_instance.cookie),
        None,
    )
    .await;
    let available = json_body(available).await;
    assert_eq!(available["grants"].as_array().unwrap().len(), 2);
    let attach = send_json(
        &mount_instance.router,
        Method::POST,
        &format!("/api/v1/remote/mounts/1/grants/{second_id}"),
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({}),
    )
    .await;
    assert_eq!(attach.status(), StatusCode::OK);
    let attach = json_body(attach).await;
    let attached_root = attach["root_id"].as_i64().unwrap();
    let attached_list = send(
        &mount_instance.router,
        Method::GET,
        &format!("/api/v1/list?root={attached_root}&path="),
        Some(&mount_instance.cookie),
        None,
    )
    .await;
    let attached_list = json_body(attached_list).await;
    assert!(
        attached_list["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"].as_str() == Some("attached.txt"))
    );

    let peer_list = send(
        &peer_instance.router,
        Method::GET,
        &format!("/api/v1/list?root={}&path=", peer_instance.root_id),
        Some(&peer_instance.cookie),
        None,
    )
    .await;
    let peer_list = json_body(peer_list).await;
    let shared_marked = peer_list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"].as_str() == Some("shared"))
        .and_then(|entry| entry["shared"].as_bool());
    assert_eq!(shared_marked, Some(true));

    let delete_shared = send_json(
        &peer_instance.router,
        Method::POST,
        "/api/v1/fs/delete/preview",
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"root": peer_instance.root_id, "paths": ["shared"]}),
    )
    .await;
    assert_eq!(delete_shared.status(), StatusCode::CONFLICT);

    let list = send(
        &mount_instance.router,
        Method::GET,
        &format!("/api/v1/list?root={remote_root}&path="),
        Some(&mount_instance.cookie),
        None,
    )
    .await;
    let list_status = list.status();
    let list = json_body(list).await;
    assert_eq!(list_status, StatusCode::OK, "remote list failed: {list}");
    let names: Vec<&str> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert!(
        names.contains(&"hello.txt"),
        "remote list missing hello.txt"
    );

    let mkdir = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/fs/mkdir",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({"root": remote_root, "path": "", "name": "created"}),
    )
    .await;
    assert_eq!(mkdir.status(), StatusCode::OK);

    let uploaded = upload(
        &mount_instance,
        remote_root,
        "created",
        "note.txt",
        b"remote body",
    )
    .await;
    assert_eq!(uploaded.status(), StatusCode::OK, "remote upload failed");
    let remote_file = peer_instance
        ._dir
        .path()
        .join("users/admin/shared/created/note.txt");
    assert_eq!(std::fs::read(&remote_file).unwrap(), b"remote body");

    let rename = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/fs/rename",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({"root": remote_root, "path": "created/note.txt", "new_name": "renamed.txt"}),
    )
    .await;
    assert_eq!(rename.status(), StatusCode::OK);
    assert!(
        peer_instance
            ._dir
            .path()
            .join("users/admin/shared/created/renamed.txt")
            .is_file()
    );

    let zip = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/download/zip",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({"root": remote_root, "paths": ["created"]}),
    )
    .await;
    assert_eq!(zip.status(), StatusCode::OK, "remote recursive zip failed");
    let bytes = to_bytes(zip.into_body(), 8 * 1024 * 1024).await.unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut found = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if entry.name().unwrap() == "created/renamed.txt" {
            let mut content = String::new();
            std::io::Read::read_to_string(&mut entry, &mut content).unwrap();
            assert_eq!(content, "remote body");
            found = true;
        }
    }
    assert!(found, "zip missing remote file");

    let preview = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/fs/delete/preview",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({"root": remote_root, "paths": ["created/renamed.txt"]}),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    let preview = json_body(preview).await;
    let delete = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/fs/delete",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({
            "token": preview["token"],
            "confirm_code": preview["confirm_code"],
        }),
    )
    .await;
    assert!(
        delete.status().is_success(),
        "delete failed: {}",
        delete.status()
    );
    assert!(
        !peer_instance
            ._dir
            .path()
            .join("users/admin/shared/created/renamed.txt")
            .exists()
    );

    std::fs::write(
        mount_instance._dir.path().join("users/admin/local-up.txt"),
        b"local body",
    )
    .unwrap();
    let cross = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/fs/transfer-jobs",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({
            "source_root": mount_instance.root_id,
            "paths": ["local-up.txt"],
            "dest_root": remote_root,
            "dest_path": "",
            "move_items": false,
        }),
    )
    .await;
    assert_eq!(
        cross.status(),
        StatusCode::OK,
        "cross transfer submit failed"
    );
    let cross = json_body(cross).await;
    let job_id = cross["job_id"].as_str().unwrap();
    wait_job(&mount_instance, job_id).await;
    let remote_copy = peer_instance
        ._dir
        .path()
        .join("users/admin/shared/local-up.txt");
    assert_eq!(std::fs::read(&remote_copy).unwrap(), b"local body");

    std::fs::write(
        mount_instance
            ._dir
            .path()
            .join("users/admin/move-local.txt"),
        b"move body",
    )
    .unwrap();
    let moved = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/fs/transfer-jobs",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({
            "source_root": mount_instance.root_id,
            "paths": ["move-local.txt"],
            "dest_root": remote_root,
            "dest_path": "",
            "move_items": true,
        }),
    )
    .await;
    assert_eq!(moved.status(), StatusCode::OK);
    let moved = json_body(moved).await;
    wait_job(&mount_instance, moved["job_id"].as_str().unwrap()).await;
    assert_eq!(
        std::fs::read(
            peer_instance
                ._dir
                .path()
                .join("users/admin/shared/move-local.txt")
        )
        .unwrap(),
        b"move body"
    );
    assert!(
        !mount_instance
            ._dir
            .path()
            .join("users/admin/move-local.txt")
            .exists()
    );

    let remote_source = peer_instance
        ._dir
        .path()
        .join("users/admin/shared/from-peer.txt");
    std::fs::write(&remote_source, b"peer body").unwrap();
    let back = send_json(
        &mount_instance.router,
        Method::POST,
        "/api/v1/fs/transfer-jobs",
        Some(&mount_instance.cookie),
        Some(&mount_instance.csrf),
        json!({
            "source_root": remote_root,
            "paths": ["from-peer.txt"],
            "dest_root": mount_instance.root_id,
            "dest_path": "",
            "move_items": false,
        }),
    )
    .await;
    assert_eq!(
        back.status(),
        StatusCode::OK,
        "remote to local submit failed"
    );
    let back = json_body(back).await;
    let job_id = back["job_id"].as_str().unwrap();
    wait_job(&mount_instance, job_id).await;
    assert_eq!(
        std::fs::read(mount_instance._dir.path().join("users/admin/from-peer.txt")).unwrap(),
        b"peer body"
    );

    let big_client = reqwest::Client::new();
    let created = big_client
        .post(format!("{}/api/federation/v1/uploads", peer_instance.base))
        .bearer_auth(&token)
        .header("idempotency-key", "big-upload")
        .json(&json!({
            "grant": grant_id,
            "dir": "",
            "filename": "big.bin",
            "total_size": 5 * 1024 * 1024,
            "chunk_size": 4 * 1024 * 1024,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let created = created.json::<Value>().await.unwrap();
    let big_id = created["id"].as_str().unwrap().to_owned();
    let chunk0 = big_client
        .put(format!(
            "{}/api/federation/v1/uploads/{big_id}/chunks/0",
            peer_instance.base
        ))
        .bearer_auth(&token)
        .header("idempotency-key", "big-chunk-0")
        .body(vec![7_u8; 4 * 1024 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(chunk0.status(), StatusCode::NO_CONTENT);
    let chunk1 = big_client
        .put(format!(
            "{}/api/federation/v1/uploads/{big_id}/chunks/1",
            peer_instance.base
        ))
        .bearer_auth(&token)
        .header("idempotency-key", "big-chunk-1")
        .body(vec![8_u8; 1024 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(chunk1.status(), StatusCode::NO_CONTENT);
    let completed = big_client
        .post(format!(
            "{}/api/federation/v1/uploads/{big_id}/complete",
            peer_instance.base
        ))
        .bearer_auth(&token)
        .header("idempotency-key", "big-complete")
        .send()
        .await
        .unwrap();
    assert_eq!(completed.status(), StatusCode::OK);

    let activity = send(
        &peer_instance.router,
        Method::GET,
        "/api/v1/remote/activity",
        Some(&peer_instance.cookie),
        None,
    )
    .await;
    assert_eq!(activity.status(), StatusCode::OK);
    let activity = json_body(activity).await;
    assert!(
        !activity["activity"].as_array().unwrap().is_empty(),
        "peer activity log is empty"
    );
}

#[tokio::test]
async fn federation_security_and_idempotency() {
    let peer_instance = start().await;
    let client = reqwest::Client::new();

    let anonymous = peer(
        &client,
        &peer_instance.base,
        Method::GET,
        "/grants",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let invalid = peer(
        &client,
        &peer_instance.base,
        Method::GET,
        "/grants",
        Some("fhfed_invalid"),
        None,
        None,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);

    let grant = send_json(
        &peer_instance.router,
        Method::POST,
        "/api/v1/remote/grants",
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"root": peer_instance.root_id, "path": "shared", "label": "Shared", "mode": "rw"}),
    )
    .await;
    let grant = json_body(grant).await;
    let grant_id = grant["id"].as_i64().unwrap();

    let created = send_json(
        &peer_instance.router,
        Method::POST,
        "/api/v1/remote/tokens",
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"name": "security", "grant_ids": [grant_id]}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = json_body(created).await;
    let raw = created["token"].as_str().unwrap().to_owned();
    let created_id = created["id"].as_i64().unwrap();

    let me = send(
        &peer_instance.router,
        Method::GET,
        "/api/v1/me",
        Some(&peer_instance.cookie),
        None,
    )
    .await;
    let me = json_body(me).await;
    assert_eq!(me["remote"]["mount"].as_bool(), Some(true));
    assert_eq!(me["remote"]["share"].as_bool(), Some(true));

    std::fs::create_dir_all(peer_instance._dir.path().join("users/admin/shared/sub")).unwrap();
    let second = send_json(
        &peer_instance.router,
        Method::POST,
        "/api/v1/remote/grants",
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"root": peer_instance.root_id, "path": "shared/sub", "label": "Shared 2", "mode": "rw"}),
    )
    .await;
    let second_id = json_body(second).await["id"].as_i64().unwrap();
    let update = send_json(
        &peer_instance.router,
        Method::PATCH,
        &format!("/api/v1/remote/tokens/{created_id}"),
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
        json!({"grant_ids": [second_id]}),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);
    let grants = peer(
        &client,
        &peer_instance.base,
        Method::GET,
        "/grants",
        Some(&raw),
        None,
        None,
    )
    .await;
    let grants = peer_json(grants).await;
    assert_eq!(grants["grants"][0]["id"].as_i64(), Some(second_id));

    let mkdir = peer(
        &client,
        &peer_instance.base,
        Method::POST,
        "/mkdir",
        Some(&raw),
        Some("key-1"),
        Some(json!({"grant": second_id, "path": "idem"})),
    )
    .await;
    let mkdir_status = mkdir.status();
    let first = mkdir.text().await.unwrap();
    assert_eq!(mkdir_status, StatusCode::CREATED, "{first}");

    let replay = peer(
        &client,
        &peer_instance.base,
        Method::POST,
        "/mkdir",
        Some(&raw),
        Some("key-1"),
        Some(json!({"grant": second_id, "path": "idem"})),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::CREATED);
    assert_eq!(replay.text().await.unwrap(), first);

    let mismatch = peer(
        &client,
        &peer_instance.base,
        Method::POST,
        "/mkdir",
        Some(&raw),
        Some("key-1"),
        Some(json!({"grant": second_id, "path": "idem-other"})),
    )
    .await;
    assert_eq!(mismatch.status(), StatusCode::CONFLICT);

    let header = peer(
        &client,
        &peer_instance.base,
        Method::GET,
        "/info",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(
        header
            .headers()
            .get("x-filehub-federation")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );

    let traversal = peer(
        &client,
        &peer_instance.base,
        Method::POST,
        "/mkdir",
        Some(&raw),
        Some("key-traversal"),
        Some(json!({"grant": second_id, "path": "../escape"})),
    )
    .await;
    assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);

    let upload = peer(
        &client,
        &peer_instance.base,
        Method::POST,
        "/uploads",
        Some(&raw),
        Some("key-upload"),
        Some(json!({
            "grant": second_id,
            "dir": "",
            "filename": "guarded.txt",
            "total_size": 3,
            "if_match": "\"stale\"",
        })),
    )
    .await;
    assert_eq!(upload.status(), StatusCode::OK);
    let upload = peer_json(upload).await;
    let upload_id = upload["id"].as_str().unwrap();
    let chunk = client
        .put(format!(
            "{}/api/federation/v1/uploads/{upload_id}/chunks/0",
            peer_instance.base
        ))
        .bearer_auth(&raw)
        .header("idempotency-key", "guard-chunk")
        .body("abc")
        .send()
        .await
        .unwrap();
    assert_eq!(chunk.status(), StatusCode::NO_CONTENT);
    let complete = client
        .post(format!(
            "{}/api/federation/v1/uploads/{upload_id}/complete",
            peer_instance.base
        ))
        .bearer_auth(&raw)
        .header("idempotency-key", "guard-complete")
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::PRECONDITION_FAILED);

    let revoke = send(
        &peer_instance.router,
        Method::DELETE,
        &format!("/api/v1/remote/tokens/{created_id}"),
        Some(&peer_instance.cookie),
        Some(&peer_instance.csrf),
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
    let revoked = peer(
        &client,
        &peer_instance.base,
        Method::GET,
        "/grants",
        Some(&raw),
        None,
        None,
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::GONE);
}
