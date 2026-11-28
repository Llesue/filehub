use std::fs;

#[test]
fn hard_links_count_bytes_once_and_paths_separately() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.bin");
    fs::write(&original, b"hello hardlink").unwrap();
    fs::hard_link(&original, dir.path().join("link-1")).unwrap();
    fs::hard_link(&original, dir.path().join("link-2")).unwrap();

    let (bytes, entries) = filehub::fsops::path_usage(dir.path()).unwrap();

    assert_eq!(bytes, b"hello hardlink".len() as u64);
    assert_eq!(entries, 3, "each hard link path is its own directory entry");
}

#[test]
fn independent_files_keep_their_own_bytes_and_entries() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.bin"), b"aaaa").unwrap();
    fs::write(dir.path().join("b.bin"), b"bbbbbb").unwrap();

    let (bytes, entries) = filehub::fsops::path_usage(dir.path()).unwrap();

    assert_eq!(bytes, 10);
    assert_eq!(entries, 2);
}

#[cfg(unix)]
#[test]
fn symlinks_do_not_follow_or_inflate_usage() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.bin");
    fs::write(&target, b"12345678").unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();

    let (bytes, entries) = filehub::fsops::path_usage(dir.path()).unwrap();

    assert_eq!(bytes, 8);
    assert_eq!(entries, 2);
}
