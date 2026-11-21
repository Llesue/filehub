-- M31: federation (remote directory mounts)

-- user_roots gains a kind. The old UNIQUE (user_id, real_path) conflicts with
-- remote roots, which have no real path, so rebuild the table and scope the
-- uniqueness to local roots only.
CREATE TABLE user_roots_new (
  id         INTEGER PRIMARY KEY,
  user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  real_path  TEXT NOT NULL,
  alias      TEXT NOT NULL,
  read_only  INTEGER NOT NULL DEFAULT 0,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  kind       TEXT NOT NULL DEFAULT 'local' CHECK (kind IN ('local', 'remote')),
  mount_id   INTEGER
);

INSERT INTO user_roots_new
  (id, user_id, real_path, alias, read_only, sort_order, created_at)
SELECT id, user_id, real_path, alias, read_only, sort_order, created_at
FROM user_roots;

DROP TABLE user_roots;
ALTER TABLE user_roots_new RENAME TO user_roots;

CREATE UNIQUE INDEX idx_user_roots_local_path
  ON user_roots(user_id, real_path) WHERE kind = 'local';
CREATE INDEX idx_user_roots_user
  ON user_roots(user_id, sort_order, id);
CREATE UNIQUE INDEX idx_user_roots_mount
  ON user_roots(mount_id) WHERE mount_id IS NOT NULL;

-- The mount side keeps a local upload session and a matching peer session.
ALTER TABLE upload_sessions ADD COLUMN remote_upload_id TEXT;

CREATE TABLE federation_mounts (
  id               INTEGER PRIMARY KEY,
  user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  root_id          INTEGER NOT NULL REFERENCES user_roots(id) ON DELETE CASCADE,
  base_url         TEXT NOT NULL,
  grant_id         INTEGER NOT NULL,
  grant_label      TEXT NOT NULL,
  mode             TEXT NOT NULL CHECK (mode IN ('ro', 'rw')),
  peer_version     TEXT NOT NULL,
  capabilities     TEXT NOT NULL,
  token_ciphertext TEXT NOT NULL,
  created_at       INTEGER NOT NULL,
  last_ok_at       INTEGER,
  last_error       TEXT,
  UNIQUE (user_id, base_url, grant_id)
);

CREATE TABLE federation_grants (
  id         INTEGER PRIMARY KEY,
  user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  root_id    INTEGER NOT NULL REFERENCES user_roots(id) ON DELETE CASCADE,
  real_path  TEXT NOT NULL,
  label      TEXT NOT NULL,
  mode       TEXT NOT NULL DEFAULT 'ro' CHECK (mode IN ('ro', 'rw')),
  revoked    INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE INDEX idx_federation_grants_user
  ON federation_grants(user_id, created_at);
CREATE UNIQUE INDEX idx_federation_grants_active
  ON federation_grants(user_id, real_path) WHERE revoked = 0;

CREATE TABLE federation_tokens (
  id           INTEGER PRIMARY KEY,
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         TEXT NOT NULL,
  token_hash   TEXT NOT NULL UNIQUE,
  prefix       TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER,
  revoked      INTEGER NOT NULL DEFAULT 0,
  last_used_at INTEGER
);

CREATE TABLE federation_token_grants (
  token_id INTEGER NOT NULL REFERENCES federation_tokens(id) ON DELETE CASCADE,
  grant_id INTEGER NOT NULL REFERENCES federation_grants(id) ON DELETE CASCADE,
  PRIMARY KEY (token_id, grant_id)
);

CREATE INDEX idx_federation_token_grants_grant
  ON federation_token_grants(grant_id);

CREATE TABLE federation_op_log (
  id          INTEGER PRIMARY KEY,
  user_id     INTEGER,
  token_id    INTEGER,
  grant_id    INTEGER,
  op          TEXT NOT NULL,
  path        TEXT,
  status      INTEGER NOT NULL,
  bytes       INTEGER NOT NULL DEFAULT 0,
  duration_ms INTEGER NOT NULL DEFAULT 0,
  ip          TEXT,
  created_at  INTEGER NOT NULL
);

CREATE INDEX idx_federation_op_log_user
  ON federation_op_log(user_id, created_at);
CREATE INDEX idx_federation_op_log_created
  ON federation_op_log(created_at);

CREATE TABLE federation_idempotency (
  token_id   INTEGER NOT NULL,
  key        TEXT NOT NULL,
  status     INTEGER NOT NULL,
  body       BLOB NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (token_id, key)
);

CREATE INDEX idx_federation_idempotency_created
  ON federation_idempotency(created_at);

ALTER TABLE users ADD COLUMN feature_federation_mount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN feature_federation_share INTEGER NOT NULL DEFAULT 0;
