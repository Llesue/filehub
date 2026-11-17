-- Soft-delete users without removing the row or their home directory.
ALTER TABLE users ADD COLUMN deleted_at INTEGER;
