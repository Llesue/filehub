-- M32: federation v1 completion

-- Upload preconditions: the peer checks these when the upload completes, so a
-- change that happens between the editor's stat and the final rename cannot be
-- silently overwritten.
ALTER TABLE upload_sessions ADD COLUMN if_match TEXT;
ALTER TABLE upload_sessions ADD COLUMN if_none_match TEXT;

-- The idempotency table exists since M31; M32 adds the request identity and
-- the pending/done state used to reserve a key before the operation runs.
ALTER TABLE federation_idempotency ADD COLUMN method TEXT NOT NULL DEFAULT '';
ALTER TABLE federation_idempotency ADD COLUMN path TEXT;
ALTER TABLE federation_idempotency ADD COLUMN state TEXT NOT NULL DEFAULT 'pending';
