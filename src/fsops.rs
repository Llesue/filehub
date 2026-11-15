use std::collections::HashSet;
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

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
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();

        entries.push(Entry {
            name,
            is_dir: file_type.is_dir(),
            is_symlink: file_type.is_symlink(),
            size: if file_type.is_file() {
                metadata.len() as i64
            } else {
                0
            },
            modified,
        });
    }

    let descending = order.eq_ignore_ascii_case("desc");
    entries.sort_by(|left, right| {
        let primary = match sort {
            "size" => left.size.cmp(&right.size),
            "modified" => left.modified.cmp(&right.modified),
            _ => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        };
        let ordered = right.is_dir.cmp(&left.is_dir).then(primary);
        if descending {
            ordered.reverse()
        } else {
            ordered
        }
    });

    Ok(entries)
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

        let name = resolved
            .absolute
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::invalid_path("invalid archive entry name"))?
            .to_owned();

        collect_one(
            &resolved.absolute,
            &name,
            &mut entries,
            &mut total_bytes,
            max_bytes,
            max_files,
        )?;
    }

    Ok(entries)
}

fn collect_one(
    path: &Path,
    name: &str,
    entries: &mut Vec<ZipEntry>,
    total_bytes: &mut u64,
    max_bytes: u64,
    max_files: usize,
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
            collect_one(
                &entry.path(),
                &format!("{name}/{child_name}"),
                entries,
                total_bytes,
                max_bytes,
                max_files,
            )?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        return Ok(());
    }

    if entries.len() >= max_files {
        return Err(AppError::too_large("too many files for a single ZIP"));
    }
    *total_bytes = total_bytes.saturating_add(metadata.len());
    if *total_bytes > max_bytes {
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

pub fn finalize_upload(temp: &Path, target: &Path) -> Result<(), AppError> {
    if target.exists() {
        return Err(AppError::conflict("target file already exists"));
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(temp)
        .map_err(io_error)?;
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

pub fn canonical_directory(path: &str) -> Result<PathBuf, AppError> {
    let canonical = std::fs::canonicalize(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            AppError::bad_request("directory does not exist on the server")
        }
        std::io::ErrorKind::PermissionDenied => AppError::Forbidden,
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

fn io_error(error: std::io::Error) -> AppError {
    AppError::internal(anyhow::Error::from(error))
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

pub fn create_directory(path: &Path) -> Result<(), AppError> {
    std::fs::create_dir(path).map_err(io_error)
}

pub fn create_directory_all(path: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(path).map_err(io_error)
}

pub fn rename_path(source: &Path, target: &Path) -> Result<(), AppError> {
    std::fs::rename(source, target).map_err(io_error)
}

pub fn copy_path(source: &Path, target: &Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(source).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::invalid_path(
            "copying symbolic links is not supported",
        ));
    }
    if metadata.is_dir() {
        std::fs::create_dir(target).map_err(io_error)?;
        for entry in std::fs::read_dir(source).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name();
            copy_path(&entry.path(), &target.join(name))?;
        }
    } else if metadata.is_file() {
        std::fs::copy(source, target).map_err(io_error)?;
    } else {
        return Err(AppError::invalid_path("unsupported file type"));
    }
    Ok(())
}

pub fn move_path(source: &Path, target: &Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(source).map_err(io_error)?;
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if crate::platform::is_cross_device_error(&error) => {
            if metadata.file_type().is_symlink() {
                return Err(AppError::invalid_path(
                    "moving symbolic links across filesystems is not supported",
                ));
            }
            copy_path(source, target)?;
            if metadata.is_dir() {
                std::fs::remove_dir_all(source).map_err(io_error)
            } else {
                std::fs::remove_file(source).map_err(io_error)
            }
        }
        Err(error) => Err(io_error(error)),
    }
}

pub fn unique_target(target: &Path) -> PathBuf {
    if !target.exists() {
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
        if !candidate.exists() {
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
