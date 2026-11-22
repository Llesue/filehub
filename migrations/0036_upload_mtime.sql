-- Preserve the source file's modified time through resumable uploads.
-- NULL keeps the previous behavior (server time at write).
ALTER TABLE upload_sessions ADD COLUMN mtime INTEGER;
ALTER TABLE share_upload_sessions ADD COLUMN mtime INTEGER;
