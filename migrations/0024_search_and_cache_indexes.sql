-- M24: index short-name searches and directory-size maintenance.
CREATE INDEX IF NOT EXISTS idx_index_entries_name_nocase
  ON index_entries(name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_directory_sizes_updated
  ON directory_sizes(updated_at);
