use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::UNIX_EPOCH;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_HANDLE_EOF, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    GetVolumePathNameW, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{FSCTL_ENUM_USN_DATA, MFT_ENUM_DATA_V0, USN_RECORD_V2};

use crate::db::IndexEntry;

pub struct ScannedDir {
    pub path: PathBuf,
    pub entries: Vec<IndexEntry>,
}

struct MftRecord {
    file_ref: u64,
    parent_ref: u64,
    name: String,
    is_dir: bool,
    is_reparse: bool,
}

pub fn scan(base: &Path, recursive: bool) -> Result<Vec<ScannedDir>, String> {
    if !recursive {
        return Err("MFT scan is only used for recursive indexing".to_owned());
    }
    let volume_path = volume_path(base)?;
    let volume = volume_device(&volume_path)?;
    let records = enumerate_mft(&volume)?;
    let root_ref = file_reference(base)?;

    let mut by_parent: HashMap<u64, Vec<MftRecord>> = HashMap::new();
    for record in records {
        by_parent.entry(record.parent_ref).or_default().push(record);
    }

    let mut queue = VecDeque::from([(root_ref, base.to_path_buf())]);
    let mut visited = HashSet::new();
    let mut scans = Vec::new();

    while let Some((parent, directory)) = queue.pop_front() {
        if !visited.insert(parent) {
            continue;
        }
        let mut entries = Vec::new();
        if let Some(children) = by_parent.get(&parent) {
            for child in children {
                if child.name.starts_with(".filehub-upload-") || child.is_reparse {
                    continue;
                }
                let path = directory.join(&child.name);
                let Ok(metadata) = std::fs::metadata(&path) else {
                    continue;
                };
                if !metadata.is_file() && !metadata.is_dir() {
                    continue;
                }
                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or_default();
                entries.push(IndexEntry {
                    name: child.name.clone(),
                    real_path: path.to_string_lossy().to_string(),
                    is_dir: metadata.is_dir(),
                    size: if metadata.is_file() {
                        metadata.len() as i64
                    } else {
                        0
                    },
                    mtime,
                });
                if child.is_dir {
                    queue.push_back((child.file_ref, path));
                }
            }
        }
        scans.push(ScannedDir {
            path: directory,
            entries,
        });
    }

    Ok(scans)
}

fn volume_path(path: &Path) -> Result<PathBuf, String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: both buffers are valid and NUL-terminated/appropriately sized.
    let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
    if ok == 0 {
        return Err(format!(
            "GetVolumePathNameW failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let length = buffer.iter().position(|value| *value == 0).unwrap_or(0);
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

fn volume_device(volume_path: &Path) -> Result<PathBuf, String> {
    let value = volume_path.to_string_lossy();
    let value = value.trim_end_matches('\\');
    let drive = value.strip_prefix("\\\\?\\").unwrap_or(value);
    if drive.len() != 2 || !drive.ends_with(':') {
        return Err("only drive-letter volumes are supported for MFT indexing".to_owned());
    }
    Ok(PathBuf::from(format!(r"\\.\{drive}")))
}

fn file_reference(path: &Path) -> Result<u64, String> {
    let handle = open_handle(path, FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: handle is valid and info is a valid output pointer.
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    // SAFETY: the handle was opened above and is not used after this point.
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(format!(
            "GetFileInformationByHandle failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64)
}

fn open_handle(path: &Path, access: u32, flags: u32) -> Result<HANDLE, String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the path buffer is NUL-terminated; all other arguments are constants.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "CreateFileW failed for {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(handle)
}

fn enumerate_mft(volume: &Path) -> Result<Vec<MftRecord>, String> {
    let volume_wide: Vec<u16> = volume.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the volume path is NUL-terminated; all other arguments are constants.
    let handle = unsafe {
        CreateFileW(
            volume_wide.as_ptr(),
            0x8000_0000, // GENERIC_READ
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "failed to open volume {}: {}",
            volume.display(),
            std::io::Error::last_os_error()
        ));
    }

    let mut records = Vec::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut start = 0_u64;
    loop {
        let input = MFT_ENUM_DATA_V0 {
            StartFileReferenceNumber: start,
            LowUsn: 0,
            HighUsn: i64::MAX,
        };
        let mut returned = 0_u32;
        // SAFETY: input and output buffers are valid for the duration of the call.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                &input as *const MFT_ENUM_DATA_V0 as *const _,
                std::mem::size_of::<MFT_ENUM_DATA_V0>() as u32,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
                break;
            }
            // SAFETY: the handle was opened above and is closed here on the error path.
            unsafe { CloseHandle(handle) };
            return Err(format!("FSCTL_ENUM_USN_DATA failed: {error}"));
        }
        if returned < 8 {
            break;
        }
        start = u64::from_ne_bytes(buffer[0..8].try_into().unwrap_or_default());
        let mut offset = 8_usize;
        while offset + 8 <= returned as usize {
            let record_len =
                u32::from_ne_bytes(buffer[offset..offset + 4].try_into().unwrap_or_default())
                    as usize;
            if record_len < 60 || offset + record_len > returned as usize {
                break;
            }
            // SAFETY: record_len was validated against the output buffer.
            let record =
                unsafe { (buffer.as_ptr().add(offset) as *const USN_RECORD_V2).read_unaligned() };
            let name_offset = record.FileNameOffset as usize;
            let name_bytes = record.FileNameLength as usize;
            if name_offset + name_bytes <= record_len {
                let name_wide = unsafe {
                    std::slice::from_raw_parts(
                        buffer.as_ptr().add(offset + name_offset) as *const u16,
                        name_bytes / 2,
                    )
                };
                let name = String::from_utf16_lossy(name_wide);
                records.push(MftRecord {
                    file_ref: record.FileReferenceNumber,
                    parent_ref: record.ParentFileReferenceNumber,
                    name,
                    is_dir: record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                    is_reparse: record.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
                });
            }
            offset += record_len;
        }
    }
    // SAFETY: handle was opened above and is not used afterwards.
    unsafe { CloseHandle(handle) };
    Ok(records)
}

#[allow(dead_code)]
fn _os(value: &OsStr) -> bool {
    value.is_empty()
}
