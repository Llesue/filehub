ALTER TABLE directory_sizes ADD COLUMN has_value INTEGER NOT NULL DEFAULT 0;
UPDATE directory_sizes SET has_value = 1 WHERE status = 'ready' OR size_bytes > 0;
