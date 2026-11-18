-- M26: short CJK substring search.
CREATE TABLE index_name_bigrams (
  entry_id INTEGER NOT NULL REFERENCES index_entries(id) ON DELETE CASCADE,
  gram     TEXT NOT NULL,
  PRIMARY KEY (entry_id, gram)
) WITHOUT ROWID;

CREATE INDEX idx_name_bigrams_gram
  ON index_name_bigrams(gram, entry_id);
