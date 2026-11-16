-- M12: unified background job queue (dir size, transfer, index, thumbnail)

CREATE TABLE jobs (
  id          TEXT PRIMARY KEY,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('dir_size', 'copy', 'move', 'index', 'thumbnail')),
  job_key     TEXT NOT NULL,
  status      TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
  progress    INTEGER NOT NULL DEFAULT 0,
  total       INTEGER NOT NULL DEFAULT 0,
  bytes_done  INTEGER NOT NULL DEFAULT 0,
  bytes_total INTEGER NOT NULL DEFAULT 0,
  payload     TEXT NOT NULL DEFAULT '{}',
  result      TEXT,
  error       TEXT,
  created_at  INTEGER NOT NULL,
  started_at  INTEGER,
  finished_at INTEGER
);

-- At most one queued/running job per (kind, key).
CREATE UNIQUE INDEX idx_jobs_active_key ON jobs(kind, job_key)
  WHERE status IN ('queued', 'running');
CREATE INDEX idx_jobs_user_created ON jobs(user_id, created_at DESC);
CREATE INDEX idx_jobs_status_created ON jobs(status, created_at);
