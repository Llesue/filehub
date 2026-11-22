-- A mount can have several ordered endpoints (LAN first, WAN fallback).
-- federation_mounts.base_url remains the currently active URL for
-- compatibility with older releases.
ALTER TABLE federation_mounts ADD COLUMN peer_instance_id TEXT NOT NULL DEFAULT '';

CREATE TABLE federation_mount_endpoints (
  id              INTEGER PRIMARY KEY,
  mount_id        INTEGER NOT NULL REFERENCES federation_mounts(id) ON DELETE CASCADE,
  url             TEXT NOT NULL,
  sort_order      INTEGER NOT NULL,
  state           TEXT NOT NULL DEFAULT 'unknown',
  last_ok_at      INTEGER,
  last_error      TEXT,
  last_error_code TEXT,
  created_at      INTEGER NOT NULL,
  UNIQUE (mount_id, url)
);

CREATE INDEX idx_federation_mount_endpoints
  ON federation_mount_endpoints(mount_id, sort_order);

INSERT INTO federation_mount_endpoints
  (mount_id, url, sort_order, state, last_ok_at, last_error, last_error_code, created_at)
SELECT id, base_url, 0,
       CASE
         WHEN last_error IS NULL THEN 'online'
         WHEN last_error_code IN ('TOKEN_REVOKED', 'TOKEN_EXPIRED') THEN 'token_invalid'
         WHEN last_error_code = 'GRANT_REVOKED' THEN 'grant_revoked'
         WHEN last_error_code = 'REMOTE_ROOT_UNSUPPORTED' THEN 'unsupported'
         ELSE 'offline'
       END,
       last_ok_at, last_error, last_error_code, created_at
FROM federation_mounts;
