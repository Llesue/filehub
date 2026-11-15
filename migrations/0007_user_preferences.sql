-- Per-user preferences and uploaded assets
CREATE TABLE user_preferences (
  user_id              INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  theme                TEXT NOT NULL DEFAULT 'light',
  language             TEXT NOT NULL DEFAULT 'zh-CN',
  show_hidden          INTEGER NOT NULL DEFAULT 1,
  show_thumbnails      INTEGER NOT NULL DEFAULT 1,
  upload_conflict      TEXT NOT NULL DEFAULT 'rename',
  custom_css_root_id   INTEGER,
  custom_css_path      TEXT,
  custom_css_version   INTEGER NOT NULL DEFAULT 0,
  avatar_path          TEXT,
  avatar_version       INTEGER NOT NULL DEFAULT 0,
  updated_at           INTEGER NOT NULL
);
