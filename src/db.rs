use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, Row, params};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_roots_uploads.sql")),
    (3, include_str!("../migrations/0003_trusted_links.sql")),
    (4, include_str!("../migrations/0004_index_search.sql")),
    (5, include_str!("../migrations/0005_shares.sql")),
    (6, include_str!("../migrations/0006_server_paths.sql")),
    (7, include_str!("../migrations/0007_user_preferences.sql")),
    (
        8,
        include_str!("../migrations/0008_terminal_preference.sql"),
    ),
    (9, include_str!("../migrations/0009_admin_defaults.sql")),
    (10, include_str!("../migrations/0010_share_visit_count.sql")),
];

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub home_dir: String,
    pub quota_bytes: Option<i64>,
    pub disabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_login_at: Option<i64>,
}

impl User {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            username: row.get("username")?,
            password_hash: row.get("password_hash")?,
            role: row.get("role")?,
            home_dir: row.get("home_dir")?,
            quota_bytes: row.get("quota_bytes")?,
            disabled: row.get::<_, i64>("disabled")? != 0,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            last_login_at: row.get("last_login_at")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user: User,
    pub csrf_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct TokenUser {
    pub token_id: i64,
    pub user: User,
    pub scope: String,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub scope: String,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Root {
    pub id: i64,
    pub user_id: i64,
    pub real_path: String,
    pub alias: String,
    pub read_only: bool,
    pub sort_order: i64,
}

#[derive(Debug, Clone)]
pub struct UploadSession {
    pub id: String,
    pub user_id: i64,
    pub root_id: i64,
    pub dir_path: String,
    pub filename: String,
    pub total_size: i64,
    pub chunk_size: i64,
    pub received: i64,
    pub chunk_bitmap: Vec<u8>,
    pub temp_path: String,
    pub expires_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub name: String,
    pub real_path: String,
    pub is_dir: bool,
    pub size: i64,
    pub mtime: i64,
}

#[derive(Debug, Clone)]
pub struct UserPreferences {
    pub theme: String,
    pub language: String,
    pub show_hidden: bool,
    pub show_thumbnails: bool,
    pub upload_conflict: String,
    pub terminal_enabled: bool,
    pub editor_enabled: bool,
    pub custom_css_root_id: Option<i64>,
    pub custom_css_path: Option<String>,
    pub custom_css_version: i64,
    pub avatar_path: Option<String>,
    pub avatar_version: i64,
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub username: Option<String>,
    pub action: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub ip: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct ServerPath {
    pub id: i64,
    pub alias: String,
    pub real_path: String,
    pub created_at: i64,
}

pub struct Share {
    pub id: i64,
    pub token_hash: String,
    pub creator_id: i64,
    pub real_path: String,
    pub is_dir: bool,
    pub password_hash: Option<String>,
    pub expires_at: Option<i64>,
    pub read_only: bool,
    pub revoked: bool,
    pub created_at: i64,
    pub visit_count: i64,
}

pub struct TrustedLink {
    pub id: i64,
    pub link_path: String,
    pub target_path: String,
    pub link_dev: i64,
    pub link_ino: i64,
    pub link_mtime: i64,
    pub target_dev: i64,
    pub target_ino: i64,
    pub access: String,
    pub created_at: i64,
}

pub struct Db {
    connections: Vec<Arc<Mutex<Connection>>>,
    next: AtomicUsize,
}

impl Db {
    pub fn open(path: &Path, pool_size: usize) -> anyhow::Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }

        let pool_size = pool_size.max(1);
        let mut connections = Vec::with_capacity(pool_size);

        for _ in 0..pool_size {
            let mut connection = Connection::open(path)
                .with_context(|| format!("failed to open database {}", path.display()))?;
            configure(&connection)?;
            migrate(&mut connection)?;
            connections.push(Arc::new(Mutex::new(connection)));
        }

        Ok(Self {
            connections,
            next: AtomicUsize::new(0),
        })
    }

    async fn call<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        let connection = Arc::clone(&self.connections[index]);

        let result = tokio::task::spawn_blocking(move || {
            let mut connection = connection.lock();
            f(&mut connection)
        })
        .await
        .context("database task failed")?;
        result.map_err(|error| anyhow::anyhow!("database query failed: {error}"))
    }

    pub async fn count_users(&self) -> anyhow::Result<i64> {
        self.call(|connection| {
            connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        })
        .await
    }

    pub async fn count_active_admins(&self) -> anyhow::Result<i64> {
        self.call(|connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0",
                [],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn find_user_by_username(&self, username: &str) -> anyhow::Result<Option<User>> {
        let username = username.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT * FROM users WHERE username = ?1",
                    params![username],
                    User::from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn find_user_by_id(&self, id: i64) -> anyhow::Result<Option<User>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT * FROM users WHERE id = ?1",
                    params![id],
                    User::from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn list_users(&self) -> anyhow::Result<Vec<User>> {
        self.call(|connection| {
            let mut statement = connection.prepare(
                "SELECT * FROM users ORDER BY role = 'admin' DESC, username COLLATE NOCASE",
            )?;
            let rows = statement.query_map([], User::from_row)?;
            rows.collect()
        })
        .await
    }

    pub async fn insert_user(
        &self,
        username: &str,
        password_hash: &str,
        role: &str,
        home_dir: &str,
        quota_bytes: Option<i64>,
    ) -> anyhow::Result<i64> {
        let username = username.to_owned();
        let password_hash = password_hash.to_owned();
        let role = role.to_owned();
        let home_dir = home_dir.to_owned();
        self.call(move |connection| {
            let now = unix_now();
            connection.execute(
                "INSERT INTO users \
                 (username, password_hash, role, home_dir, quota_bytes, disabled, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
                params![username, password_hash, role, home_dir, quota_bytes, now],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn update_user(
        &self,
        id: i64,
        role: Option<&str>,
        quota_bytes: Option<Option<i64>>,
        disabled: Option<bool>,
    ) -> anyhow::Result<bool> {
        let role = role.map(str::to_owned);
        self.call(move |connection| {
            let now = unix_now();
            let current = connection
                .query_row("SELECT role, quota_bytes, disabled FROM users WHERE id = ?1", params![id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                })
                .optional()?;

            let Some((current_role, current_quota, current_disabled)) = current else {
                return Ok(false);
            };

            connection.execute(
                "UPDATE users SET role = ?1, quota_bytes = ?2, disabled = ?3, updated_at = ?4 WHERE id = ?5",
                params![
                    role.unwrap_or(current_role),
                    quota_bytes.unwrap_or(current_quota),
                    disabled.unwrap_or(current_disabled) as i64,
                    now,
                    id
                ],
            )?;
            Ok(true)
        })
        .await
    }

    pub async fn update_user_password(&self, id: i64, password_hash: &str) -> anyhow::Result<bool> {
        let password_hash = password_hash.to_owned();
        self.call(move |connection| {
            let now = unix_now();
            let changed = connection.execute(
                "UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE id = ?3",
                params![password_hash, now, id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn insert_session(
        &self,
        token_hash: &str,
        csrf_token: &str,
        user_id: i64,
        expires_at: i64,
        ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> anyhow::Result<()> {
        let token_hash = token_hash.to_owned();
        let csrf_token = csrf_token.to_owned();
        let ip = ip.map(str::to_owned);
        let user_agent = user_agent.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO sessions \
                 (token_hash, csrf_token, user_id, created_at, expires_at, ip, user_agent) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    token_hash,
                    csrf_token,
                    user_id,
                    unix_now(),
                    expires_at,
                    ip,
                    user_agent
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn find_session(&self, token_hash: &str) -> anyhow::Result<Option<SessionUser>> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT u.*, s.csrf_token AS session_csrf, s.expires_at AS session_expires \
                     FROM sessions s JOIN users u ON u.id = s.user_id \
                     WHERE s.token_hash = ?1",
                    params![token_hash],
                    |row| {
                        Ok(SessionUser {
                            user: User::from_row(row)?,
                            csrf_token: row.get("session_csrf")?,
                            expires_at: row.get("session_expires")?,
                        })
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn delete_session(&self, token_hash: &str) -> anyhow::Result<()> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            connection.execute(
                "DELETE FROM sessions WHERE token_hash = ?1",
                params![token_hash],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn insert_token(
        &self,
        user_id: i64,
        name: &str,
        token_hash: &str,
        prefix: &str,
        scope: &str,
        expires_at: Option<i64>,
    ) -> anyhow::Result<i64> {
        let name = name.to_owned();
        let token_hash = token_hash.to_owned();
        let prefix = prefix.to_owned();
        let scope = scope.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO api_tokens \
                 (user_id, name, token_hash, prefix, scope, last_used_at, expires_at, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7)",
                params![
                    user_id,
                    name,
                    token_hash,
                    prefix,
                    scope,
                    expires_at,
                    unix_now()
                ],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn list_tokens(&self, user_id: i64) -> anyhow::Result<Vec<ApiToken>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, prefix, scope, last_used_at, expires_at, created_at \
                 FROM api_tokens WHERE user_id = ?1 ORDER BY created_at DESC",
            )?;
            let rows = statement.query_map(params![user_id], |row| {
                Ok(ApiToken {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    prefix: row.get(2)?,
                    scope: row.get(3)?,
                    last_used_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn find_token(&self, token_hash: &str) -> anyhow::Result<Option<TokenUser>> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT t.id AS token_id, t.scope AS token_scope, t.expires_at AS token_expires, u.* \
                     FROM api_tokens t JOIN users u ON u.id = t.user_id \
                     WHERE t.token_hash = ?1",
                    params![token_hash],
                    |row| {
                        Ok(TokenUser {
                            token_id: row.get("token_id")?,
                            user: User::from_row(row)?,
                            scope: row.get("token_scope")?,
                            expires_at: row.get("token_expires")?,
                        })
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn touch_token(&self, token_id: i64) -> anyhow::Result<()> {
        self.call(move |connection| {
            connection.execute(
                "UPDATE api_tokens SET last_used_at = ?1 WHERE id = ?2",
                params![unix_now(), token_id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn delete_token(&self, id: i64, user_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let deleted = connection.execute(
                "DELETE FROM api_tokens WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
            )?;
            Ok(deleted > 0)
        })
        .await
    }

    pub async fn settings_overrides(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.call(|connection| {
            let mut statement =
                connection.prepare("SELECT key, value FROM settings ORDER BY key")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
        .await
    }

    pub async fn upsert_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let key = key.to_owned();
        let value = value.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, unix_now()],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn delete_setting(&self, key: &str) -> anyhow::Result<bool> {
        let key = key.to_owned();
        self.call(move |connection| {
            let deleted =
                connection.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
            Ok(deleted > 0)
        })
        .await
    }

    pub async fn touch_last_login(&self, user_id: i64) -> anyhow::Result<()> {
        self.call(move |connection| {
            connection.execute(
                "UPDATE users SET last_login_at = ?1 WHERE id = ?2",
                params![unix_now(), user_id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn audit(
        &self,
        user_id: Option<i64>,
        action: &str,
        path: Option<&str>,
        detail: Option<&str>,
        ip: Option<&str>,
    ) -> anyhow::Result<()> {
        let action = action.to_owned();
        let path = path.map(str::to_owned);
        let detail = detail.map(str::to_owned);
        let ip = ip.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO audit_logs (user_id, action, path, detail, ip, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![user_id, action, path, detail, ip, unix_now()],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn list_audit(&self, limit: i64, offset: i64) -> anyhow::Result<Vec<AuditEntry>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT a.id, u.username, a.action, a.path, a.detail, a.ip, a.created_at \
                 FROM audit_logs a LEFT JOIN users u ON u.id = a.user_id \
                 ORDER BY a.id DESC LIMIT ?1 OFFSET ?2",
            )?;
            let rows = statement.query_map(params![limit, offset], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    action: row.get(2)?,
                    path: row.get(3)?,
                    detail: row.get(4)?,
                    ip: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn get_user_preferences(&self, user_id: i64) -> anyhow::Result<UserPreferences> {
        self.call(move |connection| {
            let preferences = connection
                .query_row(
                    "SELECT theme, language, show_hidden, show_thumbnails, upload_conflict, terminal_enabled, editor_enabled, \
                     custom_css_root_id, custom_css_path, custom_css_version, avatar_path, avatar_version \
                     FROM user_preferences WHERE user_id = ?1",
                    params![user_id],
                    |row| {
                        Ok(UserPreferences {
                            theme: row.get(0)?,
                            language: row.get(1)?,
                            show_hidden: row.get::<_, i64>(2)? != 0,
                            show_thumbnails: row.get::<_, i64>(3)? != 0,
                            upload_conflict: row.get(4)?,
                            terminal_enabled: row.get::<_, i64>(5)? != 0,
                            editor_enabled: row.get::<_, i64>(6)? != 0,
                            custom_css_root_id: row.get(7)?,
                            custom_css_path: row.get(8)?,
                            custom_css_version: row.get(9)?,
                            avatar_path: row.get(10)?,
                            avatar_version: row.get(11)?,
                        })
                    },
                )
                .optional()?;
            Ok(preferences.unwrap_or_else(|| UserPreferences {
                theme: "light".to_owned(),
                language: "en".to_owned(),
                show_hidden: true,
                show_thumbnails: true,
                upload_conflict: "rename".to_owned(),
                terminal_enabled: true,
                editor_enabled: true,
                custom_css_root_id: None,
                custom_css_path: None,
                custom_css_version: 0,
                avatar_path: None,
                avatar_version: 0,
            }))
        })
        .await
    }

    pub async fn save_user_preferences(
        &self,
        user_id: i64,
        preferences: &UserPreferences,
    ) -> anyhow::Result<()> {
        let preferences = preferences.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO user_preferences \
                 (user_id, theme, language, show_hidden, show_thumbnails, upload_conflict, terminal_enabled, editor_enabled, \
                  custom_css_root_id, custom_css_path, custom_css_version, avatar_path, avatar_version, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
                 ON CONFLICT(user_id) DO UPDATE SET \
                   theme = excluded.theme, language = excluded.language, \
                   show_hidden = excluded.show_hidden, show_thumbnails = excluded.show_thumbnails, \
                   upload_conflict = excluded.upload_conflict, terminal_enabled = excluded.terminal_enabled, \
                   editor_enabled = excluded.editor_enabled, custom_css_root_id = excluded.custom_css_root_id, \
                   custom_css_path = excluded.custom_css_path, custom_css_version = excluded.custom_css_version, \
                   avatar_path = excluded.avatar_path, avatar_version = excluded.avatar_version, \
                   updated_at = excluded.updated_at",
                params![
                    user_id,
                    preferences.theme,
                    preferences.language,
                    preferences.show_hidden as i64,
                    preferences.show_thumbnails as i64,
                    preferences.upload_conflict,
                    preferences.terminal_enabled as i64,
                    preferences.editor_enabled as i64,
                    preferences.custom_css_root_id,
                    preferences.custom_css_path,
                    preferences.custom_css_version,
                    preferences.avatar_path,
                    preferences.avatar_version,
                    unix_now(),
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn list_roots(&self, user_id: i64) -> anyhow::Result<Vec<Root>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, user_id, real_path, alias, read_only, sort_order \
                 FROM user_roots WHERE user_id = ?1 ORDER BY sort_order, id",
            )?;
            let rows = statement.query_map(params![user_id], |row| {
                Ok(Root {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    real_path: row.get(2)?,
                    alias: row.get(3)?,
                    read_only: row.get::<_, i64>(4)? != 0,
                    sort_order: row.get(5)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn find_root(&self, id: i64, user_id: i64) -> anyhow::Result<Option<Root>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, real_path, alias, read_only, sort_order \
                     FROM user_roots WHERE id = ?1 AND user_id = ?2",
                    params![id, user_id],
                    |row| {
                        Ok(Root {
                            id: row.get(0)?,
                            user_id: row.get(1)?,
                            real_path: row.get(2)?,
                            alias: row.get(3)?,
                            read_only: row.get::<_, i64>(4)? != 0,
                            sort_order: row.get(5)?,
                        })
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn ensure_root(
        &self,
        user_id: i64,
        real_path: &str,
        alias: &str,
        read_only: bool,
    ) -> anyhow::Result<i64> {
        let real_path = real_path.to_owned();
        let alias = alias.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO user_roots \
                 (user_id, real_path, alias, read_only, sort_order, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 0, ?5) \
                 ON CONFLICT(user_id, real_path) DO NOTHING",
                params![user_id, real_path, alias, read_only as i64, unix_now()],
            )?;
            connection.query_row(
                "SELECT id FROM user_roots WHERE user_id = ?1 AND real_path = ?2",
                params![user_id, real_path],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn insert_upload(&self, upload: &UploadSession) -> anyhow::Result<()> {
        let upload = upload.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO upload_sessions \
                 (id, user_id, root_id, dir_path, filename, total_size, chunk_size, received, \
                  chunk_bitmap, temp_path, sha256, expires_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?12, ?12)",
                params![
                    upload.id,
                    upload.user_id,
                    upload.root_id,
                    upload.dir_path,
                    upload.filename,
                    upload.total_size,
                    upload.chunk_size,
                    upload.received,
                    upload.chunk_bitmap,
                    upload.temp_path,
                    upload.expires_at,
                    upload.created_at
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn find_upload(
        &self,
        id: &str,
        user_id: i64,
    ) -> anyhow::Result<Option<UploadSession>> {
        let id = id.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, root_id, dir_path, filename, total_size, chunk_size, \
                     received, chunk_bitmap, temp_path, expires_at, created_at, updated_at \
                     FROM upload_sessions WHERE id = ?1 AND user_id = ?2",
                    params![id, user_id],
                    |row| {
                        Ok(UploadSession {
                            id: row.get(0)?,
                            user_id: row.get(1)?,
                            root_id: row.get(2)?,
                            dir_path: row.get(3)?,
                            filename: row.get(4)?,
                            total_size: row.get(5)?,
                            chunk_size: row.get(6)?,
                            received: row.get(7)?,
                            chunk_bitmap: row.get(8)?,
                            temp_path: row.get(9)?,
                            expires_at: row.get(10)?,
                            created_at: row.get(11)?,
                            updated_at: row.get(12)?,
                        })
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn update_upload_progress(
        &self,
        id: &str,
        user_id: i64,
        received: i64,
        chunk_bitmap: Vec<u8>,
    ) -> anyhow::Result<bool> {
        let id = id.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE upload_sessions \
                 SET received = ?1, chunk_bitmap = ?2, updated_at = ?3 \
                 WHERE id = ?4 AND user_id = ?5",
                params![received, chunk_bitmap, unix_now(), id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn delete_upload(&self, id: &str, user_id: i64) -> anyhow::Result<()> {
        let id = id.to_owned();
        self.call(move |connection| {
            connection.execute(
                "DELETE FROM upload_sessions WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn add_root(
        &self,
        user_id: i64,
        real_path: &str,
        alias: &str,
        read_only: bool,
        sort_order: i64,
    ) -> anyhow::Result<i64> {
        let real_path = real_path.to_owned();
        let alias = alias.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO user_roots \
                 (user_id, real_path, alias, read_only, sort_order, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    user_id,
                    real_path,
                    alias,
                    read_only as i64,
                    sort_order,
                    unix_now()
                ],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn update_root(
        &self,
        id: i64,
        user_id: i64,
        alias: &str,
        read_only: bool,
        sort_order: i64,
    ) -> anyhow::Result<bool> {
        let alias = alias.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE user_roots SET alias = ?1, read_only = ?2, sort_order = ?3 \
                 WHERE id = ?4 AND user_id = ?5",
                params![alias, read_only as i64, sort_order, id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn delete_root(&self, id: i64, user_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "DELETE FROM user_roots WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn list_trusted_links(&self) -> anyhow::Result<Vec<TrustedLink>> {
        self.call(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, link_path, target_path, link_dev, link_ino, link_mtime, \
                 target_dev, target_ino, access, created_at FROM trusted_links ORDER BY id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(TrustedLink {
                    id: row.get(0)?,
                    link_path: row.get(1)?,
                    target_path: row.get(2)?,
                    link_dev: row.get(3)?,
                    link_ino: row.get(4)?,
                    link_mtime: row.get(5)?,
                    target_dev: row.get(6)?,
                    target_ino: row.get(7)?,
                    access: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn insert_trusted_link(&self, link: &TrustedLink) -> anyhow::Result<i64> {
        let link_path = link.link_path.clone();
        let target_path = link.target_path.clone();
        let link_dev = link.link_dev;
        let link_ino = link.link_ino;
        let link_mtime = link.link_mtime;
        let target_dev = link.target_dev;
        let target_ino = link.target_ino;
        let access = link.access.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO trusted_links \
                 (link_path, target_path, link_dev, link_ino, link_mtime, target_dev, target_ino, access, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    link_path,
                    target_path,
                    link_dev,
                    link_ino,
                    link_mtime,
                    target_dev,
                    target_ino,
                    access,
                    unix_now()
                ],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn delete_trusted_link(&self, id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed =
                connection.execute("DELETE FROM trusted_links WHERE id = ?1", params![id])?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn upsert_index_dir(&self, real_path: &str, mtime_ns: i64) -> anyhow::Result<i64> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.query_row(
                "INSERT INTO indexed_dirs (real_path, mtime_ns, status, created_at) \
                 VALUES (?1, ?2, 'indexing', ?3) \
                 ON CONFLICT(real_path) DO UPDATE SET \
                   mtime_ns = excluded.mtime_ns, status = 'indexing', error = NULL \
                 RETURNING id",
                params![real_path, mtime_ns, unix_now()],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn replace_index_entries(
        &self,
        dir_id: i64,
        entries: Vec<IndexEntry>,
    ) -> anyhow::Result<usize> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM index_entries WHERE dir_id = ?1",
                params![dir_id],
            )?;
            for entry in &entries {
                transaction.execute(
                    "INSERT INTO index_entries \
                     (dir_id, name, real_path, is_dir, size, mtime) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        dir_id,
                        entry.name,
                        entry.real_path,
                        entry.is_dir as i64,
                        entry.size,
                        entry.mtime
                    ],
                )?;
            }
            transaction.commit()?;
            Ok(entries.len())
        })
        .await
    }

    pub async fn finish_index_dir(
        &self,
        dir_id: i64,
        status: &str,
        entry_count: i64,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let status = status.to_owned();
        let error = error.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "UPDATE indexed_dirs \
                 SET status = ?1, entry_count = ?2, error = ?3, indexed_at = ?4 \
                 WHERE id = ?5",
                params![status, entry_count, error, unix_now(), dir_id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn mark_index_stale(&self, real_path: &str) -> anyhow::Result<()> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE indexed_dirs SET status = 'stale' WHERE real_path = ?1",
                params![real_path],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn index_status(
        &self,
        real_path: &str,
    ) -> anyhow::Result<Option<(String, i64, Option<i64>)>> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT status, entry_count, indexed_at FROM indexed_dirs WHERE real_path = ?1",
                    params![real_path],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
        })
        .await
    }

    pub async fn indexed_children(
        &self,
        dir_path: &str,
        sort: &str,
        order: &str,
    ) -> anyhow::Result<Option<(i64, Vec<IndexEntry>)>> {
        let dir_path = dir_path.to_owned();
        let order_column = match sort {
            "size" => "e.size",
            "modified" => "e.mtime",
            _ => "e.name COLLATE NOCASE",
        };
        let direction = if order.eq_ignore_ascii_case("desc") {
            "DESC"
        } else {
            "ASC"
        };
        let sql = format!(
            "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime \
             FROM index_entries e JOIN indexed_dirs d ON d.id = e.dir_id \
             WHERE d.real_path = ?1 AND d.status = 'ready' \
             ORDER BY e.is_dir DESC, {order_column} {direction}, e.name COLLATE NOCASE"
        );

        self.call(move |connection| {
            let dir = connection
                .query_row(
                    "SELECT id, mtime_ns, status FROM indexed_dirs WHERE real_path = ?1",
                    params![dir_path],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;

            let Some((dir_id, mtime_ns, status)) = dir else {
                return Ok(None);
            };
            if status != "ready" {
                return Ok(None);
            }

            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(params![dir_id], |row| {
                Ok(IndexEntry {
                    name: row.get(0)?,
                    real_path: row.get(1)?,
                    is_dir: row.get::<_, i64>(2)? != 0,
                    size: row.get(3)?,
                    mtime: row.get(4)?,
                })
            })?;
            let entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some((mtime_ns, entries)))
        })
        .await
    }

    pub async fn indexed_children_page(
        &self,
        dir_path: &str,
        sort: &str,
        order: &str,
        show_hidden: bool,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<Option<(i64, Vec<IndexEntry>, bool)>> {
        let dir_path = dir_path.to_owned();
        let order_column = match sort {
            "size" => "e.size",
            "modified" => "e.mtime",
            _ => "e.name COLLATE NOCASE",
        };
        let direction = if order.eq_ignore_ascii_case("desc") {
            "DESC"
        } else {
            "ASC"
        };
        let sql = format!(
            "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime \
             FROM index_entries e JOIN indexed_dirs d ON d.id = e.dir_id \
             WHERE d.real_path = ?1 AND d.status = 'ready' \
               AND (?4 = 1 OR e.name NOT LIKE '.%') \
             ORDER BY e.is_dir DESC, {order_column} {direction}, e.name COLLATE NOCASE \
             LIMIT ?2 OFFSET ?3"
        );
        self.call(move |connection| {
            let dir = connection
                .query_row(
                    "SELECT id, mtime_ns FROM indexed_dirs WHERE real_path = ?1 AND status = 'ready'",
                    params![dir_path],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?;
            let Some((_dir_id, mtime_ns)) = dir else {
                return Ok(None);
            };
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(
                params![dir_path, (limit + 1) as i64, offset as i64, show_hidden as i64],
                |row| {
                    Ok(IndexEntry {
                        name: row.get(0)?,
                        real_path: row.get(1)?,
                        is_dir: row.get::<_, i64>(2)? != 0,
                        size: row.get(3)?,
                        mtime: row.get(4)?,
                    })
                },
            )?;
            let mut entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            let has_more = entries.len() > limit;
            if has_more {
                entries.truncate(limit);
            }
            Ok(Some((mtime_ns, entries, has_more)))
        })
        .await
    }

    pub async fn search_entries(
        &self,
        query: &str,
        root_path: &str,
        show_hidden: bool,
        limit: i64,
    ) -> anyhow::Result<Vec<IndexEntry>> {
        let query = query.to_owned();
        let root_path = root_path.to_owned();
        self.call(move |connection| {
            let limit = limit.clamp(1, 500);
            let separator = if root_path.contains('\\') { '\\' } else { '/' };
            let escaped_root = root_path
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_");
            let root_pattern = format!("{escaped_root}{separator}%");
            let mut entries = Vec::new();

            if query.chars().count() >= 3 {
                let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
                let mut statement = connection.prepare(
                    "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime \
                     FROM index_fts JOIN index_entries e ON e.id = index_fts.rowid \
                     JOIN indexed_dirs d ON d.id = e.dir_id \
                     WHERE index_fts MATCH ?1
                       AND (e.real_path = ?3 OR e.real_path LIKE ?4 ESCAPE '!')
                       AND (?5 = 1 OR e.name NOT LIKE '.%')
                     LIMIT ?2",
                )?;
                let rows = statement.query_map(
                    params![
                        fts_query,
                        limit,
                        root_path,
                        root_pattern,
                        show_hidden as i64
                    ],
                    |row| {
                        Ok(IndexEntry {
                            name: row.get(0)?,
                            real_path: row.get(1)?,
                            is_dir: row.get::<_, i64>(2)? != 0,
                            size: row.get(3)?,
                            mtime: row.get(4)?,
                        })
                    },
                )?;
                for entry in rows {
                    entries.push(entry?);
                }
            } else {
                let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
                let mut statement = connection.prepare(
                    "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime \
                     FROM index_entries e JOIN indexed_dirs d ON d.id = e.dir_id \
                     WHERE e.name LIKE ?1 ESCAPE '\\'
                       AND (e.real_path = ?3 OR e.real_path LIKE ?4 ESCAPE '!')
                       AND (?5 = 1 OR e.name NOT LIKE '.%')
                     LIMIT ?2",
                )?;
                let rows = statement.query_map(
                    params![pattern, limit, root_path, root_pattern, show_hidden as i64],
                    |row| {
                        Ok(IndexEntry {
                            name: row.get(0)?,
                            real_path: row.get(1)?,
                            is_dir: row.get::<_, i64>(2)? != 0,
                            size: row.get(3)?,
                            mtime: row.get(4)?,
                        })
                    },
                )?;
                for entry in rows {
                    entries.push(entry?);
                }
            }

            Ok(entries)
        })
        .await
    }
    pub async fn list_server_paths(&self) -> anyhow::Result<Vec<ServerPath>> {
        self.call(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, alias, real_path, created_at FROM server_paths ORDER BY alias COLLATE NOCASE",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(ServerPath {
                    id: row.get(0)?,
                    alias: row.get(1)?,
                    real_path: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn insert_server_path(&self, alias: &str, real_path: &str) -> anyhow::Result<i64> {
        let alias = alias.to_owned();
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO server_paths (alias, real_path, created_at) VALUES (?1, ?2, ?3)",
                params![alias, real_path, unix_now()],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn update_server_path_alias(&self, id: i64, alias: &str) -> anyhow::Result<bool> {
        let alias = alias.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE server_paths SET alias = ?1 WHERE id = ?2",
                params![alias, id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn delete_server_path(&self, id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed =
                connection.execute("DELETE FROM server_paths WHERE id = ?1", params![id])?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn count_roots_for_path(&self, real_path: &str) -> anyhow::Result<i64> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM user_roots WHERE real_path = ?1",
                params![real_path],
                |row| row.get(0),
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_share(
        &self,
        token_hash: &str,
        creator_id: i64,
        real_path: &str,
        is_dir: bool,
        password_hash: Option<&str>,
        expires_at: Option<i64>,
        read_only: bool,
    ) -> anyhow::Result<i64> {
        let token_hash = token_hash.to_owned();
        let real_path = real_path.to_owned();
        let password_hash = password_hash.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO shares \
                 (token_hash, creator_id, real_path, is_dir, password_hash, expires_at, read_only, revoked, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
                params![
                    token_hash,
                    creator_id,
                    real_path,
                    is_dir as i64,
                    password_hash,
                    expires_at,
                    read_only as i64,
                    unix_now()
                ],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn list_shares(
        &self,
        creator_id: i64,
        offset: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<Share>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, token_hash, creator_id, real_path, is_dir, password_hash, \
                 expires_at, read_only, revoked, created_at, visit_count FROM shares \
                 WHERE creator_id = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
            )?;
            let rows = statement.query_map(params![creator_id, limit, offset], share_from_row)?;
            rows.collect()
        })
        .await
    }

    pub async fn update_share_read_only(
        &self,
        id: i64,
        creator_id: i64,
        read_only: bool,
    ) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE shares SET read_only = ?1 WHERE id = ?2 AND creator_id = ?3",
                params![read_only as i64, id, creator_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn update_share_expiry(
        &self,
        id: i64,
        creator_id: i64,
        expires_at: Option<i64>,
    ) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE shares SET expires_at = ?1 WHERE id = ?2 AND creator_id = ?3",
                params![expires_at, id, creator_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn update_share_password(
        &self,
        id: i64,
        creator_id: i64,
        password_hash: Option<&str>,
    ) -> anyhow::Result<bool> {
        let password_hash = password_hash.map(str::to_owned);
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE shares SET password_hash = ?1 WHERE id = ?2 AND creator_id = ?3",
                params![password_hash, id, creator_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn update_share_revoked(
        &self,
        id: i64,
        creator_id: i64,
        revoked: bool,
    ) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE shares SET revoked = ?1 WHERE id = ?2 AND creator_id = ?3",
                params![revoked as i64, id, creator_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn rotate_share_token(
        &self,
        id: i64,
        creator_id: i64,
        token_hash: &str,
    ) -> anyhow::Result<bool> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE shares SET token_hash = ?1 WHERE id = ?2 AND creator_id = ?3 AND revoked = 0",
                params![token_hash, id, creator_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn purge_share(&self, id: i64, creator_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "DELETE FROM shares WHERE id = ?1 AND creator_id = ?2",
                params![id, creator_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn increment_share_visit(&self, id: i64) -> anyhow::Result<()> {
        self.call(move |connection| {
            connection.execute(
                "UPDATE shares SET visit_count = visit_count + 1 WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn find_share_by_hash(&self, token_hash: &str) -> anyhow::Result<Option<Share>> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, token_hash, creator_id, real_path, is_dir, password_hash, \
                     expires_at, read_only, revoked, created_at, visit_count FROM shares WHERE token_hash = ?1",
                    params![token_hash],
                    share_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn revoke_share(&self, id: i64, creator_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE shares SET revoked = 1 WHERE id = ?1 AND creator_id = ?2",
                params![id, creator_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }
}

fn share_from_row(row: &Row<'_>) -> rusqlite::Result<Share> {
    Ok(Share {
        id: row.get(0)?,
        token_hash: row.get(1)?,
        creator_id: row.get(2)?,
        real_path: row.get(3)?,
        is_dir: row.get::<_, i64>(4)? != 0,
        password_hash: row.get(5)?,
        expires_at: row.get(6)?,
        read_only: row.get::<_, i64>(7)? != 0,
        revoked: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        visit_count: row.get(10)?,
    })
}

fn configure(connection: &Connection) -> anyhow::Result<()> {
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;\
             PRAGMA foreign_keys = ON;\
             PRAGMA busy_timeout = 5000;\
             PRAGMA synchronous = NORMAL;",
        )
        .context("failed to configure SQLite")?;
    Ok(())
}

fn migrate(connection: &mut Connection) -> anyhow::Result<()> {
    let current: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("failed to read schema version")?;

    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }

        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.execute_batch(&format!("PRAGMA user_version = {version};"))?;
        transaction.commit()?;
    }

    Ok(())
}
