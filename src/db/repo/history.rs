use crate::{
    error::DbError,
    types::{AudioFormat, DownloadSource, HistoryStatus, ItemKey},
};
use rusqlite::{Connection, params};

pub struct NewHistoryEntry {
    pub key: ItemKey,
    pub band_id: i64,
    pub format: AudioFormat,
    pub source: DownloadSource,
    pub counts_against_quota: bool,
}

pub fn insert_started(conn: &Connection, entry: &NewHistoryEntry) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO download_history (item_type, item_id, band_id, format, status, source, counts_against_quota)
         VALUES (?1, ?2, ?3, ?4, 'started', ?5, ?6)",
        params![
            entry.key.kind.as_str(),
            entry.key.id,
            entry.band_id,
            entry.format.as_str(),
            entry.source.as_str(),
            i32::from(entry.counts_against_quota),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish(
    conn: &Connection,
    id: i64,
    status: HistoryStatus,
    bytes_size: Option<u64>,
    error_message: Option<&str>,
) -> Result<(), DbError> {
    match status {
        HistoryStatus::Success => {
            conn.execute(
                "UPDATE download_history SET
                     status = 'success',
                     downloaded_at = unixepoch(),
                     bytes_size = ?1,
                     error_message = NULL
                 WHERE id = ?2",
                params![bytes_size.map(|b| b as i64), id],
            )?;
        }
        _ => {
            conn.execute(
                "UPDATE download_history SET
                     status = ?1,
                     bytes_size = ?2,
                     error_message = ?3
                 WHERE id = ?4",
                params![
                    status.as_str(),
                    bytes_size.map(|b| b as i64),
                    error_message,
                    id
                ],
            )?;
        }
    }
    Ok(())
}

pub fn quota_used_30d(conn: &Connection, band_id: i64) -> Result<u32, DbError> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM download_history
         WHERE band_id = ?1 AND counts_against_quota = 1
           AND started_at >= unixepoch('now', '-30 days')",
        params![band_id],
        |r| r.get(0),
    )?;
    Ok(count)
}

pub fn recover_interrupted(conn: &Connection) -> Result<usize, DbError> {
    let updated = conn.execute(
        "UPDATE download_history SET status = 'failed', error_message = 'interrupted' WHERE status = 'started'",
        [],
    )?;
    Ok(updated)
}
