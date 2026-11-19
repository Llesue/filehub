use std::path::{Component, Path, PathBuf};

use crate::db::Root;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct ResolvedPath {
    pub root: Root,
    pub relative: String,
    pub absolute: PathBuf,
    pub access_read_only: bool,
}

pub async fn resolve(
    root: Root,
    path: &str,
    require_write: bool,
) -> Result<ResolvedPath, AppError> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || resolve_sync(root, &path, require_write))
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!("path resolution task failed: {err}")))?
}

fn validate_component(part: &str) -> Result<(), AppError> {
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
    #[cfg(not(windows))]
    let _ = part;
    Ok(())
}

fn resolve_sync(root: Root, path: &str, require_write: bool) -> Result<ResolvedPath, AppError> {
    if require_write && root.read_only {
        return Err(AppError::Forbidden);
    }

    if path.len() > 4096 {
        return Err(AppError::invalid_path("path is too long"));
    }
    if path.contains('\0') || path.contains('\\') {
        return Err(AppError::invalid_path(
            "path contains an unsupported character",
        ));
    }
    if path.starts_with('/') {
        return Err(AppError::invalid_path("absolute paths are not allowed"));
    }

    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| AppError::invalid_path("non-UTF-8 path"))?;
                validate_component(part)?;
                parts.push(part.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::invalid_path("path traversal is not allowed"));
            }
        }
    }

    let relative = parts.join("/");
    let root_path = std::fs::canonicalize(&root.real_path)
        .map_err(|err| AppError::internal(anyhow::anyhow!("invalid root: {err}")))?;
    if !root_path.is_dir() {
        return Err(AppError::invalid_path("root is not a directory"));
    }

    let candidate = if relative.is_empty() {
        root_path.clone()
    } else {
        root_path.join(&relative)
    };

    let absolute = if candidate.exists() {
        let canonical = std::fs::canonicalize(&candidate)
            .map_err(|_| AppError::invalid_path("path cannot be resolved"))?;
        if !crate::platform::path_within(&canonical, &root_path) {
            return Err(AppError::invalid_path("path escapes the authorized root"));
        }
        canonical
    } else {
        // exists() follows symlinks, so a dangling symlink looks absent here.
        // Never hand back a symlink leaf: callers may write through it.
        if let Ok(metadata) = std::fs::symlink_metadata(&candidate)
            && metadata.file_type().is_symlink()
        {
            return Err(AppError::invalid_path("path is a symbolic link"));
        }
        let parent = candidate
            .parent()
            .ok_or_else(|| AppError::invalid_path("path has no parent"))?;
        let parent = std::fs::canonicalize(parent)
            .map_err(|_| AppError::invalid_path("parent directory does not exist"))?;
        if !crate::platform::path_within(&parent, &root_path) {
            return Err(AppError::invalid_path("path escapes the authorized root"));
        }
        match candidate.file_name() {
            Some(name) => parent.join(name),
            None => parent,
        }
    };

    Ok(ResolvedPath {
        access_read_only: root.read_only,
        root,
        relative,
        absolute,
    })
}
