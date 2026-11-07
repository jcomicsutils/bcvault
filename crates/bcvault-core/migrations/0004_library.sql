ALTER TABLE releases ADD COLUMN local_status TEXT
    CHECK (local_status IS NULL OR local_status IN ('unverified','present','missing','archived'));
ALTER TABLE releases ADD COLUMN local_checked_at INTEGER;
ALTER TABLE releases ADD COLUMN missing_ack_at   INTEGER;     -- user chose "skip for now"
ALTER TABLE releases ADD COLUMN origin TEXT
    CHECK (origin IS NULL OR origin IN ('app','imported','redeem','manual'));
UPDATE releases SET local_status = 'unverified' WHERE downloaded = 1;

-- One folder per artist, one artist per folder.
CREATE TABLE artist_folders (
    band_id      INTEGER PRIMARY KEY REFERENCES artists(band_id) ON DELETE CASCADE,
    path         TEXT NOT NULL,                -- canonical absolute path
    path_key     TEXT NOT NULL UNIQUE,         -- case/Unicode-folded key for comparisons
    added_at     INTEGER NOT NULL DEFAULT (unixepoch()),
    last_scan_at INTEGER
) STRICT;

-- "Local only" folders the user chose to hide, per artist.
CREATE TABLE artist_folder_ignored (
    band_id  INTEGER NOT NULL REFERENCES artist_folders(band_id) ON DELETE CASCADE,
    path_key TEXT NOT NULL,
    PRIMARY KEY (band_id, path_key)
) STRICT;

CREATE TABLE library_batches (
    id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL DEFAULT (unixepoch()),
    kind TEXT NOT NULL,                        -- adopt | verify | resolve | manual | sync
    summary_json TEXT NOT NULL, undone_at INTEGER
) STRICT;
CREATE TABLE library_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT, batch_id INTEGER NOT NULL REFERENCES library_batches(id) ON DELETE CASCADE,
    item_type TEXT NOT NULL, item_id INTEGER NOT NULL,
    kind TEXT NOT NULL,                        -- adopted | relinked | archived | unmarked | restored | ignored | linked | unlinked
    old_json TEXT, new_json TEXT
) STRICT;
CREATE INDEX idx_libev_item ON library_events(item_type, item_id);

CREATE INDEX idx_releases_attention ON releases(band_id) WHERE local_status = 'missing';
CREATE INDEX idx_releases_archived  ON releases(band_id) WHERE local_status = 'archived';
