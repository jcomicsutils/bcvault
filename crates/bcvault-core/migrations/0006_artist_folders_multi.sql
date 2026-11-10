CREATE TABLE artist_folders_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    band_id          INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    path             TEXT NOT NULL,
    path_key         TEXT NOT NULL UNIQUE,
    added_at         INTEGER NOT NULL DEFAULT (unixepoch()),
    last_scan_at     INTEGER,
    local_only_count INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT INTO artist_folders_new (band_id, path, path_key, added_at, last_scan_at, local_only_count)
    SELECT band_id, path, path_key, added_at, last_scan_at, local_only_count FROM artist_folders;

CREATE TABLE artist_folder_ignored_new (
    band_id  INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    path_key TEXT NOT NULL,
    PRIMARY KEY (band_id, path_key)
) STRICT;
INSERT INTO artist_folder_ignored_new SELECT band_id, path_key FROM artist_folder_ignored;

-- Drop the child first: with foreign_keys ON, dropping a parent that still has
-- referencing rows runs an implicit DELETE and would cascade.
DROP TABLE artist_folder_ignored;
DROP TABLE artist_folders;
ALTER TABLE artist_folders_new RENAME TO artist_folders;
ALTER TABLE artist_folder_ignored_new RENAME TO artist_folder_ignored;
CREATE INDEX idx_artist_folders_band ON artist_folders(band_id);
