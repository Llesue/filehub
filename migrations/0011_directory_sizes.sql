CREATE TABLE directory_sizes (
  real_path  TEXT PRIMARY KEY,
  size_bytes INTEGER NOT NULL DEFAULT 0,
  mtime_ns   INTEGER NOT NULL DEFAULT 0,
  status     TEXT NOT NULL DEFAULT 'stale',
  error      TEXT,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_directory_sizes_status ON directory_sizes(status, updated_at);
