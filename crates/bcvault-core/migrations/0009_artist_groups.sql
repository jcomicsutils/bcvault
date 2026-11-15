CREATE TABLE artist_groups (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    pos       INTEGER NOT NULL,
    collapsed INTEGER NOT NULL DEFAULT 0 CHECK (collapsed IN (0,1)),
    rule_json TEXT
) STRICT;

CREATE TABLE artist_group_pins (
    band_id  INTEGER PRIMARY KEY REFERENCES artists(band_id) ON DELETE CASCADE,
    group_id INTEGER          REFERENCES artist_groups(id) ON DELETE CASCADE
) STRICT;
