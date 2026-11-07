use crate::{
    error::{DbError, LinkFolderError},
    types::{ArtistFolder, FolderState},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

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
}
