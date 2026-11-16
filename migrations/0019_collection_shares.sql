-- M19: collection shares (one token, many files)

ALTER TABLE shares ADD COLUMN kind TEXT NOT NULL DEFAULT 'single';

CREATE TABLE share_items (
  id           INTEGER PRIMARY KEY,
  share_id     INTEGER NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
  real_path    TEXT NOT NULL,
  display_name TEXT NOT NULL,
  is_dir       INTEGER NOT NULL DEFAULT 0,
  sort_order   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_share_items_share ON share_items(share_id, sort_order);
