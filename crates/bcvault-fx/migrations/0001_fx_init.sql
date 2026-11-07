CREATE TABLE cover_meta (
    art_id         INTEGER PRIMARY KEY,
    tiny8          BLOB,                 -- 8x8 RGB = 192 bytes: instant blocky preview
    swatches       BLOB,                 -- 5 x RGB (15 bytes), by population
    accent_rgb     INTEGER,              -- 0xRRGGBB, most vivid swatch with usable contrast
    hue            INTEGER,              -- 0..359 (OKLCH), NULL if achromatic
    chroma         INTEGER,
    luma           INTEGER,              -- 0..100
    src_fetched_at INTEGER,
    failed_at      INTEGER,
    fail_count     INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX idx_cover_hue ON cover_meta(hue) WHERE hue IS NOT NULL;

CREATE TABLE release_detail (
    item_type  TEXT NOT NULL,
    item_id    INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    tracks_json TEXT,
    about      TEXT,
    credits    TEXT,
    tags_json  TEXT,
    duration_s INTEGER,
    PRIMARY KEY (item_type, item_id)
) STRICT;

CREATE TABLE saved_views (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    state_json TEXT NOT NULL,
    pos        INTEGER NOT NULL
) STRICT;

CREATE TABLE recents (
    kind TEXT NOT NULL,
    ref  TEXT NOT NULL,
    ts   INTEGER NOT NULL,
    PRIMARY KEY (kind, ref)
) STRICT;

CREATE TABLE prefs (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
