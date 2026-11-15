-- M7: public shares

CREATE TABLE shares (
  id            INTEGER PRIMARY KEY,
  token_hash    TEXT NOT NULL UNIQUE,
  creator_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  real_path     TEXT NOT NULL,
  is_dir        INTEGER NOT NULL,
  password_hash TEXT,
  expires_at    INTEGER,
  read_only     INTEGER NOT NULL DEFAULT 1,
  revoked       INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL
);

CREATE INDEX idx_shares_creator ON shares(creator_id, created_at);
