use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::UNIX_EPOCH;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_HANDLE_EOF, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR,
    FILE_ID_DESCRIPTOR_0, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdType, GetFileInformationByHandle,
    GetFinalPathNameByHandleW, GetVolumePathNameW, OPEN_EXISTING, OpenFileById, VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, MFT_ENUM_DATA_V0,
    READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0, USN_REASON_BASIC_INFO_CHANGE,
    USN_REASON_DATA_EXTEND, USN_REASON_DATA_OVERWRITE, USN_REASON_DATA_TRUNCATION,
    USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE, USN_REASON_RENAME_NEW_NAME,
    USN_REASON_RENAME_OLD_NAME, USN_RECORD_V2,
};

use crate::db::{IndexEntry, UsnState};

pub struct ScannedDir {
    pub path: PathBuf,
    pub entries: Vec<IndexEntry>,
}

pub struct IncrementalChanges {
    pub scans: Vec<ScannedDir>,
    pub removed: Vec<PathBuf>,
    pub new_dirs: Vec<PathBuf>,
    pub state: UsnState,
    pub complete: bool,
}

const USN_REASON_MASK: u32 = USN_REASON_FILE_CREATE
    | USN_REASON_FILE_DELETE
    | USN_REASON_RENAME_OLD_NAME
    | USN_REASON_RENAME_NEW_NAME
    | USN_REASON_DATA_OVERWRITE
    | USN_REASON_DATA_EXTEND
    | USN_REASON_DATA_TRUNCATION
    | USN_REASON_BASIC_INFO_CHANGE;

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
                let created_at = metadata
                    .created()
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
                    created_at,
                    mime_type: String::new(),
                    has_preview: false,
                    is_hidden: false,
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

pub fn journal_state(base: &Path) -> Result<UsnState, String> {
    let volume_path = volume_path(base)?;
    let volume = volume_path.to_string_lossy().to_string();
    let handle = open_volume(&volume_path)?;
    let journal = query_journal(handle)?;
    // SAFETY: handle was opened by open_volume and is not used afterwards.
    unsafe { CloseHandle(handle) };
    Ok(UsnState {
        volume,
        journal_id: journal.UsnJournalID,
        next_usn: journal.NextUsn,
    })
}

pub fn incremental(
    base: &Path,
    previous: Option<&UsnState>,
    ignore: &[String],
) -> Result<Option<IncrementalChanges>, String> {
    let Some(previous) = previous else {
        return Ok(None);
    };
    let volume_path = volume_path(base)?;
    let volume = volume_path.to_string_lossy().to_string();
    if previous.volume != volume {
        return Ok(None);
    }
    let handle = open_volume(&volume_path)?;
    let journal = match query_journal(handle) {
        Ok(journal) => journal,
        Err(error) => {
            // SAFETY: handle is valid until closed here.
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
    };
    if journal.UsnJournalID != previous.journal_id || previous.next_usn < journal.LowestValidUsn {
        // SAFETY: handle is valid until closed here.
        unsafe { CloseHandle(handle) };
        return Ok(None);
    }

    let mut next_usn = previous.next_usn;
    let mut changed_parents: HashSet<PathBuf> = HashSet::new();
    let mut removed: HashSet<PathBuf> = HashSet::new();
    let mut new_dirs: HashSet<PathBuf> = HashSet::new();
    let mut parent_paths: HashMap<u64, Option<PathBuf>> = HashMap::new();
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let input = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: next_usn,
            ReasonMask: USN_REASON_MASK,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: journal.UsnJournalID,
        };
        let mut returned = 0_u32;
        // SAFETY: input and output buffers are valid for the duration of the call.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_READ_USN_JOURNAL,
                &input as *const READ_USN_JOURNAL_DATA_V0 as *const _,
                std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            // SAFETY: handle was opened above and is not used afterwards.
            unsafe { CloseHandle(handle) };
            if error.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
                break;
            }
            return Ok(None);
        }
        if returned < 8 {
            break;
        }
        let batch_next = i64::from_ne_bytes(buffer[0..8].try_into().unwrap_or_default());
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
            offset += record_len;
            if record.MajorVersion != 2 {
                continue;
            }
            let name_offset = record.FileNameOffset as usize;
            let name_bytes = record.FileNameLength as usize;
            if name_offset + name_bytes > record_len {
                continue;
            }
            // SAFETY: the name range was validated against the record length.
            let name_wide = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().add(offset - record_len + name_offset) as *const u16,
                    name_bytes / 2,
                )
            };
            let name = String::from_utf16_lossy(name_wide);
            let Some(parent) =
                resolve_parent(handle, record.ParentFileReferenceNumber, &mut parent_paths)
            else {
                continue;
            };
            let parent = normalize_like_base(parent, base);
            let path = parent.join(&name);
            if !crate::platform::path_within(&parent, base)
                && !crate::platform::path_within(&path, base)
            {
                continue;
            }
            let reason = record.Reason;
            if reason & USN_REASON_MASK == 0 {
                continue;
            }
            let is_dir = record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            let is_reparse = record.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
            if is_reparse {
                continue;
            }
            changed_parents.insert(parent.clone());
            if reason & (USN_REASON_FILE_DELETE | USN_REASON_RENAME_OLD_NAME) != 0 && is_dir {
                removed.insert(path.clone());
            }
            if is_dir && reason & (USN_REASON_FILE_CREATE | USN_REASON_RENAME_NEW_NAME) != 0 {
                new_dirs.insert(path);
            }
        }
        if batch_next <= next_usn {
            break;
        }
        next_usn = batch_next;
        if next_usn >= journal.NextUsn {
            break;
        }
    }
    // SAFETY: handle was opened above and is not used afterwards.
    unsafe { CloseHandle(handle) };

    let mut scans = Vec::new();
    let mut complete = true;
    for parent in changed_parents {
        match scan_one_dir(&parent, ignore) {
            Ok(scan) => scans.push(scan),
            Err(_) => complete = false,
        }
    }
    let mut removed: Vec<PathBuf> = removed.into_iter().collect();
    removed.sort();
    let mut new_dirs: Vec<PathBuf> = new_dirs.into_iter().collect();
    new_dirs.sort();

    Ok(Some(IncrementalChanges {
        scans,
        removed,
        new_dirs,
        state: UsnState {
            volume,
            journal_id: journal.UsnJournalID,
            next_usn,
        },
        complete,
    }))
}

pub fn volume_name(base: &Path) -> Result<String, String> {
    volume_path(base).map(|path| path.to_string_lossy().to_string())
}

fn normalize_like_base(path: PathBuf, base: &Path) -> PathBuf {
    let base_value = base.to_string_lossy();
    let path_value = path.to_string_lossy();
    if base_value.starts_with(r"\\?\") && !path_value.starts_with(r"\\?\") {
        if let Some(rest) = path_value.strip_prefix(r"\\") {
            return PathBuf::from(format!(r"\\?\UNC\{rest}"));
        }
        return PathBuf::from(format!(r"\\?\{path_value}"));
    }
    path
}

fn open_volume(volume_path: &Path) -> Result<HANDLE, String> {
    let volume = volume_device(volume_path)?;
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
    Ok(handle)
}

fn query_journal(handle: HANDLE) -> Result<USN_JOURNAL_DATA_V0, String> {
    let mut journal = USN_JOURNAL_DATA_V0::default();
    let mut returned = 0_u32;
    // SAFETY: the output structure is valid for the duration of the call.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            ptr::null(),
            0,
            &mut journal as *mut USN_JOURNAL_DATA_V0 as *mut _,
            std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(format!(
            "FSCTL_QUERY_USN_JOURNAL failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(journal)
}

fn resolve_parent(
    volume_handle: HANDLE,
    parent_ref: u64,
    cache: &mut HashMap<u64, Option<PathBuf>>,
) -> Option<PathBuf> {
    if let Some(cached) = cache.get(&parent_ref) {
        return cached.clone();
    }
    let path = file_path_by_id(volume_handle, parent_ref).ok();
    cache.insert(parent_ref, path.clone());
    path
}

fn file_path_by_id(volume_handle: HANDLE, file_id: u64) -> Result<PathBuf, String> {
    let descriptor = FILE_ID_DESCRIPTOR {
        dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: FileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 {
            FileId: file_id as i64,
        },
    };
    // SAFETY: descriptor is a valid FILE_ID_DESCRIPTOR and all other arguments are constants.
    let handle = unsafe {
        OpenFileById(
            volume_handle,
            &descriptor,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "OpenFileById failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut buffer = vec![0u16; 1024];
    let path = loop {
        // SAFETY: buffer is valid for the requested capacity.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            // SAFETY: handle was opened above and is closed here.
            unsafe { CloseHandle(handle) };
            return Err(format!(
                "GetFinalPathNameByHandleW failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        if length as usize <= buffer.len() {
            buffer.truncate(length as usize);
            break String::from_utf16_lossy(&buffer);
        }
        buffer.resize(length as usize + 1, 0);
    };
    // SAFETY: handle was opened above and is not used afterwards.
    unsafe { CloseHandle(handle) };
    let path = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        path
    };
    Ok(PathBuf::from(path))
}

fn scan_one_dir(path: &Path, ignore: &[String]) -> Result<ScannedDir, String> {
    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(path).map_err(|error| error.to_string())?;
    for entry in read_dir {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".filehub-upload-")
            || matches!(
                name.as_str(),
                "$RECYCLE.BIN" | "System Volume Information" | "@eaDir"
            )
            || ignore.iter().any(|ignored| ignored == &name)
        {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            continue;
        }
        if !metadata.is_file() && !metadata.is_dir() {
            continue;
        }
        let mtime = metadata
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
        entries.push(IndexEntry {
            name: name.clone(),
            real_path: entry.path().to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            size: if metadata.is_file() {
                metadata.len().min(i64::MAX as u64) as i64
            } else {
                0
            },
            mtime,
            created_at,
            mime_type: String::new(),
            has_preview: false,
            is_hidden: false,
        });
    }
    Ok(ScannedDir {
        path: path.to_path_buf(),
        entries,
    })
}
