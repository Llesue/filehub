-- 0041: upload hash verification state

ALTER TABLE upload_sessions ADD COLUMN verify TEXT NOT NULL DEFAULT 'auto';
ALTER TABLE upload_sessions ADD COLUMN hash_status TEXT;
ALTER TABLE upload_sessions ADD COLUMN hash_error TEXT;
ALTER TABLE upload_sessions ADD COLUMN verified_at INTEGER;
