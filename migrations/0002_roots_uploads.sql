-- M2: user roots and resumable upload sessions

CREATE TABLE user_roots (
  id         INTEGER PRIMARY KEY,
  user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  real_path  TEXT NOT NULL,
  alias      TEXT NOT NULL,
  read_only  INTEGER NOT NULL DEFAULT 0,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  UNIQUE (user_id, real_path)
);

CREATE INDEX idx_user_roots_user ON user_roots(user_id, sort_order, id);

CREATE TABLE upload_sessions (
  id           TEXT PRIMARY KEY,
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  root_id      INTEGER NOT NULL,
  dir_path     TEXT NOT NULL,
  filename     TEXT NOT NULL,
  total_size   INTEGER NOT NULL,
  chunk_size   INTEGER NOT NULL,
  received     INTEGER NOT NULL DEFAULT 0,
  chunk_bitmap BLOB NOT NULL,
  temp_path    TEXT NOT NULL,
  sha256       TEXT,
  expires_at   INTEGER NOT NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);

CREATE INDEX idx_upload_sessions_user ON upload_sessions(user_id, expires_at);
