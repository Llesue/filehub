-- Admin-registered server paths, assignable to users as roots
CREATE TABLE server_paths (
  id         INTEGER PRIMARY KEY,
  alias      TEXT NOT NULL,
  real_path  TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL
);
