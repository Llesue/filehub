use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::http::{HeaderMap, header};
use reqwest::{RequestBuilder, StatusCode};
use serde_json::Value;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct PeerError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl PeerError {
    pub fn into_app(self) -> AppError {
        match self.code.as_str() {
            "TOKEN_REVOKED" => AppError::TokenRevoked,
            "TOKEN_EXPIRED" => AppError::TokenExpired,
            "GRANT_REVOKED" => AppError::GrantRevoked,
            _ => match self.status {
                401 => AppError::Unauthorized,
                403 => AppError::Forbidden,
                404 => AppError::NotFound,
                409 => AppError::Conflict(self.message),
                412 => AppError::PreconditionFailed,
                429 => AppError::TooManyRequests,
                _ => AppError::internal(anyhow::anyhow!(self.message)),
            },
        }
    }
}

pub struct PeerClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl PeerClient {
    pub fn new(
        base_url: &str,
        host: &str,
        ip: IpAddr,
        port: u16,
        token: Option<&str>,
    ) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("FileHub")
            .resolve(host, SocketAddr::new(ip, port))
            .build()?;
        Ok(Self {
            base: base_url.trim_end_matches('/').to_owned(),
            token: token.map(str::to_owned),
            http,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/api/federation/v1{path}", self.base)
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    async fn json(&self, request: RequestBuilder) -> Result<Value, PeerError> {
        let response = request.send().await.map_err(|error| PeerError {
            status: 0,
            code: "PEER_UNREACHABLE".to_owned(),
            message: error.to_string(),
        })?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| PeerError {
            status: status.as_u16(),
            code: "PEER_ERROR".to_owned(),
            message: error.to_string(),
        })?;
        if status.is_success() {
            return Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null));
        }
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Err(PeerError {
            status: status.as_u16(),
            code: body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("PEER_ERROR")
                .to_owned(),
            message: body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("peer request failed")
                .to_owned(),
        })
    }

    pub async fn info(&self) -> Result<Value, PeerError> {
        self.json(self.http.get(self.endpoint("/info"))).await
    }

    pub async fn grants(&self) -> Result<Value, PeerError> {
        self.json(self.auth(self.http.get(self.endpoint("/grants"))))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        grant: i64,
        path: &str,
        cursor: Option<&str>,
        limit: usize,
        sort: &str,
        order: &str,
    ) -> Result<Value, PeerError> {
        let mut query = vec![
            ("grant".to_owned(), grant.to_string()),
            ("path".to_owned(), path.to_owned()),
            ("limit".to_owned(), limit.to_string()),
            ("sort".to_owned(), sort.to_owned()),
            ("order".to_owned(), order.to_owned()),
        ];
        if let Some(cursor) = cursor {
            query.push(("cursor".to_owned(), cursor.to_owned()));
        }
        self.json(
            self.auth(self.http.get(self.endpoint("/list")))
                .query(&query),
        )
        .await
    }

    pub async fn stat(&self, grant: i64, path: &str) -> Result<Value, PeerError> {
        self.json(
            self.auth(self.http.get(self.endpoint("/stat")))
                .query(&[("grant", grant.to_string()), ("path", path.to_owned())]),
        )
        .await
    }

    pub async fn download(
        &self,
        grant: i64,
        path: &str,
        inline: bool,
        headers: &HeaderMap,
    ) -> Result<reqwest::Response, PeerError> {
        let mut request = self
            .auth(
                self.http
                    .get(self.endpoint("/download"))
                    .timeout(Duration::from_secs(600)),
            )
            .query(&[
                ("grant", grant.to_string()),
                ("path", path.to_owned()),
                ("inline", if inline { "1" } else { "0" }.to_owned()),
            ]);
        if let Some(range) = headers.get(header::RANGE) {
            request = request.header(header::RANGE, range);
        }
        if let Some(etag) = headers.get(header::IF_NONE_MATCH) {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let response = request.send().await.map_err(|error| PeerError {
            status: 0,
            code: "PEER_UNREACHABLE".to_owned(),
            message: error.to_string(),
        })?;
        let status = response.status();
        if status.is_success()
            || status == StatusCode::NOT_MODIFIED
            || status == StatusCode::RANGE_NOT_SATISFIABLE
        {
            return Ok(response);
        }
        let body = response.bytes().await.unwrap_or_default();
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        Err(PeerError {
            status: status.as_u16(),
            code: body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("PEER_ERROR")
                .to_owned(),
            message: body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("peer download failed")
                .to_owned(),
        })
    }

    pub async fn read_bytes(
        &self,
        grant: i64,
        path: &str,
        max: u64,
    ) -> Result<(Vec<u8>, String), PeerError> {
        let response = self.download(grant, path, true, &HeaderMap::new()).await?;
        if response.content_length().is_some_and(|length| length > max) {
            return Err(PeerError {
                status: 413,
                code: "TOO_LARGE".to_owned(),
                message: "file exceeds the editor size limit".to_owned(),
            });
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = response.bytes().await.map_err(|error| PeerError {
            status: 0,
            code: "PEER_ERROR".to_owned(),
            message: error.to_string(),
        })?;
        if bytes.len() as u64 > max {
            return Err(PeerError {
                status: 413,
                code: "TOO_LARGE".to_owned(),
                message: "file exceeds the editor size limit".to_owned(),
            });
        }
        Ok((bytes.to_vec(), etag))
    }

    pub async fn search(&self, grant: i64, query: &str, limit: usize) -> Result<Value, PeerError> {
        self.json(self.auth(self.http.get(self.endpoint("/search"))).query(&[
            ("grant", grant.to_string()),
            ("q", query.to_owned()),
            ("limit", limit.to_string()),
        ]))
        .await
    }

    pub async fn thumb_warm(
        &self,
        grant: i64,
        paths: &[String],
        width: u32,
    ) -> Result<Value, PeerError> {
        self.json(
            self.auth(self.http.post(self.endpoint("/thumb-warm")))
                .json(&serde_json::json!({
                    "grant": grant,
                    "paths": paths,
                    "width": width,
                })),
        )
        .await
    }

    pub async fn audio_info(&self, grant: i64, path: &str) -> Result<Value, PeerError> {
        self.json(
            self.auth(self.http.get(self.endpoint("/audio-info")))
                .query(&[("grant", grant.to_string()), ("path", path.to_owned())]),
        )
        .await
    }

    pub async fn unzip(
        &self,
        grant: i64,
        path: &str,
        target: Option<&str>,
    ) -> Result<Value, PeerError> {
        let mut body = serde_json::json!({ "grant": grant, "path": path });
        if let Some(target) = target {
            body["target"] = Value::String(target.to_owned());
        }
        self.json(
            self.auth(
                self.http
                    .post(self.endpoint("/unzip"))
                    .timeout(Duration::from_secs(600)),
            )
            .json(&body),
        )
        .await
    }

    pub async fn dir_size(&self, grant: i64, path: &str) -> Result<Value, PeerError> {
        self.json(
            self.auth(self.http.get(self.endpoint("/dir-size")))
                .query(&[("grant", grant.to_string()), ("path", path.to_owned())]),
        )
        .await
    }

    pub async fn thumb(
        &self,
        grant: i64,
        path: &str,
        width: u32,
        edge: u32,
        fit: &str,
    ) -> Result<reqwest::Response, PeerError> {
        let request = self.auth(self.http.get(self.endpoint("/thumb"))).query(&[
            ("grant", grant.to_string()),
            ("path", path.to_owned()),
            ("w", width.to_string()),
            ("edge", edge.to_string()),
            ("fit", fit.to_owned()),
        ]);
        let response = request.send().await.map_err(|error| PeerError {
            status: 0,
            code: "PEER_UNREACHABLE".to_owned(),
            message: error.to_string(),
        })?;
        if response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED {
            return Ok(response);
        }
        let status = response.status();
        let body = response.bytes().await.unwrap_or_default();
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        Err(PeerError {
            status: status.as_u16(),
            code: body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("PEER_ERROR")
                .to_owned(),
            message: body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("peer thumbnail failed")
                .to_owned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_upload(
        &self,
        grant: i64,
        dir: &str,
        filename: &str,
        total_size: i64,
        chunk_size: i64,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<Value, PeerError> {
        let mut body = serde_json::json!({
            "grant": grant,
            "dir": dir,
            "filename": filename,
            "total_size": total_size,
            "chunk_size": chunk_size,
        });
        if let Some(value) = if_match {
            body["if_match"] = Value::String(value.to_owned());
        }
        if let Some(value) = if_none_match {
            body["if_none_match"] = Value::String(value.to_owned());
        }
        self.write("POST", "/uploads", &body).await
    }

    pub async fn put_chunk(&self, id: &str, index: u32, body: Vec<u8>) -> Result<(), PeerError> {
        let request = self
            .auth(
                self.http
                    .put(self.endpoint(&format!("/uploads/{id}/chunks/{index}"))),
            )
            .body(body);
        let response = request.send().await.map_err(|error| PeerError {
            status: 0,
            code: "PEER_UNREACHABLE".to_owned(),
            message: error.to_string(),
        })?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body = response.bytes().await.unwrap_or_default();
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        Err(PeerError {
            status: status.as_u16(),
            code: body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("PEER_ERROR")
                .to_owned(),
            message: body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("peer chunk upload failed")
                .to_owned(),
        })
    }

    pub async fn complete_upload(&self, id: &str) -> Result<Value, PeerError> {
        self.write(
            "POST",
            &format!("/uploads/{id}/complete"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn cancel_upload(&self, id: &str) -> Result<(), PeerError> {
        let response = self
            .auth(self.http.delete(self.endpoint(&format!("/uploads/{id}"))))
            .send()
            .await
            .map_err(|error| PeerError {
                status: 0,
                code: "PEER_UNREACHABLE".to_owned(),
                message: error.to_string(),
            })?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(PeerError {
            status: response.status().as_u16(),
            code: "PEER_ERROR".to_owned(),
            message: "peer cancel upload failed".to_owned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn write(&self, method: &str, path: &str, body: &Value) -> Result<Value, PeerError> {
        let request = match method {
            "POST" => self.http.post(self.endpoint(path)),
            "PUT" => self.http.put(self.endpoint(path)),
            "DELETE" => self.http.delete(self.endpoint(path)),
            _ => self.http.post(self.endpoint(path)),
        };
        self.json(self.auth(request).json(body)).await
    }
}
