-- M16: persist new-directory candidates that a quick scan could not queue yet

CREATE TABLE pending_index_dirs (
  real_path  TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL
);
