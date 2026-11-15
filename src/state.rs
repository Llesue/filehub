use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::auth::{RateLimiter, random_token};
use crate::config::{ConfigFile, Snapshot, build_snapshot, ensure_directories, load_config_file};
use crate::db::Db;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeleteItem {
    pub relative: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct DeletePreview {
    pub user_id: i64,
    pub root_id: i64,
    pub items: Vec<DeleteItem>,
    pub expires_at: i64,
    pub confirm_code: String,
}

#[derive(Debug, Clone)]
pub struct ShareTicket {
    pub share_id: i64,
    pub expires_at: i64,
}

pub struct SetupState {
    pub initialized: AtomicBool,
    pub token: String,
}

pub struct AppState {
    pub db: Db,
    pub config: ConfigFile,
    snapshot: ArcSwap<Snapshot>,
    pub setup: SetupState,
    pub rate_limiter: RateLimiter,
    pub delete_tokens: Mutex<HashMap<String, DeletePreview>>,
    pub share_tickets: Mutex<HashMap<String, ShareTicket>>,
    pub ffmpeg: Option<PathBuf>,
    pub index_jobs: Mutex<HashSet<String>>,
    pub thumb_jobs: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub thumb_semaphore: Arc<Semaphore>,
}

impl AppState {
    pub async fn new(config_path: Option<&Path>) -> anyhow::Result<Arc<Self>> {
        Self::build(config_path, None).await
    }

    pub async fn new_test(base: &Path, setup_token: &str) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(base.join("users"))?;
        std::fs::create_dir_all(base.join("cache"))?;
        let config_path = base.join("config.json");
        let config = serde_json::json!({
            "listen": "127.0.0.1:0",
            "db_path": base.join("filehub.db").to_string_lossy(),
            "users_root": base.join("users").to_string_lossy(),
            "cache_dir": base.join("cache").to_string_lossy(),
            "security": {
                "session_ttl_hours": 1
            }
        });
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config)?)?;
        Self::build(Some(&config_path), Some(setup_token.to_owned())).await
    }

    async fn build(
        config_path: Option<&Path>,
        explicit_setup_token: Option<String>,
    ) -> anyhow::Result<Arc<Self>> {
        let config = load_config_file(config_path)?;
        let bootstrap = build_snapshot(&config, &[])?;
        ensure_directories(&bootstrap)?;

        let db = Db::open(&bootstrap.settings.db_path, 4)?;
        ensure_default_roots(&db).await?;
        let overrides = db.settings_overrides().await?;
        let snapshot = build_snapshot(&config, &overrides)?;
        ensure_directories(&snapshot)?;

        let initialized = db.count_users().await? > 0;
        let token = match explicit_setup_token {
            Some(token) => token,
            None => match std::env::var("FILEHUB_SETUP_TOKEN") {
                Ok(token) if !token.is_empty() => token,
                _ => {
                    let token = random_token(24);
                    if !initialized {
                        tracing::info!(setup_token = %token, "first-run setup token");
                    }
                    token
                }
            },
        };

        if initialized {
            tracing::info!("users already present; setup is disabled");
        }

        let thumb_workers = snapshot.settings.thumbnails.workers.max(1);
        let ffmpeg = if snapshot.settings.thumbnails.video {
            crate::thumbs::detect_ffmpeg()
        } else {
            None
        };

        Ok(Arc::new(Self {
            db,
            config,
            snapshot: ArcSwap::from_pointee(snapshot),
            setup: SetupState {
                initialized: AtomicBool::new(initialized),
                token,
            },
            rate_limiter: RateLimiter::new(),
            delete_tokens: Mutex::new(HashMap::new()),
            share_tickets: Mutex::new(HashMap::new()),
            ffmpeg,
            index_jobs: Mutex::new(HashSet::new()),
            thumb_jobs: Mutex::new(HashMap::new()),
            thumb_semaphore: Arc::new(Semaphore::new(thumb_workers)),
        }))
    }

    pub fn settings(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }

    pub async fn reload_settings(&self) -> anyhow::Result<()> {
        let overrides = self.db.settings_overrides().await?;
        let snapshot = build_snapshot(&self.config, &overrides)?;
        ensure_directories(&snapshot)?;
        self.snapshot.store(Arc::new(snapshot));
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.setup.initialized.load(Ordering::Relaxed)
    }
}

async fn ensure_default_roots(db: &Db) -> anyhow::Result<()> {
    for user in db.list_users().await? {
        if !db.list_roots(user.id).await?.is_empty() {
            continue;
        }

        let home = std::path::PathBuf::from(&user.home_dir);
        let canonical =
            crate::fsops::ensure_directory(&home).map_err(|err| anyhow::anyhow!(err))?;
        let real_path = canonical
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("user home is not valid UTF-8"))?;
        db.ensure_root(user.id, real_path, "我的文件", false)
            .await?;
    }
    Ok(())
}
