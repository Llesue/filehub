use filehub::config::{Source, build_snapshot, ensure_directories, load_config_file};

#[test]
fn config_file_wins_over_defaults_and_db_wins_over_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{
            "listen": "127.0.0.1:18080",
            "thumbnails": { "enabled": false }
        }"#,
    )
    .unwrap();

    let config = load_config_file(Some(&path)).unwrap();
    let snapshot = build_snapshot(&config, &[]).unwrap();

    assert_eq!(snapshot.settings.listen.port(), 18080);
    assert!(!snapshot.settings.thumbnails.enabled);
    assert_eq!(
        snapshot.sources.get("thumbnails.enabled"),
        Some(&Source::Config)
    );

    let snapshot = build_snapshot(
        &config,
        &[("thumbnails.enabled".to_owned(), "true".to_owned())],
    )
    .unwrap();

    assert!(snapshot.settings.thumbnails.enabled);
    assert_eq!(
        snapshot.sources.get("thumbnails.enabled"),
        Some(&Source::Db)
    );
}

#[test]
fn bootstrap_only_keys_cannot_be_overridden_by_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, r#"{"listen":"127.0.0.1:18080"}"#).unwrap();

    let config = load_config_file(Some(&path)).unwrap();
    let snapshot = build_snapshot(
        &config,
        &[("db_path".to_owned(), r#""./elsewhere.db""#.to_owned())],
    )
    .unwrap();

    assert_eq!(snapshot.sources.get("db_path"), Some(&Source::Default));
    assert_eq!(
        snapshot.settings.db_path,
        filehub::platform::default_data_dir().join("filehub.db")
    );
}

#[test]
fn ensure_directories_creates_the_data_root_and_secures_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        serde_json::json!({
            "db_path": root.join("db/filehub.db").to_string_lossy(),
            "users_root": root.join("users").to_string_lossy(),
            "cache_dir": root.join("cache").to_string_lossy(),
        })
        .to_string(),
    )
    .unwrap();
    let config = load_config_file(Some(&config_path)).unwrap();
    let snapshot = build_snapshot(&config, &[]).unwrap();
    ensure_directories(&snapshot).unwrap();
    assert!(root.join("db").is_dir());
    assert!(root.join("users").is_dir());
    assert!(root.join("cache").is_dir());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [root.join("users"), root.join("cache")] {
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }
}

#[test]
fn ensure_directories_reports_a_file_instead_of_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let conflict = dir.path().join("users-file");
    std::fs::write(&conflict, b"not a directory").unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        serde_json::json!({
            "db_path": dir.path().join("db/filehub.db").to_string_lossy(),
            "users_root": conflict.to_string_lossy(),
            "cache_dir": dir.path().join("cache").to_string_lossy(),
        })
        .to_string(),
    )
    .unwrap();
    let config = load_config_file(Some(&config_path)).unwrap();
    let snapshot = build_snapshot(&config, &[]).unwrap();
    let error = ensure_directories(&snapshot).unwrap_err().to_string();
    assert!(error.contains("users root is not a directory"), "{error}");
}
