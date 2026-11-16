-- M18: global deployment branding (name lives in ui.brand_name, icon lives here)

CREATE TABLE branding (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  icon       BLOB NOT NULL,
  mime_type  TEXT NOT NULL,
  version    INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
