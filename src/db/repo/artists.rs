use crate::{
    db::repo::{history, releases},
    error::DbError,
    types::{ArtistStats, ArtistView, Ts},
};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Debug)]
pub struct UpsertArtist<'a> {
    pub band_id: i64,
    pub slug: &'a str,
    pub name: &'a str,
    pub url: &'a str,
    pub cover_url: Option<&'a str>,
}

pub fn upsert(conn: &Connection, artist: &UpsertArtist<'_>) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO artists (band_id, slug, name, url, cover_url)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(band_id) DO UPDATE SET
             slug = excluded.slug,
             name = excluded.name,
             url = excluded.url,
             cover_url = COALESCE(excluded.cover_url, artists.cover_url)",
        params![
            artist.band_id,
            artist.slug,
            artist.name,
            artist.url,
            artist.cover_url
        ],
    )?;
    Ok(())
}

pub fn touch_last_synced(conn: &Connection, band_id: i64) -> Result<Ts, DbError> {
    let ts: Ts = conn.query_row(
        "UPDATE artists SET last_synced_at = unixepoch() WHERE band_id = ?1 RETURNING last_synced_at",
        params![band_id],
        |r| r.get(0),
    )?;
    Ok(ts)
}

pub fn delete(conn: &Connection, band_id: i64) -> Result<(), DbError> {
    conn.execute("DELETE FROM artists WHERE band_id = ?1", params![band_id])?;
    Ok(())
}

pub fn get_view(conn: &Connection, band_id: i64) -> Result<Option<ArtistView>, DbError> {
    let row = conn
        .query_row(
            "SELECT band_id, slug, name, url, last_synced_at FROM artists WHERE band_id = ?1",
            params![band_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<Ts>>(4)?,
                ))
            },
        )
        .optional()?;

    let Some((band_id, slug, name, url, last_synced_at)) = row else {
        return Ok(None);
    };

    let stats = releases::artist_stats_for_one(conn, band_id)?;
    Ok(Some(ArtistView {
        band_id,
        slug,
        name,
        url,
        last_synced_at,
        stats,
    }))
}

pub fn list_views(conn: &Connection) -> Result<Vec<ArtistView>, DbError> {
    let stats_map = releases::all_artist_stats(conn)?;
    let mut stmt = conn.prepare(
        "SELECT band_id, slug, name, url, last_synced_at
         FROM artists
         ORDER BY name COLLATE NOCASE ASC, band_id ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<Ts>>(4)?,
        ))
    })?;

    let mut out = Vec::new();
    for r in rows {
        let (band_id, slug, name, url, last_synced_at) = r?;
        let mut stats = stats_map.get(&band_id).copied().unwrap_or_default();
        stats.quota_used_30d = history::quota_used_30d(conn, band_id)?;
        out.push(ArtistView {
            band_id,
            slug,
            name,
            url,
            last_synced_at,
            stats,
        });
    }
    Ok(out)
}

pub fn stats_with_quota(conn: &Connection, band_id: i64) -> Result<ArtistStats, DbError> {
    let mut stats = releases::artist_stats_for_one(conn, band_id)?;
    stats.quota_used_30d = history::quota_used_30d(conn, band_id)?;
    Ok(stats)
}
