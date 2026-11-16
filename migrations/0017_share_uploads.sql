-- M17: chunked uploads for writable public shares

CREATE TABLE share_upload_sessions (
  id           TEXT PRIMARY KEY,
  share_id     INTEGER NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
  dir_path     TEXT NOT NULL,
  filename     TEXT NOT NULL,
  total_size   INTEGER NOT NULL,
  chunk_size   INTEGER NOT NULL,
  received     INTEGER NOT NULL DEFAULT 0,
  chunk_bitmap BLOB NOT NULL,
  temp_path    TEXT NOT NULL,
  expires_at   INTEGER NOT NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);

CREATE INDEX idx_share_upload_sessions_share ON share_upload_sessions(share_id, expires_at);
