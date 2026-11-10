CREATE TABLE releases_new (
    item_type              TEXT    NOT NULL CHECK (item_type IN ('album','track')),
    item_id                INTEGER NOT NULL,
    band_id                INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    title                  TEXT    NOT NULL,
    artist_name            TEXT    NOT NULL,
    url                    TEXT    NOT NULL,
    cover_url              TEXT,
    art_id                 INTEGER,
    classification         TEXT    NOT NULL DEFAULT 'unknown'
                           CHECK (classification IN ('unknown','free','nyp','email_required','paid','unavailable')),
    pricing_json           TEXT,
    is_preorder            INTEGER NOT NULL DEFAULT 0 CHECK (is_preorder IN (0,1)),
    track_count            INTEGER,
    published_at           INTEGER,
    modified_at            INTEGER,
    metadata_fetched_at    INTEGER,
    first_seen_at          INTEGER NOT NULL DEFAULT (unixepoch()),
    last_seen_at           INTEGER NOT NULL DEFAULT (unixepoch()),
    downloaded             INTEGER NOT NULL DEFAULT 0 CHECK (downloaded IN (0,1)),
    downloaded_at          INTEGER,
    downloaded_format      TEXT,
    downloaded_track_count INTEGER,
    downloaded_modified_at INTEGER,
    local_path             TEXT,
    has_remote_update      INTEGER NOT NULL DEFAULT 0 CHECK (has_remote_update IN (0,1)),
    remote_update_reason   TEXT CHECK (remote_update_reason IN ('tracks_added','modified')),
    local_status           TEXT CHECK (local_status IS NULL OR local_status IN ('unverified','present','missing','archived')),
    local_checked_at       INTEGER,
    missing_ack_at         INTEGER,
    origin                 TEXT CHECK (origin IS NULL OR origin IN ('app','imported','redeem','manual')),
    PRIMARY KEY (item_type, item_id)
) STRICT;

INSERT INTO releases_new (
    item_type, item_id, band_id, title, artist_name, url, cover_url, art_id,
    classification, pricing_json, is_preorder, track_count, published_at, modified_at,
    metadata_fetched_at, first_seen_at, last_seen_at, downloaded, downloaded_at,
    downloaded_format, downloaded_track_count, downloaded_modified_at, local_path,
    has_remote_update, remote_update_reason, local_status, local_checked_at,
    missing_ack_at, origin
)
SELECT
    item_type, item_id, band_id, title, artist_name, url, cover_url, art_id,
    CASE WHEN track_count = 0 AND metadata_fetched_at IS NOT NULL THEN 'unavailable' ELSE classification END,
    pricing_json, is_preorder, track_count, published_at, modified_at,
    metadata_fetched_at, first_seen_at, last_seen_at, downloaded, downloaded_at,
    downloaded_format, downloaded_track_count, downloaded_modified_at, local_path,
    has_remote_update, remote_update_reason, local_status, local_checked_at,
    missing_ack_at, origin
FROM releases;

DROP TABLE releases;
ALTER TABLE releases_new RENAME TO releases;

CREATE INDEX idx_releases_band_pub   ON releases(band_id, published_at DESC, item_type, item_id);
CREATE INDEX idx_releases_pub        ON releases(published_at DESC, item_type, item_id);
CREATE INDEX idx_releases_class_dl   ON releases(classification, downloaded, band_id);
CREATE INDEX idx_releases_todo       ON releases(band_id, published_at DESC)
    WHERE downloaded = 0 AND classification IN ('free','nyp');
CREATE INDEX idx_releases_updates    ON releases(band_id) WHERE has_remote_update = 1;
CREATE INDEX idx_releases_unknown    ON releases(band_id) WHERE classification = 'unknown';
CREATE INDEX idx_releases_downloaded ON releases(band_id) WHERE downloaded = 1;
CREATE INDEX idx_releases_title      ON releases(title COLLATE NOCASE);
CREATE INDEX idx_releases_artistname ON releases(artist_name COLLATE NOCASE, title COLLATE NOCASE);
CREATE INDEX idx_releases_attention  ON releases(band_id) WHERE local_status = 'missing';
CREATE INDEX idx_releases_archived   ON releases(band_id) WHERE local_status = 'archived';
