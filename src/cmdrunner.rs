use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use globset::Glob;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::authz;
use crate::db::Root;
use crate::error::AppError;
use crate::fsops;
use crate::state::AppState;

const MAX_OUTPUT: usize = 1_000_000;
const MAX_EXPANSIONS: usize = 10_000;

#[derive(Debug, Deserialize)]
pub struct CmdRequest {
    root: i64,
    cwd: Option<String>,
    line: String,
    confirm: Option<bool>,
}

pub async fn run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CmdRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    let preferences = state
        .db
        .get_user_preferences(current.user.id)
        .await
        .map_err(AppError::internal)?;
    if !state.settings().settings.terminal_enabled || !preferences.terminal_enabled {
        return Err(AppError::Forbidden);
    }
    let root = load_root(&state, current.user.id, request.root).await?;
    let cwd = request.cwd.unwrap_or_default();
    let output = tokio::time::timeout(
        Duration::from_secs(60),
        execute(
            &state,
            current.user.id,
            root,
            cwd,
            request.line,
            request.confirm.unwrap_or(false),
            &headers,
        ),
    )
    .await
    .map_err(|_| AppError::bad_request("command timed out"))??;

    Ok(Json(json!({ "output": output })))
}

pub(crate) async fn execute(
    state: &Arc<AppState>,
    user_id: i64,
    root: Root,
    cwd: String,
    line: String,
    confirm: bool,
    headers: &HeaderMap,
) -> Result<String, AppError> {
    let tokens = tokenize(&line)?;
    let Some(command) = tokens.first() else {
        return Err(AppError::bad_request("empty command"));
    };
    let (mut flags, mut args) = split_args(&tokens[1..]);
    let command = command.as_str();

    let output = match command {
        "pwd" => {
            if !args.is_empty() {
                return Err(AppError::bad_request("pwd takes no arguments"));
            }
            if cwd.is_empty() {
                "/".to_owned()
            } else {
                format!("/{cwd}")
            }
        }
        "cd" => {
            let path = args
                .first()
                .ok_or_else(|| AppError::bad_request("cd requires a path"))?;
            let resolved =
                authz::resolve(state, user_id, root.id, &join(&cwd, path), false).await?;
            if !resolved.absolute.is_dir() {
                return Err(AppError::bad_request("cd target is not a directory"));
            }
            resolved.relative
        }
        "tree" => {
            let path = args.first().map(String::as_str).unwrap_or("");
            let resolved =
                authz::resolve(state, user_id, root.id, &join(&cwd, path), false).await?;
            let mut output = Vec::new();
            let mut stack = vec![(resolved.relative.clone(), 0_usize)];
            while let Some((relative, depth)) = stack.pop() {
                if depth > 8 || output.len() > 10_000 {
                    continue;
                }
                let directory = authz::resolve(state, user_id, root.id, &relative, false).await?;
                let entries = tokio::task::spawn_blocking({
                    let directory = directory.clone();
                    move || fsops::list_dir(&directory, "name", "asc")
                })
                .await
                .map_err(|err| AppError::internal(anyhow::anyhow!("tree task failed: {err}")))??;
                for entry in entries.into_iter().rev() {
                    let marker = if entry.is_dir { "/" } else { "" };
                    output.push(format!("{}{}{}", "  ".repeat(depth), entry.name, marker));
                    if entry.is_dir {
                        let child = if relative.is_empty() {
                            entry.name
                        } else {
                            format!("{relative}/{}", entry.name)
                        };
                        stack.push((child, depth + 1));
                    }
                }
            }
            truncate(output.join("\n"))
        }
        "ls" => {
            let paths = if args.is_empty() {
                vec![String::new()]
            } else {
                args.clone()
            };
            let mut lines = Vec::new();
            for path in paths {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &path), false).await?;
                let entries = tokio::task::spawn_blocking({
                    let resolved = resolved.clone();
                    move || fsops::list_dir(&resolved, "name", "asc")
                })
                .await
                .map_err(|err| AppError::internal(anyhow::anyhow!("ls task failed: {err}")))??;
                for entry in entries {
                    lines.push(if entry.is_dir {
                        format!("{}/", entry.name)
                    } else {
                        entry.name
                    });
                }
            }
            truncate(lines.join("\n"))
        }
        "mkdir" => {
            let recursive = flags.remove(&'p');
            for path in args {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &path), true).await?;
                let absolute = resolved.absolute.clone();
                tokio::task::spawn_blocking(move || {
                    if recursive {
                        std::fs::create_dir_all(absolute)
                    } else {
                        std::fs::create_dir(absolute)
                    }
                })
                .await
                .map_err(|err| AppError::internal(anyhow::anyhow!("mkdir task failed: {err}")))?
                .map_err(io_error)?;
            }
            String::new()
        }
        "touch" => {
            for path in args {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &path), true).await?;
                let absolute = resolved.absolute.clone();
                tokio::task::spawn_blocking(move || {
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(absolute)
                        .map(|_| ())
                })
                .await
                .map_err(|err| AppError::internal(anyhow::anyhow!("touch task failed: {err}")))?
                .map_err(io_error)?;
            }
            String::new()
        }
        "rm" => {
            if !flags.contains(&'y') && !confirm {
                return Err(AppError::ConfirmRequired);
            }
            if args.is_empty() {
                return Err(AppError::bad_request("rm requires a path"));
            }
            let recursive = flags.contains(&'r');
            let paths = expand_sources(state, user_id, &root, &cwd, args, true).await?;
            for path in paths {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &path), true).await?;
                let absolute = resolved.absolute.clone();
                let relative = resolved.relative.clone();
                tokio::task::spawn_blocking(move || {
                    let metadata = std::fs::symlink_metadata(&absolute)?;
                    if metadata.is_dir() && !metadata.file_type().is_symlink() {
                        if !recursive {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::IsADirectory,
                                "directory requires -r",
                            ));
                        }
                        std::fs::remove_dir_all(&absolute)
                    } else {
                        std::fs::remove_file(&absolute)
                    }
                })
                .await
                .map_err(|err| AppError::internal(anyhow::anyhow!("rm task failed: {err}")))?
                .map_err(io_error)?;
                let _ = (relative,);
            }
            String::new()
        }
        "cp" => {
            let recursive = flags.contains(&'r');
            if args.len() < 2 {
                return Err(AppError::bad_request("cp requires source and destination"));
            }
            let destination = args.pop().unwrap_or_default();
            let sources = expand_sources(state, user_id, &root, &cwd, args, false).await?;
            let dest =
                authz::resolve(state, user_id, root.id, &join(&cwd, &destination), true).await?;
            if sources.len() > 1 && !dest.absolute.is_dir() {
                return Err(AppError::bad_request(
                    "cp with multiple sources requires a directory destination",
                ));
            }
            for source in sources {
                let source =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &source), false).await?;
                let target = if dest.absolute.is_dir() {
                    dest.absolute
                        .join(source.absolute.file_name().unwrap_or_default())
                } else {
                    dest.absolute.clone()
                };
                let source_path = source.absolute.clone();
                tokio::task::spawn_blocking(move || copy_path(&source_path, &target, recursive))
                    .await
                    .map_err(|err| AppError::internal(anyhow::anyhow!("cp task failed: {err}")))?
                    .map_err(io_error)?;
            }
            String::new()
        }
        "mv" => {
            if args.len() < 2 {
                return Err(AppError::bad_request("mv requires source and destination"));
            }
            let destination = args.pop().unwrap_or_default();
            let sources = expand_sources(state, user_id, &root, &cwd, args, true).await?;
            let dest =
                authz::resolve(state, user_id, root.id, &join(&cwd, &destination), true).await?;
            if sources.len() > 1 && !dest.absolute.is_dir() {
                return Err(AppError::bad_request(
                    "mv with multiple sources requires a directory destination",
                ));
            }
            for source in sources {
                let source =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &source), true).await?;
                let target = if dest.absolute.is_dir() {
                    dest.absolute
                        .join(source.absolute.file_name().unwrap_or_default())
                } else {
                    dest.absolute.clone()
                };
                let source_path = source.absolute.clone();
                tokio::task::spawn_blocking(move || move_path(&source_path, &target))
                    .await
                    .map_err(|err| AppError::internal(anyhow::anyhow!("mv task failed: {err}")))?
                    .map_err(io_error)?;
            }
            String::new()
        }
        "ln" => {
            if args.len() != 2 {
                return Err(AppError::bad_request("ln requires target and link name"));
            }
            let target =
                authz::resolve(state, user_id, root.id, &join(&cwd, &args[0]), false).await?;
            if !target.absolute.exists() {
                return Err(AppError::NotFound);
            }
            let link = authz::resolve(state, user_id, root.id, &join(&cwd, &args[1]), true).await?;
            let target_path = target.absolute.clone();
            let link_path = link.absolute.clone();
            let symlink = flags.contains(&'s');
            tokio::task::spawn_blocking(move || {
                if symlink {
                    crate::platform::create_symlink(&target_path, &link_path)
                } else {
                    std::fs::hard_link(target_path, link_path)
                }
            })
            .await
            .map_err(|err| AppError::internal(anyhow::anyhow!("ln task failed: {err}")))?
            .map_err(io_error)?;
            String::new()
        }
        _ => {
            return Err(AppError::bad_request(format!(
                "unsupported command: {command}"
            )));
        }
    };

    audit(state, user_id, &line, &cwd, headers).await;
    Ok(truncate(output))
}

fn split_args(args: &[String]) -> (std::collections::HashSet<char>, Vec<String>) {
    let mut flags = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            for flag in arg.chars().skip(1) {
                flags.insert(flag);
            }
        } else {
            paths.push(arg.clone());
        }
    }
    (flags, paths)
}

async fn expand_sources(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    cwd: &str,
    args: Vec<String>,
    write: bool,
) -> Result<Vec<String>, AppError> {
    let mut result = Vec::new();
    for arg in args {
        if !arg.contains('*') || arg.contains('?') || arg.contains('[') {
            result.push(arg);
            continue;
        }
        if arg.contains("**") {
            return Err(AppError::bad_request("** wildcard is not supported"));
        }
        let (parent, pattern) = arg.rsplit_once('/').unwrap_or(("", &arg));
        let parent_path = join(cwd, parent);
        let resolved = authz::resolve(state, user_id, root.id, &parent_path, write).await?;
        if !resolved.absolute.is_dir() {
            return Err(AppError::bad_request("wildcard parent is not a directory"));
        }
        let matcher = Glob::new(pattern)
            .map_err(|_| AppError::bad_request("invalid wildcard"))?
            .compile_matcher();
        let entries = tokio::task::spawn_blocking({
            let resolved = resolved.clone();
            move || fsops::list_dir(&resolved, "name", "asc")
        })
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("glob task failed: {err}")))??;
        for entry in entries {
            if matcher.is_match(&entry.name) {
                result.push(if parent.is_empty() {
                    entry.name
                } else {
                    format!("{parent}/{}", entry.name)
                });
                if result.len() > MAX_EXPANSIONS {
                    return Err(AppError::bad_request("too many wildcard matches"));
                }
            }
        }
    }
    Ok(result)
}

fn join(cwd: &str, path: &str) -> String {
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        cwd.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn tokenize(line: &str) -> Result<Vec<String>, AppError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            ';' | '|' | '&' | '$' | '>' | '<' | '`' => {
                return Err(AppError::bad_request(
                    "shell metacharacters are not allowed",
                ));
            }
            character if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if escaped || quote.is_some() {
        return Err(AppError::bad_request("unterminated quote"));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn copy_path(source: &Path, target: &Path, recursive: bool) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.is_dir() {
        if !recursive {
            return Err(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                "directory requires -r",
            ));
        }
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_path(&entry.path(), &target.join(entry.file_name()), true)?;
        }
    } else {
        std::fs::copy(source, target)?;
    }
    Ok(())
}

fn move_path(source: &Path, target: &Path) -> std::io::Result<()> {
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(18) => {
            copy_path(source, target, true)?;
            let metadata = std::fs::symlink_metadata(source)?;
            if metadata.is_dir() {
                std::fs::remove_dir_all(source)
            } else {
                std::fs::remove_file(source)
            }
        }
        Err(error) => Err(error),
    }
}

fn truncate(output: String) -> String {
    output.chars().take(MAX_OUTPUT).collect()
}

fn io_error(error: std::io::Error) -> AppError {
    match error.kind() {
        std::io::ErrorKind::NotFound => AppError::bad_request("path not found"),
        std::io::ErrorKind::PermissionDenied => AppError::Forbidden,
        std::io::ErrorKind::NotADirectory => AppError::bad_request("not a directory"),
        std::io::ErrorKind::IsADirectory => AppError::bad_request("is a directory"),
        _ => AppError::internal(anyhow::Error::from(error)),
    }
}

pub(crate) async fn load_root(
    state: &Arc<AppState>,
    user_id: i64,
    root_id: i64,
) -> Result<Root, AppError> {
    state
        .db
        .find_root(root_id, user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::NotFound)
}

async fn audit(state: &Arc<AppState>, user_id: i64, line: &str, cwd: &str, headers: &HeaderMap) {
    let ip = auth::client_ip(headers);
    if let Err(error) = state
        .db
        .audit(
            Some(user_id),
            "command",
            Some(cwd),
            Some(line),
            ip.as_deref(),
        )
        .await
    {
        tracing::warn!(error = %error, "failed to write command audit log");
    }
}
