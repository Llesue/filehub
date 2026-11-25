use std::collections::HashSet;
#[cfg(unix)]
use std::fs::File;
use std::fs::{FileTimes, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::db::{Root, TrustedLink};
use crate::error::AppError;
use crate::pathresolver::ResolvedPath;

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: i64,
    pub modified: i64,
    pub created_at: i64,
    pub mime_type: String,
    pub has_preview: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub symlink_state: Option<String>,
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub size: u64,
    pub etag: String,
    pub modified: i64,
}

#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
}

pub fn list_dir(resolved: &ResolvedPath, sort: &str, order: &str) -> Result<Vec<Entry>, AppError> {
    let metadata = std::fs::metadata(&resolved.absolute).map_err(io_error)?;
    if !metadata.is_dir() {
        return Err(AppError::bad_request("path is not a directory"));
    }

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&resolved.absolute).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".filehub-upload-") {
            continue;
        }

        let file_type = entry.file_type().map_err(io_error)?;
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(io_error)?;
        let is_symlink = file_type.is_symlink();
        let (is_dir, size, symlink_state, symlink_target) = if is_symlink {
            let target = std::fs::read_link(entry.path()).ok().and_then(|target| {
                target
                    .is_relative()
                    .then(|| target.to_string_lossy().into_owned())
            });
            match std::fs::metadata(entry.path()) {
                Ok(target_meta) => (
                    target_meta.is_dir(),
                    if target_meta.is_file() {
                        target_meta.len() as i64
                    } else {
                        0
                    },
                    Some("ok".to_owned()),
                    target,
                ),
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    (false, 0, Some("denied".to_owned()), None)
                }
                Err(_) => (false, 0, Some("broken".to_owned()), None),
            }
        } else {
            (
                file_type.is_dir(),
                if file_type.is_file() {
                    metadata.len() as i64
                } else {
                    0
                },
                None,
                None,
            )
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        let created_at = metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        let mime_type = mime_guess::from_path(&name)
            .first_or_octet_stream()
            .to_string();

        entries.push(Entry {
            name,
            is_dir,
            is_symlink,
            size,
            modified,
            created_at,
            mime_type,
            has_preview: false,
            sha256: None,
            symlink_state,
            symlink_target,
        });
    }

    let descending = order.eq_ignore_ascii_case("desc");
    entries.sort_by(|left, right| {
        let primary = match sort {
            "size" => left.size.cmp(&right.size),
            "modified" => left.modified.cmp(&right.modified),
            _ => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        };
        let primary = if descending {
            primary.reverse()
        } else {
            primary
        };
        let name = left.name.to_lowercase().cmp(&right.name.to_lowercase());
        let name = if descending { name.reverse() } else { name };
        right.is_dir.cmp(&left.is_dir).then(primary).then(name)
    });

    Ok(entries)
}

pub fn has_symlinks(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .file_type()
                    .map(|file_type| file_type.is_symlink())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn dir_mtime_ns(path: &Path) -> Result<i64, AppError> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    Ok(metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or_default())
}

pub fn file_info(path: &Path) -> Result<FileInfo, AppError> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(AppError::bad_request("path is not a file"));
    }

    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let etag = format!("\"{}-{}\"", metadata.len(), modified_ns);
    let modified = (modified_ns / 1_000_000_000) as i64;

    Ok(FileInfo {
        size: metadata.len(),
        etag,
        modified,
    })
}

pub fn collect_zip(
    selected: &[ResolvedPath],
    max_bytes: u64,
    max_files: usize,
) -> Result<Vec<ZipEntry>, AppError> {
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;

    for resolved in selected {
        if resolved.relative.is_empty() {
            return Err(AppError::invalid_path("cannot archive the root directory"));
        }
        collect_zip_path(
            resolved,
            &mut entries,
            &mut total_bytes,
            max_bytes,
            max_files,
            false,
        )?;
    }

    Ok(entries)
}

/// Collect one resolved path (file or directory) for a ZIP download. Unlike
/// `collect_zip`, a share root with an empty relative path is allowed.
pub fn collect_zip_path(
    resolved: &ResolvedPath,
    entries: &mut Vec<ZipEntry>,
    total_bytes: &mut u64,
    max_bytes: u64,
    max_files: usize,
    skip_hidden: bool,
) -> Result<(), AppError> {
    let name = resolved
        .absolute
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::invalid_path("invalid archive entry name"))?
        .to_owned();
    collect_one(
        &resolved.absolute,
        &name,
        entries,
        total_bytes,
        max_bytes,
        max_files,
        skip_hidden,
    )
}

pub fn collect_zip_named(
    path: &Path,
    name: &str,
    entries: &mut Vec<ZipEntry>,
    total_bytes: &mut u64,
    max_bytes: u64,
    max_files: usize,
    skip_hidden: bool,
) -> Result<(), AppError> {
    collect_one(
        path,
        name,
        entries,
        total_bytes,
        max_bytes,
        max_files,
        skip_hidden,
    )
}

fn collect_one(
    path: &Path,
    name: &str,
    entries: &mut Vec<ZipEntry>,
    total_bytes: &mut u64,
    max_bytes: u64,
    max_files: usize,
    skip_hidden: bool,
) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;

    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let child_name = entry.file_name().to_string_lossy().into_owned();
            if child_name.starts_with(".filehub-upload-") {
                continue;
            }
            if skip_hidden && child_name.starts_with('.') {
                continue;
            }
            collect_one(
                &entry.path(),
                &format!("{name}/{child_name}"),
                entries,
                total_bytes,
                max_bytes,
                max_files,
                skip_hidden,
            )?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        return Ok(());
    }

    if max_files != 0 && entries.len() >= max_files {
        return Err(AppError::too_large("too many files for a single ZIP"));
    }
    *total_bytes = total_bytes.saturating_add(metadata.len());
    if max_bytes != 0 && *total_bytes > max_bytes {
        return Err(AppError::too_large("ZIP exceeds the configured byte limit"));
    }

    entries.push(ZipEntry {
        path: path.to_path_buf(),
        name: name.to_owned(),
        size: metadata.len(),
    });
    Ok(())
}

pub fn remove_path(path: &Path, relative: &str) -> Result<(), AppError> {
    if relative.is_empty() {
        return Err(AppError::invalid_path(
            "the root directory cannot be removed",
        ));
    }

    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path).map_err(io_error)
    } else {
        std::fs::remove_file(path).map_err(io_error)
    }
}

pub fn ensure_directory(path: &Path) -> Result<PathBuf, AppError> {
    std::fs::create_dir_all(path).map_err(io_error)?;
    std::fs::canonicalize(path).map_err(io_error)
}

pub fn create_upload_temp(directory: &Path, upload_id: &str) -> Result<PathBuf, AppError> {
    let path = directory.join(format!(".filehub-upload-{upload_id}.part"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    crate::platform::set_private_mode(&mut options);
    options.open(&path).map_err(io_error)?;
    Ok(path)
}

pub fn write_upload_chunk(path: &Path, offset: u64, data: &[u8]) -> Result<(), AppError> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io_error)?;
    file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    file.write_all(data).map_err(io_error)?;
    Ok(())
}

/// Create `target` exclusively and write `data` into it.
///
/// `create_new` refuses to follow or replace an existing entry, including a
/// dangling symlink, so this cannot write outside the intended directory.
pub fn write_new_file(target: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    crate::platform::set_private_mode(&mut options);
    let mut file = options.open(target)?;
    file.write_all(data)?;
    file.sync_all()
}

pub fn finalize_upload(temp: &Path, target: &Path, mtime: Option<i64>) -> Result<(), AppError> {
    if target.exists() {
        return Err(AppError::conflict("target file already exists"));
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(temp)
        .map_err(io_error)?;
    if let Some(mtime) = mtime {
        let modified = UNIX_EPOCH
            .checked_add(Duration::from_secs(mtime as u64))
            .ok_or_else(|| AppError::bad_request("mtime is out of range"))?;
        file.set_times(FileTimes::new().set_modified(modified))
            .map_err(io_error)?;
    }
    file.sync_all().map_err(io_error)?;
    std::fs::rename(temp, target).map_err(io_error)?;

    #[cfg(unix)]
    if let Some(parent) = target.parent()
        && let Ok(directory) = File::open(parent)
    {
        directory.sync_all().map_err(io_error)?;
    }

    Ok(())
}

pub fn write_text_atomic(path: &Path, contents: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::invalid_path("file has no parent"))?;
    let temp = parent.join(format!(
        ".filehub-edit-{}.tmp",
        crate::auth::random_token(8)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    crate::platform::set_private_mode(&mut options);
    let mut file = options.open(&temp).map_err(io_error)?;
    file.write_all(contents).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    drop(file);
    std::fs::rename(&temp, path).map_err(io_error)?;
    #[cfg(unix)]
    if let Ok(directory) = File::open(parent) {
        directory.sync_all().map_err(io_error)?;
    }
    Ok(())
}

pub fn delete_temp(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Remove stale `.filehub-edit-*.tmp` files under `root`.
///
/// Only files with the exact editor temp prefix are touched. Symlinks are not
/// followed and the walk is capped so a huge tree cannot stall maintenance.
pub fn cleanup_edit_temp(root: &Path, cutoff: i64, max_dirs: usize) -> (usize, usize, bool) {
    let mut removed = 0_usize;
    let mut visited = 0_usize;
    let mut truncated = false;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        if visited >= max_dirs {
            truncated = true;
            break;
        }
        visited += 1;
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(".filehub-edit-") || !name.ends_with(".tmp") {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(i64::MAX);
            if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }
    (removed, visited, truncated)
}

pub fn canonical_directory(path: &str) -> Result<PathBuf, AppError> {
    let canonical = std::fs::canonicalize(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            AppError::bad_request("directory does not exist on the server")
        }
        std::io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        _ => io_error(error),
    })?;
    if !canonical.is_dir() {
        return Err(AppError::bad_request("path is not a directory"));
    }
    Ok(canonical)
}

pub fn prepare_trusted_link(
    link_path: &str,
    target_path: &str,
    access: &str,
) -> Result<TrustedLink, AppError> {
    if !matches!(access, "inherit" | "read_only" | "read_write") {
        return Err(AppError::bad_request("invalid link access mode"));
    }

    let link = PathBuf::from(link_path);
    if !link.is_absolute() {
        return Err(AppError::invalid_path("link_path must be absolute"));
    }
    let parent = link
        .parent()
        .ok_or_else(|| AppError::invalid_path("link_path has no parent"))?;
    let parent = std::fs::canonicalize(parent).map_err(io_error)?;
    let name = link
        .file_name()
        .ok_or_else(|| AppError::invalid_path("link_path has no file name"))?;
    let link = parent.join(name);

    let link_metadata = std::fs::symlink_metadata(&link).map_err(io_error)?;
    if !link_metadata.file_type().is_symlink() {
        return Err(AppError::bad_request("link_path is not a symlink"));
    }

    let raw_target = std::fs::read_link(&link).map_err(io_error)?;
    let raw_target = if raw_target.is_absolute() {
        raw_target
    } else {
        link.parent()
            .ok_or_else(|| AppError::invalid_path("link_path has no parent"))?
            .join(raw_target)
    };
    let canonical_target = std::fs::canonicalize(raw_target).map_err(io_error)?;
    let expected_target = std::fs::canonicalize(target_path).map_err(io_error)?;
    if !crate::platform::paths_equal(&canonical_target, &expected_target) {
        return Err(AppError::bad_request(
            "target_path does not match the symlink target",
        ));
    }

    let target_metadata = std::fs::metadata(&canonical_target).map_err(io_error)?;
    let link_path = link
        .to_str()
        .ok_or_else(|| AppError::invalid_path("link_path is not valid UTF-8"))?
        .to_owned();
    let target_path = canonical_target
        .to_str()
        .ok_or_else(|| AppError::invalid_path("target_path is not valid UTF-8"))?
        .to_owned();

    let (link_dev, link_ino) = crate::platform::metadata_identity(&link_metadata);
    let (target_dev, target_ino) = crate::platform::metadata_identity(&target_metadata);
    Ok(TrustedLink {
        id: 0,
        link_path,
        target_path,
        link_dev,
        link_ino,
        link_mtime: crate::platform::metadata_mtime(&link_metadata),
        target_dev,
        target_ino,
        access: access.to_owned(),
        created_at: 0,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskUsage {
    pub name: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

#[cfg(unix)]
pub fn disk_usage(roots: &[Root]) -> Result<Vec<DiskUsage>, AppError> {
    let mut mounts = HashSet::new();
    let mut disks = Vec::new();

    for root in roots {
        if root.is_remote() {
            continue;
        }
        let mount = mount_point(Path::new(&root.real_path))?;
        if !mounts.insert(mount.clone()) {
            continue;
        }
        let stat = rustix::fs::statvfs(&root.real_path)
            .map_err(|err| AppError::internal(anyhow::anyhow!("statvfs failed: {err}")))?;
        let total = stat.f_blocks.saturating_mul(stat.f_frsize);
        let free = stat.f_bfree.saturating_mul(stat.f_frsize);
        disks.push(DiskUsage {
            name: mount.to_string_lossy().into_owned(),
            total,
            used: total.saturating_sub(free),
            available: stat.f_bavail.saturating_mul(stat.f_frsize),
        });
    }

    Ok(disks)
}

#[cfg(unix)]
fn mount_point(path: &Path) -> Result<PathBuf, AppError> {
    use std::os::unix::fs::MetadataExt;
    let mut current = std::fs::canonicalize(path).map_err(io_error)?;
    loop {
        let Some(parent) = current.parent() else {
            return Ok(current);
        };
        if parent == current {
            return Ok(current);
        }
        let current_dev = std::fs::metadata(&current).map_err(io_error)?.dev();
        let parent_dev = std::fs::metadata(parent).map_err(io_error)?.dev();
        if current_dev != parent_dev {
            return Ok(current);
        }
        current = parent.to_path_buf();
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn disk_usage(roots: &[Root]) -> Result<Vec<DiskUsage>, AppError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut volumes = HashSet::new();
    let mut disks = Vec::new();

    for root in roots {
        let path = Path::new(&root.real_path);
        let key = volume_key(path);
        if volumes.contains(&key) {
            continue;
        }
        let name = if key.len() == 2 && key.ends_with(':') {
            format!("{key}\\")
        } else {
            key.clone()
        };
        volumes.insert(key);
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0_u64;
        let mut total = 0_u64;
        let mut free = 0_u64;
        // SAFETY: the path is a NUL-terminated UTF-16 buffer and all output pointers are valid.
        let ok =
            unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) };
        if ok == 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        disks.push(DiskUsage {
            name,
            total,
            used: total.saturating_sub(free),
            available,
        });
    }

    Ok(disks)
}

#[cfg(windows)]
fn volume_key(path: &Path) -> String {
    let value = path.to_string_lossy().to_lowercase();
    let value = value.strip_prefix(r"\\?\").unwrap_or(&value);
    let mut parts = value.split('\\');
    let first = parts.next().unwrap_or_default();
    if first.is_empty() {
        format!(
            r"\\{}\{}",
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default()
        )
    } else {
        first.to_owned()
    }
}

pub fn directory_usage(roots: &[Root]) -> Result<u64, AppError> {
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut total = 0_u64;

    for root in roots {
        if root.is_remote() {
            continue;
        }
        walk_usage(Path::new(&root.real_path), &mut seen, &mut total)?;
    }

    Ok(total)
}

fn walk_usage(
    path: &Path,
    seen: &mut HashSet<(u64, u64)>,
    total: &mut u64,
) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".filehub-upload-")
            {
                continue;
            }
            walk_usage(&entry.path(), seen, total)?;
        }
        return Ok(());
    }

    if metadata.is_file()
        && seen.insert((
            crate::platform::metadata_identity(&metadata).0 as u64,
            crate::platform::metadata_identity(&metadata).1 as u64,
        ))
    {
        *total = total.saturating_add(metadata.len());
    }
    Ok(())
}

pub(crate) fn io_error(error: std::io::Error) -> AppError {
    match error.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        std::io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        std::io::ErrorKind::AlreadyExists => AppError::Conflict(error.to_string()),
        _ => AppError::internal(anyhow::Error::from(error)),
    }
}

/// Whether the service process can create, rename or delete entries in `path`.
///
/// `None` means the platform cannot probe reliably (Windows); callers then
/// fall back to the configured/policy value.
#[cfg(unix)]
pub fn directory_writable(path: &Path) -> Option<bool> {
    use rustix::fs::{Access, StatVfsMountFlags, access, statvfs};

    // `access` checks the real UID/GID. FileHub runs as the same user (or as
    // root), so the real/effective distinction does not matter here. Android
    // seccomp kills `faccessat2` with SIGSYS on Termux, so avoid it.
    match access(path, Access::WRITE_OK | Access::EXEC_OK) {
        Ok(()) => {}
        Err(rustix::io::Errno::ACCESS | rustix::io::Errno::ROFS | rustix::io::Errno::PERM) => {
            return Some(false);
        }
        Err(_) => return None,
    }
    match statvfs(path) {
        Ok(stats) => Some(!stats.f_flag.contains(StatVfsMountFlags::RDONLY)),
        Err(_) => None,
    }
}

#[cfg(not(unix))]
pub fn directory_writable(_path: &Path) -> Option<bool> {
    None
}

#[cfg(unix)]
pub fn file_writable(path: &Path) -> Option<bool> {
    use rustix::fs::{Access, access};

    match access(path, Access::WRITE_OK) {
        Ok(()) => Some(true),
        Err(rustix::io::Errno::ACCESS | rustix::io::Errno::ROFS | rustix::io::Errno::PERM) => {
            Some(false)
        }
        Err(_) => None,
    }
}

#[cfg(not(unix))]
pub fn file_writable(_path: &Path) -> Option<bool> {
    None
}

pub fn path_size(path: &Path) -> Result<u64, AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    for entry in std::fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        total = total.saturating_add(path_size(&entry.path())?);
    }
    Ok(total)
}

/// Total bytes and entry count (files + directories, symlinks excluded).
pub fn path_usage(path: &Path) -> Result<(u64, u64), AppError> {
    fn walk(path: &Path, bytes: &mut u64, entries: &mut u64) -> Result<(), AppError> {
        let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if metadata.is_file() {
            *bytes = bytes.saturating_add(metadata.len());
            *entries = entries.saturating_add(1);
            return Ok(());
        }
        if !metadata.is_dir() {
            return Ok(());
        }
        for entry in std::fs::read_dir(path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            *entries = entries.saturating_add(1);
            walk(&entry.path(), bytes, entries)?;
        }
        Ok(())
    }

    let mut bytes = 0_u64;
    let mut entries = 0_u64;
    if std::fs::symlink_metadata(path).is_ok() {
        walk(path, &mut bytes, &mut entries)?;
    }
    Ok((bytes, entries))
}

/// Count entries under `path` without following symlinks, stopping at `limit`.
pub fn count_entries_limited(path: &Path, limit: u64) -> u64 {
    if limit == 0 {
        return 0;
    }
    let mut count = 0_u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if count >= limit {
                return count;
            }
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            count = count.saturating_add(1);
            if metadata.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    count
}

pub fn create_directory(path: &Path) -> Result<bool, AppError> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_dir() => Ok(false),
                Ok(_) => Err(AppError::bad_request(
                    "path already exists and is not a directory",
                )),
                Err(error) => Err(io_error(error)),
            }
        }
        Err(error) => Err(io_error(error)),
    }
}

pub fn create_directory_all(path: &Path) -> Result<bool, AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(false),
        Ok(_) => Err(AppError::bad_request(
            "path already exists and is not a directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path).map_err(io_error)?;
            Ok(true)
        }
        Err(error) => Err(io_error(error)),
    }
}

pub fn rename_path(source: &Path, target: &Path) -> Result<(), AppError> {
    std::fs::rename(source, target).map_err(io_error)
}

pub fn copy_path(source: &Path, target: &Path) -> Result<(), AppError> {
    let cancel = CancellationToken::new();
    copy_path_with_progress(source, target, &cancel, 0, &mut |_, _, _| {})
}

pub fn copy_path_with_progress(
    source: &Path,
    target: &Path,
    cancel: &CancellationToken,
    total_bytes: u64,
    progress: &mut dyn FnMut(u64, u64, u64),
) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(source).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::invalid_path(
            "copying symbolic links is not supported",
        ));
    }
    if target.exists() {
        return Err(AppError::conflict("copy target already exists"));
    }
    let result = copy_tree(source, target, cancel, total_bytes, progress);
    if result.is_err() {
        remove_path_quiet(target);
    }
    result
}

fn copy_tree(
    source: &Path,
    target: &Path,
    cancel: &CancellationToken,
    total_bytes: u64,
    progress: &mut dyn FnMut(u64, u64, u64),
) -> Result<(), AppError> {
    let mut buffer = vec![0_u8; 8 * 1024 * 1024];
    let mut bytes_done = 0_u64;
    let mut files_done = 0_u64;
    let mut stack = vec![(source.to_path_buf(), target.to_path_buf())];

    while let Some((source_path, target_path)) = stack.pop() {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let metadata = std::fs::symlink_metadata(&source_path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            std::fs::create_dir(&target_path).map_err(io_error)?;
            for entry in std::fs::read_dir(&source_path).map_err(io_error)? {
                let entry = entry.map_err(io_error)?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".filehub-upload-")
                {
                    continue;
                }
                stack.push((entry.path(), target_path.join(entry.file_name())));
            }
        } else if metadata.is_file() {
            copy_file_cancellable(
                &source_path,
                &target_path,
                &metadata,
                cancel,
                &mut buffer,
                &mut bytes_done,
                total_bytes,
                progress,
            )?;
            files_done += 1;
            progress(bytes_done, total_bytes, files_done);
        } else {
            return Err(AppError::invalid_path("unsupported file type"));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn copy_file_cancellable(
    source: &Path,
    target: &Path,
    metadata: &std::fs::Metadata,
    cancel: &CancellationToken,
    buffer: &mut [u8],
    bytes_done: &mut u64,
    total_bytes: u64,
    progress: &mut dyn FnMut(u64, u64, u64),
) -> Result<(), AppError> {
    let mut input = std::fs::File::open(source).map_err(io_error)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut output = options.open(target).map_err(io_error)?;
    let mut copied = 0_u64;
    loop {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let read = std::io::Read::read(&mut input, buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(io_error)?;
        copied = copied.saturating_add(read as u64);
        *bytes_done = bytes_done.saturating_add(read as u64);
        if copied % (64 * 1024 * 1024) < read as u64 {
            progress(*bytes_done, total_bytes, 0);
        }
    }
    output.sync_all().map_err(io_error)?;
    std::fs::set_permissions(target, metadata.permissions()).map_err(io_error)?;
    Ok(())
}

/// Copy a tree with a bounded pool of file-copy workers.
///
/// Directory creation stays sequential; file contents are copied in parallel.
/// Progress is reported from the coordinating thread so the callback does not
/// need to be `Sync`.
pub fn copy_path_parallel(
    source: &Path,
    target: &Path,
    cancel: &CancellationToken,
    total_bytes: u64,
    workers: usize,
    progress: &mut dyn FnMut(u64, u64, u64),
) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(source).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::invalid_path(
            "copying symbolic links is not supported",
        ));
    }
    if std::fs::symlink_metadata(target).is_ok() {
        return Err(AppError::conflict("copy target already exists"));
    }

    let mut directories = Vec::new();
    let mut files: Vec<(PathBuf, PathBuf, std::fs::Permissions)> = Vec::new();
    let mut stack = vec![(source.to_path_buf(), target.to_path_buf())];
    while let Some((source_path, target_path)) = stack.pop() {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let metadata = std::fs::symlink_metadata(&source_path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            directories.push(target_path.clone());
            for entry in std::fs::read_dir(&source_path).map_err(io_error)? {
                let entry = entry.map_err(io_error)?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".filehub-upload-")
                {
                    continue;
                }
                stack.push((entry.path(), target_path.join(entry.file_name())));
            }
        } else if metadata.is_file() {
            files.push((source_path, target_path, metadata.permissions()));
        } else {
            remove_path_quiet(target);
            return Err(AppError::invalid_path("unsupported file type"));
        }
    }

    for directory in &directories {
        if let Err(error) = std::fs::create_dir(directory) {
            remove_path_quiet(target);
            return Err(io_error(error));
        }
    }
    if files.is_empty() {
        progress(0, total_bytes, 0);
        return Ok(());
    }

    let next = AtomicUsize::new(0);
    let bytes_done = AtomicU64::new(0);
    let files_done = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failure: Mutex<Option<AppError>> = Mutex::new(None);
    let (sender, receiver) = std::sync::mpsc::channel::<()>();
    let worker_count = workers.max(1).min(files.len());

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let files = &files;
            let stop = &stop;
            let next = &next;
            let cancel = &cancel;
            let bytes_done = &bytes_done;
            let files_done = &files_done;
            let failure = &failure;
            scope.spawn(move || {
                let mut buffer = vec![0_u8; 1024 * 1024];
                loop {
                    if stop.load(Ordering::Relaxed) || cancel.is_cancelled() {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= files.len() {
                        break;
                    }
                    let (source_path, target_path, permissions) = &files[index];
                    match copy_file_worker(
                        source_path,
                        target_path,
                        permissions,
                        cancel,
                        &mut buffer,
                    ) {
                        Ok(copied) => {
                            bytes_done.fetch_add(copied, Ordering::Relaxed);
                            files_done.fetch_add(1, Ordering::Relaxed);
                            let _ = sender.send(());
                        }
                        Err(AppError::Cancelled) => {
                            stop.store(true, Ordering::Relaxed);
                            break;
                        }
                        Err(error) => {
                            let mut slot =
                                failure.lock().unwrap_or_else(|poison| poison.into_inner());
                            if slot.is_none() {
                                *slot = Some(error);
                            }
                            stop.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            });
        }
        drop(sender);
        for _ in receiver {
            progress(
                bytes_done.load(Ordering::Relaxed),
                total_bytes,
                files_done.load(Ordering::Relaxed) as u64,
            );
        }
    });

    if cancel.is_cancelled() {
        remove_path_quiet(target);
        return Err(AppError::Cancelled);
    }
    if let Some(error) = failure
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner())
    {
        remove_path_quiet(target);
        return Err(error);
    }
    progress(
        bytes_done.load(Ordering::Relaxed),
        total_bytes,
        files_done.load(Ordering::Relaxed) as u64,
    );
    Ok(())
}

fn copy_file_worker(
    source: &Path,
    target: &Path,
    permissions: &std::fs::Permissions,
    cancel: &CancellationToken,
    buffer: &mut [u8],
) -> Result<u64, AppError> {
    let mut input = std::fs::File::open(source).map_err(io_error)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut output = options.open(target).map_err(io_error)?;
    let mut copied = 0_u64;
    loop {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let read = std::io::Read::read(&mut input, buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(io_error)?;
        copied = copied.saturating_add(read as u64);
    }
    output.sync_all().map_err(io_error)?;
    std::fs::set_permissions(target, permissions.clone()).map_err(io_error)?;
    Ok(copied)
}

pub fn move_path_parallel(
    source: &Path,
    target: &Path,
    cancel: &CancellationToken,
    total_bytes: u64,
    workers: usize,
    progress: &mut dyn FnMut(u64, u64, u64),
) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(source).map_err(io_error)?;
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    match std::fs::rename(source, target) {
        Ok(()) => {
            progress(total_bytes, total_bytes, 1);
            Ok(())
        }
        Err(error) if crate::platform::is_cross_device_error(&error) => {
            if metadata.file_type().is_symlink() {
                return Err(AppError::invalid_path(
                    "moving symbolic links across filesystems is not supported",
                ));
            }
            copy_path_parallel(source, target, cancel, total_bytes, workers, progress)?;
            if cancel.is_cancelled() {
                remove_path_quiet(target);
                return Err(AppError::Cancelled);
            }
            if metadata.is_dir() {
                std::fs::remove_dir_all(source).map_err(io_error)
            } else {
                std::fs::remove_file(source).map_err(io_error)
            }
        }
        Err(error) => Err(io_error(error)),
    }
}

pub fn move_path(source: &Path, target: &Path) -> Result<(), AppError> {
    let cancel = CancellationToken::new();
    move_path_with_progress(source, target, &cancel, 0, &mut |_, _, _| {})
}

pub fn move_path_with_progress(
    source: &Path,
    target: &Path,
    cancel: &CancellationToken,
    total_bytes: u64,
    progress: &mut dyn FnMut(u64, u64, u64),
) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(source).map_err(io_error)?;
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    match std::fs::rename(source, target) {
        Ok(()) => {
            progress(total_bytes, total_bytes, 1);
            Ok(())
        }
        Err(error) if crate::platform::is_cross_device_error(&error) => {
            if metadata.file_type().is_symlink() {
                return Err(AppError::invalid_path(
                    "moving symbolic links across filesystems is not supported",
                ));
            }
            copy_path_with_progress(source, target, cancel, total_bytes, progress)?;
            if cancel.is_cancelled() {
                remove_path_quiet(target);
                return Err(AppError::Cancelled);
            }
            if metadata.is_dir() {
                std::fs::remove_dir_all(source).map_err(io_error)
            } else {
                std::fs::remove_file(source).map_err(io_error)
            }
        }
        Err(error) => Err(io_error(error)),
    }
}

fn remove_path_quiet(path: &Path) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

fn path_occupied(path: &Path) -> bool {
    // symlink_metadata sees dangling symlinks, exists() does not.
    std::fs::symlink_metadata(path).is_ok()
}

pub fn unique_target(target: &Path) -> PathBuf {
    if !path_occupied(target) {
        return target.to_path_buf();
    }
    let parent = target.parent().unwrap_or_else(|| Path::new(""));
    let stem = target
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let extension = target.extension().and_then(|value| value.to_str());
    for index in 2..10_000 {
        let name = match extension {
            Some(extension) if !extension.is_empty() => {
                format!("{stem} ({index}).{extension}")
            }
            _ => format!("{stem} ({index})"),
        };
        let candidate = parent.join(name);
        if !path_occupied(&candidate) {
            return candidate;
        }
    }
    target.to_path_buf()
}

pub fn touch_file(path: &Path) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    crate::platform::set_private_mode(&mut options);
    options.open(path).map(|_| ()).map_err(io_error)
}

pub fn path_size_limited(path: &Path, max_entries: usize) -> u64 {
    fn walk(path: &Path, remaining: &mut usize) -> u64 {
        if *remaining == 0 {
            return 0;
        }
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return 0;
        };
        if metadata.file_type().is_symlink() {
            return 0;
        }
        if metadata.is_file() {
            *remaining = remaining.saturating_sub(1);
            return metadata.len();
        }
        if !metadata.is_dir() {
            return 0;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        let mut total = 0_u64;
        for entry in entries.flatten() {
            if *remaining == 0 {
                break;
            }
            total = total.saturating_add(walk(&entry.path(), remaining));
        }
        total
    }
    walk(path, &mut max_entries.max(1))
}

#[cfg(unix)]
fn disk_size_for_file(
    metadata: &std::fs::Metadata,
    use_logical: bool,
    seen: &mut HashSet<(u64, u64)>,
) -> u64 {
    use std::os::unix::fs::MetadataExt;
    if use_logical {
        return metadata.len();
    }
    let identity = (metadata.dev(), metadata.ino());
    if metadata.nlink() > 1 && !seen.insert(identity) {
        return 0;
    }
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn disk_size_for_file(
    metadata: &std::fs::Metadata,
    use_logical: bool,
    _seen: &mut HashSet<(u64, u64)>,
) -> u64 {
    if use_logical {
        return metadata.len();
    }
    let size = metadata.len();
    if size == 0 {
        0
    } else {
        size.saturating_add(4095) / 4096 * 4096
    }
}

#[derive(Debug, Clone)]
pub struct DirSizeResult {
    pub total: u64,
    pub partial: bool,
    pub error: Option<String>,
    pub directories: u64,
}

struct SizeFrame {
    path: PathBuf,
    mtime_ns: i64,
    total: u64,
    partial: bool,
    error: Option<String>,
    children: Vec<PathBuf>,
}

fn build_size_frame(
    path: PathBuf,
    ignore: &[String],
    cancel: &CancellationToken,
    use_logical: bool,
    seen: &mut HashSet<(u64, u64)>,
) -> Result<SizeFrame, AppError> {
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    let metadata = std::fs::metadata(&path).map_err(io_error)?;
    if !metadata.is_dir() {
        return Err(AppError::bad_request("path is not a directory"));
    }
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or_default();
    let mut frame = SizeFrame {
        path,
        mtime_ns,
        total: 0,
        partial: false,
        error: None,
        children: Vec::new(),
    };
    let entries = match std::fs::read_dir(&frame.path) {
        Ok(entries) => entries,
        Err(error) => {
            if frame.children.is_empty() && frame.total == 0 {
                return Err(io_error(error));
            }
            frame.partial = true;
            frame.error = Some(error.to_string());
            return Ok(frame);
        }
    };
    for entry in entries {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                frame.partial = true;
                frame.error.get_or_insert_with(|| error.to_string());
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".filehub-upload-") || ignore.iter().any(|ignored| ignored == &name) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                frame.partial = true;
                frame.error.get_or_insert_with(|| error.to_string());
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            frame.children.push(entry.path());
        } else if file_type.is_file() {
            match entry.metadata() {
                Ok(metadata) => {
                    frame.total = frame.total.saturating_add(disk_size_for_file(
                        &metadata,
                        use_logical,
                        seen,
                    ));
                }
                Err(error) => {
                    frame.partial = true;
                    frame.error.get_or_insert_with(|| error.to_string());
                }
            }
        }
    }
    Ok(frame)
}

/// Compute the logical size of a directory tree with an iterative walk.
///
/// The callback is invoked for every directory after its subtree has been
/// fully measured, so callers can persist nested results and answer later
/// navigation without rescanning.
pub fn dir_size_scan(
    root: &Path,
    ignore: &[String],
    cancel: &CancellationToken,
    use_logical: bool,
    budget: Duration,
    max_entries: u64,
    on_dir: &mut dyn FnMut(&Path, u64, i64, bool),
) -> Result<DirSizeResult, AppError> {
    let mut seen = HashSet::new();
    let mut stack = vec![build_size_frame(
        root.to_path_buf(),
        ignore,
        cancel,
        use_logical,
        &mut seen,
    )?];
    let mut directories = 1_u64;
    let mut first_error: Option<String> = None;
    let mut partial = false;
    let started = Instant::now();

    while !stack.is_empty() {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if started.elapsed() >= budget || directories >= max_entries {
            partial = true;
            first_error.get_or_insert_with(|| "directory size budget reached".to_owned());
            let total = stack.iter().map(|frame| frame.total).sum();
            return Ok(DirSizeResult {
                total,
                partial,
                error: first_error,
                directories,
            });
        }
        let Some(frame) = stack.last_mut() else {
            break;
        };
        if let Some(child) = frame.children.pop() {
            let child_path = child.clone();
            match build_size_frame(child, ignore, cancel, use_logical, &mut seen) {
                Ok(child_frame) => {
                    directories += 1;
                    stack.push(child_frame);
                }
                Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                Err(error) => {
                    let message = format!("{}: {error}", child_path.display());
                    frame.partial = true;
                    frame.error.get_or_insert_with(|| message.clone());
                    partial = true;
                    first_error.get_or_insert(message);
                }
            }
            continue;
        }

        let Some(mut done) = stack.pop() else {
            break;
        };
        if !use_logical {
            done.total = done.total.max(4096);
        }
        on_dir(&done.path, done.total, done.mtime_ns, done.partial);
        if let Some(parent) = stack.last_mut() {
            parent.total = parent.total.saturating_add(done.total);
            parent.partial |= done.partial;
            if let Some(error) = done.error {
                parent.error.get_or_insert(error);
            }
        } else {
            partial |= done.partial;
            if let Some(error) = done.error {
                first_error.get_or_insert(error);
            }
            return Ok(DirSizeResult {
                total: done.total,
                partial,
                error: first_error,
                directories,
            });
        }
    }

    Ok(DirSizeResult {
        total: 0,
        partial,
        error: first_error,
        directories,
    })
}

pub fn path_size_cancellable(
    path: &Path,
    cancel: &CancellationToken,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<u64, AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        let size = metadata.len();
        on_bytes(size);
        return Ok(size);
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let result = dir_size_scan(
        path,
        &[],
        cancel,
        true,
        Duration::from_secs(60),
        200_000,
        &mut |_, _, _, _| {},
    )?;
    on_bytes(result.total);
    Ok(result.total)
}
