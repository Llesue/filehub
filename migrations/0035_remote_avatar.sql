-- M35: remote owner avatar metadata

ALTER TABLE federation_mounts ADD COLUMN owner_name TEXT NOT NULL DEFAULT '';
ALTER TABLE federation_mounts ADD COLUMN avatar_version INTEGER NOT NULL DEFAULT 0;
