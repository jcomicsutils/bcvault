use crate::{
    error::{DbError, LinkFolderError},
    types::{
        ArtistFolder, AudioFormat, BatchSummary, FolderState, ItemKey, ItemType, LocalStatus,
        Origin, Ts,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub fn normalize_path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

pub fn probe_folder_state(path: &Path) -> FolderState {
    if !path.exists() {
        FolderState::Missing
    } else if std::fs::read_dir(path).is_err() {
        FolderState::Unreadable
    } else {
        FolderState::Ok
    }
}

pub fn link_folder(
    conn: &mut Connection,
    band_id: i64,
    path: &Path,
    out_dir: &Path,
) -> Result<ArtistFolder, LinkFolderError> {
    if !path.exists() {
        return Err(LinkFolderError::NotFound(path.to_path_buf()));
    }
    if !path.is_dir() {
        return Err(LinkFolderError::NotADirectory(path.to_path_buf()));
    }
    if std::fs::read_dir(path).is_err() {
        return Err(LinkFolderError::Unreadable(path.to_path_buf()));
    }

    let canon = path
        .canonicalize()
        .map_err(|_| LinkFolderError::NotFound(path.to_path_buf()))?;
    let path_key = normalize_path_key(&canon);

    // Validate against out_dir
    if let Ok(out_canon) = out_dir.canonicalize() {
        let out_key = normalize_path_key(&out_canon);
        if path_key == out_key || canon.starts_with(&out_canon) || out_canon.starts_with(&canon) {
            return Err(LinkFolderError::OverlapsOutDir(canon));
        }
    } else {
        let out_key = normalize_path_key(out_dir);
        if path_key == out_key || canon.starts_with(out_dir) || out_dir.starts_with(&canon) {
            return Err(LinkFolderError::OverlapsOutDir(canon));
        }
    }

    // Validate against existing artist folders (excluding this artist)
    {
        let mut stmt = conn.prepare("SELECT band_id, path, path_key FROM artist_folders")?;
        let existing_rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;

        for row in existing_rows {
            let (other_band_id, other_path_str, other_key) = row?;
            if other_band_id == band_id {
                continue; // Re-linking the same artist is allowed (replaces link)
            }
            let other_canon = PathBuf::from(&other_path_str);
            if path_key == other_key
                || canon == other_canon
                || canon.starts_with(&other_canon)
                || other_canon.starts_with(&canon)
            {
                return Err(LinkFolderError::OverlapsExisting {
                    band_id: other_band_id,
                    path: other_canon,
                });
            }
        }
    }

    let tx = conn.transaction()?;
    let path_str = canon.to_string_lossy().into_owned();
    tx.execute(
        "INSERT INTO artist_folders (band_id, path, path_key, added_at)
         VALUES (?1, ?2, ?3, unixepoch())
         ON CONFLICT(band_id) DO UPDATE SET
             path = excluded.path,
             path_key = excluded.path_key,
             added_at = unixepoch()",
        params![band_id, path_str, path_key],
    )?;
    tx.commit()?;

    Ok(ArtistFolder {
        band_id,
        path: canon,
        state: FolderState::Ok,
    })
}

pub fn unlink_folder(conn: &Connection, band_id: i64) -> Result<bool, DbError> {
    let affected = conn.execute(
        "DELETE FROM artist_folders WHERE band_id = ?1",
        params![band_id],
    )?;
    Ok(affected > 0)
}

pub fn get_folder(conn: &Connection, band_id: i64) -> Result<Option<ArtistFolder>, DbError> {
    let path_opt: Option<String> = conn
        .query_row(
            "SELECT path FROM artist_folders WHERE band_id = ?1",
            params![band_id],
            |r| r.get(0),
        )
        .optional()?;

    Ok(path_opt.map(|p| {
        let path = PathBuf::from(p);
        let state = probe_folder_state(&path);
        ArtistFolder {
            band_id,
            path,
            state,
        }
    }))
}

pub fn all_folders(conn: &Connection) -> Result<Vec<ArtistFolder>, DbError> {
    let mut stmt = conn.prepare("SELECT band_id, path FROM artist_folders ORDER BY band_id")?;
    let rows = stmt.query_map([], |r| {
        let band_id: i64 = r.get(0)?;
        let path_str: String = r.get(1)?;
        let path = PathBuf::from(path_str);
        let state = probe_folder_state(&path);
        Ok(ArtistFolder {
            band_id,
            path,
            state,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseSnapshot {
    pub downloaded: bool,
    pub downloaded_at: Option<Ts>,
    pub downloaded_format: Option<AudioFormat>,
    pub downloaded_track_count: Option<u32>,
    pub downloaded_modified_at: Option<Ts>,
    pub local_path: Option<PathBuf>,
    pub local_status: Option<LocalStatus>,
    pub local_checked_at: Option<Ts>,
    pub missing_ack_at: Option<Ts>,
    pub origin: Option<Origin>,
}

pub fn get_release_snapshot(
    conn: &Connection,
    key: ItemKey,
) -> Result<Option<ReleaseSnapshot>, DbError> {
    conn.query_row(
        "SELECT downloaded, downloaded_at, downloaded_format, downloaded_track_count,
                downloaded_modified_at, local_path, local_status, local_checked_at,
                missing_ack_at, origin
         FROM releases WHERE item_type = ?1 AND item_id = ?2",
        params![key.kind.as_str(), key.id],
        |r| {
            let downloaded: i64 = r.get(0)?;
            let downloaded_at: Option<Ts> = r.get(1)?;
            let fmt_str: Option<String> = r.get(2)?;
            let downloaded_format = fmt_str.as_deref().and_then(AudioFormat::parse);
            let downloaded_track_count: Option<u32> = r.get(3)?;
            let downloaded_modified_at: Option<Ts> = r.get(4)?;
            let local_path: Option<PathBuf> = r.get::<_, Option<String>>(5)?.map(PathBuf::from);
            let status_str: Option<String> = r.get(6)?;
            let local_status = status_str.as_deref().and_then(LocalStatus::parse);
            let local_checked_at: Option<Ts> = r.get(7)?;
            let missing_ack_at: Option<Ts> = r.get(8)?;
            let origin_str: Option<String> = r.get(9)?;
            let origin = origin_str.as_deref().and_then(Origin::parse);

            Ok(ReleaseSnapshot {
                downloaded: downloaded != 0,
                downloaded_at,
                downloaded_format,
                downloaded_track_count,
                downloaded_modified_at,
                local_path,
                local_status,
                local_checked_at,
                missing_ack_at,
                origin,
            })
        },
    )
    .optional()
    .map_err(DbError::from)
}

fn apply_release_snapshot(
    conn: &Connection,
    key: ItemKey,
    snap: &ReleaseSnapshot,
) -> Result<(), DbError> {
    // Invariant check: downloaded = 0 => local_status IS NULL
    let status_str = if snap.downloaded {
        snap.local_status.map(|s| s.as_str())
    } else {
        None
    };
    conn.execute(
        "UPDATE releases SET
             downloaded = ?1,
             downloaded_at = ?2,
             downloaded_format = ?3,
             downloaded_track_count = ?4,
             downloaded_modified_at = ?5,
             local_path = ?6,
             local_status = ?7,
             local_checked_at = ?8,
             missing_ack_at = ?9,
             origin = ?10
         WHERE item_type = ?11 AND item_id = ?12",
        params![
            if snap.downloaded { 1 } else { 0 },
            snap.downloaded_at,
            snap.downloaded_format.map(|f| f.as_str()),
            snap.downloaded_track_count,
            snap.downloaded_modified_at,
            snap.local_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            status_str,
            snap.local_checked_at,
            snap.missing_ack_at,
            snap.origin.map(|o| o.as_str()),
            key.kind.as_str(),
            key.id,
        ],
    )?;
    Ok(())
}

fn record_library_event(
    conn: &Connection,
    batch_id: i64,
    key: ItemKey,
    kind: &str,
    old_snap: Option<&ReleaseSnapshot>,
    new_snap: Option<&ReleaseSnapshot>,
) -> Result<(), DbError> {
    let old_json = match old_snap {
        Some(s) => Some(serde_json::to_string(s)?),
        None => None,
    };
    let new_json = match new_snap {
        Some(s) => Some(serde_json::to_string(s)?),
        None => None,
    };
    conn.execute(
        "INSERT INTO library_events (batch_id, item_type, item_id, kind, old_json, new_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            batch_id,
            key.kind.as_str(),
            key.id,
            kind,
            old_json,
            new_json,
        ],
    )?;
    Ok(())
}

pub fn set_local_status(
    conn: &Connection,
    key: ItemKey,
    status: Option<LocalStatus>,
) -> Result<(), DbError> {
    if let Some(st) = status {
        conn.execute(
            "UPDATE releases SET local_status = ?3, local_checked_at = unixepoch()
             WHERE item_type = ?1 AND item_id = ?2 AND downloaded = 1",
            params![key.kind.as_str(), key.id, st.as_str()],
        )?;
    } else {
        conn.execute(
            "UPDATE releases SET local_status = NULL, local_checked_at = unixepoch()
             WHERE item_type = ?1 AND item_id = ?2",
            params![key.kind.as_str(), key.id],
        )?;
    }
    Ok(())
}

pub fn skip_missing(conn: &Connection, key: ItemKey) -> Result<(), DbError> {
    conn.execute(
        "UPDATE releases SET
             local_status = 'missing',
             missing_ack_at = unixepoch(),
             local_checked_at = unixepoch()
         WHERE item_type = ?1 AND item_id = ?2",
        params![key.kind.as_str(), key.id],
    )?;
    Ok(())
}

pub fn adopt(
    conn: &mut Connection,
    key: ItemKey,
    path: &Path,
    format: Option<AudioFormat>,
    origin: Origin,
    folder_mtime: Option<Ts>,
    batch_id: i64,
) -> Result<(), DbError> {
    let old_snap = get_release_snapshot(conn, key)?
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;

    let path_str = path.to_string_lossy().into_owned();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE releases SET
             downloaded = 1,
             downloaded_at = COALESCE(?3, unixepoch()),
             downloaded_format = COALESCE(?4, downloaded_format),
             downloaded_track_count = track_count,
             downloaded_modified_at = modified_at,
             local_path = ?5,
             local_status = 'present',
             local_checked_at = unixepoch(),
             origin = ?6,
             has_remote_update = 0,
             remote_update_reason = NULL
         WHERE item_type = ?1 AND item_id = ?2",
        params![
            key.kind.as_str(),
            key.id,
            folder_mtime,
            format.map(|f| f.as_str()),
            path_str,
            origin.as_str(),
        ],
    )?;

    let new_snap = get_release_snapshot(&tx, key)?
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;

    record_library_event(
        &tx,
        batch_id,
        key,
        "adopted",
        Some(&old_snap),
        Some(&new_snap),
    )?;
    tx.commit()?;
    Ok(())
}

pub fn relink(
    conn: &mut Connection,
    key: ItemKey,
    new_path: &Path,
    batch_id: i64,
) -> Result<(), DbError> {
    let old_snap = get_release_snapshot(conn, key)?
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;

    let path_str = new_path.to_string_lossy().into_owned();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE releases SET
             local_path = ?3,
             local_status = 'present',
             local_checked_at = unixepoch()
         WHERE item_type = ?1 AND item_id = ?2 AND downloaded = 1",
        params![key.kind.as_str(), key.id, path_str],
    )?;

    let new_snap = get_release_snapshot(&tx, key)?
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;

    record_library_event(
        &tx,
        batch_id,
        key,
        "relinked",
        Some(&old_snap),
        Some(&new_snap),
    )?;
    tx.commit()?;
    Ok(())
}

pub fn archive(conn: &mut Connection, keys: &[ItemKey], batch_id: i64) -> Result<(), DbError> {
    let mut old_snaps = Vec::with_capacity(keys.len());
    for &k in keys {
        if let Some(snap) = get_release_snapshot(conn, k)? {
            old_snaps.push((k, snap));
        }
    }

    let tx = conn.transaction()?;
    for (key, old_snap) in old_snaps {
        tx.execute(
            "UPDATE releases SET
                 local_status = 'archived',
                 local_checked_at = unixepoch()
             WHERE item_type = ?1 AND item_id = ?2 AND downloaded = 1",
            params![key.kind.as_str(), key.id],
        )?;
        let new_snap = get_release_snapshot(&tx, key)?
            .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;
        record_library_event(
            &tx,
            batch_id,
            key,
            "archived",
            Some(&old_snap),
            Some(&new_snap),
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn unmark(conn: &mut Connection, keys: &[ItemKey], batch_id: i64) -> Result<(), DbError> {
    let mut old_snaps = Vec::with_capacity(keys.len());
    for &k in keys {
        if let Some(snap) = get_release_snapshot(conn, k)? {
            old_snaps.push((k, snap));
        }
    }

    let tx = conn.transaction()?;
    for (key, old_snap) in old_snaps {
        tx.execute(
            "UPDATE releases SET
                 downloaded = 0,
                 downloaded_at = NULL,
                 downloaded_format = NULL,
                 downloaded_track_count = NULL,
                 downloaded_modified_at = NULL,
                 local_path = NULL,
                 local_status = NULL,
                 local_checked_at = NULL,
                 missing_ack_at = NULL,
                 origin = NULL
             WHERE item_type = ?1 AND item_id = ?2",
            params![key.kind.as_str(), key.id],
        )?;
        let new_snap = get_release_snapshot(&tx, key)?
            .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;
        record_library_event(
            &tx,
            batch_id,
            key,
            "unmarked",
            Some(&old_snap),
            Some(&new_snap),
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn restore(
    conn: &mut Connection,
    key: ItemKey,
    path: &Path,
    batch_id: i64,
) -> Result<(), DbError> {
    let old_snap = get_release_snapshot(conn, key)?
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;

    let path_str = path.to_string_lossy().into_owned();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE releases SET
             downloaded = 1,
             local_path = ?3,
             local_status = 'present',
             local_checked_at = unixepoch()
         WHERE item_type = ?1 AND item_id = ?2",
        params![key.kind.as_str(), key.id, path_str],
    )?;

    let new_snap = get_release_snapshot(&tx, key)?
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;
    record_library_event(
        &tx,
        batch_id,
        key,
        "restored",
        Some(&old_snap),
        Some(&new_snap),
    )?;
    tx.commit()?;
    Ok(())
}

pub fn ignore_local(conn: &Connection, band_id: i64, path_key: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR IGNORE INTO artist_folder_ignored (band_id, path_key) VALUES (?1, ?2)",
        params![band_id, path_key],
    )?;
    Ok(())
}

pub fn unignore_local(conn: &Connection, band_id: i64, path_key: &str) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM artist_folder_ignored WHERE band_id = ?1 AND path_key = ?2",
        params![band_id, path_key],
    )?;
    Ok(())
}

pub fn get_ignored_local(conn: &Connection, band_id: i64) -> Result<HashSet<String>, DbError> {
    let mut stmt = conn.prepare("SELECT path_key FROM artist_folder_ignored WHERE band_id = ?1")?;
    let rows = stmt.query_map(params![band_id], |r| r.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for r in rows {
        set.insert(r?);
    }
    Ok(set)
}

pub fn create_batch(conn: &Connection, kind: &str, summary: &BatchSummary) -> Result<i64, DbError> {
    let summary_json = serde_json::to_string(summary)?;
    conn.execute(
        "INSERT INTO library_batches (kind, summary_json, ts) VALUES (?1, ?2, unixepoch())",
        params![kind, summary_json],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_batch_summary(
    conn: &Connection,
    batch_id: i64,
    summary: &BatchSummary,
) -> Result<(), DbError> {
    let summary_json = serde_json::to_string(summary)?;
    conn.execute(
        "UPDATE library_batches SET summary_json = ?2 WHERE id = ?1",
        params![batch_id, summary_json],
    )?;
    Ok(())
}

pub fn get_batch_summary(
    conn: &Connection,
    batch_id: i64,
) -> Result<Option<BatchSummary>, DbError> {
    let summary_opt: Option<String> = conn
        .query_row(
            "SELECT summary_json FROM library_batches WHERE id = ?1",
            params![batch_id],
            |r| r.get(0),
        )
        .optional()?;

    match summary_opt {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

pub fn undo_batch(conn: &mut Connection, batch_id: i64) -> Result<BatchSummary, DbError> {
    let (undone_at, summary_json): (Option<Ts>, String) = conn
        .query_row(
            "SELECT undone_at, summary_json FROM library_batches WHERE id = ?1",
            params![batch_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;

    if undone_at.is_some() {
        return Ok(serde_json::from_str(&summary_json)?);
    }

    let summary: BatchSummary = serde_json::from_str(&summary_json)?;

    // Fetch events in reverse order
    let events = {
        let mut stmt = conn.prepare(
            "SELECT item_type, item_id, kind, old_json FROM library_events
             WHERE batch_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![batch_id], |r| {
            let t_str: String = r.get(0)?;
            let kind = ItemType::parse(&t_str).unwrap_or(ItemType::Album);
            let id: i64 = r.get(1)?;
            let ev_kind: String = r.get(2)?;
            let old_json: Option<String> = r.get(3)?;
            Ok((ItemKey { kind, id }, ev_kind, old_json))
        })?;
        let mut evs = Vec::new();
        for r in rows {
            evs.push(r?);
        }
        evs
    };

    let tx = conn.transaction()?;
    for (key, _ev_kind, old_json) in events {
        if let Some(json_str) = old_json {
            let snap: ReleaseSnapshot = serde_json::from_str(&json_str)?;
            apply_release_snapshot(&tx, key, &snap)?;
        }
    }

    tx.execute(
        "UPDATE library_batches SET undone_at = unixepoch() WHERE id = ?1",
        params![batch_id],
    )?;
    tx.commit()?;

    Ok(summary)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LibraryTimelineEvent {
    pub id: i64,
    pub batch_id: i64,
    pub ts: i64,
    pub kind: String,
    pub old_json: Option<String>,
    pub new_json: Option<String>,
}

pub fn get_item_timeline(
    conn: &Connection,
    key: ItemKey,
) -> Result<Vec<LibraryTimelineEvent>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT le.id, le.batch_id, lb.ts, le.kind, le.old_json, le.new_json
         FROM library_events le
         JOIN library_batches lb ON lb.id = le.batch_id
         WHERE le.item_type = ?1 AND le.item_id = ?2
         ORDER BY le.id ASC",
    )?;
    let rows = stmt.query_map(params![key.kind.as_str(), key.id], |r| {
        Ok(LibraryTimelineEvent {
            id: r.get(0)?,
            batch_id: r.get(1)?,
            ts: r.get(2)?,
            kind: r.get(3)?,
            old_json: r.get(4)?,
            new_json: r.get(5)?,
        })
    })?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

pub fn assert_library_invariants(conn: &Connection) -> Result<(), DbError> {
    let violated: i64 = conn.query_row(
        "SELECT COUNT(*) FROM releases WHERE downloaded = 0 AND local_status IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    if violated != 0 {
        return Err(DbError::SchemaMismatch(format!(
            "Invariant violation: {violated} releases have downloaded = 0 but local_status IS NOT NULL"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::migrations;

    fn test_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'art1', 'Artist 1', 'https://art1.bandcamp.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (2, 'art2', 'Artist 2', 'https://art2.bandcamp.com')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn link_folder_validates_existence_and_directory() {
        let mut conn = test_db();
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("downloads");
        std::fs::create_dir(&out_dir).unwrap();

        let bogus = tmp.path().join("nonexistent");
        let err = link_folder(&mut conn, 1, &bogus, &out_dir).unwrap_err();
        assert!(matches!(err, LinkFolderError::NotFound(_)));

        let file = tmp.path().join("file.txt");
        std::fs::write(&file, "hello").unwrap();
        let err = link_folder(&mut conn, 1, &file, &out_dir).unwrap_err();
        assert!(matches!(err, LinkFolderError::NotADirectory(_)));
    }

    #[test]
    fn link_folder_rejects_out_dir_overlap() {
        let mut conn = test_db();
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("downloads");
        std::fs::create_dir(&out_dir).unwrap();

        // Exact equal
        let err = link_folder(&mut conn, 1, &out_dir, &out_dir).unwrap_err();
        assert!(matches!(err, LinkFolderError::OverlapsOutDir(_)));

        // Folder inside out_dir
        let sub = out_dir.join("artist1");
        std::fs::create_dir(&sub).unwrap();
        let err = link_folder(&mut conn, 1, &sub, &out_dir).unwrap_err();
        assert!(matches!(err, LinkFolderError::OverlapsOutDir(_)));

        // Folder containing out_dir
        let err = link_folder(&mut conn, 1, tmp.path(), &out_dir).unwrap_err();
        assert!(matches!(err, LinkFolderError::OverlapsOutDir(_)));
    }

    #[test]
    fn link_folder_rejects_overlap_with_other_artist_folder() {
        let mut conn = test_db();
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("downloads");
        std::fs::create_dir(&out_dir).unwrap();

        let music = tmp.path().join("music");
        let art1_dir = music.join("art1");
        std::fs::create_dir_all(&art1_dir).unwrap();

        // Link art1
        let f1 = link_folder(&mut conn, 1, &art1_dir, &out_dir).unwrap();
        assert_eq!(f1.band_id, 1);

        // Try link art2 with exact same folder
        let err = link_folder(&mut conn, 2, &art1_dir, &out_dir).unwrap_err();
        assert!(matches!(
            err,
            LinkFolderError::OverlapsExisting { band_id: 1, .. }
        ));

        // Try link art2 with parent folder containing art1_dir
        let err = link_folder(&mut conn, 2, &music, &out_dir).unwrap_err();
        assert!(matches!(
            err,
            LinkFolderError::OverlapsExisting { band_id: 1, .. }
        ));

        // Try link art2 with subfolder inside art1_dir
        let sub = art1_dir.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let err = link_folder(&mut conn, 2, &sub, &out_dir).unwrap_err();
        assert!(matches!(
            err,
            LinkFolderError::OverlapsExisting { band_id: 1, .. }
        ));

        // Re-linking art1 with a new folder is allowed (replaces link)
        let art1_new = music.join("art1_new");
        std::fs::create_dir(&art1_new).unwrap();
        let f1_new = link_folder(&mut conn, 1, &art1_new, &out_dir).unwrap();
        assert_eq!(f1_new.path, art1_new.canonicalize().unwrap());
    }

    #[test]
    fn unlinking_leaves_release_marks_intact() {
        let mut conn = test_db();
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("downloads");
        std::fs::create_dir(&out_dir).unwrap();

        let art_dir = tmp.path().join("artist1");
        std::fs::create_dir(&art_dir).unwrap();
        link_folder(&mut conn, 1, &art_dir, &out_dir).unwrap();

        // Insert release with downloaded marks
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, downloaded, local_path, local_status)
             VALUES ('album', 10, 1, 'Alb', 'Art', 'https://art1.bandcamp.com/album/a', 1, '/some/path', 'present')",
            [],
        )
        .unwrap();

        assert!(unlink_folder(&conn, 1).unwrap());
        assert!(get_folder(&conn, 1).unwrap().is_none());

        // Marks must stay intact
        let (dl, lp, ls): (i64, String, String) = conn
            .query_row(
                "SELECT downloaded, local_path, local_status FROM releases WHERE item_id = 10",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(dl, 1);
        assert_eq!(lp, "/some/path");
        assert_eq!(ls, "present");
    }

    #[test]
    fn batches_events_undo_and_invariants() {
        let mut conn = test_db();
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, downloaded, track_count, modified_at)
             VALUES ('album', 100, 1, 'Album 100', 'Artist 1', 'https://art1.bandcamp.com/album/100', 0, 10, 1700000000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, downloaded, track_count, modified_at)
             VALUES ('album', 200, 1, 'Album 200', 'Artist 1', 'https://art1.bandcamp.com/album/200', 0, 8, 1700000000)",
            [],
        ).unwrap();

        // Initial invariant check: downloaded = 0 => local_status IS NULL
        assert_library_invariants(&conn).unwrap();

        // 1. Adopt release 100 in a batch
        let summary1 = BatchSummary {
            adopted: 1,
            ..Default::default()
        };
        let b1 = create_batch(&conn, "adopt", &summary1).unwrap();
        let key100 = ItemKey::new(ItemType::Album, 100);
        let path100 = Path::new("/music/art1/album100");
        adopt(
            &mut conn,
            key100,
            path100,
            Some(AudioFormat::Flac),
            Origin::Imported,
            Some(1700005000),
            b1,
        )
        .unwrap();

        assert_library_invariants(&conn).unwrap();
        let snap100 = get_release_snapshot(&conn, key100).unwrap().unwrap();
        assert!(snap100.downloaded);
        assert_eq!(snap100.local_status, Some(LocalStatus::Present));
        assert_eq!(snap100.downloaded_format, Some(AudioFormat::Flac));
        assert_eq!(snap100.origin, Some(Origin::Imported));
        assert_eq!(snap100.downloaded_track_count, Some(10));

        // 2. Relink release 100
        let b2 = create_batch(
            &conn,
            "relink",
            &BatchSummary {
                relinked: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let path100_moved = Path::new("/music/art1/album100_moved");
        relink(&mut conn, key100, path100_moved, b2).unwrap();
        let snap_relinked = get_release_snapshot(&conn, key100).unwrap().unwrap();
        assert_eq!(
            snap_relinked.local_path,
            Some(PathBuf::from("/music/art1/album100_moved"))
        );
        assert_library_invariants(&conn).unwrap();

        // 3. Archive release 100
        let b3 = create_batch(
            &conn,
            "resolve",
            &BatchSummary {
                archived: 1,
                ..Default::default()
            },
        )
        .unwrap();
        archive(&mut conn, &[key100], b3).unwrap();
        let snap_archived = get_release_snapshot(&conn, key100).unwrap().unwrap();
        assert_eq!(snap_archived.local_status, Some(LocalStatus::Archived));
        assert!(snap_archived.downloaded); // Still downloaded!
        assert_library_invariants(&conn).unwrap();

        // 4. Undo archive batch (b3)
        let undid3 = undo_batch(&mut conn, b3).unwrap();
        assert_eq!(undid3.archived, 1);
        let snap_unarchived = get_release_snapshot(&conn, key100).unwrap().unwrap();
        assert_eq!(snap_unarchived.local_status, Some(LocalStatus::Present));
        assert_library_invariants(&conn).unwrap();

        // 5. Unmark release 100
        let b4 = create_batch(
            &conn,
            "resolve",
            &BatchSummary {
                unmarked: 1,
                ..Default::default()
            },
        )
        .unwrap();
        unmark(&mut conn, &[key100], b4).unwrap();
        let snap_unmarked = get_release_snapshot(&conn, key100).unwrap().unwrap();
        assert!(!snap_unmarked.downloaded);
        assert_eq!(snap_unmarked.local_status, None);
        assert_eq!(snap_unmarked.local_path, None);
        assert_library_invariants(&conn).unwrap();

        // 6. Undo unmark (b4)
        undo_batch(&mut conn, b4).unwrap();
        let snap_restored = get_release_snapshot(&conn, key100).unwrap().unwrap();
        assert!(snap_restored.downloaded);
        assert_eq!(snap_restored.local_status, Some(LocalStatus::Present));
        assert_library_invariants(&conn).unwrap();

        // Check timeline events for key100
        let timeline = get_item_timeline(&conn, key100).unwrap();
        assert!(!timeline.is_empty());
        assert_eq!(timeline[0].kind, "adopted");

        // 7. Undo adopt (b1) - restores release 100 back to downloaded = 0
        undo_batch(&mut conn, b1).unwrap();
        let snap_init = get_release_snapshot(&conn, key100).unwrap().unwrap();
        assert!(!snap_init.downloaded);
        assert_eq!(snap_init.local_status, None);
        assert_library_invariants(&conn).unwrap();

        // 8. Test ignore_local and unignore_local (requires linked folder)
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("downloads");
        std::fs::create_dir(&out_dir).unwrap();
        let art_dir = tmp.path().join("artist1");
        std::fs::create_dir(&art_dir).unwrap();
        link_folder(&mut conn, 1, &art_dir, &out_dir).unwrap();

        ignore_local(&conn, 1, "orphan_folder").unwrap();
        let ignored = get_ignored_local(&conn, 1).unwrap();
        assert!(ignored.contains("orphan_folder"));
        unignore_local(&conn, 1, "orphan_folder").unwrap();
        let ignored2 = get_ignored_local(&conn, 1).unwrap();
        assert!(!ignored2.contains("orphan_folder"));

        // 9. set_local_status
        set_local_status(&conn, key100, Some(LocalStatus::Present)).unwrap();
        // key100 has downloaded = 0, so set_local_status Some does not set local_status
        assert_library_invariants(&conn).unwrap();
    }
}
