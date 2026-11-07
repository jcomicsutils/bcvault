CREATE UNIQUE INDEX idx_artists_url  ON artists(url);
CREATE INDEX idx_artists_name        ON artists(name COLLATE NOCASE);

-- Table view: per-artist browsing, default sort newest first
CREATE INDEX idx_releases_band_pub   ON releases(band_id, published_at DESC, item_type, item_id);
CREATE INDEX idx_releases_pub        ON releases(published_at DESC, item_type, item_id);
CREATE INDEX idx_releases_class_dl   ON releases(classification, downloaded, band_id);

-- Small, hot partial indexes
CREATE INDEX idx_releases_todo       ON releases(band_id, published_at DESC)
    WHERE downloaded = 0 AND classification IN ('free','nyp');
CREATE INDEX idx_releases_updates    ON releases(band_id) WHERE has_remote_update = 1;
CREATE INDEX idx_releases_unknown    ON releases(band_id) WHERE classification = 'unknown';
CREATE INDEX idx_releases_downloaded ON releases(band_id) WHERE downloaded = 1;

-- Text sorts
CREATE INDEX idx_releases_title      ON releases(title COLLATE NOCASE);
CREATE INDEX idx_releases_artistname ON releases(artist_name COLLATE NOCASE, title COLLATE NOCASE);

-- History and quota math
CREATE INDEX idx_hist_quota ON download_history(band_id, started_at) WHERE counts_against_quota = 1;
CREATE INDEX idx_hist_item  ON download_history(item_type, item_id, started_at DESC);
