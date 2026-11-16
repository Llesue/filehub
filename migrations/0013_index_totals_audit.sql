-- M13: persist indexed subtree sizes and add audit keyset indexes

ALTER TABLE indexed_dirs ADD COLUMN total_size_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE indexed_dirs ADD COLUMN partial INTEGER NOT NULL DEFAULT 0;

-- Rows indexed before this migration have no trustworthy aggregate size.
UPDATE indexed_dirs SET partial = 1;

CREATE INDEX idx_audit_action_created ON audit_logs(action, created_at);
CREATE INDEX idx_audit_user_created ON audit_logs(user_id, created_at);
