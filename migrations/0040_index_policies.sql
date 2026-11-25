-- 0040: per-path index policy

CREATE TABLE IF NOT EXISTS index_policies (
  real_path  TEXT PRIMARY KEY,
  policy     TEXT NOT NULL CHECK (policy IN ('normal', 'background', 'on_demand', 'ignore')),
  updated_by INTEGER,
  updated_at INTEGER NOT NULL
);

-- Denormalized policy on indexed_dirs lets quick scan skip background dirs
-- without walking every row. Policy changes update the subtree in batches.
ALTER TABLE indexed_dirs ADD COLUMN policy TEXT NOT NULL DEFAULT 'normal';

CREATE INDEX IF NOT EXISTS idx_index_policies_updated
  ON index_policies(updated_at);
CREATE INDEX IF NOT EXISTS idx_indexed_dirs_policy
  ON indexed_dirs(policy, last_checked_at);
