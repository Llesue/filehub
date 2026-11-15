use std::fs::{Metadata, OpenOptions};
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub fn default_config_path() -> PathBuf {
    program_data().join("config.json")
}

#[cfg(not(windows))]
pub fn default_config_path() -> PathBuf {
    PathBuf::from("/etc/filehub/config.json")
}

#[cfg(windows)]
pub fn default_data_dir() -> PathBuf {
    program_data()
}

#[cfg(not(windows))]
pub fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}

#[cfg(windows)]
fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("FileHub")
}

pub fn set_private_mode(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = options;
}

pub fn metadata_identity(metadata: &Metadata) -> (i64, i64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev() as i64, metadata.ino() as i64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        (
            metadata.last_write_time() as i64,
            metadata.file_size() as i64,
        )
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = metadata;
        (0, 0)
    }
}

pub fn metadata_mtime(metadata: &Metadata) -> i64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.mtime()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.last_write_time() as i64
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = metadata;
        0
    }
}

#[cfg(windows)]
fn normalized_windows_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/").to_lowercase();
    value
        .strip_prefix("//?/unc/")
        .map(|value| format!("//{value}"))
        .or_else(|| value.strip_prefix("//?/").map(str::to_owned))
        .unwrap_or(value)
}

pub fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        normalized_windows_path(left) == normalized_windows_path(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub fn path_within(path: &Path, base: &Path) -> bool {
    #[cfg(windows)]
    {
        let path = normalized_windows_path(path);
        let base = normalized_windows_path(base);
        let base = base.trim_end_matches('/');
        path == base || path.starts_with(&format!("{base}/"))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(base)
    }
}

pub fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "symbolic links are unsupported",
        ))
    }
}

pub fn is_cross_device_error(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(18)
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(17)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        false
    }
}
