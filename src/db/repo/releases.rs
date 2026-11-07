use crate::{
    error::DbError,
    types::{
        ArtistStats, AudioFormat, Classification, ItemKey, ItemType, ReleaseFilter, ReleaseQuery,
        ReleaseRow, SortDir, SortKey, Ts, UpdateReason,
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

pub fn all_artist_stats(conn: &Connection) -> Result<HashMap<i64, ArtistStats>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT band_id,
                COUNT(*) AS total,
                COALESCE(SUM(downloaded), 0) AS downloaded,
                COALESCE(SUM(classification IN ('free','nyp')), 0) AS eligible,
                COALESCE(SUM(classification = 'email_required'), 0) AS email_required,
                COALESCE(SUM(classification = 'unknown'), 0) AS unknown
         FROM releases
         GROUP BY band_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            ArtistStats {
                total: r.get::<_, u32>(1)?,
                downloaded: r.get::<_, u32>(2)?,
                eligible: r.get::<_, u32>(3)?,
                email_required: r.get::<_, u32>(4)?,
                unknown: r.get::<_, u32>(5)?,
                quota_used_30d: 0,
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
            "SELECT COUNT(*) AS total,
                    COALESCE(SUM(downloaded), 0) AS downloaded,
                    COALESCE(SUM(classification IN ('free','nyp')), 0) AS eligible,
                    COALESCE(SUM(classification = 'email_required'), 0) AS email_required,
                    COALESCE(SUM(classification = 'unknown'), 0) AS unknown
             FROM releases
             WHERE band_id = ?1",
            params![band_id],
            |r| {
                Ok(ArtistStats {
                    total: r.get::<_, u32>(0)?,
                    downloaded: r.get::<_, u32>(1)?,
                    eligible: r.get::<_, u32>(2)?,
                    email_required: r.get::<_, u32>(3)?,
                    unknown: r.get::<_, u32>(4)?,
                    quota_used_30d: 0,
                })
            },
        )
        .optional()?
        .unwrap_or_default();
    Ok(stats)
}

const SELECT_RELEASE_COLS: &str = "item_type, item_id, band_id, title, artist_name, url, cover_url, art_id, \
     classification, is_preorder, track_count, published_at, modified_at, \
     downloaded, downloaded_at, downloaded_format, local_path, \
     has_remote_update, remote_update_reason";

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
        has_remote_update: has_remote_update != 0,
        remote_update_reason,
    })
}

pub fn get_release(conn: &Connection, key: ItemKey) -> Result<Option<ReleaseRow>, DbError> {
    let sql =
        format!("SELECT {SELECT_RELEASE_COLS} FROM releases WHERE item_type = ?1 AND item_id = ?2");
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
        (SortKey::Released, SortDir::Desc) => "published_at DESC, item_type, item_id",
        (SortKey::Released, SortDir::Asc) => "published_at ASC, item_type, item_id",
        (SortKey::Artist, SortDir::Asc) => {
            "artist_name COLLATE NOCASE ASC, title COLLATE NOCASE ASC, item_type, item_id"
        }
        (SortKey::Artist, SortDir::Desc) => {
            "artist_name COLLATE NOCASE DESC, title COLLATE NOCASE DESC, item_type, item_id"
        }
        (SortKey::Title, SortDir::Asc) => "title COLLATE NOCASE ASC, item_type, item_id",
        (SortKey::Title, SortDir::Desc) => "title COLLATE NOCASE DESC, item_type, item_id",
        (SortKey::Type, SortDir::Asc) => "item_type ASC, published_at DESC, item_id",
        (SortKey::Type, SortDir::Desc) => "item_type DESC, published_at DESC, item_id",
        (SortKey::Tracks, SortDir::Asc) => "track_count ASC, item_type, item_id",
        (SortKey::Tracks, SortDir::Desc) => "track_count DESC, item_type, item_id",
        (SortKey::Classification, SortDir::Asc) => "classification ASC, item_type, item_id",
        (SortKey::Classification, SortDir::Desc) => "classification DESC, item_type, item_id",
        (SortKey::Format, SortDir::Asc) => "downloaded_format ASC, item_type, item_id",
        (SortKey::Format, SortDir::Desc) => "downloaded_format DESC, item_type, item_id",
        (SortKey::Status, SortDir::Asc) => {
            "downloaded ASC, has_remote_update ASC, item_type, item_id"
        }
        (SortKey::Status, SortDir::Desc) => {
            "downloaded DESC, has_remote_update DESC, item_type, item_id"
        }
    }
}

pub fn build_query_sql(query: &ReleaseQuery) -> (String, Vec<Value>) {
    let mut where_parts: Vec<&'static str> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(band_id) = query.band_id {
        where_parts.push("band_id = ?");
        values.push(Value::Integer(band_id));
    }

    match query.filter {
        ReleaseFilter::All => {}
        ReleaseFilter::Free => where_parts.push("classification = 'free'"),
        ReleaseFilter::Nyp => where_parts.push("classification = 'nyp'"),
        ReleaseFilter::EmailRequired => where_parts.push("classification = 'email_required'"),
        ReleaseFilter::Paid => where_parts.push("classification = 'paid'"),
        ReleaseFilter::NotDownloaded => where_parts
            .push("downloaded = 0 AND classification IN ('free','nyp','email_required','unknown')"),
        ReleaseFilter::Updated => where_parts.push("has_remote_update = 1"),
    }

    let trimmed = query.search.trim();
    if !trimmed.is_empty() {
        let pattern = format!("%{}%", escape_like(trimmed));
        where_parts.push("(title LIKE ? ESCAPE '!' OR artist_name LIKE ? ESCAPE '!')");
        values.push(Value::Text(pattern.clone()));
        values.push(Value::Text(pattern));
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    let order_sql = order_by_clause(query.sort, query.dir);
    let sql =
        format!("SELECT {SELECT_RELEASE_COLS} FROM releases {where_sql} ORDER BY {order_sql}");
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
