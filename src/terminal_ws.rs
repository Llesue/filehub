use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::auth::{self, CurrentUser};
use crate::cmdrunner;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Init {
        root: i64,
        cwd: Option<String>,
        csrf: String,
    },
    Exec {
        line: String,
        confirm: Option<bool>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready { cwd: String },
    Result { output: String, cwd: String },
    Error { message: String },
}

pub async fn websocket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !state.settings().settings.terminal_enabled || !preferences.terminal_enabled {
        return Err(AppError::Forbidden);
    }
    Ok(upgrade.on_upgrade(move |socket| session(state, current, headers, socket)))
}

async fn session(
    state: Arc<AppState>,
    current: CurrentUser,
    headers: HeaderMap,
    socket: WebSocket,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut authenticated = false;
    let mut root_id = 0_i64;
    let mut cwd = String::new();

    while let Some(message) = receiver.next().await {
        let Ok(Message::Text(text)) = message else {
            continue;
        };
        let Ok(message) = serde_json::from_str::<ClientMessage>(&text) else {
            let _ = send(
                &mut sender,
                &ServerMessage::Error {
                    message: "invalid terminal message".to_owned(),
                },
            )
            .await;
            continue;
        };

        match message {
            ClientMessage::Init {
                root,
                cwd: requested_cwd,
                csrf,
            } => {
                let expected = current.csrf_token.as_deref().unwrap_or_default();
                if expected.is_empty() || !auth::constant_time_eq(expected, &csrf) {
                    let _ = send(
                        &mut sender,
                        &ServerMessage::Error {
                            message: "invalid CSRF token".to_owned(),
                        },
                    )
                    .await;
                    break;
                }
                let root = match cmdrunner::load_root(&state, current.user.id, root).await {
                    Ok(root) => root,
                    Err(error) => {
                        let _ = send_error(&mut sender, error).await;
                        break;
                    }
                };
                let relative = requested_cwd.unwrap_or_default();
                if let Err(error) =
                    crate::authz::resolve(&state, current.user.id, root.id, &relative, false).await
                {
                    let _ = send_error(&mut sender, error).await;
                    break;
                }
                root_id = root.id;
                cwd = relative;
                authenticated = true;
                let _ = send(&mut sender, &ServerMessage::Ready { cwd: cwd.clone() }).await;
            }
            ClientMessage::Exec { line, confirm } => {
                if !authenticated {
                    let _ = send(
                        &mut sender,
                        &ServerMessage::Error {
                            message: "terminal is not initialized".to_owned(),
                        },
                    )
                    .await;
                    continue;
                }
                let root = match cmdrunner::load_root(&state, current.user.id, root_id).await {
                    Ok(root) => root,
                    Err(error) => {
                        let _ = send_error(&mut sender, error).await;
                        continue;
                    }
                };
                let next_cwd = cwd.clone();
                let is_cd = line.split_whitespace().next() == Some("cd");
                let result = tokio::time::timeout(
                    Duration::from_secs(60),
                    cmdrunner::execute(
                        &state,
                        current.user.id,
                        root,
                        next_cwd.clone(),
                        line,
                        confirm.unwrap_or(false),
                        &headers,
                    ),
                )
                .await;
                match result {
                    Ok(Ok(output)) => {
                        if is_cd {
                            // `cd` returns the new path relative to the authorized root.
                            cwd = output.clone();
                        }
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Result {
                                output,
                                cwd: cwd.clone(),
                            },
                        )
                        .await;
                    }
                    Ok(Err(error)) => {
                        let _ = send_error(&mut sender, error).await;
                    }
                    Err(_) => {
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Error {
                                message: "command timed out".to_owned(),
                            },
                        )
                        .await;
                    }
                }
            }
        }
    }
}

async fn send(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &ServerMessage,
) -> Result<(), axum::Error> {
    let value = serde_json::to_string(message)
        .unwrap_or_else(|_| "{\"type\":\"error\",\"message\":\"encoding failed\"}".to_owned());
    sender.send(Message::Text(value.into())).await
}

async fn send_error(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    error: AppError,
) -> Result<(), axum::Error> {
    send(
        sender,
        &ServerMessage::Error {
            message: error.to_string(),
        },
    )
    .await
}
