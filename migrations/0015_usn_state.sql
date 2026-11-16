-- M15: Windows USN Journal incremental indexing state per volume

CREATE TABLE usn_state (
  volume     TEXT PRIMARY KEY,
  journal_id INTEGER NOT NULL,
  next_usn   INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
