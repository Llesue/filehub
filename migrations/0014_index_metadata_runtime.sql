-- M14: richer index metadata and small runtime key/value store

ALTER TABLE index_entries ADD COLUMN mime_type TEXT NOT NULL DEFAULT '';
ALTER TABLE index_entries ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE index_entries ADD COLUMN has_preview INTEGER NOT NULL DEFAULT 0;
ALTER TABLE index_entries ADD COLUMN is_hidden INTEGER NOT NULL DEFAULT 0;

-- Existing rows predate mime/preview/created metadata; let the next quick scan
-- rebuild each directory once instead of serving incomplete search filters.
UPDATE indexed_dirs SET status = 'stale';

CREATE INDEX idx_index_entries_size ON index_entries(size);
CREATE INDEX idx_index_entries_mtime ON index_entries(mtime);

CREATE TABLE runtime_state (
  key        TEXT PRIMARY KEY,
  value      INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
