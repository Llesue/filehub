-- M4: directory index and full-text search

CREATE TABLE indexed_dirs (
  id          INTEGER PRIMARY KEY,
  real_path   TEXT NOT NULL UNIQUE,
  mtime_ns    INTEGER NOT NULL,
  status      TEXT NOT NULL CHECK (status IN ('pending', 'indexing', 'ready', 'stale', 'error')),
  error       TEXT,
  entry_count INTEGER NOT NULL DEFAULT 0,
  indexed_at  INTEGER,
  created_at  INTEGER NOT NULL
);

CREATE TABLE index_entries (
  id         INTEGER PRIMARY KEY,
  dir_id     INTEGER NOT NULL REFERENCES indexed_dirs(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  real_path  TEXT NOT NULL UNIQUE,
  is_dir     INTEGER NOT NULL,
  size       INTEGER NOT NULL,
  mtime      INTEGER NOT NULL,
  UNIQUE (dir_id, name)
);

CREATE INDEX idx_index_entries_dir ON index_entries(dir_id, name);
CREATE INDEX idx_index_entries_path ON index_entries(real_path);

CREATE VIRTUAL TABLE index_fts USING fts5(
  name,
  content='index_entries',
  content_rowid='id',
  tokenize='trigram'
);

CREATE TRIGGER index_entries_ai AFTER INSERT ON index_entries BEGIN
  INSERT INTO index_fts(rowid, name) VALUES (new.id, new.name);
END;

CREATE TRIGGER index_entries_ad AFTER DELETE ON index_entries BEGIN
  INSERT INTO index_fts(index_fts, rowid, name) VALUES ('delete', old.id, old.name);
END;

CREATE TRIGGER index_entries_au AFTER UPDATE ON index_entries BEGIN
  INSERT INTO index_fts(index_fts, rowid, name) VALUES ('delete', old.id, old.name);
  INSERT INTO index_fts(rowid, name) VALUES (new.id, new.name);
END;
