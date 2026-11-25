-- 0039: resumable/fair index scanning state

ALTER TABLE indexed_dirs ADD COLUMN visible INTEGER NOT NULL DEFAULT 1;
ALTER TABLE indexed_dirs ADD COLUMN last_checked_at INTEGER;
ALTER TABLE indexed_dirs ADD COLUMN scan_generation INTEGER;

CREATE INDEX IF NOT EXISTS idx_indexed_dirs_checked
  ON indexed_dirs(last_checked_at, real_path);
CREATE INDEX IF NOT EXISTS idx_indexed_dirs_visible
  ON indexed_dirs(visible, status);

CREATE TABLE IF NOT EXISTS index_scan_state (
  kind             TEXT PRIMARY KEY,
  cursor           TEXT,
  pass_started_at  INTEGER,
  last_progress_at INTEGER,
  completed_at     INTEGER,
  pass_id          INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS index_scan_frontier (
  kind      TEXT NOT NULL,
  real_path TEXT NOT NULL,
  PRIMARY KEY (kind, real_path)
);

CREATE INDEX IF NOT EXISTS idx_index_scan_frontier_kind
  ON index_scan_frontier(kind, real_path);
