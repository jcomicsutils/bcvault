CREATE TABLE artists (
    band_id        INTEGER PRIMARY KEY,
    slug           TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    url            TEXT    NOT NULL,   -- canonical base URL, custom domains allowed, no trailing slash
    cover_url      TEXT,
    last_synced_at INTEGER,            -- unix seconds UTC
    created_at     INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE releases (
    item_type              TEXT    NOT NULL CHECK (item_type IN ('album','track')),
    item_id                INTEGER NOT NULL,
    band_id                INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    title                  TEXT    NOT NULL,
    artist_name            TEXT    NOT NULL,   -- per release: label pages host many artists
    url                    TEXT    NOT NULL,   -- absolute release URL
    cover_url              TEXT,
    art_id                 INTEGER,
    classification         TEXT    NOT NULL DEFAULT 'unknown'
                           CHECK (classification IN ('unknown','free','nyp','email_required','paid')),
    pricing_json           TEXT,               -- raw pricing signals, enables re-classification offline
    is_preorder            INTEGER NOT NULL DEFAULT 0 CHECK (is_preorder IN (0,1)),
    track_count            INTEGER,
    published_at           INTEGER,
    modified_at            INTEGER,
    metadata_fetched_at    INTEGER,            -- NULL until the release page was parsed
    first_seen_at          INTEGER NOT NULL DEFAULT (unixepoch()),
    last_seen_at           INTEGER NOT NULL DEFAULT (unixepoch()),
    downloaded             INTEGER NOT NULL DEFAULT 0 CHECK (downloaded IN (0,1)),
    downloaded_at          INTEGER,
    downloaded_format      TEXT,
    downloaded_track_count INTEGER,            -- snapshot used to detect bonus tracks
    downloaded_modified_at INTEGER,
    local_path             TEXT,
    has_remote_update      INTEGER NOT NULL DEFAULT 0 CHECK (has_remote_update IN (0,1)),
    remote_update_reason   TEXT CHECK (remote_update_reason IN ('tracks_added','modified')),
    PRIMARY KEY (item_type, item_id)
) STRICT;

CREATE TABLE download_history (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    item_type            TEXT    NOT NULL CHECK (item_type IN ('album','track')),
    item_id              INTEGER NOT NULL,
    band_id              INTEGER NOT NULL,     -- deliberately NOT a FK: history and quota math survive artist removal
    started_at           INTEGER NOT NULL DEFAULT (unixepoch()),
    downloaded_at        INTEGER,              -- set when status becomes 'success'
    format               TEXT    NOT NULL,
    bytes_size           INTEGER,
    status               TEXT    NOT NULL CHECK (status IN ('started','success','failed','cancelled')),
    source               TEXT    NOT NULL DEFAULT 'api' CHECK (source IN ('api','redeem')),
    counts_against_quota INTEGER NOT NULL DEFAULT 0 CHECK (counts_against_quota IN (0,1)),
    error_message        TEXT
) STRICT;

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;  -- JSON values
