ALTER TABLE users ADD COLUMN last_login_at INTEGER;
ALTER TABLE user_preferences ADD COLUMN editor_enabled INTEGER NOT NULL DEFAULT 1;
