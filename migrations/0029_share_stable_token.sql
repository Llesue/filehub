-- Stable, derivable share links: nonce + server secret.
ALTER TABLE shares ADD COLUMN token_nonce INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS server_secrets (
  name  TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
