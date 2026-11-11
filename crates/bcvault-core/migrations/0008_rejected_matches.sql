CREATE TABLE artist_folder_rejected (
    band_id    INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    item_type  TEXT    NOT NULL CHECK (item_type IN ('album','track')),
    item_id    INTEGER NOT NULL,
    path_key   TEXT    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (band_id, item_type, item_id, path_key),
    FOREIGN KEY (item_type, item_id) REFERENCES releases(item_type, item_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_artist_folder_rejected_band ON artist_folder_rejected(band_id);
