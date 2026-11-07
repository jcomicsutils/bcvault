use crate::{
    error::DbError,
    types::{Classification, ItemType},
};
use rusqlite::{Connection, params};

/// Generates `n` synthetic releases spread across `num_artists` artists for benchmarking and testing.
pub fn seed(conn: &mut Connection, n: usize) -> Result<(), DbError> {
    let num_artists = (n / 100).clamp(1, 200);
    let tx = conn.transaction()?;

    {
        let mut artist_stmt = tx.prepare(
            "INSERT INTO artists (band_id, slug, name, url, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, 1700000000)
             ON CONFLICT(band_id) DO NOTHING",
        )?;

        for a in 1..=num_artists {
            let band_id = a as i64;
            let slug = format!("artist-{a:04}");
            let name = format!("Artist {a:04}");
            let url = format!("https://{slug}.bandcamp.com");
            artist_stmt.execute(params![band_id, slug, name, url])?;
        }
    }

    {
        let mut rel_stmt = tx.prepare(
            "INSERT INTO releases (
                item_type, item_id, band_id, title, artist_name, url,
                classification, is_preorder, track_count, published_at, modified_at,
                downloaded, has_remote_update
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?9, ?10, ?11)
             ON CONFLICT(item_type, item_id) DO NOTHING",
        )?;

        let classes = [
            Classification::Free,
            Classification::Nyp,
            Classification::EmailRequired,
            Classification::Paid,
            Classification::Unknown,
        ];

        for i in 0..n {
            let band_id = ((i % num_artists) + 1) as i64;
            let kind = if i % 4 == 0 {
                ItemType::Track
            } else {
                ItemType::Album
            };
            let item_id = (i + 1) as i64;
            let title = format!("Release Title {i:06}");
            let artist_name = format!("Artist {band_id:04}");
            let url = format!("https://artist-{band_id:04}.bandcamp.com/album/release-{i}");
            let class = classes[i % classes.len()];
            let tracks = if kind == ItemType::Track {
                1
            } else {
                ((i % 12) + 2) as i64
            };
            let pub_ts = 1_600_000_000_i64 + (i as i64) * 60;
            let downloaded = i32::from(i % 5 == 0);
            let has_update = i32::from(downloaded == 1 && i % 25 == 0);

            rel_stmt.execute(params![
                kind.as_str(),
                item_id,
                band_id,
                title,
                artist_name,
                url,
                class.as_str(),
                tracks,
                pub_ts,
                downloaded,
                has_update,
            ])?;
        }
    }

    tx.commit()?;
    Ok(())
}
