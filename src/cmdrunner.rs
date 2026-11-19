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
    confirm_code: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfirmPrompt {
    pub prompt: String,
    pub code: String,
    pub expires_at: i64,
}

pub enum ExecOutcome {
    Output(String),
    Confirm(ConfirmPrompt),
}

pub async fn run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CmdRequest>,
) -> Result<Json<Value>, AppError> {
    let current = auth::current_user(&state, &headers).await?;
    auth::require_csrf(&current, &headers)?;
    crate::capabilities::require_terminal(&state, &current.user).await?;
    let root = load_root(&state, current.user.id, request.root).await?;
    let cwd = request.cwd.unwrap_or_default();
    let long = is_long_command(&request.line);
    let future = execute(
        &state,
        current.user.id,
        root,
        cwd,
        request.line,
        request.confirm.unwrap_or(false),
        request.confirm_code,
        &headers,
    );
    let outcome = if long {
        future.await?
    } else {
        tokio::time::timeout(Duration::from_secs(60), future)
            .await
            .map_err(|_| AppError::bad_request("command timed out"))??
    };

    Ok(match outcome {
        ExecOutcome::Output(output) => Json(json!({ "output": output })),
        ExecOutcome::Confirm(prompt) => Json(json!({ "output": "", "confirm": prompt })),
    })
}

pub(crate) fn is_long_command(line: &str) -> bool {
    let command = line.split_whitespace().next().unwrap_or_default();
    matches!(command, "rm" | "cp" | "mv" | "zip" | "unzip")
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    state: &Arc<AppState>,
    user_id: i64,
    root: Root,
    cwd: String,
    line: String,
    confirm: bool,
    confirm_code: Option<String>,
    headers: &HeaderMap,
) -> Result<ExecOutcome, AppError> {
    let user = state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    let tokens = tokenize(&line)?;
    let Some(command) = tokens.first() else {
        return Err(AppError::bad_request("empty command"));
    };
    let (mut flags, mut args) = split_args(&tokens[1..]);
    let command = command.as_str();

    if command == "help" {
        return Ok(ExecOutcome::Output(help_text(
            args.first().map(String::as_str),
        )));
    }
    if tokens[1..]
        .iter()
        .any(|token| token == "-h" || token == "--help")
    {
        return Ok(ExecOutcome::Output(help_text(Some(command))));
    }

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
                    let line = format!("{}{}{}", "  ".repeat(depth), entry.name, marker);
                    output.push(if entry.is_dir {
                        format!("\x1b[34m{line}\x1b[0m")
                    } else {
                        line
                    });
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
            let paths = if paths.iter().any(|path| path.contains('*')) {
                expand_sources(state, user_id, &root, &cwd, paths, false).await?
            } else {
                paths
            };
            let multiple = paths.len() > 1;
            let mut lines = Vec::new();
            let mut total_dirs = 0_usize;
            let mut total_files = 0_usize;
            let mut total_bytes = 0_u64;
            let mut total_unknown = 0_usize;
            for path in paths {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &path), false).await?;
                if resolved.absolute.is_file() {
                    let metadata = std::fs::metadata(&resolved.absolute).map_err(|err| {
                        AppError::internal(anyhow::anyhow!("ls task failed: {err}"))
                    })?;
                    let name = resolved
                        .absolute
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("file");
                    lines.push(format!(
                        "{:>9}  {:16}  {}",
                        format_size(metadata.len()),
                        format_local_time(
                            metadata
                                .modified()
                                .ok()
                                .and_then(|time| {
                                    time.duration_since(std::time::UNIX_EPOCH)
                                        .ok()
                                        .map(|duration| duration.as_secs() as i64)
                                })
                                .unwrap_or_default()
                        ),
                        name
                    ));
                    total_files += 1;
                    total_bytes = total_bytes.saturating_add(metadata.len());
                    continue;
                }
                let entries = tokio::task::spawn_blocking({
                    let resolved = resolved.clone();
                    move || fsops::list_dir(&resolved, "name", "asc")
                })
                .await
                .map_err(|err| AppError::internal(anyhow::anyhow!("ls task failed: {err}")))??;
                let dir_paths: Vec<String> = entries
                    .iter()
                    .filter(|entry| entry.is_dir)
                    .map(|entry| {
                        resolved
                            .absolute
                            .join(&entry.name)
                            .to_string_lossy()
                            .to_string()
                    })
                    .collect();
                let dir_stats = if dir_paths.is_empty() {
                    std::collections::HashMap::new()
                } else {
                    state
                        .db
                        .dir_index_stats(dir_paths)
                        .await
                        .map_err(AppError::internal)?
                };
                if multiple {
                    lines.push(format!(
                        "{}:",
                        if resolved.relative.is_empty() {
                            "/".to_owned()
                        } else {
                            format!("/{}", resolved.relative)
                        }
                    ));
                }
                lines.push(format!(
                    "\x1b[1m{:>9}  {:16}  {}\x1b[0m",
                    "SIZE", "MODIFIED", "NAME"
                ));
                let mut dirs = 0_usize;
                let mut files = 0_usize;
                let mut bytes = 0_u64;
                let mut unknown = 0_usize;
                for entry in entries {
                    let stat = if entry.is_dir {
                        dir_stats.get(
                            &resolved
                                .absolute
                                .join(&entry.name)
                                .to_string_lossy()
                                .to_string(),
                        )
                    } else {
                        None
                    };
                    let size = if entry.is_dir {
                        stat.and_then(|value| value.size)
                    } else {
                        Some(entry.size.max(0) as u64)
                    };
                    if let Some(size) = size {
                        bytes = bytes.saturating_add(size);
                        if entry.is_dir {
                            dirs += 1;
                        } else {
                            files += 1;
                        }
                        let name = if entry.is_dir {
                            format!("\x1b[34m{}/\x1b[0m", entry.name)
                        } else {
                            entry.name.clone()
                        };
                        lines.push(format!(
                            "{:>9}  {:16}  {}",
                            format_size(size),
                            format_local_time(entry.modified),
                            name
                        ));
                    } else {
                        unknown += 1;
                        dirs += 1;
                        lines.push(format!(
                            "{:>9}  {:16}  \x1b[34m{}/\x1b[0m",
                            "-",
                            format_local_time(entry.modified),
                            entry.name
                        ));
                    }
                }
                let suffix = if unknown > 0 {
                    format!("（{unknown} 个目录大小未知）")
                } else {
                    String::new()
                };
                lines.push(format!(
                    "\x1b[90m{} 个目录，{} 个文件，共计 {}{}\x1b[0m",
                    dirs,
                    files,
                    format_size(bytes),
                    suffix
                ));
                total_dirs += dirs;
                total_files += files;
                total_bytes = total_bytes.saturating_add(bytes);
                total_unknown += unknown;
            }
            if multiple {
                let suffix = if total_unknown > 0 {
                    format!("（{total_unknown} 个目录大小未知）")
                } else {
                    String::new()
                };
                lines.push(format!(
                    "\x1b[90m全部：{} 个目录，{} 个文件，共计 {}{}\x1b[0m",
                    total_dirs,
                    total_files,
                    format_size(total_bytes),
                    suffix
                ));
            }
            truncate(lines.join("\n"))
        }
        "roots" => {
            let roots = state
                .db
                .list_roots(user_id)
                .await
                .map_err(AppError::internal)?;
            roots
                .iter()
                .map(|root| {
                    format!(
                        "\x1b[36m@{}\x1b[0m  {}  {}  {}",
                        root.id,
                        root.alias,
                        root.real_path,
                        if root.read_only { "ro" } else { "rw" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "mkdir" => {
            let recursive = flags.remove(&'p');
            for path in args {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &path), true).await?;
                if std::fs::symlink_metadata(&resolved.absolute).is_err() {
                    crate::usage::ensure_entry_capacity(state, &user, 1).await?;
                }
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
                state.adjust_entries(user_id, 1);
            }
            String::new()
        }
        "touch" => {
            for path in args {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &path), true).await?;
                match std::fs::symlink_metadata(&resolved.absolute) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(AppError::invalid_path(
                            "touch does not follow symbolic links",
                        ));
                    }
                    Ok(_) => continue,
                    Err(_) => {}
                }
                crate::usage::ensure_entry_capacity(state, &user, 1).await?;
                let absolute = resolved.absolute.clone();
                tokio::task::spawn_blocking(move || fsops::touch_file(&absolute))
                    .await
                    .map_err(|err| {
                        AppError::internal(anyhow::anyhow!("touch task failed: {err}"))
                    })??;
                state.adjust_entries(user_id, 1);
            }
            String::new()
        }
        "rm" => {
            if args.is_empty() {
                return Err(AppError::bad_request("rm requires a path"));
            }
            let recursive = flags.contains(&'r');
            let paths = expand_sources(state, user_id, &root, &cwd, args, true).await?;
            let mut items = Vec::with_capacity(paths.len());
            for path in &paths {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, path), true).await?;
                if resolved.relative.is_empty() {
                    return Err(AppError::invalid_path(
                        "the root directory cannot be removed",
                    ));
                }
                let metadata = std::fs::symlink_metadata(&resolved.absolute)
                    .map_err(|_| AppError::NotFound)?;
                let is_dir = metadata.is_dir() && !metadata.file_type().is_symlink();
                if is_dir && !recursive {
                    return Err(AppError::bad_request("directory requires -r"));
                }
                let size = if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                };
                items.push((resolved, is_dir, size));
            }
            let candidates: Vec<(bool, u64)> = items
                .iter()
                .map(|(_, is_dir, size)| (*is_dir, *size))
                .collect();
            let needs_code = crate::files::delete_requires_code(&candidates);
            if needs_code {
                let key = format!("{user_id}:{}:{cwd}", root.id);
                let targets: Vec<String> = items
                    .iter()
                    .map(|(item, _, _)| item.relative.clone())
                    .collect();
                if let Some(code) = confirm_code.as_deref() {
                    let mut pending = state.cmd_confirmations.lock();
                    let Some(entry) = pending.get_mut(&key) else {
                        return Err(AppError::bad_request(
                            "confirmation code is missing or expired",
                        ));
                    };
                    if entry.expires_at <= crate::db::unix_now() {
                        pending.remove(&key);
                        return Err(AppError::bad_request("confirmation code has expired"));
                    }
                    if entry.targets != targets {
                        pending.remove(&key);
                        return Err(AppError::bad_request("confirmation targets have changed"));
                    }
                    if !auth::constant_time_eq(&entry.code, code) {
                        entry.attempts = entry.attempts.saturating_add(1);
                        let attempts = entry.attempts;
                        if attempts >= 3 {
                            pending.remove(&key);
                        }
                        return Err(AppError::bad_request("invalid confirmation code"));
                    }
                    pending.remove(&key);
                } else {
                    let random = auth::sha256_hex(&auth::random_token(8));
                    let code = format!(
                        "{:04}",
                        u32::from_str_radix(&random[..8], 16).unwrap_or_default() % 10_000
                    );
                    let expires_at = crate::db::unix_now() + 60;
                    state.cmd_confirmations.lock().insert(
                        key,
                        crate::state::CmdConfirmation {
                            user_id,
                            root_id: root.id,
                            cwd: cwd.clone(),
                            code: code.clone(),
                            targets,
                            expires_at,
                            attempts: 0,
                        },
                    );
                    return Ok(ExecOutcome::Confirm(ConfirmPrompt {
                        prompt: format!(
                            "高危删除：将删除 {} 项。请输入验证码 {code} 确认（60 秒内有效）",
                            items.len()
                        ),
                        code,
                        expires_at,
                    }));
                }
            } else if !flags.contains(&'y') && !confirm {
                return Err(AppError::ConfirmRequired);
            }
            let mut cache_targets = Vec::with_capacity(items.len());
            for (resolved, _, _) in &items {
                let absolute = resolved.absolute.clone();
                let relative = resolved.relative.clone();
                tokio::task::spawn_blocking(move || fsops::remove_path(&absolute, &relative))
                    .await
                    .map_err(|err| {
                        AppError::internal(anyhow::anyhow!("rm task failed: {err}"))
                    })??;
                cache_targets.push(resolved.absolute.clone());
            }
            state
                .invalidate_paths_exact(&cache_targets, Some(user_id))
                .await;
            state.schedule_cache_cleanup(cache_targets);
            state.adjust_entries(user_id, -(items.len() as i64));
            String::new()
        }
        "cp" => {
            let recursive = flags.contains(&'r');
            if args.len() < 2 {
                return Err(AppError::bad_request("cp requires source and destination"));
            }
            let destination = args.pop().unwrap_or_default();
            if let Some(output) = cross_root_transfer(
                state,
                user_id,
                &root,
                &cwd,
                &args,
                &destination,
                recursive,
                false,
                &line,
                headers,
            )
            .await?
            {
                return Ok(ExecOutcome::Output(output));
            }
            let sources = expand_sources(state, user_id, &root, &cwd, args, false).await?;
            let dest =
                authz::resolve(state, user_id, root.id, &join(&cwd, &destination), true).await?;
            if sources.len() > 1 && !dest.absolute.is_dir() {
                return Err(AppError::bad_request(
                    "cp with multiple sources requires a directory destination",
                ));
            }
            let mut wanted = 0_u64;
            for source in &sources {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, source), false).await?;
                wanted =
                    wanted.saturating_add(fsops::count_entries_limited(&resolved.absolute, 10_000));
            }
            crate::usage::ensure_entry_capacity(state, &user, wanted).await?;
            for source in sources {
                let source =
                    authz::resolve(state, user_id, root.id, &join(&cwd, &source), false).await?;
                if source.relative.is_empty() {
                    return Err(AppError::invalid_path(
                        "the root directory cannot be copied",
                    ));
                }
                let target = if dest.absolute.is_dir() {
                    dest.absolute
                        .join(source.absolute.file_name().unwrap_or_default())
                } else {
                    dest.absolute.clone()
                };
                if source.absolute.is_dir()
                    && (target == source.absolute || target.starts_with(&source.absolute))
                {
                    return Err(AppError::invalid_path(
                        "cannot copy a directory into itself",
                    ));
                }
                let source_path = source.absolute.clone();
                tokio::task::spawn_blocking(move || copy_path(&source_path, &target, recursive))
                    .await
                    .map_err(|err| AppError::internal(anyhow::anyhow!("cp task failed: {err}")))?
                    .map_err(io_error)?;
            }
            state.adjust_entries(user_id, wanted as i64);
            String::new()
        }
        "mv" => {
            if args.len() < 2 {
                return Err(AppError::bad_request("mv requires source and destination"));
            }
            let destination = args.pop().unwrap_or_default();
            if let Some(output) = cross_root_transfer(
                state,
                user_id,
                &root,
                &cwd,
                &args,
                &destination,
                true,
                true,
                &line,
                headers,
            )
            .await?
            {
                return Ok(ExecOutcome::Output(output));
            }
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
                if source.relative.is_empty() {
                    return Err(AppError::invalid_path("the root directory cannot be moved"));
                }
                let target = if dest.absolute.is_dir() {
                    dest.absolute
                        .join(source.absolute.file_name().unwrap_or_default())
                } else {
                    dest.absolute.clone()
                };
                if source.absolute.is_dir()
                    && (target == source.absolute || target.starts_with(&source.absolute))
                {
                    return Err(AppError::invalid_path(
                        "cannot move a directory into itself",
                    ));
                }
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
            crate::usage::ensure_entry_capacity(state, &user, 1).await?;
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
            state.adjust_entries(user_id, 1);
            String::new()
        }
        "job" | "jobs" => {
            let jobs = state
                .jobs
                .list_for_user(user_id, 0)
                .await
                .map_err(AppError::internal)?;
            match args.first().map(String::as_str) {
                Some(id) if id.starts_with("job_") => {
                    let Some(job) = state.jobs.get(id).await.map_err(AppError::internal)? else {
                        return Err(AppError::NotFound);
                    };
                    if job.user_id != user_id {
                        return Err(AppError::NotFound);
                    }
                    format_job_detail(&job)
                }
                Some(limit) => {
                    let limit: usize = limit
                        .parse()
                        .map_err(|_| AppError::bad_request("job expects a count or job id"))?;
                    format_job_list(
                        &jobs
                            .into_iter()
                            .take(limit.clamp(1, 50))
                            .collect::<Vec<_>>(),
                    )
                }
                None => format_job_list(&jobs.into_iter().take(10).collect::<Vec<_>>()),
            }
        }
        "zip" => {
            if args.len() < 2 {
                return Err(AppError::bad_request(
                    "zip requires an archive name and at least one source",
                ));
            }
            crate::capabilities::require_zip(state, &user).await?;
            let recursive = flags.contains(&'r');
            let mut name = args.remove(0);
            if !name.to_ascii_lowercase().ends_with(".zip") {
                name.push_str(".zip");
            }
            if name.contains('/') || name.contains('\\') || name.contains('\0') {
                return Err(AppError::invalid_path("invalid archive name"));
            }
            let name = crate::files::validate_filename(&name)?;
            let sources = expand_sources(state, user_id, &root, &cwd, args, false).await?;
            if sources.is_empty() {
                return Err(AppError::bad_request("no sources to archive"));
            }
            let base = if cwd.is_empty() {
                String::new()
            } else {
                format!("{cwd}/")
            };
            let mut names = Vec::with_capacity(sources.len());
            for source in &sources {
                let resolved =
                    authz::resolve(state, user_id, root.id, &join(&cwd, source), false).await?;
                if resolved.relative.is_empty() {
                    return Err(AppError::invalid_path("cannot archive the root directory"));
                }
                if resolved.absolute.is_dir() && !recursive {
                    return Err(AppError::bad_request("directory requires -r"));
                }
                let relative = resolved
                    .relative
                    .strip_prefix(&base)
                    .ok_or_else(|| {
                        AppError::invalid_path("source is outside the current directory")
                    })?
                    .to_owned();
                if relative.is_empty() {
                    return Err(AppError::invalid_path("invalid archive source"));
                }
                names.push(relative);
            }
            let outcome = crate::files::submit_zip_job(
                state,
                user_id,
                root.id,
                cwd.clone(),
                names,
                name.clone(),
                sources.len(),
            )
            .await?;
            format!(
                "queued archive job {}: {}\nuse `job` to check progress",
                outcome.job.id, name
            )
        }
        "unzip" => {
            crate::capabilities::require_unzip(state, &user).await?;
            let mut archive = None;
            let mut target = None;
            let mut rest = tokens[1..].iter();
            while let Some(token) = rest.next() {
                if token == "-d" {
                    target = Some(
                        rest.next()
                            .cloned()
                            .ok_or_else(|| AppError::bad_request("unzip -d requires a target"))?,
                    );
                } else if archive.is_none() {
                    archive = Some(token.clone());
                } else {
                    return Err(AppError::bad_request("unzip takes one archive"));
                }
            }
            let archive =
                archive.ok_or_else(|| AppError::bad_request("unzip requires an archive"))?;
            let archive_resolved =
                authz::resolve(state, user_id, root.id, &join(&cwd, &archive), false).await?;
            if !archive_resolved.absolute.is_file() {
                return Err(AppError::NotFound);
            }
            let (target_relative, existing) = if let Some(target) = target {
                if target == "." {
                    (cwd.clone(), true)
                } else {
                    let resolved =
                        authz::resolve(state, user_id, root.id, &join(&cwd, &target), true).await?;
                    let exists = std::fs::symlink_metadata(&resolved.absolute).is_ok();
                    if exists && !resolved.absolute.is_dir() {
                        return Err(AppError::bad_request("target is a file, not a directory"));
                    }
                    (resolved.relative, exists)
                }
            } else {
                let file_name = archive_resolved
                    .absolute
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| AppError::invalid_path("invalid archive name"))?;
                let stem = file_name
                    .strip_suffix(".zip")
                    .or_else(|| file_name.strip_suffix(".ZIP"))
                    .unwrap_or(file_name);
                let parent = archive_resolved
                    .relative
                    .rsplit_once('/')
                    .map(|(parent, _)| parent.to_owned())
                    .unwrap_or_default();
                let target = if parent.is_empty() {
                    stem.to_owned()
                } else {
                    format!("{parent}/{stem}")
                };
                let resolved = authz::resolve(state, user_id, root.id, &target, true).await?;
                (resolved.relative, false)
            };
            if existing {
                let archive_path = archive_resolved.absolute.clone();
                let target_path = authz::resolve(state, user_id, root.id, &target_relative, true)
                    .await?
                    .absolute;
                tokio::task::spawn_blocking(move || {
                    crate::unzip::ensure_no_conflicts(&archive_path, &target_path)
                })
                .await
                .map_err(|err| {
                    AppError::internal(anyhow::anyhow!("unzip preflight task failed: {err}"))
                })??;
            }
            let outcome = crate::unzip::submit_unzip_job(
                state,
                user_id,
                root.id,
                archive_resolved.relative,
                target_relative.clone(),
                existing,
                target_relative.clone(),
            )
            .await?;
            let job = state
                .jobs
                .wait_for(&outcome.job.id)
                .await
                .ok_or(AppError::NotFound)?;
            match job.status.as_str() {
                "completed" => {
                    let path = job
                        .result
                        .as_ref()
                        .and_then(|result| result.get("path"))
                        .and_then(Value::as_str)
                        .unwrap_or(&target_relative);
                    format!("extracted to {path}")
                }
                "cancelled" => return Err(AppError::Cancelled),
                _ => {
                    return Err(AppError::bad_request(
                        job.error.unwrap_or_else(|| "unzip failed".to_owned()),
                    ));
                }
            }
        }
        _ => {
            return Err(AppError::bad_request(format!(
                "unsupported command: {command}"
            )));
        }
    };

    audit(state, user_id, &line, &cwd, headers).await;
    Ok(ExecOutcome::Output(truncate(output)))
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0_usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_local_time(seconds: i64) -> String {
    let Ok(utc) = time::OffsetDateTime::from_unix_timestamp(seconds) else {
        return "-".to_owned();
    };
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let local = utc.to_offset(offset);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        local.year(),
        u8::from(local.month()),
        local.day(),
        local.hour(),
        local.minute()
    )
}

fn help_text(command: Option<&str>) -> String {
    let overview = "Available commands:
  pwd       print the current directory
  cd        change directory
  ls        list entries
  tree      show a directory tree
  mkdir     create directories
  touch     create empty files
  rm        remove files/directories
  cp        copy files/directories
  mv        move/rename files/directories
  ln        create hard/symbolic links
  zip       create a ZIP archive
  unzip     extract a ZIP archive
  job       list background jobs
  help      show help

Run `help <command>` or `<command> -h` for details.";
    let Some(command) = command else {
        return overview.to_owned();
    };
    let text = match command {
        "pwd" => "pwd\n\nPrint the current directory.",
        "cd" => "cd <path>\n\nChange the current directory. `cd` or `cd /` returns to the root.",
        "ls" => {
            "ls [-l] [-a] [path]\n\nList entries. -l shows size and modified time, -a includes hidden entries."
        }
        "tree" => "tree [path]\n\nShow a directory tree (depth and output are capped).",
        "mkdir" => "mkdir [-p] <path>...\n\nCreate directories. -p creates missing parents.",
        "touch" => {
            "touch <path>...\n\nCreate empty files. Existing files and symbolic links are not modified."
        }
        "rm" => {
            "rm [-r] [-y] <path>...\n\nRemove files. Directories require -r. -y skips the simple y/N confirmation.\nHigh-risk deletions always require a 4-digit code: any directory, more than 5 items, or more than 10 MiB."
        }
        "cp" => {
            "cp [-r] <source>... <destination>\n\nCopy files or directories. Directories require -r."
        }
        "mv" => "mv <source>... <destination>\n\nMove or rename files and directories.",
        "ln" => "ln [-s] <target> <link>\n\nCreate a hard link, or a symbolic link with -s.",
        "zip" => {
            "zip [-r] <archive.zip> <source>...\n\nCreate a background ZIP job in the current directory. Directories require -r. Existing archives get a numeric suffix."
        }
        "unzip" => {
            "unzip <archive.zip> [-d <target>]\n\nExtract a ZIP archive. Without -d it creates a directory named after the archive. Encrypted archives and symbolic-link entries are rejected."
        }
        "job" => {
            "job [count | job_id]\njobs [count | job_id]\n\nList recent background jobs or show one job."
        }
        "help" => "help [command]\n\nShow the command list or help for one command.",
        _ => return format!("unknown command: {command}"),
    };
    text.to_owned()
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

fn job_status_rank(status: &str) -> u8 {
    match status {
        "running" => 0,
        "queued" => 1,
        _ => 2,
    }
}

fn job_status_color(status: &str) -> &'static str {
    match status {
        "running" => "32",
        "queued" => "33",
        "failed" => "31",
        "cancelled" => "33",
        _ => "90",
    }
}

fn format_job_list(jobs: &[crate::jobs::Job]) -> String {
    if jobs.is_empty() {
        return "\x1b[90mno jobs\x1b[0m".to_owned();
    }
    let mut sorted = jobs.to_vec();
    sorted.sort_by(|left, right| {
        job_status_rank(&left.status)
            .cmp(&job_status_rank(&right.status))
            .then_with(|| right.created_at.cmp(&left.created_at))
    });
    sorted
        .iter()
        .map(format_job_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_job_line(job: &crate::jobs::Job) -> String {
    let progress = if job.total > 0 {
        format!("{}/{}", job.progress, job.total)
    } else {
        String::new()
    };
    let bytes = if job.bytes_total > 0 {
        format!("  {}/{}", job.bytes_done, job.bytes_total)
    } else {
        String::new()
    };
    format!(
        "\x1b[36m{}\x1b[0m  {}  \x1b[{}m{}\x1b[0m  {}{}{}",
        job.id,
        job.kind,
        job_status_color(&job.status),
        job.status,
        progress,
        bytes,
        job.error
            .as_deref()
            .map(|error| format!("  \x1b[31m{error}\x1b[0m"))
            .unwrap_or_default()
    )
}

fn format_job_detail(job: &crate::jobs::Job) -> String {
    let mut lines = vec![
        format!("id: {}", job.id),
        format!("kind: {}", job.kind),
        format!(
            "status: \x1b[{}m{}\x1b[0m",
            job_status_color(&job.status),
            job.status
        ),
        format!("progress: {}/{}", job.progress, job.total),
        format!("bytes: {}/{}", job.bytes_done, job.bytes_total),
        format!("created: {}", job.created_at),
    ];
    if let Some(started) = job.started_at {
        lines.push(format!("started: {started}"));
    }
    if let Some(finished) = job.finished_at {
        lines.push(format!("finished: {finished}"));
    }
    if let Some(error) = job.error.as_deref() {
        lines.push(format!("error: \x1b[31m{error}\x1b[0m"));
    }
    if let Some(result) = job.result.as_ref() {
        lines.push(format!("result: {result}"));
    }
    lines.join("\n")
}

fn parse_root_ref(roots: &[Root], token: &str) -> Result<Option<(i64, String)>, AppError> {
    let Some(rest) = token.strip_prefix('@') else {
        return Ok(None);
    };
    let Some((name, path)) = rest.split_once(":/") else {
        return Ok(None);
    };
    let root = if let Ok(id) = name.parse::<i64>() {
        roots.iter().find(|root| root.id == id)
    } else {
        let matches: Vec<&Root> = roots.iter().filter(|root| root.alias == name).collect();
        if matches.len() > 1 {
            return Err(AppError::bad_request(format!(
                "duplicate root alias: {name}; use @<id>:/path"
            )));
        }
        matches.first().copied()
    }
    .ok_or_else(|| AppError::bad_request(format!("unknown root: @{name}")))?;
    Ok(Some((root.id, path.trim_start_matches('/').to_owned())))
}

#[allow(clippy::too_many_arguments)]
async fn cross_root_transfer(
    state: &Arc<AppState>,
    user_id: i64,
    root: &Root,
    cwd: &str,
    source_tokens: &[String],
    destination: &str,
    recursive: bool,
    move_items: bool,
    line: &str,
    headers: &HeaderMap,
) -> Result<Option<String>, AppError> {
    let uses_ref =
        destination.starts_with('@') || source_tokens.iter().any(|token| token.starts_with('@'));
    if !uses_ref {
        return Ok(None);
    }
    if source_tokens.is_empty() {
        return Err(AppError::bad_request(if move_items {
            "mv requires source and destination"
        } else {
            "cp requires source and destination"
        }));
    }
    let roots = state
        .db
        .list_roots(user_id)
        .await
        .map_err(AppError::internal)?;
    let (dest_root_id, dest_path) = match parse_root_ref(&roots, destination)? {
        Some((id, path)) => (id, path),
        None => (root.id, join(cwd, destination)),
    };
    let mut sources = Vec::with_capacity(source_tokens.len());
    for token in source_tokens {
        match parse_root_ref(&roots, token)? {
            Some((id, path)) => sources.push((id, path)),
            None => sources.push((root.id, join(cwd, token))),
        }
    }
    let dest = authz::resolve(state, user_id, dest_root_id, &dest_path, true).await?;
    if !dest.absolute.is_dir() {
        return Err(AppError::bad_request(
            "destination directory does not exist",
        ));
    }
    let source_root_ids: std::collections::HashSet<i64> =
        sources.iter().map(|(id, _)| *id).collect();
    if source_root_ids.len() > 1 {
        return Err(AppError::bad_request(
            "cross-root sources must share the same root",
        ));
    }
    let same_root = source_root_ids.contains(&dest_root_id);
    let mut resolved_sources = Vec::with_capacity(sources.len());
    for (root_id, path) in &sources {
        let resolved = authz::resolve(state, user_id, *root_id, path, move_items).await?;
        if resolved.relative.is_empty() {
            return Err(AppError::invalid_path(if move_items {
                "the root directory cannot be moved"
            } else {
                "the root directory cannot be copied"
            }));
        }
        if resolved.absolute.is_dir() && !recursive && !move_items {
            return Err(AppError::bad_request("directory requires -r"));
        }
        let target = dest
            .absolute
            .join(resolved.absolute.file_name().unwrap_or_default());
        if resolved.absolute.is_dir()
            && (target == resolved.absolute || target.starts_with(&resolved.absolute))
        {
            return Err(AppError::invalid_path(if move_items {
                "cannot move a directory into itself"
            } else {
                "cannot copy a directory into itself"
            }));
        }
        resolved_sources.push(resolved);
    }
    if same_root {
        for resolved in &resolved_sources {
            let source = resolved.absolute.clone();
            let target = dest
                .absolute
                .join(resolved.absolute.file_name().unwrap_or_default());
            if move_items {
                tokio::task::spawn_blocking(move || move_path(&source, &target))
                    .await
                    .map_err(|err| AppError::internal(anyhow::anyhow!("mv task failed: {err}")))?
                    .map_err(io_error)?;
            } else {
                tokio::task::spawn_blocking(move || copy_path(&source, &target, recursive))
                    .await
                    .map_err(|err| AppError::internal(anyhow::anyhow!("cp task failed: {err}")))?
                    .map_err(io_error)?;
            }
        }
        audit(state, user_id, line, cwd, headers).await;
        return Ok(Some(String::new()));
    }
    let source_root = *source_root_ids
        .iter()
        .next()
        .ok_or_else(|| AppError::bad_request("no sources"))?;
    let paths: Vec<String> = resolved_sources
        .iter()
        .map(|resolved| resolved.relative.clone())
        .collect();
    let outcome = crate::files::submit_transfer_job(
        state,
        user_id,
        crate::files::TransferJobPayload {
            source_root,
            paths,
            dest_root: dest_root_id,
            dest_path: Some(dest.relative.clone()),
            move_items,
        },
    )
    .await?;
    audit(state, user_id, line, cwd, headers).await;
    Ok(Some(format!(
        "queued {} job {}\nuse `job` to check progress",
        if move_items { "move" } else { "copy" },
        outcome.job.id
    )))
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
        let include_hidden = pattern.starts_with('.');
        let mut matched = 0_usize;
        for entry in entries {
            if !include_hidden && entry.name.starts_with('.') {
                continue;
            }
            if matcher.is_match(&entry.name) {
                matched += 1;
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
        if matched == 0 {
            return Err(AppError::bad_request(format!("no matches: {arg}")));
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
    state
        .enqueue_audit(crate::db::AuditRecord {
            user_id: Some(user_id),
            action: "command".to_owned(),
            path: Some(cwd.to_owned()),
            detail: Some(line.to_owned()),
            ip: auth::client_ip(headers),
        })
        .await;
}
