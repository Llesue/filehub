-- Drop explicit indexes that duplicate UNIQUE constraint indexes.
DROP INDEX IF EXISTS idx_index_entries_path;
DROP INDEX IF EXISTS idx_index_entries_dir;
