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
        confirm_code: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready {
        cwd: String,
    },
    Result {
        output: String,
        cwd: String,
    },
    Confirm {
        prompt: String,
        code: String,
        expires_at: i64,
    },
    Error {
        message: String,
    },
}

pub async fn websocket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    crate::capabilities::require_terminal(&state, &current.user).await?;
    Ok(upgrade.on_upgrade(move |socket| session(state, current, headers, socket)))
}

async fn session(
    state: Arc<AppState>,
    current: CurrentUser,
    headers: HeaderMap,
    socket: WebSocket,
) {
    let (mut sender, mut receiver) = socket.split();
    let shutdown = state.shutdown.clone();
    let mut authenticated = false;
    let mut root_id = 0_i64;
    let mut cwd = String::new();

    loop {
        let message = tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                close_socket(&mut sender).await;
                break;
            }
            message = receiver.next() => message,
        };
        let Some(message) = message else {
            break;
        };
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
            ClientMessage::Exec {
                line,
                confirm,
                confirm_code,
            } => {
                match crate::capabilities::load_by_id(&state, current.user.id).await {
                    Ok(capabilities) if capabilities.terminal => {}
                    Ok(_) => {
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Error {
                                message: "terminal has been disabled by the administrator"
                                    .to_owned(),
                            },
                        )
                        .await;
                        break;
                    }
                    Err(error) => {
                        let _ = send_error(&mut sender, error).await;
                        break;
                    }
                }
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
                let future = cmdrunner::execute(
                    &state,
                    current.user.id,
                    root,
                    next_cwd.clone(),
                    line.clone(),
                    confirm.unwrap_or(false),
                    confirm_code,
                    &headers,
                );
                let result = tokio::select! {
                    biased;
                    () = shutdown.cancelled() => {
                        close_socket(&mut sender).await;
                        break;
                    }
                    result = async {
                        if cmdrunner::is_long_command(&line) {
                            Ok(future.await)
                        } else {
                            tokio::time::timeout(Duration::from_secs(60), future).await
                        }
                    } => result,
                };
                match result {
                    Ok(Ok(cmdrunner::ExecOutcome::Output(output))) => {
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
                    Ok(Ok(cmdrunner::ExecOutcome::Confirm(prompt))) => {
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Confirm {
                                prompt: prompt.prompt,
                                code: prompt.code,
                                expires_at: prompt.expires_at,
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

async fn close_socket(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>) {
    let _ = tokio::time::timeout(Duration::from_secs(2), sender.send(Message::Close(None))).await;
}
