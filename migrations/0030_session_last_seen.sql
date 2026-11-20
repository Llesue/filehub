-- Track recent activity so the admin user list can show an online dot.
ALTER TABLE sessions ADD COLUMN last_seen_at INTEGER;
