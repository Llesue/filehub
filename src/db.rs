use std::collections::{HashMap, HashSet};
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
    (11, include_str!("../migrations/0011_directory_sizes.sql")),
    (12, include_str!("../migrations/0012_jobs.sql")),
    (
        13,
        include_str!("../migrations/0013_index_totals_audit.sql"),
    ),
    (
        14,
        include_str!("../migrations/0014_index_metadata_runtime.sql"),
    ),
    (15, include_str!("../migrations/0015_usn_state.sql")),
    (
        16,
        include_str!("../migrations/0016_pending_index_dirs.sql"),
    ),
    (17, include_str!("../migrations/0017_share_uploads.sql")),
    (18, include_str!("../migrations/0018_branding.sql")),
    (19, include_str!("../migrations/0019_collection_shares.sql")),
    (
        20,
        include_str!("../migrations/0020_directory_size_has_value.sql"),
    ),
    (
        21,
        include_str!("../migrations/0021_drop_duplicate_index_indexes.sql"),
    ),
    (22, include_str!("../migrations/0022_user_soft_delete.sql")),
    (23, include_str!("../migrations/0023_global_custom_css.sql")),
    (
        24,
        include_str!("../migrations/0024_search_and_cache_indexes.sql"),
    ),
    (
        25,
        include_str!("../migrations/0025_drop_short_search_index.sql"),
    ),
    (26, include_str!("../migrations/0026_name_bigrams.sql")),
    (27, include_str!("../migrations/0027_user_features.sql")),
    (28, include_str!("../migrations/0028_job_kinds_archive.sql")),
    (
        29,
        include_str!("../migrations/0029_share_stable_token.sql"),
    ),
    (30, include_str!("../migrations/0030_session_last_seen.sql")),
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
    pub feature_terminal: bool,
    pub feature_zip: bool,
    pub feature_unzip: bool,
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
            feature_terminal: row.get::<_, i64>("feature_terminal")? != 0,
            feature_zip: row.get::<_, i64>("feature_zip")? != 0,
            feature_unzip: row.get::<_, i64>("feature_unzip")? != 0,
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
    pub last_seen_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TokenUser {
    pub token_id: i64,
    pub user: User,
    pub scope: String,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
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
pub struct ShareUploadSession {
    pub id: String,
    pub share_id: i64,
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
pub struct BrandingIcon {
    pub icon: Vec<u8>,
    pub mime_type: String,
    pub version: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct IndexSearchFilters {
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub modified_after: Option<i64>,
    pub modified_before: Option<i64>,
    pub is_dir: Option<bool>,
    pub mime_prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DirIndexStat {
    pub size: Option<u64>,
    pub entry_count: Option<u64>,
    pub source: String,
    pub stale: bool,
}

#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub name: String,
    pub real_path: String,
    pub is_dir: bool,
    pub size: i64,
    pub mtime: i64,
    pub created_at: i64,
    pub mime_type: String,
    pub has_preview: bool,
    pub is_hidden: bool,
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
pub struct UsnState {
    pub volume: String,
    pub journal_id: u64,
    pub next_usn: i64,
}

#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub user_id: Option<i64>,
    pub action: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub ip: Option<String>,
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
    pub kind: String,
    pub item_count: i64,
    pub token_hash: String,
    pub token_nonce: i64,
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

pub struct ShareItem {
    pub id: i64,
    pub share_id: i64,
    pub real_path: String,
    pub display_name: String,
    pub is_dir: bool,
    pub sort_order: i64,
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

impl Clone for Db {
    fn clone(&self) -> Self {
        Self {
            connections: self.connections.clone(),
            next: AtomicUsize::new(0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobRecord {
    pub id: String,
    pub user_id: i64,
    pub kind: String,
    pub job_key: String,
    pub status: String,
    pub progress: i64,
    pub total: i64,
    pub bytes_done: i64,
    pub bytes_total: i64,
    pub payload: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

fn is_cjk(character: char) -> bool {
    let code = character as u32;
    (0x3400..=0x4DBF).contains(&code)
        || (0x4E00..=0x9FFF).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
}

fn name_bigrams(name: &str) -> Vec<String> {
    let mut grams = Vec::new();
    let mut seen = HashSet::new();
    let mut previous: Option<char> = None;
    for character in name.chars() {
        if let Some(prev) = previous
            && (is_cjk(prev) || is_cjk(character))
        {
            let gram = format!("{prev}{character}");
            if seen.insert(gram.clone()) {
                grams.push(gram);
            }
        }
        previous = Some(character);
    }
    grams
}

#[allow(clippy::too_many_arguments)]
fn locate_child_offset(
    connection: &Connection,
    dir_id: i64,
    show_hidden: bool,
    is_dir: i64,
    key_column: &str,
    comparison: &str,
    key_value: rusqlite::types::Value,
    target_name: &str,
    total: i64,
    mtime_ns: i64,
) -> rusqlite::Result<Option<(i64, usize, usize)>> {
    let sql = format!(
        "SELECT COUNT(*) FROM index_entries \
         WHERE dir_id = ?1 AND (?2 = 1 OR name NOT LIKE '.%') AND (\
           is_dir > ?3 \
           OR (is_dir = ?3 AND {key_column} {comparison} ?4) \
           OR (is_dir = ?3 AND {key_column} = ?4 AND name COLLATE NOCASE {comparison} ?5))"
    );
    let offset: i64 = connection.query_row(
        &sql,
        params![dir_id, show_hidden as i64, is_dir, key_value, target_name],
        |row| row.get(0),
    )?;
    Ok(Some((mtime_ns, offset as usize, total as usize)))
}

#[derive(Debug, Clone)]
pub struct DbStats {
    pub page_count: i64,
    pub page_size: i64,
    pub freelist_count: i64,
    pub wal_bytes: u64,
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

        #[cfg(unix)]
        {
            // The database stores password hashes, session hashes and audit data.
            if path.exists() {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            // WAL/SHM are created next to the database and inherit restrictive
            // modes from it, but tighten them explicitly in case they already
            // exist from an older release.
            for suffix in ["-wal", "-shm"] {
                let mut sidecar = path.as_os_str().to_owned();
                sidecar.push(suffix);
                let sidecar = std::path::PathBuf::from(sidecar);
                if sidecar.exists() {
                    let _ =
                        std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o600));
                }
            }
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
        let start = self.next.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        let connections = self.connections.clone();

        let result = tokio::task::spawn_blocking(move || {
            // Prefer an idle connection. Long index transactions can hold one
            // pooled connection for a while; blindly queueing behind it makes
            // reads and deletes feel blocked even when other connections are free.
            let mut chosen = start;
            for offset in 0..connections.len() {
                let candidate = (start + offset) % connections.len();
                if connections[candidate].try_lock().is_some() {
                    chosen = candidate;
                    break;
                }
            }
            let mut connection = connections[chosen].lock();
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
                "SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0 AND deleted_at IS NULL",
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
                    "SELECT * FROM users WHERE username = ?1 AND deleted_at IS NULL",
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
                    "SELECT * FROM users WHERE id = ?1 AND deleted_at IS NULL",
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
                "SELECT * FROM users WHERE deleted_at IS NULL ORDER BY role = 'admin' DESC, username COLLATE NOCASE",
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
                .query_row("SELECT role, quota_bytes, disabled FROM users WHERE id = ?1 AND deleted_at IS NULL", params![id], |row| {
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
                "UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
                params![password_hash, now, id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn update_user_features(
        &self,
        id: i64,
        terminal: Option<bool>,
        zip: Option<bool>,
        unzip: Option<bool>,
    ) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let now = unix_now();
            let changed = connection.execute(
                "UPDATE users SET \
                 feature_terminal = COALESCE(?1, feature_terminal), \
                 feature_zip = COALESCE(?2, feature_zip), \
                 feature_unzip = COALESCE(?3, feature_unzip), \
                 updated_at = ?4 \
                 WHERE id = ?5 AND deleted_at IS NULL",
                params![
                    terminal.map(i64::from),
                    zip.map(i64::from),
                    unzip.map(i64::from),
                    now,
                    id
                ],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn soft_delete_user(&self, id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let now = unix_now();
            let transaction = connection.transaction()?;
            let changed = transaction.execute(
                "UPDATE users SET deleted_at = ?1, disabled = 1, updated_at = ?1 \
                 WHERE id = ?2 AND deleted_at IS NULL",
                params![now, id],
            )?;
            if changed == 0 {
                transaction.commit()?;
                return Ok(false);
            }
            transaction.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
            transaction.execute("DELETE FROM api_tokens WHERE user_id = ?1", params![id])?;
            transaction.execute(
                "DELETE FROM upload_sessions WHERE user_id = ?1",
                params![id],
            )?;
            transaction.execute(
                "UPDATE shares SET revoked = 1 WHERE creator_id = ?1",
                params![id],
            )?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    pub async fn custom_css(&self) -> anyhow::Result<Option<(String, i64)>> {
        self.call(|connection| {
            connection
                .query_row(
                    "SELECT content, version FROM custom_css WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
        })
        .await
    }

    pub async fn set_custom_css(&self, content: &str) -> anyhow::Result<i64> {
        let content = content.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO custom_css (id, content, version, updated_at) \
                 VALUES (1, ?1, 1, ?2) \
                 ON CONFLICT(id) DO UPDATE SET content = excluded.content, \
                   version = custom_css.version + 1, updated_at = excluded.updated_at",
                params![content, unix_now()],
            )?;
            connection.query_row("SELECT version FROM custom_css WHERE id = 1", [], |row| {
                row.get(0)
            })
        })
        .await
    }

    pub async fn clear_custom_css(&self) -> anyhow::Result<i64> {
        self.set_custom_css("").await
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
                    "SELECT u.*, s.csrf_token AS session_csrf, s.expires_at AS session_expires, \
                     s.last_seen_at AS session_last_seen \
                     FROM sessions s JOIN users u ON u.id = s.user_id \
                     WHERE s.token_hash = ?1 AND u.deleted_at IS NULL",
                    params![token_hash],
                    |row| {
                        Ok(SessionUser {
                            user: User::from_row(row)?,
                            csrf_token: row.get("session_csrf")?,
                            expires_at: row.get("session_expires")?,
                            last_seen_at: row.get("session_last_seen")?,
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
                    "SELECT t.id AS token_id, t.scope AS token_scope, \
                            t.expires_at AS token_expires, t.last_used_at AS token_last_used, u.* \
                     FROM api_tokens t JOIN users u ON u.id = t.user_id \
                     WHERE t.token_hash = ?1 AND u.deleted_at IS NULL",
                    params![token_hash],
                    |row| {
                        Ok(TokenUser {
                            token_id: row.get("token_id")?,
                            user: User::from_row(row)?,
                            scope: row.get("token_scope")?,
                            expires_at: row.get("token_expires")?,
                            last_used_at: row.get("token_last_used")?,
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

    pub async fn upsert_settings_batch(&self, values: Vec<(String, String)>) -> anyhow::Result<()> {
        if values.is_empty() {
            return Ok(());
        }
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            for (key, value) in &values {
                transaction.execute(
                    "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                    params![key, value, unix_now()],
                )?;
            }
            transaction.commit()?;
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
        self.audit_batch(vec![AuditRecord {
            user_id,
            action: action.to_owned(),
            path: path.map(str::to_owned),
            detail: detail.map(str::to_owned),
            ip: ip.map(str::to_owned),
        }])
        .await
    }

    pub async fn audit_batch(&self, records: Vec<AuditRecord>) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let now = unix_now();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            {
                let mut statement = transaction.prepare(
                    "INSERT INTO audit_logs (user_id, action, path, detail, ip, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?;
                for record in &records {
                    statement.execute(params![
                        record.user_id,
                        record.action,
                        record.path,
                        record.detail,
                        record.ip,
                        now
                    ])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn list_audit(
        &self,
        limit: i64,
        before_id: Option<i64>,
    ) -> anyhow::Result<Vec<AuditEntry>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT a.id, u.username, a.action, a.path, a.detail, a.ip, a.created_at \
                 FROM audit_logs a LEFT JOIN users u ON u.id = a.user_id \
                 WHERE (?1 IS NULL OR a.id < ?1) \
                 ORDER BY a.id DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![before_id, limit], |row| {
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

    pub async fn prune_audit_batch(
        &self,
        retention_days: u64,
        max_rows: u64,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let limit = limit.clamp(1, 5000) as i64;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let mut removed = 0_usize;
            if retention_days > 0 {
                let cutoff = unix_now() - (retention_days.min(36_500) as i64) * 86_400;
                removed += transaction.execute(
                    "DELETE FROM audit_logs WHERE id IN (\
                       SELECT id FROM audit_logs WHERE created_at < ?1 LIMIT ?2)",
                    params![cutoff, limit],
                )?;
            }
            if max_rows > 0 && removed < limit as usize {
                let remaining = limit - removed as i64;
                removed += transaction.execute(
                    "DELETE FROM audit_logs WHERE id IN (\
                       SELECT id FROM audit_logs WHERE id <= (\
                         SELECT id FROM audit_logs ORDER BY id DESC LIMIT 1 OFFSET ?1\
                       ) LIMIT ?2)",
                    params![max_rows.min(i64::MAX as u64) as i64, remaining],
                )?;
            }
            transaction.commit()?;
            Ok(removed)
        })
        .await
    }

    pub async fn prune_directory_sizes_batch(
        &self,
        before: i64,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let limit = limit.clamp(1, 5000) as i64;
        self.call(move |connection| {
            connection.execute(
                "DELETE FROM directory_sizes WHERE rowid IN (\
                   SELECT rowid FROM directory_sizes WHERE updated_at < ?1 LIMIT ?2)",
                params![before, limit],
            )
        })
        .await
    }

    pub async fn prune_audit(&self, retention_days: u64, max_rows: u64) -> anyhow::Result<usize> {
        self.call(move |connection| {
            let mut removed = 0;
            if retention_days > 0 {
                let cutoff = unix_now() - (retention_days.min(36_500) as i64) * 86_400;
                removed += connection.execute(
                    "DELETE FROM audit_logs WHERE created_at < ?1",
                    params![cutoff],
                )?;
            }
            if max_rows > 0 {
                removed += connection.execute(
                    "DELETE FROM audit_logs WHERE id NOT IN \
                       (SELECT id FROM audit_logs ORDER BY id DESC LIMIT ?1)",
                    params![max_rows.min(i64::MAX as u64) as i64],
                )?;
            }
            Ok(removed)
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

    pub async fn list_all_roots(&self) -> anyhow::Result<Vec<Root>> {
        self.call(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, user_id, real_path, alias, read_only, sort_order \
                 FROM user_roots ORDER BY user_id, sort_order, id",
            )?;
            let rows = statement.query_map([], |row| {
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

    pub async fn insert_share_upload(&self, upload: &ShareUploadSession) -> anyhow::Result<()> {
        let upload = upload.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO share_upload_sessions \
                 (id, share_id, dir_path, filename, total_size, chunk_size, received, \
                  chunk_bitmap, temp_path, expires_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    upload.id,
                    upload.share_id,
                    upload.dir_path,
                    upload.filename,
                    upload.total_size,
                    upload.chunk_size,
                    upload.received,
                    upload.chunk_bitmap,
                    upload.temp_path,
                    upload.expires_at,
                    upload.created_at,
                    upload.updated_at
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn find_share_upload(
        &self,
        id: &str,
        share_id: i64,
    ) -> anyhow::Result<Option<ShareUploadSession>> {
        let id = id.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, share_id, dir_path, filename, total_size, chunk_size, received, \
                     chunk_bitmap, temp_path, expires_at, created_at, updated_at \
                     FROM share_upload_sessions WHERE id = ?1 AND share_id = ?2",
                    params![id, share_id],
                    |row| {
                        Ok(ShareUploadSession {
                            id: row.get(0)?,
                            share_id: row.get(1)?,
                            dir_path: row.get(2)?,
                            filename: row.get(3)?,
                            total_size: row.get(4)?,
                            chunk_size: row.get(5)?,
                            received: row.get(6)?,
                            chunk_bitmap: row.get(7)?,
                            temp_path: row.get(8)?,
                            expires_at: row.get(9)?,
                            created_at: row.get(10)?,
                            updated_at: row.get(11)?,
                        })
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn update_share_upload_progress(
        &self,
        id: &str,
        share_id: i64,
        received: i64,
        chunk_bitmap: Vec<u8>,
    ) -> anyhow::Result<bool> {
        let id = id.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE share_upload_sessions \
                 SET received = ?1, chunk_bitmap = ?2, updated_at = ?3 \
                 WHERE id = ?4 AND share_id = ?5",
                params![received, chunk_bitmap, unix_now(), id, share_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn delete_share_upload(&self, id: &str, share_id: i64) -> anyhow::Result<()> {
        let id = id.to_owned();
        self.call(move |connection| {
            connection.execute(
                "DELETE FROM share_upload_sessions WHERE id = ?1 AND share_id = ?2",
                params![id, share_id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn take_expired_share_upload_temps(&self, now: i64) -> anyhow::Result<Vec<String>> {
        self.call(move |connection| {
            let mut statement = connection
                .prepare("SELECT temp_path FROM share_upload_sessions WHERE expires_at < ?1")?;
            let paths = statement
                .query_map(params![now], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            connection.execute(
                "DELETE FROM share_upload_sessions WHERE expires_at < ?1",
                params![now],
            )?;
            Ok(paths)
        })
        .await
    }

    pub async fn branding_icon(&self) -> anyhow::Result<Option<BrandingIcon>> {
        self.call(|connection| {
            connection
                .query_row(
                    "SELECT icon, mime_type, version, updated_at FROM branding WHERE id = 1",
                    [],
                    |row| {
                        Ok(BrandingIcon {
                            icon: row.get(0)?,
                            mime_type: row.get(1)?,
                            version: row.get(2)?,
                            updated_at: row.get(3)?,
                        })
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn set_branding_icon(&self, icon: Vec<u8>, mime_type: &str) -> anyhow::Result<i64> {
        let mime_type = mime_type.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO branding (id, icon, mime_type, version, updated_at) \
                 VALUES (1, ?1, ?2, 1, ?3) \
                 ON CONFLICT(id) DO UPDATE SET \
                   icon = excluded.icon, mime_type = excluded.mime_type, \
                   version = branding.version + 1, updated_at = excluded.updated_at",
                params![icon, mime_type, unix_now()],
            )?;
            let version =
                connection.query_row("SELECT version FROM branding WHERE id = 1", [], |row| {
                    row.get(0)
                })?;
            Ok(version)
        })
        .await
    }

    pub async fn delete_branding_icon(&self) -> anyhow::Result<bool> {
        self.call(|connection| {
            let removed = connection.execute("DELETE FROM branding WHERE id = 1", [])?;
            Ok(removed > 0)
        })
        .await
    }

    pub async fn count_active_uploads(&self, user_id: i64, now: i64) -> anyhow::Result<usize> {
        self.call(move |connection| {
            let count = connection.query_row(
                "SELECT COUNT(*) FROM upload_sessions WHERE user_id = ?1 AND expires_at > ?2",
                params![user_id, now],
                |row| row.get::<_, i64>(0),
            )?;
            Ok(count.max(0) as usize)
        })
        .await
    }

    pub async fn touch_session(&self, token_hash: &str) -> anyhow::Result<()> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE sessions SET last_seen_at = ?1 WHERE token_hash = ?2",
                params![unix_now(), token_hash],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn online_user_ids(&self, since: i64, now: i64) -> anyhow::Result<Vec<i64>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT user_id FROM sessions \
                 WHERE expires_at > ?1 AND COALESCE(last_seen_at, 0) >= ?2",
            )?;
            let rows = statement.query_map(params![now, since], |row| row.get(0))?;
            rows.collect()
        })
        .await
    }

    pub async fn prune_expired_sessions(&self, now: i64) -> anyhow::Result<usize> {
        self.call(move |connection| {
            let removed =
                connection.execute("DELETE FROM sessions WHERE expires_at < ?1", params![now])?;
            Ok(removed)
        })
        .await
    }

    pub async fn take_expired_upload_temps(&self, now: i64) -> anyhow::Result<Vec<String>> {
        self.call(move |connection| {
            let mut statement = connection
                .prepare("SELECT temp_path FROM upload_sessions WHERE expires_at < ?1")?;
            let paths = statement
                .query_map(params![now], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            connection.execute(
                "DELETE FROM upload_sessions WHERE expires_at < ?1",
                params![now],
            )?;
            Ok(paths)
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
        // Commit in bounded chunks. A single transaction for a directory with
        // tens of thousands of entries keeps the SQLite writer lock for seconds
        // and makes deletes/uploads/listing wait behind it.
        const CHUNK: usize = 500;
        loop {
            let removed = self
                .call(move |connection| {
                    connection.execute(
                        "DELETE FROM index_entries WHERE rowid IN (\
                           SELECT rowid FROM index_entries WHERE dir_id = ?1 LIMIT ?2)",
                        params![dir_id, CHUNK as i64],
                    )
                })
                .await?;
            if removed == 0 {
                break;
            }
        }

        let total = entries.len();
        for chunk in entries.chunks(CHUNK) {
            let rows = chunk.to_vec();
            self.call(move |connection| {
                let transaction = connection.transaction()?;
                {
                    let mut statement = transaction.prepare(
                        "INSERT INTO index_entries \
                         (dir_id, name, real_path, is_dir, size, mtime, created_at, mime_type, \
                          has_preview, is_hidden) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING id",
                    )?;
                    let mut gram_statement = transaction.prepare(
                        "INSERT OR IGNORE INTO index_name_bigrams (entry_id, gram) VALUES (?1, ?2)",
                    )?;
                    for entry in &rows {
                        let entry_id: i64 = statement.query_row(
                            params![
                                dir_id,
                                entry.name,
                                entry.real_path,
                                entry.is_dir as i64,
                                entry.size,
                                entry.mtime,
                                entry.created_at,
                                entry.mime_type,
                                entry.has_preview as i64,
                                entry.is_hidden as i64
                            ],
                            |row| row.get(0),
                        )?;
                        for gram in name_bigrams(&entry.name) {
                            gram_statement.execute(params![entry_id, gram])?;
                        }
                    }
                }
                transaction.commit()?;
                Ok(())
            })
            .await?;
        }
        Ok(total)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_index_dir(
        &self,
        dir_id: i64,
        status: &str,
        entry_count: i64,
        total_size_bytes: i64,
        partial: bool,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let status = status.to_owned();
        let error = error.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "UPDATE indexed_dirs \
                 SET status = ?1, entry_count = ?2, total_size_bytes = ?3, partial = ?4, \
                     error = ?5, indexed_at = ?6 \
                 WHERE id = ?7",
                params![
                    status,
                    entry_count,
                    total_size_bytes,
                    partial as i64,
                    error,
                    unix_now(),
                    dir_id
                ],
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

    pub async fn indexed_directory_state(
        &self,
        real_path: &str,
    ) -> anyhow::Result<Option<(i64, u64, bool, String)>> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT mtime_ns, total_size_bytes, partial, status FROM indexed_dirs \
                     WHERE real_path = ?1",
                    params![real_path],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get::<_, i64>(1)?.max(0) as u64,
                            row.get::<_, i64>(2)? != 0,
                            row.get(3)?,
                        ))
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn indexed_directory_states(
        &self,
        paths: Vec<String>,
    ) -> anyhow::Result<HashMap<String, (i64, u64, bool, String)>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT mtime_ns, total_size_bytes, partial, status FROM indexed_dirs \
                 WHERE real_path = ?1",
            )?;
            let mut states = HashMap::new();
            for path in paths {
                if let Some(state) = statement
                    .query_row(params![path], |row| {
                        Ok((
                            row.get(0)?,
                            row.get::<_, i64>(1)?.max(0) as u64,
                            row.get::<_, i64>(2)? != 0,
                            row.get::<_, String>(3)?,
                        ))
                    })
                    .optional()?
                {
                    states.insert(path, state);
                }
            }
            Ok(states)
        })
        .await
    }

    pub async fn dir_index_stats(
        &self,
        paths: Vec<String>,
    ) -> anyhow::Result<HashMap<String, DirIndexStat>> {
        self.call(move |connection| {
            let mut indexed = connection.prepare(
                "SELECT total_size_bytes, entry_count, status FROM indexed_dirs WHERE real_path = ?1",
            )?;
            let mut cached = connection.prepare(
                "SELECT size_bytes, status, has_value FROM directory_sizes WHERE real_path = ?1",
            )?;
            let mut result = HashMap::new();
            for path in paths {
                if let Some((size, entries, status)) = indexed
                    .query_row(params![path], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .optional()?
                {
                    result.insert(
                        path,
                        DirIndexStat {
                            size: Some(size.max(0) as u64),
                            entry_count: Some(entries.max(0) as u64),
                            source: "index".to_owned(),
                            stale: status != "ready",
                        },
                    );
                    continue;
                }
                if let Some((size, status, has_value)) = cached
                    .query_row(params![path], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    })
                    .optional()?
                    && has_value != 0
                {
                    result.insert(
                        path,
                        DirIndexStat {
                            size: Some(size.max(0) as u64),
                            entry_count: None,
                            source: "cache".to_owned(),
                            stale: status != "ready",
                        },
                    );
                }
            }
            Ok(result)
        })
        .await
    }

    pub async fn directory_size_states(
        &self,
        paths: Vec<String>,
    ) -> anyhow::Result<HashMap<String, (String, i64, Option<String>)>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT status, size_bytes, error FROM directory_sizes WHERE real_path = ?1",
            )?;
            let mut states = HashMap::new();
            for path in paths {
                if let Some(state) = statement
                    .query_row(params![path], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .optional()?
                {
                    states.insert(path, state);
                }
            }
            Ok(states)
        })
        .await
    }

    pub async fn directory_size_values(
        &self,
        paths: Vec<String>,
    ) -> anyhow::Result<std::collections::HashMap<String, (String, i64)>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT status, size_bytes FROM directory_sizes \
                 WHERE real_path = ?1 AND size_bytes IS NOT NULL",
            )?;
            let mut values = std::collections::HashMap::new();
            for path in paths {
                if let Some(value) = statement
                    .query_row(params![path.clone()], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })
                    .optional()?
                {
                    values.insert(path, value);
                }
            }
            Ok(values)
        })
        .await
    }

    pub async fn directory_size_state(
        &self,
        real_path: &str,
    ) -> anyhow::Result<Option<(String, i64, i64, Option<String>, i64, bool)>> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT status, size_bytes, mtime_ns, error, updated_at, has_value \
                     FROM directory_sizes WHERE real_path = ?1",
                    params![real_path],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get::<_, i64>(5)? != 0,
                        ))
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn set_directory_size(
        &self,
        real_path: &str,
        status: &str,
        size_bytes: i64,
        mtime_ns: i64,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let real_path = real_path.to_owned();
        let status = status.to_owned();
        let has_value = i64::from(status == "ready");
        let error = error.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO directory_sizes \
                 (real_path, size_bytes, mtime_ns, status, error, updated_at, has_value) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(real_path) DO UPDATE SET \
                   size_bytes = excluded.size_bytes, mtime_ns = excluded.mtime_ns, \
                   status = excluded.status, error = excluded.error, \
                   updated_at = excluded.updated_at, \
                   has_value = CASE WHEN excluded.has_value = 1 \
                                    THEN 1 ELSE directory_sizes.has_value END",
                params![
                    real_path,
                    size_bytes,
                    mtime_ns,
                    status,
                    error,
                    unix_now(),
                    has_value,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Marks a directory size calculation as running while keeping the last
    /// successfully calculated value available for stale-while-revalidate
    /// responses.
    pub async fn begin_directory_size(&self, real_path: &str, mtime_ns: i64) -> anyhow::Result<()> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO directory_sizes \
                 (real_path, size_bytes, mtime_ns, status, error, updated_at, has_value) \
                 VALUES (?1, 0, ?2, 'running', NULL, ?3, 0) \
                 ON CONFLICT(real_path) DO UPDATE SET \
                   mtime_ns = excluded.mtime_ns, status = 'running', \
                   error = NULL, updated_at = excluded.updated_at",
                params![real_path, mtime_ns, unix_now()],
            )?;
            Ok(())
        })
        .await
    }

    /// Records a failed or cancelled calculation without discarding the last
    /// known size.
    pub async fn fail_directory_size(
        &self,
        real_path: &str,
        status: &str,
        mtime_ns: i64,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let real_path = real_path.to_owned();
        let status = status.to_owned();
        let error = error.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO directory_sizes \
                 (real_path, size_bytes, mtime_ns, status, error, updated_at, has_value) \
                 VALUES (?1, 0, ?2, ?3, ?4, ?5, 0) \
                 ON CONFLICT(real_path) DO UPDATE SET \
                   mtime_ns = excluded.mtime_ns, status = excluded.status, \
                   error = excluded.error, updated_at = excluded.updated_at",
                params![real_path, mtime_ns, status, error, unix_now()],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn delete_directory_size_exact(&self, real_path: &str) -> anyhow::Result<()> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.execute(
                "DELETE FROM directory_sizes WHERE real_path = ?1",
                params![real_path],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn set_directory_sizes_batch(
        &self,
        rows: Vec<(String, i64, i64, Option<String>)>,
    ) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let now = unix_now();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            {
                let mut statement = transaction.prepare(
                    "INSERT INTO directory_sizes \
                     (real_path, size_bytes, mtime_ns, status, error, updated_at, has_value) \
                     VALUES (?1, ?2, ?3, 'ready', ?4, ?5, 1) \
                     ON CONFLICT(real_path) DO UPDATE SET \
                       size_bytes = excluded.size_bytes, mtime_ns = excluded.mtime_ns, \
                       status = 'ready', error = excluded.error, \
                       updated_at = excluded.updated_at, has_value = 1",
                )?;
                for (real_path, size_bytes, mtime_ns, error) in &rows {
                    statement.execute(params![real_path, size_bytes, mtime_ns, error, now])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn mark_directory_size_stale(&self, real_path: &str) -> anyhow::Result<()> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE directory_sizes SET status = 'stale' \
                 WHERE real_path = ?1 AND status IN ('ready', 'error')",
                params![real_path],
            )?;
            Ok(())
        })
        .await
    }

    fn subtree_bounds(prefix: &str) -> Vec<(String, String)> {
        let mut bounds = Vec::new();
        for (separator, upper) in [("/", "0"), ("\\", "]")] {
            let base = prefix.strip_suffix(separator).unwrap_or(prefix);
            let start = format!("{base}{separator}");
            let end = format!("{base}{upper}");
            if start < end {
                bounds.push((start, end));
            }
        }
        bounds
    }

    pub async fn delete_cached_subtree_batch(
        &self,
        prefix: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let bounds = Self::subtree_bounds(prefix);
        if bounds.is_empty() {
            return Ok(0);
        }
        let limit = limit.clamp(1, 5000) as i64;
        self.call(move |connection| {
            let mut deleted = 0usize;
            let transaction = connection.transaction()?;
            for (start, end) in &bounds {
                let remaining = limit - deleted as i64;
                if remaining <= 0 {
                    break;
                }
                deleted += transaction.execute(
                    "DELETE FROM directory_sizes WHERE rowid IN (\
                       SELECT rowid FROM directory_sizes \
                       WHERE real_path >= ?1 AND real_path < ?2 LIMIT ?3)",
                    params![start, end, remaining],
                )?;
            }
            transaction.commit()?;

            // Deleting indexed_dirs directly cascades through FTS triggers for
            // every entry. Remove entries in small transactions first so the
            // writer lock is released between chunks.
            let mut dir_ids: Vec<i64> = Vec::new();
            for (start, end) in &bounds {
                let mut statement = connection.prepare(
                    "SELECT id FROM indexed_dirs WHERE real_path >= ?1 AND real_path < ?2 \
                     ORDER BY id LIMIT 5",
                )?;
                let rows = statement.query_map(params![start, end], |row| row.get(0))?;
                for id in rows {
                    dir_ids.push(id?);
                }
            }
            for dir_id in dir_ids {
                loop {
                    let transaction = connection.transaction()?;
                    let removed = transaction.execute(
                        "DELETE FROM index_entries WHERE rowid IN (\
                           SELECT rowid FROM index_entries WHERE dir_id = ?1 LIMIT 1000)",
                        params![dir_id],
                    )?;
                    transaction.commit()?;
                    if removed == 0 {
                        break;
                    }
                }
                let transaction = connection.transaction()?;
                deleted += transaction
                    .execute("DELETE FROM indexed_dirs WHERE id = ?1", params![dir_id])?;
                transaction.commit()?;
            }
            Ok(deleted)
        })
        .await
    }

    /// Updates a bounded batch of cached descendant sizes to `stale`.
    pub async fn mark_cached_sizes_stale_batch(
        &self,
        prefix: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let bounds = Self::subtree_bounds(prefix);
        if bounds.is_empty() {
            return Ok(0);
        }
        let limit = limit.clamp(1, 5000) as i64;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let mut updated = 0usize;
            for (start, end) in &bounds {
                let remaining = limit - updated as i64;
                if remaining <= 0 {
                    break;
                }
                updated += transaction.execute(
                    "UPDATE directory_sizes SET status = 'stale' WHERE rowid IN (\
                       SELECT rowid FROM directory_sizes \
                       WHERE real_path >= ?1 AND real_path < ?2 \
                         AND status IN ('ready', 'error') LIMIT ?3)",
                    params![start, end, remaining],
                )?;
            }
            transaction.commit()?;
            Ok(updated)
        })
        .await
    }

    /// Apply a target + ancestor cache invalidation chain in one transaction.
    /// This collapses many short write attempts into a single writer-lock wait.
    pub async fn invalidate_path_chain(&self, paths: Vec<String>) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            for path in &paths {
                transaction.execute(
                    "DELETE FROM directory_sizes WHERE real_path = ?1",
                    params![path],
                )?;
                transaction.execute(
                    "UPDATE indexed_dirs SET status = 'stale' WHERE real_path = ?1",
                    params![path],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn runtime_state_get(&self, key: &str) -> anyhow::Result<Option<i64>> {
        let key = key.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT value FROM runtime_state WHERE key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .optional()
        })
        .await
    }

    pub async fn runtime_state_set(&self, key: &str, value: i64) -> anyhow::Result<()> {
        let key = key.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO runtime_state (key, value, updated_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, unix_now()],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn usn_states(&self) -> anyhow::Result<Vec<UsnState>> {
        self.call(|connection| {
            let mut statement =
                connection.prepare("SELECT volume, journal_id, next_usn FROM usn_state")?;
            let rows = statement.query_map([], |row| {
                Ok(UsnState {
                    volume: row.get(0)?,
                    journal_id: row.get::<_, i64>(1)? as u64,
                    next_usn: row.get(2)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn delete_usn_state(&self, volume: &str) -> anyhow::Result<()> {
        let volume = volume.to_owned();
        self.call(move |connection| {
            connection.execute("DELETE FROM usn_state WHERE volume = ?1", params![volume])?;
            Ok(())
        })
        .await
    }

    pub async fn upsert_usn_state(&self, state: &UsnState) -> anyhow::Result<()> {
        let volume = state.volume.clone();
        let journal_id = state.journal_id as i64;
        let next_usn = state.next_usn;
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO usn_state (volume, journal_id, next_usn, updated_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(volume) DO UPDATE SET journal_id = excluded.journal_id, \
                   next_usn = excluded.next_usn, updated_at = excluded.updated_at",
                params![volume, journal_id, next_usn, unix_now()],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn existing_index_dirs(&self, paths: Vec<String>) -> anyhow::Result<HashSet<String>> {
        self.call(move |connection| {
            let mut found = HashSet::new();
            for chunk in paths.chunks(500) {
                let placeholders = std::iter::repeat_n("?", chunk.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT real_path FROM indexed_dirs WHERE real_path IN ({placeholders})"
                );
                let mut statement = connection.prepare(&sql)?;
                let rows = statement
                    .query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                        row.get::<_, String>(0)
                    })?;
                for row in rows {
                    found.insert(row?);
                }
            }
            Ok(found)
        })
        .await
    }

    pub async fn count_indexed_dirs(&self) -> anyhow::Result<i64> {
        self.call(|connection| {
            connection.query_row("SELECT COUNT(*) FROM indexed_dirs", [], |row| row.get(0))
        })
        .await
    }

    pub async fn indexed_dirs_page(
        &self,
        after_path: Option<String>,
        limit: i64,
    ) -> anyhow::Result<Vec<(String, i64, i64, String)>> {
        let after = after_path.unwrap_or_default();
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT real_path, id, mtime_ns, status FROM indexed_dirs \
                 WHERE real_path > ?1 ORDER BY real_path LIMIT ?2",
            )?;
            let rows = statement.query_map(params![after, limit.max(1)], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn mark_index_partial(&self, real_path: &str) -> anyhow::Result<()> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE indexed_dirs SET total_size_bytes = 0, partial = 1 \
                 WHERE real_path = ?1 AND status = 'ready'",
                params![real_path],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn append_pending_index_dirs(&self, paths: &[String]) -> anyhow::Result<usize> {
        let paths = paths.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let now = unix_now();
            let mut inserted = 0_usize;
            for path in paths {
                inserted += transaction.execute(
                    "INSERT OR IGNORE INTO pending_index_dirs (real_path, created_at) \
                     VALUES (?1, ?2)",
                    params![path, now],
                )?;
            }
            transaction.commit()?;
            Ok(inserted)
        })
        .await
    }

    pub async fn take_pending_index_dirs(&self, limit: i64) -> anyhow::Result<Vec<String>> {
        self.call(move |connection| {
            let limit = limit.clamp(1, 5000);
            let mut statement = connection
                .prepare("SELECT real_path FROM pending_index_dirs ORDER BY real_path LIMIT ?1")?;
            let paths = statement
                .query_map(params![limit], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for path in &paths {
                connection.execute(
                    "DELETE FROM pending_index_dirs WHERE real_path = ?1",
                    params![path],
                )?;
            }
            Ok(paths)
        })
        .await
    }

    pub async fn prune_indexed_pending_dirs(&self) -> anyhow::Result<usize> {
        self.call(|connection| {
            let removed = connection.execute(
                "DELETE FROM pending_index_dirs \
                 WHERE real_path IN (SELECT real_path FROM indexed_dirs)",
                [],
            )?;
            Ok(removed)
        })
        .await
    }

    pub async fn create_job(&self, record: JobRecord) -> anyhow::Result<(JobRecord, bool)> {
        self.call(move |connection| {
            let changed = connection.execute(
                "INSERT INTO jobs \
                 (id, user_id, kind, job_key, status, progress, total, bytes_done, bytes_total, \
                  payload, result, error, created_at, started_at, finished_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
                 ON CONFLICT DO NOTHING",
                params![
                    record.id,
                    record.user_id,
                    record.kind,
                    record.job_key,
                    record.status,
                    record.progress,
                    record.total,
                    record.bytes_done,
                    record.bytes_total,
                    record.payload,
                    record.result,
                    record.error,
                    record.created_at,
                    record.started_at,
                    record.finished_at
                ],
            )?;
            if changed > 0 {
                return Ok((record, true));
            }
            let existing = connection
                .query_row(
                    "SELECT id, user_id, kind, job_key, status, progress, total, bytes_done, \
                     bytes_total, payload, result, error, created_at, started_at, finished_at \
                     FROM jobs WHERE kind = ?1 AND job_key = ?2 \
                     AND status IN ('queued', 'running') ORDER BY created_at DESC LIMIT 1",
                    params![record.kind, record.job_key],
                    job_from_row,
                )
                .optional()?;
            match existing {
                Some(job) => Ok((job, false)),
                None => Err(rusqlite::Error::QueryReturnedNoRows),
            }
        })
        .await
    }

    pub async fn mark_job_running(&self, id: &str) -> anyhow::Result<()> {
        let id = id.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE jobs SET status = 'running', started_at = ?1 \
                 WHERE id = ?2 AND status = 'queued'",
                params![unix_now(), id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn update_job_progress(
        &self,
        id: &str,
        progress: i64,
        total: i64,
        bytes_done: i64,
        bytes_total: i64,
    ) -> anyhow::Result<()> {
        let id = id.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE jobs SET progress = ?2, total = ?3, bytes_done = ?4, bytes_total = ?5 \
                 WHERE id = ?1 AND status = 'running'",
                params![id, progress, total, bytes_done, bytes_total],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn finish_job(
        &self,
        id: &str,
        status: &str,
        result: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let id = id.to_owned();
        let status = status.to_owned();
        let result = result.map(str::to_owned);
        let error = error.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "UPDATE jobs SET status = ?2, result = ?3, error = ?4, finished_at = ?5 \
                 WHERE id = ?1 AND status IN ('queued', 'running')",
                params![id, status, result, error, unix_now()],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn find_job(&self, id: &str) -> anyhow::Result<Option<JobRecord>> {
        let id = id.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, kind, job_key, status, progress, total, bytes_done, \
                     bytes_total, payload, result, error, created_at, started_at, finished_at \
                     FROM jobs WHERE id = ?1",
                    params![id],
                    job_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn list_jobs_for_user(
        &self,
        user_id: i64,
        since: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<JobRecord>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, user_id, kind, job_key, status, progress, total, bytes_done, \
                 bytes_total, payload, result, error, created_at, started_at, finished_at \
                 FROM jobs WHERE user_id = ?1 \
                 AND (status IN ('queued', 'running') OR COALESCE(finished_at, created_at) >= ?2) \
                 ORDER BY created_at DESC LIMIT ?3",
            )?;
            let rows = statement.query_map(params![user_id, since, limit.max(1)], job_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
    }

    pub async fn recover_jobs(&self) -> anyhow::Result<usize> {
        self.call(|connection| {
            let changed = connection.execute(
                "UPDATE jobs SET status = 'failed', error = 'interrupted by server restart', \
                 finished_at = ?1 WHERE status IN ('queued', 'running')",
                params![unix_now()],
            )?;
            connection.execute(
                "UPDATE directory_sizes SET status = 'stale' \
                 WHERE status IN ('queued', 'running')",
                [],
            )?;
            Ok(changed)
        })
        .await
    }

    pub async fn active_job_count(&self, kind: &str) -> anyhow::Result<i64> {
        let kind = kind.to_owned();
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM jobs WHERE kind = ?1 AND status IN ('queued', 'running')",
                params![kind],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn job_counts(&self) -> anyhow::Result<Vec<(String, String, i64)>> {
        self.call(|connection| {
            let mut statement = connection.prepare(
                "SELECT kind, status, COUNT(*) FROM jobs \
                 WHERE status IN ('queued', 'running') GROUP BY kind, status",
            )?;
            let rows =
                statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            rows.collect()
        })
        .await
    }

    pub async fn prune_jobs(&self, before: i64) -> anyhow::Result<usize> {
        self.call(move |connection| {
            let removed = connection.execute(
                "DELETE FROM jobs WHERE finished_at IS NOT NULL AND finished_at < ?1",
                params![before],
            )?;
            Ok(removed)
        })
        .await
    }

    pub async fn prune_directory_sizes(&self, before: i64) -> anyhow::Result<usize> {
        self.call(move |connection| {
            let removed = connection.execute(
                "DELETE FROM directory_sizes WHERE updated_at < ?1",
                params![before],
            )?;
            Ok(removed)
        })
        .await
    }

    pub async fn clear_directory_sizes(&self) -> anyhow::Result<usize> {
        self.call(|connection| {
            let removed = connection.execute("DELETE FROM directory_sizes", [])?;
            Ok(removed)
        })
        .await
    }

    pub async fn clear_directory_sizes_batch(&self, limit: usize) -> anyhow::Result<usize> {
        let limit = limit.clamp(1, 5000) as i64;
        self.call(move |connection| {
            connection.execute(
                "DELETE FROM directory_sizes WHERE rowid IN (\
                   SELECT rowid FROM directory_sizes LIMIT ?1)",
                params![limit],
            )
        })
        .await
    }

    pub async fn directory_size_counts(&self) -> anyhow::Result<Vec<(String, i64)>> {
        self.call(|connection| {
            let mut statement = connection
                .prepare("SELECT status, COUNT(*) FROM directory_sizes GROUP BY status")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
        .await
    }

    /// Run a TRUNCATE checkpoint and return whether another connection was
    /// still reading (busy). Busy checkpoints are retried by the next
    /// maintenance pass.
    pub async fn checkpoint(&self) -> anyhow::Result<bool> {
        self.call(|connection| {
            let busy: i64 =
                connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
            Ok(busy != 0)
        })
        .await
    }

    pub async fn stats(&self) -> anyhow::Result<DbStats> {
        self.call(|connection| {
            let page_count: i64 =
                connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
            let page_size: i64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
            let freelist_count: i64 =
                connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
            let path: String = connection
                .query_row(
                    "SELECT file FROM pragma_database_list WHERE name = 'main'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            let wal_bytes = std::fs::metadata(format!("{path}-wal"))
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            Ok(DbStats {
                page_count,
                page_size,
                freelist_count,
                wal_bytes,
            })
        })
        .await
    }

    pub async fn index_status(
        &self,
        real_path: &str,
    ) -> anyhow::Result<Option<(String, i64, Option<i64>, u64, bool)>> {
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT status, entry_count, indexed_at, total_size_bytes, partial \
                     FROM indexed_dirs WHERE real_path = ?1",
                    params![real_path],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get::<_, i64>(3)?.max(0) as u64,
                            row.get::<_, i64>(4)? != 0,
                        ))
                    },
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
            "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime, e.created_at, e.mime_type, \
                    e.has_preview, e.is_hidden \
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
                    created_at: row.get(5)?,
                    mime_type: row.get(6)?,
                    has_preview: row.get::<_, i64>(7)? != 0,
                    is_hidden: row.get::<_, i64>(8)? != 0,
                })
            })?;
            let entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some((mtime_ns, entries)))
        })
        .await
    }

    /// Returns `(mtime_ns, offset, total)` for a named child in a fresh
    /// indexed directory, using the same ordering as [`Self::indexed_children_page`].
    pub async fn locate_indexed_child(
        &self,
        dir_path: &str,
        name: &str,
        sort: &str,
        order: &str,
        show_hidden: bool,
    ) -> anyhow::Result<Option<(i64, usize, usize)>> {
        let dir_path = dir_path.to_owned();
        let name = name.to_owned();
        let sort = sort.to_owned();
        let order = order.to_owned();
        self.call(move |connection| {
            let dir = connection
                .query_row(
                    "SELECT id, mtime_ns FROM indexed_dirs WHERE real_path = ?1 AND status = 'ready'",
                    params![dir_path],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?;
            let Some((dir_id, mtime_ns)) = dir else {
                return Ok(None);
            };
            let target = connection
                .query_row(
                    "SELECT is_dir, size, mtime, name FROM index_entries \
                     WHERE dir_id = ?1 AND name = ?2 AND (?3 = 1 OR name NOT LIKE '.%')",
                    params![dir_id, name, show_hidden as i64],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?;
            let Some((is_dir, size, mtime, target_name)) = target else {
                return Ok(None);
            };
            let total: i64 = connection.query_row(
                "SELECT COUNT(*) FROM index_entries \
                 WHERE dir_id = ?1 AND (?2 = 1 OR name NOT LIKE '.%')",
                params![dir_id, show_hidden as i64],
                |row| row.get(0),
            )?;
            let reverse = if order.eq_ignore_ascii_case("desc") {
                ">"
            } else {
                "<"
            };
            let (key_column, key_value) = match sort.as_str() {
                "size" => (
                    "size",
                    rusqlite::types::Value::Integer(size),
                ),
                "modified" => (
                    "mtime",
                    rusqlite::types::Value::Integer(mtime),
                ),
                _ => (
                    "name COLLATE NOCASE",
                    rusqlite::types::Value::Text(target_name.clone()),
                ),
            };
            locate_child_offset(
                connection,
                dir_id,
                show_hidden,
                is_dir,
                key_column,
                reverse,
                key_value,
                &target_name,
                total,
                mtime_ns,
            )
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
            "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime, e.created_at, e.mime_type, \
                    e.has_preview, e.is_hidden \
             FROM index_entries e JOIN indexed_dirs d ON d.id = e.dir_id \
             WHERE d.real_path = ?1 AND d.status = 'ready' \
               AND (?4 = 1 OR e.name NOT LIKE '.%') \
             ORDER BY e.is_dir DESC, {order_column} {direction}, e.name COLLATE NOCASE {direction} \
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
                        created_at: row.get(5)?,
                        mime_type: row.get(6)?,
                        has_preview: row.get::<_, i64>(7)? != 0,
                        is_hidden: row.get::<_, i64>(8)? != 0,
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

    /// Searches all accessible roots in one FTS pass with unioned path ranges.
    /// This prevents a root with few matches from scanning the whole FTS set on
    /// its own, which is the main source of multi-second searches.
    pub async fn search_entries_all(
        &self,
        query: &str,
        root_ranges: Vec<(String, String)>,
        show_hidden: bool,
        limit: i64,
        cursor: Option<i64>,
        filters: IndexSearchFilters,
    ) -> anyhow::Result<(Vec<IndexEntry>, Option<i64>)> {
        if root_ranges.is_empty() {
            return Ok((Vec::new(), None));
        }
        let query = query.to_owned();
        self.call(move |connection| {
            let limit = limit.clamp(1, 200);
            let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
            let mut sql = String::from(
                "SELECT e.id, e.name, e.real_path, e.is_dir, e.size, e.mtime, e.created_at, \
                        e.mime_type, e.has_preview, e.is_hidden \
                 FROM index_fts JOIN index_entries e ON e.id = index_fts.rowid \
                 JOIN indexed_dirs d ON d.id = e.dir_id \
                 WHERE index_fts MATCH ?1 AND (",
            );
            let mut params: Vec<rusqlite::types::Value> =
                vec![rusqlite::types::Value::Text(fts_query)];
            let mut index = 2;
            for (position, (start, end)) in root_ranges.iter().enumerate() {
                if position > 0 {
                    sql.push_str(" OR ");
                }
                sql.push_str(&format!(
                    "(e.real_path >= ?{index} AND e.real_path < ?{})",
                    index + 1
                ));
                params.push(rusqlite::types::Value::Text(start.clone()));
                params.push(rusqlite::types::Value::Text(end.clone()));
                index += 2;
            }
            sql.push_str(&format!(
                ") AND (?{index} = 1 OR e.name NOT LIKE '.%') \
                 AND (?{next} IS NULL OR e.size >= ?{next}) \
                 AND (?{after} IS NULL OR e.size <= ?{after}) \
                 AND (?{mtime_after} IS NULL OR e.mtime >= ?{mtime_after}) \
                 AND (?{mtime_before} IS NULL OR e.mtime <= ?{mtime_before}) \
                 AND (?{is_dir} IS NULL OR e.is_dir = ?{is_dir}) \
                 AND (?{mime} IS NULL OR e.mime_type LIKE ?{mime}) \
                 AND index_fts.rowid > ?{cursor} \
                 LIMIT ?{limit_param}",
                next = index + 1,
                after = index + 2,
                mtime_after = index + 3,
                mtime_before = index + 4,
                is_dir = index + 5,
                mime = index + 6,
                cursor = index + 7,
                limit_param = index + 8,
            ));
            // The format above uses `?{index}` at the start and named offsets;
            // push values in the same order.
            params.push(rusqlite::types::Value::Integer(show_hidden as i64));
            params.push(
                filters
                    .min_size
                    .map(|value| rusqlite::types::Value::Integer(value.min(i64::MAX as u64) as i64))
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .max_size
                    .map(|value| rusqlite::types::Value::Integer(value.min(i64::MAX as u64) as i64))
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .modified_after
                    .map(rusqlite::types::Value::Integer)
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .modified_before
                    .map(rusqlite::types::Value::Integer)
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .is_dir
                    .map(|value| rusqlite::types::Value::Integer(value as i64))
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .mime_prefix
                    .clone()
                    .map(rusqlite::types::Value::Text)
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(rusqlite::types::Value::Integer(cursor.unwrap_or(0)));
            params.push(rusqlite::types::Value::Integer(limit + 1));

            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    IndexEntry {
                        name: row.get(1)?,
                        real_path: row.get(2)?,
                        is_dir: row.get::<_, i64>(3)? != 0,
                        size: row.get(4)?,
                        mtime: row.get(5)?,
                        created_at: row.get(6)?,
                        mime_type: row.get(7)?,
                        has_preview: row.get::<_, i64>(8)? != 0,
                        is_hidden: row.get::<_, i64>(9)? != 0,
                    },
                ))
            })?;
            let mut rows: Vec<(i64, IndexEntry)> = rows.collect::<rusqlite::Result<_>>()?;
            let has_more = rows.len() > limit as usize;
            if has_more {
                rows.truncate(limit as usize);
            }
            let next_cursor = has_more.then(|| rows.last().map(|(id, _)| *id)).flatten();
            Ok((
                rows.into_iter().map(|(_, entry)| entry).collect(),
                next_cursor,
            ))
        })
        .await
    }

    /// Short CJK query path: exact bigram lookup, ordered by entry id so the
    /// same cursor pagination as the trigram path can be reused.
    pub async fn search_entries_bigram(
        &self,
        gram: &str,
        root_ranges: Vec<(String, String)>,
        show_hidden: bool,
        limit: i64,
        cursor: Option<i64>,
        filters: IndexSearchFilters,
    ) -> anyhow::Result<(Vec<IndexEntry>, Option<i64>)> {
        if root_ranges.is_empty() {
            return Ok((Vec::new(), None));
        }
        let gram = gram.to_owned();
        self.call(move |connection| {
            let limit = limit.clamp(1, 200);
            let mut sql = String::from(
                "SELECT e.id, e.name, e.real_path, e.is_dir, e.size, e.mtime, e.created_at, \
                        e.mime_type, e.has_preview, e.is_hidden \
                 FROM index_name_bigrams b JOIN index_entries e ON e.id = b.entry_id \
                 JOIN indexed_dirs d ON d.id = e.dir_id \
                 WHERE b.gram = ?1 AND (",
            );
            let mut params: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(gram)];
            let mut index = 2;
            for (position, (start, end)) in root_ranges.iter().enumerate() {
                if position > 0 {
                    sql.push_str(" OR ");
                }
                sql.push_str(&format!(
                    "(e.real_path >= ?{index} AND e.real_path < ?{})",
                    index + 1
                ));
                params.push(rusqlite::types::Value::Text(start.clone()));
                params.push(rusqlite::types::Value::Text(end.clone()));
                index += 2;
            }
            sql.push_str(&format!(
                ") AND (?{index} = 1 OR e.name NOT LIKE '.%') \
                 AND (?{next} IS NULL OR e.size >= ?{next}) \
                 AND (?{after} IS NULL OR e.size <= ?{after}) \
                 AND (?{mtime_after} IS NULL OR e.mtime >= ?{mtime_after}) \
                 AND (?{mtime_before} IS NULL OR e.mtime <= ?{mtime_before}) \
                 AND (?{is_dir} IS NULL OR e.is_dir = ?{is_dir}) \
                 AND (?{mime} IS NULL OR e.mime_type LIKE ?{mime}) \
                 AND b.entry_id > ?{cursor} \
                 ORDER BY b.entry_id ASC LIMIT ?{limit_param}",
                next = index + 1,
                after = index + 2,
                mtime_after = index + 3,
                mtime_before = index + 4,
                is_dir = index + 5,
                mime = index + 6,
                cursor = index + 7,
                limit_param = index + 8,
            ));
            params.push(rusqlite::types::Value::Integer(show_hidden as i64));
            params.push(
                filters
                    .min_size
                    .map(|value| rusqlite::types::Value::Integer(value.min(i64::MAX as u64) as i64))
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .max_size
                    .map(|value| rusqlite::types::Value::Integer(value.min(i64::MAX as u64) as i64))
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .modified_after
                    .map(rusqlite::types::Value::Integer)
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .modified_before
                    .map(rusqlite::types::Value::Integer)
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .is_dir
                    .map(|value| rusqlite::types::Value::Integer(value as i64))
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(
                filters
                    .mime_prefix
                    .clone()
                    .map(rusqlite::types::Value::Text)
                    .unwrap_or(rusqlite::types::Value::Null),
            );
            params.push(rusqlite::types::Value::Integer(cursor.unwrap_or(0)));
            params.push(rusqlite::types::Value::Integer(limit + 1));

            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    IndexEntry {
                        name: row.get(1)?,
                        real_path: row.get(2)?,
                        is_dir: row.get::<_, i64>(3)? != 0,
                        size: row.get(4)?,
                        mtime: row.get(5)?,
                        created_at: row.get(6)?,
                        mime_type: row.get(7)?,
                        has_preview: row.get::<_, i64>(8)? != 0,
                        is_hidden: row.get::<_, i64>(9)? != 0,
                    },
                ))
            })?;
            let mut rows: Vec<(i64, IndexEntry)> = rows.collect::<rusqlite::Result<_>>()?;
            let has_more = rows.len() > limit as usize;
            if has_more {
                rows.truncate(limit as usize);
            }
            let next_cursor = has_more.then(|| rows.last().map(|(id, _)| *id)).flatten();
            Ok((
                rows.into_iter().map(|(_, entry)| entry).collect(),
                next_cursor,
            ))
        })
        .await
    }

    /// One-time/restart-safe backfill for entries indexed before M26.
    pub async fn backfill_name_bigrams_batch(
        &self,
        after_id: i64,
        limit: usize,
    ) -> anyhow::Result<(usize, Option<i64>)> {
        let limit = limit.clamp(1, 5000) as i64;
        self.call(move |connection| {
            let mut statement = connection
                .prepare("SELECT id, name FROM index_entries WHERE id > ?1 ORDER BY id LIMIT ?2")?;
            let rows = statement
                .query_map(params![after_id, limit], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            if rows.is_empty() {
                return Ok((0, None));
            }
            let last_id = rows.last().map(|(id, _)| *id);
            let transaction = connection.transaction()?;
            {
                let mut insert = transaction.prepare(
                    "INSERT OR IGNORE INTO index_name_bigrams (entry_id, gram) \
                     SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM index_entries WHERE id = ?1)",
                )?;
                for (entry_id, name) in &rows {
                    for gram in name_bigrams(name) {
                        insert.execute(params![entry_id, gram])?;
                    }
                }
            }
            transaction.commit()?;
            Ok((rows.len(), last_id))
        })
        .await
    }

    pub async fn search_entries(
        &self,
        query: &str,
        root_path: &str,
        show_hidden: bool,
        limit: i64,
        filters: IndexSearchFilters,
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
                    "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime, e.created_at, \
                            e.mime_type, e.has_preview, e.is_hidden \
                     FROM index_fts JOIN index_entries e ON e.id = index_fts.rowid \
                     JOIN indexed_dirs d ON d.id = e.dir_id \
                     WHERE index_fts MATCH ?1
                       AND (e.real_path = ?3 OR e.real_path LIKE ?4 ESCAPE '!')
                       AND (?5 = 1 OR e.name NOT LIKE '.%')
                       AND (?6 IS NULL OR e.size >= ?6)
                       AND (?7 IS NULL OR e.size <= ?7)
                       AND (?8 IS NULL OR e.mtime >= ?8)
                       AND (?9 IS NULL OR e.mtime <= ?9)
                       AND (?10 IS NULL OR e.is_dir = ?10)
                       AND (?11 IS NULL OR e.mime_type LIKE ?11)
                     LIMIT ?2",
                )?;
                let rows = statement.query_map(
                    params![
                        fts_query,
                        limit,
                        root_path,
                        root_pattern,
                        show_hidden as i64,
                        filters
                            .min_size
                            .map(|value| value.min(i64::MAX as u64) as i64),
                        filters
                            .max_size
                            .map(|value| value.min(i64::MAX as u64) as i64),
                        filters.modified_after,
                        filters.modified_before,
                        filters.is_dir.map(|value| value as i64),
                        filters.mime_prefix
                    ],
                    |row| {
                        Ok(IndexEntry {
                            name: row.get(0)?,
                            real_path: row.get(1)?,
                            is_dir: row.get::<_, i64>(2)? != 0,
                            size: row.get(3)?,
                            mtime: row.get(4)?,
                            created_at: row.get(5)?,
                            mime_type: row.get(6)?,
                            has_preview: row.get::<_, i64>(7)? != 0,
                            is_hidden: row.get::<_, i64>(8)? != 0,
                        })
                    },
                )?;
                for entry in rows {
                    entries.push(entry?);
                }
            } else {
                // FTS trigram needs at least three characters. Keep substring
                // semantics for short queries; a prefix-only fallback silently
                // missed matches that were not at the start of the name.
                let pattern = format!(
                    "%{}%",
                    query
                        .replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                );
                let mut statement = connection.prepare(
                    "SELECT e.name, e.real_path, e.is_dir, e.size, e.mtime, e.created_at, \
                            e.mime_type, e.has_preview, e.is_hidden \
                     FROM index_entries e JOIN indexed_dirs d ON d.id = e.dir_id \
                     WHERE e.name LIKE ?1 ESCAPE '\\'
                       AND (e.real_path = ?3 OR e.real_path LIKE ?4 ESCAPE '!')
                       AND (?5 = 1 OR e.name NOT LIKE '.%')
                       AND (?6 IS NULL OR e.size >= ?6)
                       AND (?7 IS NULL OR e.size <= ?7)
                       AND (?8 IS NULL OR e.mtime >= ?8)
                       AND (?9 IS NULL OR e.mtime <= ?9)
                       AND (?10 IS NULL OR e.is_dir = ?10)
                       AND (?11 IS NULL OR e.mime_type LIKE ?11)
                     LIMIT ?2",
                )?;
                let rows = statement.query_map(
                    params![
                        pattern,
                        limit,
                        root_path,
                        root_pattern,
                        show_hidden as i64,
                        filters
                            .min_size
                            .map(|value| value.min(i64::MAX as u64) as i64),
                        filters
                            .max_size
                            .map(|value| value.min(i64::MAX as u64) as i64),
                        filters.modified_after,
                        filters.modified_before,
                        filters.is_dir.map(|value| value as i64),
                        filters.mime_prefix
                    ],
                    |row| {
                        Ok(IndexEntry {
                            name: row.get(0)?,
                            real_path: row.get(1)?,
                            is_dir: row.get::<_, i64>(2)? != 0,
                            size: row.get(3)?,
                            mtime: row.get(4)?,
                            created_at: row.get(5)?,
                            mime_type: row.get(6)?,
                            has_preview: row.get::<_, i64>(7)? != 0,
                            is_hidden: row.get::<_, i64>(8)? != 0,
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

    pub async fn update_server_path_alias(
        &self,
        id: i64,
        alias: &str,
    ) -> anyhow::Result<Option<Vec<i64>>> {
        let alias = alias.to_owned();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let real_path = transaction
                .query_row(
                    "SELECT real_path FROM server_paths WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(real_path) = real_path else {
                transaction.commit()?;
                return Ok(None);
            };
            transaction.execute(
                "UPDATE server_paths SET alias = ?1 WHERE id = ?2",
                params![alias, id],
            )?;
            // Assigned roots mirror the server path alias, so renaming an
            // external path updates every user card without a second edit.
            transaction.execute(
                "UPDATE user_roots SET alias = ?1 WHERE real_path = ?2",
                params![alias, real_path],
            )?;
            let mut statement = transaction
                .prepare("SELECT DISTINCT user_id FROM user_roots WHERE real_path = ?1")?;
            let owners = statement
                .query_map(params![real_path], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            transaction.commit()?;
            Ok(Some(owners))
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
                 (token_hash, creator_id, real_path, is_dir, password_hash, expires_at, read_only, revoked, created_at, kind) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, 'single')",
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

    pub async fn insert_collection_share(
        &self,
        token_hash: &str,
        creator_id: i64,
        items: &[(String, String, bool)],
        password_hash: Option<&str>,
        expires_at: Option<i64>,
        read_only: bool,
    ) -> anyhow::Result<i64> {
        let token_hash = token_hash.to_owned();
        let password_hash = password_hash.map(str::to_owned);
        let items = items.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO shares \
                 (token_hash, creator_id, real_path, is_dir, password_hash, expires_at, read_only, revoked, created_at, kind) \
                 VALUES (?1, ?2, '', 0, ?3, ?4, ?5, 0, ?6, 'collection')",
                params![token_hash, creator_id, password_hash, expires_at, read_only as i64, unix_now()],
            )?;
            let share_id = transaction.last_insert_rowid();
            for (index, (real_path, display_name, is_dir)) in items.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO share_items (share_id, real_path, display_name, is_dir, sort_order) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![share_id, real_path, display_name, *is_dir as i64, index as i64],
                )?;
            }
            transaction.commit()?;
            Ok(share_id)
        })
        .await
    }

    pub async fn share_items(&self, share_id: i64) -> anyhow::Result<Vec<ShareItem>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, share_id, real_path, display_name, is_dir, sort_order \
                 FROM share_items WHERE share_id = ?1 ORDER BY sort_order, id",
            )?;
            let rows = statement.query_map(params![share_id], |row| {
                Ok(ShareItem {
                    id: row.get(0)?,
                    share_id: row.get(1)?,
                    real_path: row.get(2)?,
                    display_name: row.get(3)?,
                    is_dir: row.get::<_, i64>(4)? != 0,
                    sort_order: row.get(5)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn share_item(
        &self,
        share_id: i64,
        item_id: i64,
    ) -> anyhow::Result<Option<ShareItem>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, share_id, real_path, display_name, is_dir, sort_order \
                     FROM share_items WHERE share_id = ?1 AND id = ?2",
                    params![share_id, item_id],
                    |row| {
                        Ok(ShareItem {
                            id: row.get(0)?,
                            share_id: row.get(1)?,
                            real_path: row.get(2)?,
                            display_name: row.get(3)?,
                            is_dir: row.get::<_, i64>(4)? != 0,
                            sort_order: row.get(5)?,
                        })
                    },
                )
                .optional()
        })
        .await
    }

    pub async fn count_share_items(&self, share_id: i64) -> anyhow::Result<i64> {
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM share_items WHERE share_id = ?1",
                params![share_id],
                |row| row.get(0),
            )
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
                 expires_at, read_only, revoked, created_at, visit_count, kind, \
                 (SELECT COUNT(*) FROM share_items i WHERE i.share_id = shares.id) AS item_count, \
                 token_nonce \
                 FROM shares WHERE creator_id = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
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

    pub async fn find_share_by_id(&self, id: i64) -> anyhow::Result<Option<Share>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, token_hash, creator_id, real_path, is_dir, password_hash, \
                     expires_at, read_only, revoked, created_at, visit_count, kind, \
                     (SELECT COUNT(*) FROM share_items i WHERE i.share_id = shares.id) AS item_count, \
                     token_nonce \
                     FROM shares WHERE id = ?1",
                    params![id],
                    share_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn share_token_secret(&self) -> anyhow::Result<String> {
        self.call(move |connection| {
            let existing: Option<String> = connection
                .query_row(
                    "SELECT value FROM server_secrets WHERE name = 'share.token_secret'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(value) = existing {
                return Ok(value);
            }
            let value = format!(
                "{}{}",
                crate::auth::random_token(24),
                crate::auth::random_token(24)
            );
            connection.execute(
                "INSERT OR IGNORE INTO server_secrets (name, value) \
                 VALUES ('share.token_secret', ?1)",
                params![value],
            )?;
            connection.query_row(
                "SELECT value FROM server_secrets WHERE name = 'share.token_secret'",
                [],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn bump_share_nonce(&self, id: i64, creator_id: i64) -> anyhow::Result<Option<i64>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "UPDATE shares SET token_nonce = token_nonce + 1 \
                     WHERE id = ?1 AND creator_id = ?2 RETURNING token_nonce",
                    params![id, creator_id],
                    |row| row.get(0),
                )
                .optional()
        })
        .await
    }

    pub async fn update_share_token_hash(
        &self,
        id: i64,
        creator_id: i64,
        token_hash: &str,
    ) -> anyhow::Result<bool> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE shares SET token_hash = ?1 WHERE id = ?2 AND creator_id = ?3",
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
                     expires_at, read_only, revoked, created_at, visit_count, kind, 0 AS item_count, \
                     token_nonce \
                     FROM shares WHERE token_hash = ?1",
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
        kind: row.get(11)?,
        item_count: row.get(12)?,
        token_nonce: row.get(13)?,
    })
}

fn job_from_row(row: &Row<'_>) -> rusqlite::Result<JobRecord> {
    Ok(JobRecord {
        id: row.get(0)?,
        user_id: row.get(1)?,
        kind: row.get(2)?,
        job_key: row.get(3)?,
        status: row.get(4)?,
        progress: row.get(5)?,
        total: row.get(6)?,
        bytes_done: row.get(7)?,
        bytes_total: row.get(8)?,
        payload: row.get(9)?,
        result: row.get(10)?,
        error: row.get(11)?,
        created_at: row.get(12)?,
        started_at: row.get(13)?,
        finished_at: row.get(14)?,
    })
}

fn configure(connection: &Connection) -> anyhow::Result<()> {
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;\
             PRAGMA foreign_keys = ON;\
             PRAGMA busy_timeout = 5000;\
             PRAGMA synchronous = NORMAL;\
             PRAGMA wal_autocheckpoint = 1000;\
             PRAGMA journal_size_limit = 67108864;",
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

#[cfg(test)]
mod tests {
    use super::Db;

    #[test]
    fn name_bigrams_cover_cjk_pairs_only() {
        assert_eq!(
            super::name_bigrams("山西旅游"),
            vec!["山西", "西旅", "旅游"]
        );
        assert!(super::name_bigrams("plain-ascii").is_empty());
        assert_eq!(super::name_bigrams("A山B"), vec!["A山", "山B"]);
        assert_eq!(super::name_bigrams("哈哈"), vec!["哈哈"]);
    }

    #[test]
    fn subtree_bounds_handle_both_separators() {
        let bounds = Db::subtree_bounds("/mnt/media");
        assert!(bounds.contains(&("/mnt/media/".to_owned(), "/mnt/media0".to_owned())));
        assert!(bounds.contains(&("/mnt/media\\".to_owned(), "/mnt/media]".to_owned())));
        let windows = Db::subtree_bounds("C:\\Music");
        assert!(windows.contains(&("C:\\Music\\".to_owned(), "C:\\Music]".to_owned())));
        assert!(windows.contains(&("C:\\Music/".to_owned(), "C:\\Music0".to_owned())));
        assert!(
            Db::subtree_bounds("/mnt/media/")
                .contains(&("/mnt/media/".to_owned(), "/mnt/media0".to_owned()))
        );
    }

    #[tokio::test]
    async fn cached_subtree_cleanup_is_batched_and_precise() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("cache.db"), 1).unwrap();
        for path in [
            "/data/keep",
            "/data/del",
            "/data/del/a",
            "/data/del/b",
            "/data/del/deep/c",
            "/data/del2/x",
            "/data/delx",
        ] {
            db.set_directory_size(path, "ready", 1, 0, None)
                .await
                .unwrap();
        }
        for path in ["/data/del", "/data/del/a", "/data/del2/x"] {
            db.upsert_index_dir(path, 0).await.unwrap();
        }

        let mut removed = 0;
        loop {
            let count = db
                .delete_cached_subtree_batch("/data/del", 2)
                .await
                .unwrap();
            if count == 0 {
                break;
            }
            removed += count;
        }
        assert!(removed >= 2);

        let sizes = db
            .directory_size_values(vec![
                "/data/keep".to_owned(),
                "/data/del".to_owned(),
                "/data/del/a".to_owned(),
                "/data/del/deep/c".to_owned(),
                "/data/del2/x".to_owned(),
                "/data/delx".to_owned(),
            ])
            .await
            .unwrap();
        assert!(sizes.contains_key("/data/keep"));
        assert!(sizes.contains_key("/data/del"));
        assert!(!sizes.contains_key("/data/del/a"));
        assert!(!sizes.contains_key("/data/del/deep/c"));
        assert!(sizes.contains_key("/data/del2/x"));
        assert!(sizes.contains_key("/data/delx"));
        assert_eq!(db.count_indexed_dirs().await.unwrap(), 2);
    }
}
