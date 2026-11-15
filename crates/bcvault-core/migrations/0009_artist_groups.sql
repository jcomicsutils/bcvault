CREATE TABLE artist_group_sets (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    pos  INTEGER NOT NULL
) STRICT;

CREATE TABLE artist_groups (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    set_id    INTEGER NOT NULL REFERENCES artist_group_sets(id) ON DELETE CASCADE,
    name      TEXT    NOT NULL COLLATE NOCASE,
    pos       INTEGER NOT NULL,
    collapsed INTEGER NOT NULL DEFAULT 0 CHECK (collapsed IN (0,1)),
    UNIQUE (set_id, name)
) STRICT;

CREATE TABLE artist_group_pins (
    set_id   INTEGER NOT NULL REFERENCES artist_group_sets(id) ON DELETE CASCADE,
    band_id  INTEGER NOT NULL REFERENCES artists(band_id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES artist_groups(id) ON DELETE CASCADE,
    PRIMARY KEY (set_id, band_id)
) STRICT;
