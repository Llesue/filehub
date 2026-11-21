-- M33: federation protocol hardening

-- Peer upload sessions are bound to the token that created them.
ALTER TABLE upload_sessions ADD COLUMN token_id INTEGER;

-- Reusing an idempotency key with a different request body must conflict.
ALTER TABLE federation_idempotency ADD COLUMN body_hash TEXT NOT NULL DEFAULT '';

-- Keep the machine-readable peer error so the mount list can show a precise
-- state (token invalid, grant revoked, unsupported) instead of "offline".
ALTER TABLE federation_mounts ADD COLUMN last_error_code TEXT;
