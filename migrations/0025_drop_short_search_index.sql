-- M25: the short-query prefix index changed search semantics and is not used
-- by substring queries. Keep search behavior consistent with >=3-char FTS.
DROP INDEX IF EXISTS idx_index_entries_name_nocase;
