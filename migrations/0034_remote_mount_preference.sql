-- M34: per-user remote mount visibility

ALTER TABLE user_preferences
  ADD COLUMN remote_mount_enabled INTEGER NOT NULL DEFAULT 1;
