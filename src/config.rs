use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub listen: SocketAddr,
    pub db_path: PathBuf,
    pub users_root: PathBuf,
    pub cache_dir: PathBuf,
    pub thumbnails: ThumbnailsSettings,
    pub symlinks: SymlinkSettings,
    pub index: IndexSettings,
    pub uploads: UploadSettings,
    pub downloads: DownloadSettings,
    pub editor: EditorSettings,
    pub ui: UiSettings,
    pub security: SecuritySettings,
    pub jobs: JobsSettings,
    pub log: LogSettings,
    pub default_quota_bytes: i64,
    pub terminal_enabled: bool,
    pub show_hidden_default: bool,
    pub upload_conflict_default: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 8080)),
            db_path: crate::platform::default_data_dir().join("filehub.db"),
            users_root: crate::platform::default_data_dir().join("users"),
            cache_dir: crate::platform::default_data_dir().join("cache"),
            thumbnails: ThumbnailsSettings::default(),
            symlinks: SymlinkSettings::default(),
            index: IndexSettings::default(),
            uploads: UploadSettings::default(),
            downloads: DownloadSettings::default(),
            editor: EditorSettings::default(),
            ui: UiSettings::default(),
            security: SecuritySettings::default(),
            jobs: JobsSettings::default(),
            log: LogSettings::default(),
            default_quota_bytes: 15 * 1024 * 1024 * 1024,
            terminal_enabled: true,
            show_hidden_default: true,
            upload_conflict_default: "rename".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThumbnailsSettings {
    pub enabled: bool,
    pub workers: usize,
    /// Reserved workers for full-size preview thumbnails. List thumbnails use
    /// the remaining workers, so scrolling a folder cannot block a preview.
    pub preview_workers: usize,
    /// Long-edge limit for lightbox preview thumbnails (512..=4096).
    pub preview_edge: u32,
    pub max_source_bytes: u64,
    pub max_pixels: u64,
    pub jpeg_max_pixels: u64,
    pub video: bool,
    pub video_seek_seconds: u64,
    pub video_seek_percent: u32,
    pub video_timeout_secs: u64,
    pub cache_max_bytes: u64,
}

impl Default for ThumbnailsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            workers: 2,
            preview_workers: 1,
            preview_edge: 2048,
            max_source_bytes: 512 * 1024 * 1024,
            max_pixels: 50_000_000,
            jpeg_max_pixels: 1_000_000_000,
            video: true,
            video_seek_seconds: 1,
            video_seek_percent: 33,
            video_timeout_secs: 15,
            cache_max_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SymlinkSettings {
    pub allow_create: bool,
    pub follow_in_index: bool,
    pub follow_admin_links: bool,
    pub max_depth: usize,
    pub max_hops: usize,
}

impl Default for SymlinkSettings {
    fn default() -> Self {
        Self {
            allow_create: true,
            follow_in_index: false,
            follow_admin_links: true,
            max_depth: 32,
            max_hops: 40,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexSettings {
    pub ignore: Vec<String>,
    pub max_entries_per_dir: usize,
    pub auto_index_on_browse: bool,
    pub scheduled_scans: bool,
    pub quick_scan_minutes: u64,
    pub full_scan_hours: u64,
}

impl Default for IndexSettings {
    fn default() -> Self {
        Self {
            ignore: vec![".git".into(), "node_modules".into(), "@eaDir".into()],
            max_entries_per_dir: 200_000,
            auto_index_on_browse: true,
            scheduled_scans: true,
            quick_scan_minutes: 10,
            full_scan_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UploadSettings {
    pub max_file_bytes: u64,
    pub chunk_size: u64,
    pub session_ttl_hours: u64,
    pub concurrency_per_user: usize,
}

impl Default for UploadSettings {
    fn default() -> Self {
        Self {
            max_file_bytes: 10 * 1024 * 1024 * 1024,
            chunk_size: 8 * 1024 * 1024,
            session_ttl_hours: 24,
            // Paused uploads keep their session, so the limit must be well above
            // the three concurrent transfers the UI runs.
            concurrency_per_user: 32,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadSettings {
    pub zip_max_bytes: u64,
    pub zip_max_files: usize,
    pub zip_concurrency: usize,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            zip_max_bytes: 2 * 1024 * 1024 * 1024,
            zip_max_files: 1000,
            zip_concurrency: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub enabled: bool,
    pub max_file_bytes: u64,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_file_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub default_theme: String,
    pub default_locale: String,
    pub locales: Vec<String>,
    pub brand_name: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            default_theme: "system".into(),
            default_locale: "en".into(),
            locales: vec!["zh-CN".into(), "en".into()],
            brand_name: "FileHub".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecuritySettings {
    pub session_ttl_hours: u64,
    pub cookie_secure: bool,
    pub trusted_proxies: Vec<String>,
    pub public_rate_limit_per_minute: u32,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            session_ttl_hours: 168,
            cookie_secure: false,
            trusted_proxies: Vec::new(),
            public_rate_limit_per_minute: 240,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JobsSettings {
    pub io_workers: usize,
    pub interactive_workers: usize,
    pub dir_size_workers: usize,
    pub queue_max: usize,
    pub thumbnail_queue_max: usize,
    pub dir_size_ttl_seconds: u64,
    pub dir_size_error_retry_seconds: u64,
    pub usage_cache_seconds: u64,
    pub dir_size_use_logical: bool,
    pub audit_retention_days: u64,
    pub audit_max_rows: u64,
    pub job_retention_days: u64,
}

impl Default for JobsSettings {
    fn default() -> Self {
        Self {
            io_workers: 1,
            interactive_workers: 2,
            dir_size_workers: 1,
            queue_max: 256,
            thumbnail_queue_max: 64,
            dir_size_ttl_seconds: 300,
            dir_size_error_retry_seconds: 60,
            usage_cache_seconds: 900,
            dir_size_use_logical: true,
            audit_retention_days: 90,
            audit_max_rows: 200_000,
            job_retention_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogSettings {
    pub level: String,
    pub format: String,
    pub retention_days: u64,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: "text".into(),
            retention_days: 14,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.thumbnails.workers == 0 {
            bail!("thumbnails.workers must be greater than zero");
        }
        if self.thumbnails.preview_workers == 0 {
            bail!("thumbnails.preview_workers must be greater than zero");
        }
        if !(512..=4096).contains(&self.thumbnails.preview_edge) {
            bail!("thumbnails.preview_edge must be between 512 and 4096");
        }
        if self.thumbnails.video_seek_percent > 100 {
            bail!("thumbnails.video_seek_percent must be between 0 and 100");
        }
        if self.uploads.chunk_size == 0 {
            bail!("uploads.chunk_size must be greater than zero");
        }
        if self.uploads.session_ttl_hours == 0 {
            bail!("uploads.session_ttl_hours must be greater than zero");
        }
        if self.downloads.zip_concurrency == 0 {
            bail!("downloads.zip_concurrency must be greater than zero");
        }
        if self.security.session_ttl_hours == 0 {
            bail!("security.session_ttl_hours must be greater than zero");
        }
        if self.log.retention_days > 3650 {
            bail!("log.retention_days must not exceed 3650");
        }
        let brand_name = self.ui.brand_name.trim();
        if brand_name.is_empty() {
            bail!("ui.brand_name must not be empty");
        }
        if brand_name.chars().count() > 64 {
            bail!("ui.brand_name must not exceed 64 characters");
        }
        for rule in &self.security.trusted_proxies {
            if !crate::auth::valid_ip_rule(rule) {
                bail!("security.trusted_proxies contains an invalid IP or CIDR: {rule}");
            }
        }
        if self.jobs.io_workers == 0 {
            bail!("jobs.io_workers must be greater than zero");
        }
        if self.jobs.interactive_workers == 0 {
            bail!("jobs.interactive_workers must be greater than zero");
        }
        if self.jobs.dir_size_workers == 0 {
            bail!("jobs.dir_size_workers must be greater than zero");
        }
        if self.jobs.queue_max == 0 {
            bail!("jobs.queue_max must be greater than zero");
        }
        if self.jobs.thumbnail_queue_max == 0 {
            bail!("jobs.thumbnail_queue_max must be greater than zero");
        }
        if self.index.scheduled_scans {
            if self.index.quick_scan_minutes == 0 {
                bail!("index.quick_scan_minutes must be greater than zero");
            }
            if self.index.full_scan_hours == 0 {
                bail!("index.full_scan_hours must be greater than zero");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Default,
    Config,
    Db,
}

#[derive(Debug, Clone)]
pub struct ConfigFile {
    pub value: Value,
    pub keys: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub settings: Settings,
    pub effective: Value,
    pub sources: BTreeMap<String, Source>,
    pub restart_required: Vec<String>,
}

pub fn default_value() -> Value {
    serde_json::to_value(Settings::default()).unwrap_or_else(|_| Value::Object(Map::new()))
}

pub fn load_config_file(path: Option<&Path>) -> anyhow::Result<ConfigFile> {
    let path = match path {
        Some(path) => Some(path.to_path_buf()),
        None => {
            let default = crate::platform::default_config_path();
            default.exists().then_some(default)
        }
    };

    let value = match path {
        Some(path) => {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read config file {}", path.display()))?;
            let value: Value = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse config file {}", path.display()))?;
            if !value.is_object() {
                bail!("config file must contain a JSON object");
            }
            value
        }
        None => Value::Object(Map::new()),
    };

    let mut keys = BTreeSet::new();
    flatten_keys(&value, "", &mut keys);

    Ok(ConfigFile { value, keys })
}

pub fn build_snapshot(
    config: &ConfigFile,
    overrides: &[(String, String)],
) -> anyhow::Result<Snapshot> {
    let defaults = default_value();
    let mut effective = defaults.clone();
    merge_value(&mut effective, &config.value);
    apply_overrides(&mut effective, overrides)?;

    let settings: Settings =
        serde_json::from_value(effective.clone()).context("invalid effective settings")?;
    settings.validate()?;

    let db_keys: BTreeSet<String> = overrides
        .iter()
        .map(|(key, _)| key.clone())
        .filter(|key| !is_bootstrap_key(key))
        .collect();
    let sources = compute_sources(&defaults, &config.keys, &db_keys);
    let restart_required = if db_keys.contains("listen") {
        vec!["listen".to_owned()]
    } else {
        Vec::new()
    };

    Ok(Snapshot {
        settings,
        effective,
        sources,
        restart_required,
    })
}

pub fn ensure_directories(snapshot: &Snapshot) -> anyhow::Result<()> {
    let settings = &snapshot.settings;
    if let Some(parent) = settings
        .db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        ensure_directory(parent, "database directory", false)?;
    }

    #[cfg(unix)]
    {
        // Linux deployments must be able to start with an empty data root.
        ensure_directory(&settings.users_root, "users root", true)?;
        ensure_directory(&settings.cache_dir, "cache directory", true)?;
    }

    #[cfg(not(unix))]
    {
        ensure_directory(&settings.cache_dir, "cache directory", false)?;
        let users_root_is_default = snapshot
            .sources
            .get("users_root")
            .is_some_and(|source| *source == Source::Default);
        if users_root_is_default {
            ensure_directory(&settings.users_root, "users root", false)?;
        } else {
            let metadata = std::fs::metadata(&settings.users_root).with_context(|| {
                format!(
                    "users root does not exist: {}",
                    settings.users_root.display()
                )
            })?;
            if !metadata.is_dir() {
                bail!(
                    "users root is not a directory: {}",
                    settings.users_root.display()
                );
            }
        }
    }

    Ok(())
}

fn ensure_directory(path: &Path, label: &str, private: bool) -> anyhow::Result<()> {
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_dir() => {
            bail!("{label} is not a directory: {}", path.display());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path).with_context(|| {
                format!(
                    "failed to create {label} {}; check ownership and permissions",
                    path.display()
                )
            })?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to access {label} {}", path.display()));
        }
    }

    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {label} {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = private;

    Ok(())
}

pub fn is_known_key(key: &str) -> bool {
    get_dotted(&default_value(), key).is_some()
}

pub fn is_bootstrap_key(key: &str) -> bool {
    key == "db_path" || key == "log.level" || key == "log.format"
}

fn merge_value(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key) {
                    Some(existing) => merge_value(existing, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

fn apply_overrides(base: &mut Value, overrides: &[(String, String)]) -> anyhow::Result<()> {
    for (key, raw) in overrides {
        if is_bootstrap_key(key) {
            continue;
        }

        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()));
        set_dotted(base, key, value)
            .with_context(|| format!("invalid settings override key {key}"))?;
    }
    Ok(())
}

fn set_dotted(root: &mut Value, path: &str, value: Value) -> anyhow::Result<()> {
    let parts: Vec<&str> = path.split('.').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        bail!("empty settings key");
    }
    set_dotted_parts(root, &parts, value)
}

fn set_dotted_parts(current: &mut Value, parts: &[&str], value: Value) -> anyhow::Result<()> {
    let Some((head, tail)) = parts.split_first() else {
        bail!("empty settings key");
    };

    let object = current
        .as_object_mut()
        .context("settings path parent is not an object")?;

    if tail.is_empty() {
        object.insert((*head).to_owned(), value);
        return Ok(());
    }

    let next = object
        .entry((*head).to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    set_dotted_parts(next, tail, value)
}

fn get_dotted<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.').filter(|part| !part.is_empty()) {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn flatten_keys(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) if !object.is_empty() => {
            for (key, value) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_keys(value, &path, out);
            }
        }
        _ if !prefix.is_empty() => {
            out.insert(prefix.to_owned());
        }
        _ => {}
    }
}

fn compute_sources(
    defaults: &Value,
    config_keys: &BTreeSet<String>,
    db_keys: &BTreeSet<String>,
) -> BTreeMap<String, Source> {
    let mut default_keys = BTreeSet::new();
    flatten_keys(defaults, "", &mut default_keys);

    let all_keys: BTreeSet<String> = default_keys
        .into_iter()
        .chain(config_keys.iter().cloned())
        .chain(db_keys.iter().cloned())
        .collect();

    all_keys
        .into_iter()
        .map(|key| {
            let source = if db_keys.contains(&key) {
                Source::Db
            } else if config_keys.contains(&key) {
                Source::Config
            } else {
                Source::Default
            };
            (key, source)
        })
        .collect()
}
