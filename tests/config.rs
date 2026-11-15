use filehub::config::{Source, build_snapshot, load_config_file};

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
        std::path::PathBuf::from("./data/filehub.db")
    );
}
