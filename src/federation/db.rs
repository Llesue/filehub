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
    pub peer_version: String,
    pub capabilities: String,
    pub token_ciphertext: String,
    pub created_at: i64,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
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
        peer_version: row.get(7)?,
        capabilities: row.get(8)?,
        token_ciphertext: row.get(9)?,
        created_at: row.get(10)?,
        last_ok_at: row.get(11)?,
        last_error: row.get(12)?,
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
    ) -> anyhow::Result<i64> {
        let base_url = base_url.to_owned();
        let grant_label = grant_label.to_owned();
        let mode = mode.to_owned();
        let peer_version = peer_version.to_owned();
        let capabilities = capabilities.to_owned();
        let token_ciphertext = token_ciphertext.to_owned();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO federation_mounts \
                 (user_id, root_id, base_url, grant_id, grant_label, mode, peer_version, \
                  capabilities, token_ciphertext, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                    unix_now()
                ],
            )?;
            Ok(connection.last_insert_rowid())
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
                        m.last_ok_at, m.last_error \
                 FROM federation_mounts m JOIN user_roots r ON r.id = m.root_id \
                 WHERE m.user_id = ?1 AND r.kind = 'remote' ORDER BY r.sort_order, m.id",
            )?;
            let rows = statement.query_map(params![user_id], mount_from_row)?;
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
                            last_ok_at, last_error \
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
                            last_ok_at, last_error \
                     FROM federation_mounts WHERE id = ?1 AND user_id = ?2",
                    params![id, user_id],
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

    pub async fn federation_update_mount_state(
        &self,
        id: i64,
        ok: bool,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let error = error.map(str::to_owned);
        self.call(move |connection| {
            if ok {
                connection.execute(
                    "UPDATE federation_mounts SET last_ok_at = ?1, last_error = NULL WHERE id = ?2",
                    params![unix_now(), id],
                )?;
            } else {
                connection.execute(
                    "UPDATE federation_mounts SET last_error = ?1 WHERE id = ?2",
                    params![error, id],
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
