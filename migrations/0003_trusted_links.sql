-- M3: admin-managed trusted symlinks

CREATE TABLE trusted_links (
  id           INTEGER PRIMARY KEY,
  link_path    TEXT NOT NULL UNIQUE,
  target_path  TEXT NOT NULL,
  link_dev     INTEGER NOT NULL,
  link_ino     INTEGER NOT NULL,
  link_mtime   INTEGER NOT NULL,
  target_dev   INTEGER NOT NULL,
  target_ino   INTEGER NOT NULL,
  access       TEXT NOT NULL CHECK (access IN ('inherit', 'read_only', 'read_write')),
  created_at   INTEGER NOT NULL
);
