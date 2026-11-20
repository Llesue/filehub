use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::config::SymlinkSettings;
use crate::db::{Root, TrustedLink};
use crate::error::AppError;
use crate::pathresolver::ResolvedPath;
use crate::state::AppState;

pub async fn resolve(
    state: &Arc<AppState>,
    user_id: i64,
    root_id: i64,
    path: &str,
    require_write: bool,
) -> Result<ResolvedPath, AppError> {
    let roots = state
        .db
        .list_roots(user_id)
        .await
        .map_err(AppError::internal)?;
    let root = roots
        .iter()
        .find(|root| root.id == root_id)
        .cloned()
        .ok_or(AppError::NotFound)?;
    let links = state
        .db
        .list_trusted_links()
        .await
        .map_err(AppError::internal)?;
    let path = path.to_owned();
    let symlinks = state.settings().settings.symlinks.clone();

    tokio::task::spawn_blocking(move || {
        resolve_sync(root, roots, path, require_write, &links, &symlinks)
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("authorization task failed: {err}")))?
}

pub async fn resolve_many(
    state: &Arc<AppState>,
    user_id: i64,
    root_id: i64,
    paths: Vec<String>,
    require_write: bool,
) -> Result<Vec<ResolvedPath>, AppError> {
    let roots = state
        .db
        .list_roots(user_id)
        .await
        .map_err(AppError::internal)?;
    let root = roots
        .iter()
        .find(|root| root.id == root_id)
        .cloned()
        .ok_or(AppError::NotFound)?;
    let links = state
        .db
        .list_trusted_links()
        .await
        .map_err(AppError::internal)?;
    let symlinks = state.settings().settings.symlinks.clone();
    tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| {
                resolve_sync(
                    root.clone(),
                    roots.clone(),
                    path,
                    require_write,
                    &links,
                    &symlinks,
                )
            })
            .collect()
    })
    .await
    .map_err(|err| AppError::internal(anyhow::anyhow!("authorization task failed: {err}")))?
}

fn resolve_sync(
    root: Root,
    roots: Vec<Root>,
    path: String,
    require_write: bool,
    links: &[TrustedLink],
    symlinks: &SymlinkSettings,
) -> Result<ResolvedPath, AppError> {
    if path.len() > 4096 || path.contains('\0') || path.contains('\\') || path.starts_with('/') {
        return Err(AppError::invalid_path("invalid path"));
    }

    let mut components = Vec::new();
    for component in Path::new(&path).components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| AppError::invalid_path("non-UTF-8 path"))?;
                #[cfg(windows)]
                if part.contains(':')
                    || part.ends_with(' ')
                    || part.ends_with('.')
                    || part
                        .chars()
                        .any(|value| matches!(value, '<' | '>' | '"' | '|' | '?' | '*'))
                {
                    return Err(AppError::invalid_path("invalid Windows path component"));
                }
                components.push(part.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::invalid_path("path traversal is not allowed"));
            }
        }
    }

    let relative = components.join("/");
    let root_path = std::fs::canonicalize(&root.real_path)
        .map_err(|err| AppError::internal(anyhow::anyhow!("invalid root: {err}")))?;
    if !root_path.is_dir() {
        return Err(AppError::invalid_path("root is not a directory"));
    }

    let trusted: HashMap<&str, &TrustedLink> = links
        .iter()
        .map(|link| (link.link_path.as_str(), link))
        .collect();
    let mut current = root_path.clone();
    let mut access_override: Option<bool> = None;
    let mut used_trusted = false;
    let mut hops = 0usize;

    for (index, component) in components.iter().enumerate() {
        let next = current.join(component);
        let is_last = index + 1 == components.len();

        if let Ok(metadata) = std::fs::symlink_metadata(&next) {
            if metadata.file_type().is_symlink() {
                hops += 1;
                if hops > symlinks.max_hops {
                    return Err(AppError::invalid_path("too many symbolic link hops"));
                }
                let link_path = next.to_string_lossy().to_string();
                let target = std::fs::canonicalize(&next)
                    .map_err(|_| AppError::invalid_path("broken symlink"))?;
                let target_metadata = std::fs::metadata(&target)
                    .map_err(|_| AppError::invalid_path("symlink target is unavailable"))?;

                let inside_root = roots.iter().any(|candidate| {
                    let candidate = PathBuf::from(&candidate.real_path);
                    crate::platform::path_within(&target, &candidate)
                });

                if !inside_root {
                    if !symlinks.follow_admin_links {
                        return Err(AppError::invalid_path(
                            "symlink target is outside the authorized roots",
                        ));
                    }
                    let Some(link) = trusted.get(link_path.as_str()) else {
                        return Err(AppError::invalid_path(
                            "symlink target is outside the authorized roots",
                        ));
                    };

                    let (link_dev, link_ino) = crate::platform::metadata_identity(&metadata);
                    let (target_dev, target_ino) =
                        crate::platform::metadata_identity(&target_metadata);
                    let link_matches = link.link_dev == link_dev
                        && link.link_ino == link_ino
                        && link.link_mtime == crate::platform::metadata_mtime(&metadata);
                    let target_matches =
                        link.target_dev == target_dev && link.target_ino == target_ino;

                    if !link_matches || !target_matches {
                        return Err(AppError::invalid_path(
                            "trusted symlink identity has changed",
                        ));
                    }

                    used_trusted = true;
                    access_override = match link.access.as_str() {
                        "read_only" => Some(true),
                        "read_write" => Some(false),
                        _ => None,
                    };
                }

                current = target;
                continue;
            }
        } else if !is_last {
            return Err(AppError::invalid_path("parent directory does not exist"));
        }

        current = next;
    }

    let absolute = if current.exists() {
        std::fs::canonicalize(&current)
            .map_err(|_| AppError::invalid_path("path cannot be resolved"))?
    } else {
        let parent = current
            .parent()
            .ok_or_else(|| AppError::invalid_path("path has no parent"))?;
        let parent = std::fs::canonicalize(parent)
            .map_err(|_| AppError::invalid_path("parent directory does not exist"))?;
        match current.file_name() {
            Some(name) => parent.join(name),
            None => parent,
        }
    };

    let effective_root = roots
        .iter()
        .filter(|candidate| {
            crate::platform::path_within(&absolute, Path::new(&candidate.real_path))
        })
        .max_by_key(|candidate| candidate.real_path.len());

    if effective_root.is_none() && !used_trusted {
        return Err(AppError::invalid_path("path escapes the authorized roots"));
    }

    let mut access_read_only = effective_root
        .map(|candidate| candidate.read_only)
        .unwrap_or(root.read_only);
    if let Some(override_value) = access_override {
        access_read_only = override_value;
    }

    if require_write && access_read_only {
        return Err(AppError::Forbidden);
    }

    Ok(ResolvedPath {
        root,
        relative,
        absolute,
        access_read_only,
    })
}
