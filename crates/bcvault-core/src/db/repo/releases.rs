use crate::{
    error::DbError,
    types::{
        ArtistStats, AudioFormat, Classification, ItemKey, ItemType, LocalStatus, Origin,
        ReleaseFilter, ReleaseQuery, ReleaseRow, SortDir, SortKey, Ts, UpdateReason,
    },
};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct DiscoveredRelease<'a> {
    pub key: ItemKey,
    pub band_id: i64,
    pub title: &'a str,
    pub artist_name: &'a str,
    pub url: &'a str,
    pub cover_url: Option<&'a str>,
    pub art_id: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct ReleaseMetadata<'a> {
    pub key: ItemKey,
    pub classification: Classification,
    pub pricing_json: Option<&'a str>,
    pub is_preorder: bool,
    pub track_count: Option<u32>,
    pub published_at: Option<Ts>,
    pub modified_at: Option<Ts>,
    pub art_id: Option<i64>,
}

pub fn upsert_discovered(conn: &Connection, item: &DiscoveredRelease<'_>) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, cover_url, art_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(item_type, item_id) DO UPDATE SET
             last_seen_at = unixepoch(),
             title = excluded.title,
             artist_name = excluded.artist_name,
             url = excluded.url,
             cover_url = COALESCE(excluded.cover_url, releases.cover_url),
             art_id    = COALESCE(excluded.art_id, releases.art_id)",
        params![
            item.key.kind.as_str(),
            item.key.id,
            item.band_id,
            item.title,
            item.artist_name,
            item.url,
            item.cover_url,
            item.art_id,
        ],
    )?;
    Ok(())
}

pub fn upsert_discovered_batch(
    conn: &mut Connection,
    items: &[DiscoveredRelease<'_>],
) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    for item in items {
        upsert_discovered(&tx, item)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn update_metadata(conn: &mut Connection, meta: &ReleaseMetadata<'_>) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE releases SET
             classification = ?3,
             pricing_json = ?4,
             is_preorder = ?5,
             track_count = ?6,
             published_at = ?7,
             modified_at = ?8,
             art_id = COALESCE(?9, art_id),
             metadata_fetched_at = unixepoch()
         WHERE item_type = ?1 AND item_id = ?2",
        params![
            meta.key.kind.as_str(),
            meta.key.id,
            meta.classification.as_str(),
            meta.pricing_json,
            i32::from(meta.is_preorder),
            meta.track_count.map(i64::from),
            meta.published_at,
            meta.modified_at,
            meta.art_id,
        ],
    )?;

    tx.execute(
        "UPDATE releases SET
           has_remote_update = CASE WHEN downloaded = 1 AND (
                 track_count > COALESCE(downloaded_track_count, track_count)
              OR modified_at > COALESCE(downloaded_modified_at, modified_at)) THEN 1 ELSE 0 END,
           remote_update_reason = CASE
              WHEN downloaded = 1 AND track_count > COALESCE(downloaded_track_count, track_count) THEN 'tracks_added'
              WHEN downloaded = 1 AND modified_at > COALESCE(downloaded_modified_at, modified_at) THEN 'modified'
              ELSE NULL
           END
         WHERE item_type = ?1 AND item_id = ?2",
        params![meta.key.kind.as_str(), meta.key.id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn set_classification(
    conn: &Connection,
    key: ItemKey,
    classification: Classification,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE releases SET classification = ?3 WHERE item_type = ?1 AND item_id = ?2",
        params![key.kind.as_str(), key.id, classification.as_str()],
    )?;
    Ok(())
}

pub fn set_downloaded_manual(
    conn: &Connection,
    key: ItemKey,
    downloaded: bool,
) -> Result<(), DbError> {
    if downloaded {
        conn.execute(
            "UPDATE releases SET
                 downloaded = 1,
                 downloaded_at = COALESCE(downloaded_at, unixepoch()),
                 downloaded_track_count = COALESCE(downloaded_track_count, track_count),
                 downloaded_modified_at = COALESCE(downloaded_modified_at, modified_at),
                 has_remote_update = 0,
                 remote_update_reason = NULL
             WHERE item_type = ?1 AND item_id = ?2",
            params![key.kind.as_str(), key.id],
        )?;
    } else {
        conn.execute(
            "UPDATE releases SET
                 downloaded = 0,
                 downloaded_at = NULL,
                 downloaded_format = NULL,
                 downloaded_track_count = NULL,
                 downloaded_modified_at = NULL,
                 local_path = NULL,
                 has_remote_update = 0,
                 remote_update_reason = NULL
             WHERE item_type = ?1 AND item_id = ?2",
            params![key.kind.as_str(), key.id],
        )?;
    }
    Ok(())
}

pub fn commit_download(
    conn: &mut Connection,
    key: ItemKey,
    format: AudioFormat,
    local_path: &Path,
    bytes_size: u64,
    history_id: i64,
) -> Result<(), DbError> {
    let path_str = local_path.to_string_lossy().into_owned();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE releases SET
             downloaded = 1,
             downloaded_at = unixepoch(),
             downloaded_format = ?3,
             downloaded_track_count = track_count,
             downloaded_modified_at = modified_at,
             local_path = ?4,
             has_remote_update = 0,
             remote_update_reason = NULL
         WHERE item_type = ?1 AND item_id = ?2",
        params![key.kind.as_str(), key.id, format.as_str(), path_str],
    )?;
    tx.execute(
        "UPDATE download_history SET
             status = 'success',
             downloaded_at = unixepoch(),
             bytes_size = ?1
         WHERE id = ?2",
        params![bytes_size as i64, history_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn known_keys(conn: &Connection, band_id: i64) -> Result<HashSet<ItemKey>, DbError> {
    query_keys_where(
        conn,
        "SELECT item_type, item_id FROM releases WHERE band_id = ?1",
        band_id,
    )
}

pub fn unknown_keys(conn: &Connection, band_id: i64) -> Result<Vec<ItemKey>, DbError> {
    query_key_vec_where(
        conn,
        "SELECT item_type, item_id FROM releases WHERE band_id = ?1 AND classification = 'unknown' ORDER BY item_type, item_id",
        band_id,
    )
}

pub fn downloaded_keys(conn: &Connection, band_id: i64) -> Result<Vec<ItemKey>, DbError> {
    query_key_vec_where(
        conn,
        "SELECT item_type, item_id FROM releases WHERE band_id = ?1 AND downloaded = 1 ORDER BY item_type, item_id",
        band_id,
    )
}

fn query_keys_where(
    conn: &Connection,
    sql: &str,
    band_id: i64,
) -> Result<HashSet<ItemKey>, DbError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![band_id], parse_key_row)?;
    let mut set = HashSet::new();
    for r in rows {
        set.insert(r?);
    }
    Ok(set)
}

fn query_key_vec_where(
    conn: &Connection,
    sql: &str,
    band_id: i64,
) -> Result<Vec<ItemKey>, DbError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![band_id], parse_key_row)?;
    let mut vec = Vec::new();
    for r in rows {
        vec.push(r?);
    }
    Ok(vec)
}

fn parse_key_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ItemKey> {
    let t_str: String = r.get(0)?;
    let id: i64 = r.get(1)?;
    let kind = ItemType::parse(&t_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("Invalid item_type: {t_str}").into(),
        )
    })?;
    Ok(ItemKey { kind, id })
}

pub fn reclassify_from_pricing_json(conn: &mut Connection) -> Result<(), DbError> {
    let rows: Vec<(String, i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT item_type, item_id, classification, pricing_json
             FROM releases
             WHERE pricing_json IS NOT NULL AND classification IN ('free', 'nyp')",
        )?;
        let mapped = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut v = Vec::new();
        for row in mapped {
            v.push(row?);
        }
        v
    };

    let tx = conn.transaction()?;
    for (item_type, item_id, old_class, pricing_json) in rows {
        if let Ok(signals) =
            serde_json::from_str::<crate::bandcamp::classify::PricingSignals>(&pricing_json)
        {
            let new_class = crate::bandcamp::classify::classify(&signals);
            if new_class.as_str() != old_class {
                tx.execute(
                    "UPDATE releases SET classification = ?3 WHERE item_type = ?1 AND item_id = ?2",
                    params![item_type, item_id, new_class.as_str()],
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn all_artist_stats(conn: &Connection) -> Result<HashMap<i64, ArtistStats>, DbError> {
    let mut stmt = conn.prepare(
        "WITH max_seen AS (
             SELECT band_id, MAX(last_seen_at) AS latest_seen
             FROM releases
             GROUP BY band_id
         )
         SELECT r.band_id,
                COUNT(CASE WHEN r.last_seen_at >= m.latest_seen THEN 1 END) AS total,
                COALESCE(SUM(r.downloaded), 0) AS downloaded,
                COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification IN ('free','nyp','email_required') AND r.downloaded = 1 THEN 1 ELSE 0 END), 0) AS eligible_downloaded,
                COALESCE(SUM(CASE WHEN r.downloaded = 1 AND (r.last_seen_at < m.latest_seen OR r.classification NOT IN ('free','nyp','email_required')) THEN 1 ELSE 0 END), 0) AS non_eligible_downloaded,
                COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification IN ('free','nyp','email_required') THEN 1 ELSE 0 END), 0) AS eligible,
                COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification = 'email_required' THEN 1 ELSE 0 END), 0) AS email_required,
                COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification = 'unknown' THEN 1 ELSE 0 END), 0) AS unknown,
                COALESCE(SUM(CASE WHEN r.local_status = 'archived' THEN 1 ELSE 0 END), 0) AS archived,
                COALESCE(SUM(CASE WHEN r.local_status = 'missing' THEN 1 ELSE 0 END), 0) AS missing_files
         FROM releases r
         JOIN max_seen m ON m.band_id = r.band_id
         GROUP BY r.band_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            ArtistStats {
                total: r.get::<_, u32>(1)?,
                downloaded: r.get::<_, u32>(2)?,
                eligible_downloaded: r.get::<_, u32>(3)?,
                non_eligible_downloaded: r.get::<_, u32>(4)?,
                eligible: r.get::<_, u32>(5)?,
                email_required: r.get::<_, u32>(6)?,
                unknown: r.get::<_, u32>(7)?,
                quota_used_30d: 0,
                archived: r.get::<_, u32>(8)?,
                missing_files: r.get::<_, u32>(9)?,
            },
        ))
    })?;

    let mut map = HashMap::new();
    for r in rows {
        let (band_id, stats) = r?;
        map.insert(band_id, stats);
    }
    Ok(map)
}

pub fn artist_stats_for_one(conn: &Connection, band_id: i64) -> Result<ArtistStats, DbError> {
    let stats = conn
        .query_row(
            "WITH max_seen AS (
                 SELECT MAX(last_seen_at) AS latest_seen
                 FROM releases
                 WHERE band_id = ?1
             )
             SELECT COUNT(CASE WHEN r.last_seen_at >= m.latest_seen THEN 1 END) AS total,
                    COALESCE(SUM(r.downloaded), 0) AS downloaded,
                    COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification IN ('free','nyp','email_required') AND r.downloaded = 1 THEN 1 ELSE 0 END), 0) AS eligible_downloaded,
                    COALESCE(SUM(CASE WHEN r.downloaded = 1 AND (r.last_seen_at < m.latest_seen OR r.classification NOT IN ('free','nyp','email_required')) THEN 1 ELSE 0 END), 0) AS non_eligible_downloaded,
                    COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification IN ('free','nyp','email_required') THEN 1 ELSE 0 END), 0) AS eligible,
                    COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification = 'email_required' THEN 1 ELSE 0 END), 0) AS email_required,
                    COALESCE(SUM(CASE WHEN r.last_seen_at >= m.latest_seen AND r.classification = 'unknown' THEN 1 ELSE 0 END), 0) AS unknown,
                    COALESCE(SUM(CASE WHEN r.local_status = 'archived' THEN 1 ELSE 0 END), 0) AS archived,
                    COALESCE(SUM(CASE WHEN r.local_status = 'missing' THEN 1 ELSE 0 END), 0) AS missing_files
             FROM releases r
             CROSS JOIN max_seen m
             WHERE r.band_id = ?1",
            params![band_id],
            |r| {
                Ok(ArtistStats {
                    total: r.get::<_, u32>(0)?,
                    downloaded: r.get::<_, u32>(1)?,
                    eligible_downloaded: r.get::<_, u32>(2)?,
                    non_eligible_downloaded: r.get::<_, u32>(3)?,
                    eligible: r.get::<_, u32>(4)?,
                    email_required: r.get::<_, u32>(5)?,
                    unknown: r.get::<_, u32>(6)?,
                    quota_used_30d: 0,
                    archived: r.get::<_, u32>(7)?,
                    missing_files: r.get::<_, u32>(8)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default();
    Ok(stats)
}

const SELECT_RELEASE_COLS: &str = "r.item_type, r.item_id, r.band_id, r.title, \
     CASE WHEN a.custom_name IS NOT NULL AND a.custom_name != '' AND r.artist_name = a.name \
          THEN a.custom_name ELSE r.artist_name END AS artist_name, \
     r.url, r.cover_url, r.art_id, \
     r.classification, r.is_preorder, r.track_count, r.published_at, r.modified_at, \
     r.downloaded, r.downloaded_at, r.downloaded_format, r.local_path, \
     r.has_remote_update, r.remote_update_reason, r.local_status, r.origin, r.missing_ack_at";

const FROM_RELEASES_JOIN: &str = "releases r LEFT JOIN artists a ON r.band_id = a.band_id";

fn map_release_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ReleaseRow> {
    let t_str: String = r.get(0)?;
    let kind = ItemType::parse(&t_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("Invalid item_type: {t_str}").into(),
        )
    })?;
    let id: i64 = r.get(1)?;
    let band_id: i64 = r.get(2)?;
    let title: String = r.get(3)?;
    let artist_name: String = r.get(4)?;
    let url: String = r.get(5)?;
    let cover_url: Option<String> = r.get(6)?;
    let art_id: Option<i64> = r.get(7)?;
    let class_str: String = r.get(8)?;
    let classification = Classification::parse(&class_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            format!("Invalid classification: {class_str}").into(),
        )
    })?;
    let is_preorder: i64 = r.get(9)?;
    let track_count: Option<u32> = r.get(10)?;
    let published_at: Option<Ts> = r.get(11)?;
    let modified_at: Option<Ts> = r.get(12)?;
    let downloaded: i64 = r.get(13)?;
    let downloaded_at: Option<Ts> = r.get(14)?;
    let fmt_str: Option<String> = r.get(15)?;
    let downloaded_format = fmt_str.as_deref().and_then(AudioFormat::parse);
    let local_path_str: Option<String> = r.get(16)?;
    let local_path = local_path_str.map(PathBuf::from);
    let has_remote_update: i64 = r.get(17)?;
    let reason_str: Option<String> = r.get(18)?;
    let remote_update_reason = reason_str.as_deref().and_then(UpdateReason::parse);
    let local_status_str: Option<String> = r.get(19)?;
    let local_status = local_status_str.as_deref().and_then(LocalStatus::parse);
    let origin_str: Option<String> = r.get(20)?;
    let origin = origin_str.as_deref().and_then(Origin::parse);
    let missing_ack_at: Option<Ts> = r.get(21)?;

    Ok(ReleaseRow {
        key: ItemKey { kind, id },
        band_id,
        title,
        artist_name,
        url,
        cover_url,
        art_id,
        classification,
        is_preorder: is_preorder != 0,
        track_count,
        published_at,
        modified_at,
        downloaded: downloaded != 0,
        downloaded_at,
        downloaded_format,
        local_path,
        local_status,
        origin,
        missing_ack_at,
        has_remote_update: has_remote_update != 0,
        remote_update_reason,
    })
}

pub fn get_release(conn: &Connection, key: ItemKey) -> Result<Option<ReleaseRow>, DbError> {
    let sql = format!(
        "SELECT {SELECT_RELEASE_COLS} FROM {FROM_RELEASES_JOIN} WHERE r.item_type = ?1 AND r.item_id = ?2"
    );
    let row = conn
        .query_row(&sql, params![key.kind.as_str(), key.id], map_release_row)
        .optional()?;
    Ok(row)
}

pub fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    for ch in input.chars() {
        if matches!(ch, '%' | '_' | '!') {
            out.push('!');
        }
        out.push(ch);
    }
    out
}

pub const fn order_by_clause(sort: SortKey, dir: SortDir) -> &'static str {
    match (sort, dir) {
        (SortKey::Released, SortDir::Desc) => "r.published_at DESC, r.item_type, r.item_id",
        (SortKey::Released, SortDir::Asc) => "r.published_at ASC, r.item_type, r.item_id",
        (SortKey::Artist, SortDir::Asc) => {
            "artist_name COLLATE NOCASE ASC, r.title COLLATE NOCASE ASC, r.item_type, r.item_id"
        }
        (SortKey::Artist, SortDir::Desc) => {
            "artist_name COLLATE NOCASE DESC, r.title COLLATE NOCASE DESC, r.item_type, r.item_id"
        }
        (SortKey::Title, SortDir::Asc) => "r.title COLLATE NOCASE ASC, r.item_type, r.item_id",
        (SortKey::Title, SortDir::Desc) => "r.title COLLATE NOCASE DESC, r.item_type, r.item_id",
        (SortKey::Type, SortDir::Asc) => "r.item_type ASC, r.published_at DESC, r.item_id",
        (SortKey::Type, SortDir::Desc) => "r.item_type DESC, r.published_at DESC, r.item_id",
        (SortKey::Tracks, SortDir::Asc) => "r.track_count ASC, r.item_type, r.item_id",
        (SortKey::Tracks, SortDir::Desc) => "r.track_count DESC, r.item_type, r.item_id",
        (SortKey::Classification, SortDir::Asc) => "r.classification ASC, r.item_type, r.item_id",
        (SortKey::Classification, SortDir::Desc) => "r.classification DESC, r.item_type, r.item_id",
        (SortKey::Format, SortDir::Asc) => "r.downloaded_format ASC, r.item_type, r.item_id",
        (SortKey::Format, SortDir::Desc) => "r.downloaded_format DESC, r.item_type, r.item_id",
        (SortKey::Status, SortDir::Asc) => {
            "r.downloaded ASC, r.has_remote_update ASC, r.item_type, r.item_id"
        }
        (SortKey::Status, SortDir::Desc) => {
            "r.downloaded DESC, r.has_remote_update DESC, r.item_type, r.item_id"
        }
    }
}

pub fn build_query_sql(query: &ReleaseQuery) -> (String, Vec<Value>) {
    let mut where_parts: Vec<&'static str> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(band_id) = query.band_id {
        where_parts.push("r.band_id = ?");
        values.push(Value::Integer(band_id));
    }

    match query.filter {
        ReleaseFilter::All => {}
        ReleaseFilter::Free => where_parts.push("r.classification = 'free'"),
        ReleaseFilter::Nyp => where_parts.push("r.classification = 'nyp'"),
        ReleaseFilter::EmailRequired => where_parts.push("r.classification = 'email_required'"),
        ReleaseFilter::Paid => where_parts.push("r.classification = 'paid'"),
        ReleaseFilter::NotDownloaded => where_parts.push(
            "r.downloaded = 0 AND r.classification IN ('free','nyp','email_required','unknown')",
        ),
        ReleaseFilter::Updated => where_parts.push("r.has_remote_update = 1"),
        ReleaseFilter::Archived => where_parts.push("r.local_status = 'archived'"),
        ReleaseFilter::MissingFiles => where_parts.push("r.local_status = 'missing'"),
    }

    let trimmed = query.search.trim();
    if !trimmed.is_empty() {
        let pattern = format!("%{}%", escape_like(trimmed));
        where_parts.push(
            "(r.title LIKE ? ESCAPE '!' OR r.artist_name LIKE ? ESCAPE '!' OR a.custom_name LIKE ? ESCAPE '!')",
        );
        values.push(Value::Text(pattern.clone()));
        values.push(Value::Text(pattern.clone()));
        values.push(Value::Text(pattern));
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    let order_sql = order_by_clause(query.sort, query.dir);
    let sql = format!(
        "SELECT {SELECT_RELEASE_COLS} FROM {FROM_RELEASES_JOIN} {where_sql} ORDER BY {order_sql}"
    );
    (sql, values)
}

pub fn query_releases(conn: &Connection, query: &ReleaseQuery) -> Result<Vec<ReleaseRow>, DbError> {
    let (sql, values) = build_query_sql(query);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), map_release_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::migrations;

    #[test]
    fn archived_and_missing_filters_and_stats() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'art1', 'Artist 1', 'https://art1.bandcamp.com')",
            [],
        )
        .unwrap();

        // Release 1: present, downloaded = 1
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, local_status)
             VALUES ('album', 1, 1, 'Alb 1', 'Artist 1', 'https://art1.bandcamp.com/album/1', 'free', 1, 'present')",
            [],
        )
        .unwrap();

        // Release 2: archived, downloaded = 1
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, local_status)
             VALUES ('album', 2, 1, 'Alb 2', 'Artist 1', 'https://art1.bandcamp.com/album/2', 'nyp', 1, 'archived')",
            [],
        )
        .unwrap();

        // Release 3: missing, downloaded = 1
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, local_status)
             VALUES ('album', 3, 1, 'Alb 3', 'Artist 1', 'https://art1.bandcamp.com/album/3', 'free', 1, 'missing')",
            [],
        )
        .unwrap();

        // Release 4: not downloaded
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, local_status)
             VALUES ('album', 4, 1, 'Alb 4', 'Artist 1', 'https://art1.bandcamp.com/album/4', 'paid', 0, NULL)",
            [],
        )
        .unwrap();

        // Test stats
        let stats = artist_stats_for_one(&conn, 1).unwrap();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.downloaded, 3); // 1 present + 1 archived + 1 missing
        assert_eq!(stats.archived, 1);
        assert_eq!(stats.missing_files, 1);

        let all_stats = all_artist_stats(&conn).unwrap();
        assert_eq!(all_stats.get(&1), Some(&stats));

        // Test Archived filter
        let q_arch = ReleaseQuery {
            band_id: Some(1),
            filter: ReleaseFilter::Archived,
            ..Default::default()
        };
        let arch_rows = query_releases(&conn, &q_arch).unwrap();
        assert_eq!(arch_rows.len(), 1);
        assert_eq!(arch_rows[0].key.id, 2);
        assert_eq!(arch_rows[0].local_status, Some(LocalStatus::Archived));

        // Test MissingFiles filter
        let q_miss = ReleaseQuery {
            band_id: Some(1),
            filter: ReleaseFilter::MissingFiles,
            ..Default::default()
        };
        let miss_rows = query_releases(&conn, &q_miss).unwrap();
        assert_eq!(miss_rows.len(), 1);
        assert_eq!(miss_rows[0].key.id, 3);
        assert_eq!(miss_rows[0].local_status, Some(LocalStatus::Missing));
    }

    #[test]
    fn test_artist_stats_eligible_and_non_eligible_breakdown() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();

        // Artist 10: Nigeria Futebol Clube (3 on page: 2 free, 1 paid; all 3 downloaded)
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (10, 'nfc', 'Nigeria Futebol Clube', 'https://nfc.bandcamp.com')",
            [],
        )
        .unwrap();

        let t_page_nfc = 1_000_000;
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, last_seen_at)
             VALUES ('album', 101, 10, 'Free 1', 'NFC', 'https://nfc.bandcamp.com/album/101', 'free', 1, ?1),
                    ('album', 102, 10, 'Free 2', 'NFC', 'https://nfc.bandcamp.com/album/102', 'nyp', 1, ?1),
                    ('album', 103, 10, 'Paid 1', 'NFC', 'https://nfc.bandcamp.com/album/103', 'paid', 1, ?1)",
            params![t_page_nfc],
        )
        .unwrap();

        let nfc_stats = artist_stats_for_one(&conn, 10).unwrap();
        assert_eq!(nfc_stats.total, 3);
        assert_eq!(nfc_stats.eligible, 2);
        assert_eq!(nfc_stats.eligible_downloaded, 2);
        assert_eq!(nfc_stats.non_eligible_downloaded, 1);
        assert_eq!(nfc_stats.downloaded, 3);
        assert_eq!(nfc_stats.format_sidebar_badge(), "2 (+ 1)/2/3");

        // Artist 20: Leichen (40 on page all free & downloaded; 2 extra off-page downloaded)
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (20, 'leichen', 'Leichen', 'https://leichen.bandcamp.com')",
            [],
        )
        .unwrap();

        let t_page_leichen = 2_000_000;
        for i in 1..=40 {
            conn.execute(
                "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, last_seen_at)
                 VALUES ('album', ?1, 20, ?2, 'Leichen', ?3, 'free', 1, ?4)",
                params![200 + i, format!("Leiche {}", i), format!("https://leichen.bandcamp.com/album/{}", i), t_page_leichen],
            )
            .unwrap();
        }

        // 2 extra albums not on current page (older last_seen_at) but downloaded
        let t_older = t_page_leichen - 500;
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, last_seen_at)
             VALUES ('album', 301, 20, 'Extra 1', 'Leichen', 'https://leichen.bandcamp.com/album/301', 'free', 1, ?1),
                    ('album', 302, 20, 'Extra 2', 'Leichen', 'https://leichen.bandcamp.com/album/302', 'free', 1, ?1)",
            params![t_older],
        )
        .unwrap();

        let leichen_stats = artist_stats_for_one(&conn, 20).unwrap();
        assert_eq!(leichen_stats.total, 40);
        assert_eq!(leichen_stats.eligible, 40);
        assert_eq!(leichen_stats.eligible_downloaded, 40);
        assert_eq!(leichen_stats.non_eligible_downloaded, 2);
        assert_eq!(leichen_stats.downloaded, 42);
        assert_eq!(leichen_stats.format_sidebar_badge(), "40 (+ 2)/40/40");

        // Artist 30: Standard clean artist (5 on page: 3 free, 2 paid; 2 free downloaded)
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (30, 'clean', 'Clean Artist', 'https://clean.bandcamp.com')",
            [],
        )
        .unwrap();

        let t_clean = 3_000_000;
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, downloaded, last_seen_at)
             VALUES ('album', 401, 30, 'C1', 'Clean', 'https://clean.bandcamp.com/album/401', 'free', 1, ?1),
                    ('album', 402, 30, 'C2', 'Clean', 'https://clean.bandcamp.com/album/402', 'free', 1, ?1),
                    ('album', 403, 30, 'C3', 'Clean', 'https://clean.bandcamp.com/album/403', 'free', 0, ?1),
                    ('album', 404, 30, 'C4', 'Clean', 'https://clean.bandcamp.com/album/404', 'paid', 0, ?1),
                    ('album', 405, 30, 'C5', 'Clean', 'https://clean.bandcamp.com/album/405', 'paid', 0, ?1)",
            params![t_clean],
        )
        .unwrap();

        let clean_stats = artist_stats_for_one(&conn, 30).unwrap();
        assert_eq!(clean_stats.total, 5);
        assert_eq!(clean_stats.eligible, 3);
        assert_eq!(clean_stats.eligible_downloaded, 2);
        assert_eq!(clean_stats.non_eligible_downloaded, 0);
        assert_eq!(clean_stats.downloaded, 2);
        assert_eq!(clean_stats.format_sidebar_badge(), "2/3/5");

        // Verify all_artist_stats map matches individual queries
        let all = all_artist_stats(&conn).unwrap();
        assert_eq!(all.get(&10), Some(&nfc_stats));
        assert_eq!(all.get(&20), Some(&leichen_stats));
        assert_eq!(all.get(&30), Some(&clean_stats));
    }
}
