-- 0038: content hash cache (SHA-256)

CREATE TABLE IF NOT EXISTS file_hashes (
  real_path   TEXT NOT NULL,
  algo        TEXT NOT NULL DEFAULT 'sha256',
  sha256      TEXT NOT NULL,
  size        INTEGER NOT NULL,
  mtime_ns    INTEGER NOT NULL,
  computed_at INTEGER NOT NULL,
  PRIMARY KEY (real_path, algo)
);

-- Reserved for Gate 3 hash lookup/dedupe. Keep it out of Gate 1 to avoid
-- extra write amplification on every upload.
-- CREATE INDEX IF NOT EXISTS idx_file_hashes_sha ON file_hashes(sha256, size);
