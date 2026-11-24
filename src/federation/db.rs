use rusqlite::{OptionalExtension, params};

use crate::db::{Db, unix_now};

#[derive(Debug, Clone)]
pub struct FederationGrant {
    pub id: i64,
    pub user_id: i64,
    pub root_id: i64,
    pub real_path: String,
    pub label: String,
    pub mode: String,
    pub revoked: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct FederationToken {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub prefix: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub revoked: bool,
    pub last_used_at: Option<i64>,
}

fn grant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FederationGrant> {
    Ok(FederationGrant {
        id: row.get(0)?,
        user_id: row.get(1)?,
        root_id: row.get(2)?,
        real_path: row.get(3)?,
        label: row.get(4)?,
        mode: row.get(5)?,
        revoked: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

fn token_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FederationToken> {
    Ok(FederationToken {
        id: row.get(0)?,
        user_id: row.get(1)?,
        name: row.get(2)?,
        prefix: row.get(3)?,
        created_at: row.get(4)?,
        expires_at: row.get(5)?,
        revoked: row.get::<_, i64>(6)? != 0,
        last_used_at: row.get(7)?,
    })
}

impl Db {
    pub async fn federation_instance_id(&self) -> anyhow::Result<String> {
        self.call(|connection| {
            let existing: Option<String> = connection
                .query_row(
                    "SELECT value FROM server_secrets WHERE name = 'federation.instance_id'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(value) = existing {
                return Ok(value);
            }
            let value = crate::auth::random_token(18);
            connection.execute(
                "INSERT INTO server_secrets (name, value) VALUES ('federation.instance_id', ?1)",
                params![value],
            )?;
            Ok(value)
        })
        .await
    }

    pub async fn federation_find_token(
        &self,
        token_hash: &str,
    ) -> anyhow::Result<Option<FederationToken>> {
        let token_hash = token_hash.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, name, prefix, created_at, expires_at, revoked, last_used_at \
                     FROM federation_tokens WHERE token_hash = ?1",
                    params![token_hash],
                    token_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn federation_token_grants(
        &self,
        token_id: i64,
    ) -> anyhow::Result<Vec<FederationGrant>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT g.id, g.user_id, g.root_id, g.real_path, g.label, g.mode, g.revoked, g.created_at \
                 FROM federation_grants g \
                 JOIN federation_token_grants tg ON tg.grant_id = g.id \
                 WHERE tg.token_id = ?1 AND g.revoked = 0 ORDER BY g.id",
            )?;
            let rows = statement.query_map(params![token_id], grant_from_row)?;
            rows.collect()
        })
        .await
    }

    pub async fn federation_grant(&self, id: i64) -> anyhow::Result<Option<FederationGrant>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, root_id, real_path, label, mode, revoked, created_at \
                     FROM federation_grants WHERE id = ?1",
                    params![id],
                    grant_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn federation_list_grants(
        &self,
        user_id: i64,
    ) -> anyhow::Result<Vec<FederationGrant>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, user_id, root_id, real_path, label, mode, revoked, created_at \
                 FROM federation_grants WHERE user_id = ?1 ORDER BY created_at DESC, id DESC",
            )?;
            let rows = statement.query_map(params![user_id], grant_from_row)?;
            rows.collect()
        })
        .await
    }

    pub async fn federation_create_grant(
        &self,
        user_id: i64,
        root_id: i64,
        real_path: &str,
        label: &str,
        mode: &str,
    ) -> anyhow::Result<i64> {
        let real_path = real_path.to_owned();
        let label = label.to_owned();
        let mode = mode.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO federation_grants \
                 (user_id, root_id, real_path, label, mode, revoked, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                params![user_id, root_id, real_path, label, mode, unix_now()],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn federation_revoke_grant(&self, id: i64, user_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE federation_grants SET revoked = 1 WHERE id = ?1 AND user_id = ?2 AND revoked = 0",
                params![id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn federation_list_tokens(
        &self,
        user_id: i64,
    ) -> anyhow::Result<Vec<FederationToken>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, user_id, name, prefix, created_at, expires_at, revoked, last_used_at \
                 FROM federation_tokens WHERE user_id = ?1 ORDER BY created_at DESC, id DESC",
            )?;
            let rows = statement.query_map(params![user_id], token_from_row)?;
            rows.collect()
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn federation_create_token(
        &self,
        user_id: i64,
        name: &str,
        token_hash: &str,
        prefix: &str,
        expires_at: Option<i64>,
        grant_ids: &[i64],
    ) -> anyhow::Result<i64> {
        let name = name.to_owned();
        let token_hash = token_hash.to_owned();
        let prefix = prefix.to_owned();
        let grant_ids = grant_ids.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO federation_tokens \
                 (user_id, name, token_hash, prefix, created_at, expires_at, revoked) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![user_id, name, token_hash, prefix, unix_now(), expires_at],
            )?;
            let token_id = transaction.last_insert_rowid();
            for grant_id in grant_ids {
                transaction.execute(
                    "INSERT OR IGNORE INTO federation_token_grants (token_id, grant_id) \
                     VALUES (?1, ?2)",
                    params![token_id, grant_id],
                )?;
            }
            transaction.commit()?;
            Ok(token_id)
        })
        .await
    }

    pub async fn federation_revoke_token(&self, id: i64, user_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE federation_tokens SET revoked = 1 WHERE id = ?1 AND user_id = ?2 AND revoked = 0",
                params![id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn federation_update_token_hash(
        &self,
        id: i64,
        user_id: i64,
        token_hash: &str,
        prefix: &str,
    ) -> anyhow::Result<bool> {
        let token_hash = token_hash.to_owned();
        let prefix = prefix.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE federation_tokens SET token_hash = ?1, prefix = ?2 \
                 WHERE id = ?3 AND user_id = ?4 AND revoked = 0",
                params![token_hash, prefix, id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn federation_token_grant_ids(&self, token_id: i64) -> anyhow::Result<Vec<i64>> {
        self.call(move |connection| {
            let mut statement = connection
                .prepare("SELECT grant_id FROM federation_token_grants WHERE token_id = ?1 ORDER BY grant_id")?;
            let rows = statement.query_map(params![token_id], |row| row.get(0))?;
            rows.collect()
        })
        .await
    }

    pub async fn federation_count_grants(&self, user_id: i64) -> anyhow::Result<i64> {
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM federation_grants WHERE user_id = ?1 AND revoked = 0",
                params![user_id],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn federation_count_tokens(&self, user_id: i64) -> anyhow::Result<i64> {
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM federation_tokens WHERE user_id = ?1 AND revoked = 0",
                params![user_id],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn federation_grant_token_count(&self, grant_id: i64) -> anyhow::Result<i64> {
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM federation_token_grants tg \
                 JOIN federation_tokens t ON t.id = tg.token_id \
                 WHERE tg.grant_id = ?1 AND t.revoked = 0",
                params![grant_id],
                |row| row.get(0),
            )
        })
        .await
    }

    pub async fn federation_touch_token(&self, id: i64) -> anyhow::Result<()> {
        self.call(move |connection| {
            connection.execute(
                "UPDATE federation_tokens SET last_used_at = ?1 WHERE id = ?2",
                params![unix_now(), id],
            )?;
            Ok(())
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn federation_log(
        &self,
        user_id: i64,
        token_id: i64,
        grant_id: Option<i64>,
        op: &str,
        path: Option<&str>,
        status: i64,
        bytes: i64,
        duration_ms: i64,
        ip: Option<&str>,
    ) -> anyhow::Result<()> {
        let op = op.to_owned();
        let path = path.map(str::to_owned);
        let ip = ip.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO federation_op_log \
                 (user_id, token_id, grant_id, op, path, status, bytes, duration_ms, ip, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    user_id,
                    token_id,
                    grant_id,
                    op,
                    path,
                    status,
                    bytes,
                    duration_ms,
                    ip,
                    unix_now()
                ],
            )?;
            Ok(())
        })
        .await
    }
}

#[derive(Debug, Clone)]
pub struct FederationMount {
    pub id: i64,
    pub user_id: i64,
    pub root_id: i64,
    pub base_url: String,
    pub grant_id: i64,
    pub grant_label: String,
    pub mode: String,
    pub owner_name: String,
    pub avatar_version: i64,
    pub peer_version: String,
    pub capabilities: String,
    pub token_ciphertext: String,
    pub created_at: i64,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_error_code: Option<String>,
    pub peer_instance_id: String,
}

#[derive(Debug, Clone)]
pub struct FederationMountEndpoint {
    pub id: i64,
    pub mount_id: i64,
    pub url: String,
    pub sort_order: i64,
    pub state: String,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_error_code: Option<String>,
}

fn mount_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FederationMount> {
    Ok(FederationMount {
        id: row.get(0)?,
        user_id: row.get(1)?,
        root_id: row.get(2)?,
        base_url: row.get(3)?,
        grant_id: row.get(4)?,
        grant_label: row.get(5)?,
        mode: row.get(6)?,
        owner_name: row.get(14)?,
        avatar_version: row.get(15)?,
        peer_version: row.get(7)?,
        capabilities: row.get(8)?,
        token_ciphertext: row.get(9)?,
        created_at: row.get(10)?,
        last_ok_at: row.get(11)?,
        last_error: row.get(12)?,
        last_error_code: row.get(13)?,
        peer_instance_id: row.get(16)?,
    })
}

impl Db {
    pub async fn federation_mount_key(&self) -> anyhow::Result<String> {
        self.call(|connection| {
            let existing: Option<String> = connection
                .query_row(
                    "SELECT value FROM server_secrets WHERE name = 'federation.mount_key'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(value) = existing {
                return Ok(value);
            }
            let value = crate::auth::random_token(32);
            connection.execute(
                "INSERT INTO server_secrets (name, value) VALUES ('federation.mount_key', ?1)",
                params![value],
            )?;
            Ok(value)
        })
        .await
    }

    pub async fn federation_add_remote_root(
        &self,
        user_id: i64,
        alias: &str,
        real_path: &str,
        read_only: bool,
    ) -> anyhow::Result<i64> {
        let alias = alias.to_owned();
        let real_path = real_path.to_owned();
        self.call(move |connection| {
            let next: i64 = connection.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM user_roots WHERE user_id = ?1",
                params![user_id],
                |row| row.get(0),
            )?;
            connection.execute(
                "INSERT INTO user_roots \
                 (user_id, real_path, alias, read_only, sort_order, created_at, kind, mount_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'remote', NULL)",
                params![
                    user_id,
                    real_path,
                    alias,
                    read_only as i64,
                    next,
                    unix_now()
                ],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn federation_bind_root(&self, root_id: i64, mount_id: i64) -> anyhow::Result<()> {
        self.call(move |connection| {
            connection.execute(
                "UPDATE user_roots SET mount_id = ?1 WHERE id = ?2",
                params![mount_id, root_id],
            )?;
            Ok(())
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn federation_create_mount(
        &self,
        user_id: i64,
        root_id: i64,
        base_url: &str,
        grant_id: i64,
        grant_label: &str,
        mode: &str,
        peer_version: &str,
        capabilities: &str,
        token_ciphertext: &str,
        owner_name: &str,
        avatar_version: i64,
        peer_instance_id: &str,
    ) -> anyhow::Result<i64> {
        let base_url = base_url.to_owned();
        let grant_label = grant_label.to_owned();
        let mode = mode.to_owned();
        let peer_version = peer_version.to_owned();
        let capabilities = capabilities.to_owned();
        let token_ciphertext = token_ciphertext.to_owned();
        let owner_name = owner_name.to_owned();
        let peer_instance_id = peer_instance_id.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO federation_mounts \
                 (user_id, root_id, base_url, grant_id, grant_label, mode, peer_version, \
                  capabilities, token_ciphertext, created_at, owner_name, avatar_version, \
                  peer_instance_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    user_id,
                    root_id,
                    base_url,
                    grant_id,
                    grant_label,
                    mode,
                    peer_version,
                    capabilities,
                    token_ciphertext,
                    unix_now(),
                    owner_name,
                    avatar_version,
                    peer_instance_id
                ],
            )?;
            Ok(connection.last_insert_rowid())
        })
        .await
    }

    pub async fn federation_list_endpoints(
        &self,
        mount_id: i64,
    ) -> anyhow::Result<Vec<FederationMountEndpoint>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, mount_id, url, sort_order, state, last_ok_at, last_error, \
                        last_error_code \
                 FROM federation_mount_endpoints WHERE mount_id = ?1 ORDER BY sort_order, id",
            )?;
            let rows = statement.query_map(params![mount_id], |row| {
                Ok(FederationMountEndpoint {
                    id: row.get(0)?,
                    mount_id: row.get(1)?,
                    url: row.get(2)?,
                    sort_order: row.get(3)?,
                    state: row.get(4)?,
                    last_ok_at: row.get(5)?,
                    last_error: row.get(6)?,
                    last_error_code: row.get(7)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// Replace the ordered endpoint list while keeping health state for URLs
    /// that were already present.
    pub async fn federation_replace_endpoints(
        &self,
        mount_id: i64,
        urls: &[String],
    ) -> anyhow::Result<()> {
        let urls = urls.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let mut existing = std::collections::HashMap::new();
            {
                let mut statement = transaction.prepare(
                    "SELECT url, state, last_ok_at, last_error, last_error_code \
                     FROM federation_mount_endpoints WHERE mount_id = ?1",
                )?;
                let rows = statement.query_map(params![mount_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        (
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ),
                    ))
                })?;
                for row in rows {
                    let (url, state) = row?;
                    existing.insert(url, state);
                }
            }
            transaction.execute(
                "DELETE FROM federation_mount_endpoints WHERE mount_id = ?1",
                params![mount_id],
            )?;
            let now = unix_now();
            for (index, url) in urls.iter().enumerate() {
                let (state, last_ok_at, last_error, last_error_code) = existing
                    .get(url)
                    .cloned()
                    .unwrap_or_else(|| ("unknown".to_owned(), None, None, None));
                transaction.execute(
                    "INSERT INTO federation_mount_endpoints \
                     (mount_id, url, sort_order, state, last_ok_at, last_error, \
                      last_error_code, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        mount_id,
                        url,
                        index as i64,
                        state,
                        last_ok_at,
                        last_error,
                        last_error_code,
                        now
                    ],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn federation_update_endpoint_state(
        &self,
        mount_id: i64,
        url: &str,
        state: &str,
        last_ok: bool,
        error: Option<&str>,
        error_code: Option<&str>,
    ) -> anyhow::Result<()> {
        let url = url.to_owned();
        let state = state.to_owned();
        let error = error.map(str::to_owned);
        let error_code = error_code.map(str::to_owned);
        self.call(move |connection| {
            if last_ok {
                connection.execute(
                    "UPDATE federation_mount_endpoints \
                     SET state = ?1, last_ok_at = ?2, last_error = NULL, last_error_code = NULL \
                     WHERE mount_id = ?3 AND url = ?4",
                    params![state, unix_now(), mount_id, url],
                )?;
            } else {
                connection.execute(
                    "UPDATE federation_mount_endpoints \
                     SET state = ?1, last_error = ?2, last_error_code = ?3 \
                     WHERE mount_id = ?4 AND url = ?5",
                    params![state, error, error_code, mount_id, url],
                )?;
            }
            Ok(())
        })
        .await
    }

    pub async fn federation_set_mount_active_url(
        &self,
        id: i64,
        user_id: i64,
        url: &str,
    ) -> anyhow::Result<bool> {
        let url = url.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE federation_mounts SET base_url = ?1 WHERE id = ?2 AND user_id = ?3",
                params![url, id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn federation_set_mount_peer_instance(
        &self,
        id: i64,
        instance_id: &str,
    ) -> anyhow::Result<()> {
        let instance_id = instance_id.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE federation_mounts SET peer_instance_id = ?1 WHERE id = ?2",
                params![instance_id, id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn federation_find_mount_by_peer(
        &self,
        user_id: i64,
        peer_instance_id: &str,
        grant_id: i64,
    ) -> anyhow::Result<Option<FederationMount>> {
        let peer_instance_id = peer_instance_id.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, root_id, base_url, grant_id, grant_label, mode, \
                            peer_version, capabilities, token_ciphertext, created_at, \
                            last_ok_at, last_error, last_error_code, owner_name, avatar_version, \
                            peer_instance_id \
                     FROM federation_mounts \
                     WHERE user_id = ?1 AND peer_instance_id = ?2 AND grant_id = ?3 \
                     ORDER BY id LIMIT 1",
                    params![user_id, peer_instance_id, grant_id],
                    mount_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn federation_list_mounts(
        &self,
        user_id: i64,
    ) -> anyhow::Result<Vec<FederationMount>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT m.id, m.user_id, m.root_id, m.base_url, m.grant_id, m.grant_label, m.mode, \
                        m.peer_version, m.capabilities, m.token_ciphertext, m.created_at, \
                        m.last_ok_at, m.last_error, m.last_error_code, \
                        m.owner_name, m.avatar_version, m.peer_instance_id \
                 FROM federation_mounts m JOIN user_roots r ON r.id = m.root_id \
                 WHERE m.user_id = ?1 AND r.kind = 'remote' ORDER BY r.sort_order, m.id",
            )?;
            let rows = statement.query_map(params![user_id], mount_from_row)?;
            rows.collect()
        })
        .await
    }

    pub async fn federation_all_mounts(&self) -> anyhow::Result<Vec<FederationMount>> {
        self.call(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, user_id, root_id, base_url, grant_id, grant_label, mode, \
                        peer_version, capabilities, token_ciphertext, created_at, \
                        last_ok_at, last_error, last_error_code, owner_name, avatar_version, \
                        peer_instance_id \
                 FROM federation_mounts ORDER BY id",
            )?;
            let rows = statement.query_map([], mount_from_row)?;
            rows.collect()
        })
        .await
    }

    pub async fn federation_mount_by_root(
        &self,
        root_id: i64,
        user_id: i64,
    ) -> anyhow::Result<Option<FederationMount>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, root_id, base_url, grant_id, grant_label, mode, \
                            peer_version, capabilities, token_ciphertext, created_at, \
                            last_ok_at, last_error, last_error_code, owner_name, avatar_version, \
                            peer_instance_id \
                     FROM federation_mounts WHERE root_id = ?1 AND user_id = ?2",
                    params![root_id, user_id],
                    mount_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn federation_mount(
        &self,
        id: i64,
        user_id: i64,
    ) -> anyhow::Result<Option<FederationMount>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, root_id, base_url, grant_id, grant_label, mode, \
                            peer_version, capabilities, token_ciphertext, created_at, \
                            last_ok_at, last_error, last_error_code, owner_name, avatar_version, \
                            peer_instance_id \
                     FROM federation_mounts WHERE id = ?1 AND user_id = ?2",
                    params![id, user_id],
                    mount_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn federation_mount_any(&self, id: i64) -> anyhow::Result<Option<FederationMount>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT id, user_id, root_id, base_url, grant_id, grant_label, mode, \
                            peer_version, capabilities, token_ciphertext, created_at, \
                            last_ok_at, last_error, last_error_code, owner_name, avatar_version, \
                            peer_instance_id \
                     FROM federation_mounts WHERE id = ?1",
                    params![id],
                    mount_from_row,
                )
                .optional()
        })
        .await
    }

    pub async fn federation_delete_mount(&self, id: i64, user_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let root_id: Option<i64> = connection
                .query_row(
                    "SELECT root_id FROM federation_mounts WHERE id = ?1 AND user_id = ?2",
                    params![id, user_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(root_id) = root_id else {
                return Ok(false);
            };
            connection.execute(
                "DELETE FROM user_roots WHERE id = ?1 AND user_id = ?2 AND kind = 'remote'",
                params![root_id, user_id],
            )?;
            Ok(true)
        })
        .await
    }

    pub async fn federation_update_mount_token(
        &self,
        id: i64,
        user_id: i64,
        token_ciphertext: &str,
    ) -> anyhow::Result<bool> {
        let token_ciphertext = token_ciphertext.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE federation_mounts SET token_ciphertext = ?1, last_error = NULL \
                 WHERE id = ?2 AND user_id = ?3",
                params![token_ciphertext, id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn federation_update_mount_capabilities(
        &self,
        id: i64,
        user_id: i64,
        capabilities: &str,
    ) -> anyhow::Result<bool> {
        let capabilities = capabilities.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE federation_mounts SET capabilities = ?1 WHERE id = ?2 AND user_id = ?3",
                params![capabilities, id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn federation_update_mount_state(
        &self,
        id: i64,
        ok: bool,
        error: Option<&str>,
        error_code: Option<&str>,
    ) -> anyhow::Result<()> {
        let error = error.map(str::to_owned);
        let error_code = error_code.map(str::to_owned);
        self.call(move |connection| {
            if ok {
                connection.execute(
                    "UPDATE federation_mounts SET last_ok_at = ?1, last_error = NULL, \
                     last_error_code = NULL WHERE id = ?2",
                    params![unix_now(), id],
                )?;
            } else {
                connection.execute(
                    "UPDATE federation_mounts SET last_error = ?1, last_error_code = ?2 \
                     WHERE id = ?3",
                    params![error, error_code, id],
                )?;
            }
            Ok(())
        })
        .await
    }

    pub async fn federation_count_mounts(&self, user_id: i64) -> anyhow::Result<i64> {
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM federation_mounts WHERE user_id = ?1",
                params![user_id],
                |row| row.get(0),
            )
        })
        .await
    }
}

#[derive(Debug, Clone)]
pub struct FederationSearchHit {
    pub name: String,
    pub real_path: String,
    pub is_dir: bool,
    pub size: i64,
    pub mtime: i64,
}

impl Db {
    pub async fn federation_search(
        &self,
        base: &str,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<Vec<FederationSearchHit>> {
        let base = base.trim_end_matches('/').to_owned();
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT name, real_path, is_dir, size, mtime FROM index_entries \
                 WHERE name LIKE ?1 ESCAPE '\\' \
                   AND (real_path = ?2 OR real_path LIKE ?2 || '/%') \
                 ORDER BY name COLLATE NOCASE LIMIT ?3 OFFSET ?4",
            )?;
            let rows = statement.query_map(
                params![pattern, base, limit as i64, offset as i64],
                |row| {
                    Ok(FederationSearchHit {
                        name: row.get(0)?,
                        real_path: row.get(1)?,
                        is_dir: row.get::<_, i64>(2)? != 0,
                        size: row.get(3)?,
                        mtime: row.get(4)?,
                    })
                },
            )?;
            rows.collect()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn grants_and_tokens_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("federation.db"), 2).unwrap();
        db.call(|connection| {
            connection.execute(
                "INSERT INTO users (username, password_hash, role, home_dir, created_at, updated_at) \
                 VALUES ('u', 'x', 'admin', '/tmp', 0, 0)",
                [],
            )?;
            connection.execute(
                "INSERT INTO user_roots \
                 (user_id, real_path, alias, read_only, sort_order, created_at, kind) \
                 VALUES (1, '/tmp', 'root', 0, 0, 0, 'local')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let grant = db
            .federation_create_grant(1, 1, "/tmp", "Test", "rw")
            .await
            .unwrap();
        let token = db
            .federation_create_token(1, "t", "hash", "fhfed_abc", None, &[grant])
            .await
            .unwrap();
        let found = db
            .federation_find_token("hash")
            .await
            .unwrap()
            .expect("token exists");
        assert_eq!(found.id, token);
        assert_eq!(found.user_id, 1);
        let grants = db.federation_token_grants(token).await.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].id, grant);
        assert_eq!(grants[0].mode, "rw");

        assert!(db.federation_revoke_grant(grant, 1).await.unwrap());
        assert!(db.federation_token_grants(token).await.unwrap().is_empty());
        assert!(db.federation_revoke_token(token, 1).await.unwrap());
        assert!(
            db.federation_find_token("hash")
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
    }
}

#[derive(Debug, Clone)]
pub struct FederationActivity {
    pub op: String,
    pub path: Option<String>,
    pub status: i64,
    pub bytes: i64,
    pub created_at: i64,
}

impl Db {
    pub async fn federation_activity(
        &self,
        user_id: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<FederationActivity>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT op, path, status, bytes, created_at FROM federation_op_log \
                 WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![user_id, limit as i64], |row| {
                Ok(FederationActivity {
                    op: row.get(0)?,
                    path: row.get(1)?,
                    status: row.get(2)?,
                    bytes: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?;
            rows.collect()
        })
        .await
    }
}

impl Db {
    pub async fn federation_grant_revoked(&self, id: i64, user_id: i64) -> anyhow::Result<bool> {
        self.call(move |connection| {
            let revoked = connection
                .query_row(
                    "SELECT revoked FROM federation_grants WHERE id = ?1 AND user_id = ?2",
                    params![id, user_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            Ok(revoked == Some(1))
        })
        .await
    }

    pub async fn federation_update_mount_error_code(
        &self,
        id: i64,
        code: Option<&str>,
    ) -> anyhow::Result<()> {
        let code = code.map(str::to_owned);
        self.call(move |connection| {
            connection.execute(
                "UPDATE federation_mounts SET last_error_code = ?1 WHERE id = ?2",
                params![code, id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn federation_update_mount_mode(
        &self,
        id: i64,
        root_id: i64,
        mode: &str,
        read_only: bool,
    ) -> anyhow::Result<()> {
        let mode = mode.to_owned();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "UPDATE federation_mounts SET mode = ?1 WHERE id = ?2",
                params![mode, id],
            )?;
            transaction.execute(
                "UPDATE user_roots SET read_only = ?1 WHERE id = ?2",
                params![read_only as i64, root_id],
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn federation_update_mount_owner(
        &self,
        id: i64,
        owner_name: &str,
        avatar_version: i64,
    ) -> anyhow::Result<()> {
        let owner_name = owner_name.to_owned();
        self.call(move |connection| {
            connection.execute(
                "UPDATE federation_mounts SET owner_name = ?1, avatar_version = ?2 WHERE id = ?3",
                params![owner_name, avatar_version, id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn federation_update_mount_alias(
        &self,
        id: i64,
        user_id: i64,
        alias: &str,
    ) -> anyhow::Result<bool> {
        let alias = alias.to_owned();
        self.call(move |connection| {
            let changed = connection.execute(
                "UPDATE user_roots SET alias = ?1 \
                 WHERE id = (SELECT root_id FROM federation_mounts WHERE id = ?2 AND user_id = ?3) \
                 AND user_id = ?3",
                params![alias, id, user_id],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    pub async fn federation_prune_op_log(
        &self,
        retention_days: u64,
        max_rows: u64,
    ) -> anyhow::Result<usize> {
        self.call(move |connection| {
            let cutoff = unix_now().saturating_sub((retention_days as i64).saturating_mul(86_400));
            let removed = connection.execute(
                "DELETE FROM federation_op_log WHERE id IN (\
                   SELECT id FROM federation_op_log WHERE created_at < ?1 LIMIT ?2)",
                params![cutoff, max_rows as i64],
            )?;
            Ok(removed)
        })
        .await
    }
}

impl Db {
    pub async fn federation_update_token_grants(
        &self,
        id: i64,
        user_id: i64,
        grant_ids: &[i64],
    ) -> anyhow::Result<bool> {
        let grant_ids = grant_ids.to_vec();
        self.call(move |connection| {
            let owned: Option<i64> = connection
                .query_row(
                    "SELECT id FROM federation_tokens \
                     WHERE id = ?1 AND user_id = ?2 AND revoked = 0",
                    params![id, user_id],
                    |row| row.get(0),
                )
                .optional()?;
            if owned.is_none() {
                return Ok(false);
            }
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM federation_token_grants WHERE token_id = ?1",
                params![id],
            )?;
            for grant_id in grant_ids {
                let valid: Option<i64> = transaction
                    .query_row(
                        "SELECT id FROM federation_grants \
                         WHERE id = ?1 AND user_id = ?2 AND revoked = 0",
                        params![grant_id, user_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if valid.is_some() {
                    transaction.execute(
                        "INSERT OR IGNORE INTO federation_token_grants (token_id, grant_id) \
                         VALUES (?1, ?2)",
                        params![id, grant_id],
                    )?;
                }
            }
            transaction.commit()?;
            Ok(true)
        })
        .await
    }
}
