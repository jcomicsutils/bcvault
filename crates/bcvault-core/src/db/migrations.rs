use crate::error::DbError;
use rusqlite::Connection;

pub const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../../migrations/0001_init.sql")),
    (2, include_str!("../../migrations/0002_indexes.sql")),
    (
        3,
        include_str!("../../migrations/0003_custom_artist_name.sql"),
    ),
    (4, include_str!("../../migrations/0004_library.sql")),
    (
        5,
        include_str!("../../migrations/0005_artist_folder_local_only.sql"),
    ),
    (
        6,
        include_str!("../../migrations/0006_artist_folders_multi.sql"),
    ),
    (7, include_str!("../../migrations/0007_unavailable.sql")),
    (
        8,
        include_str!("../../migrations/0008_rejected_matches.sql"),
    ),
];

pub const MAX_VERSION: i32 = 8;

pub fn run(conn: &mut Connection) -> Result<(), DbError> {
    run_with(conn, MIGRATIONS)
}

pub fn run_with(conn: &mut Connection, migrations: &[(i32, &str)]) -> Result<(), DbError> {
    let max_version = migrations.iter().map(|(v, _)| *v).max().unwrap_or(0);
    let current_version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current_version > max_version {
        return Err(DbError::TooNew {
            found: current_version,
            max_supported: max_version,
        });
    }

    for &(version, sql) in migrations {
        if version > current_version {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent_and_reject_too_new() {
        let mut conn = Connection::open_in_memory().unwrap();
        run(&mut conn).unwrap();
        let v: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 8);

        // Re-running is a no-op
        run(&mut conn).unwrap();
        let v2: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 8);

        // Bump to version 99 and verify TooNew
        conn.pragma_update(None, "user_version", 99).unwrap();
        let err = run(&mut conn).unwrap_err();
        assert!(matches!(
            err,
            DbError::TooNew {
                found: 99,
                max_supported: 8
            }
        ));
    }

    #[test]
    fn migration_0004_sets_unverified_and_creates_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Run migrations 1..=3 first
        run_with(&mut conn, &MIGRATIONS[..3]).unwrap();
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'artist1', 'Artist 1', 'https://artist1.bandcamp.com')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, downloaded)
             VALUES ('album', 101, 1, 'Album 1', 'Artist 1', 'https://artist1.bandcamp.com/album/1', 1)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, downloaded)
             VALUES ('album', 102, 1, 'Album 2', 'Artist 1', 'https://artist1.bandcamp.com/album/2', 0)",
            [],
        ).unwrap();

        // Now run migration 4
        run(&mut conn).unwrap();

        let status101: Option<String> = conn
            .query_row(
                "SELECT local_status FROM releases WHERE item_id = 101",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status101.as_deref(), Some("unverified"));

        let status102: Option<String> = conn
            .query_row(
                "SELECT local_status FROM releases WHERE item_id = 102",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status102, None);

        // Check new tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(tables.contains(&"artist_folders".to_string()));
        assert!(tables.contains(&"artist_folder_ignored".to_string()));
        assert!(tables.contains(&"library_batches".to_string()));
        assert!(tables.contains(&"library_events".to_string()));
    }

    #[test]
    fn migration_0005_adds_local_only_count_to_artist_folders() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Run migrations 1..=4 first
        run_with(&mut conn, &MIGRATIONS[..4]).unwrap();
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'artist1', 'Artist 1', 'https://artist1.bandcamp.com')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO artist_folders (band_id, path, path_key, added_at) VALUES (1, '/music/artist1', 'artist1', 1000)",
            [],
        ).unwrap();

        // Run migration 5
        run(&mut conn).unwrap();

        let count: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE band_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        conn.execute(
            "UPDATE artist_folders SET local_only_count = 5 WHERE band_id = 1",
            [],
        )
        .unwrap();
        let updated: u32 = conn
            .query_row(
                "SELECT local_only_count FROM artist_folders WHERE band_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(updated, 5);
    }

    #[test]
    fn migration_0006_upgrades_to_multi_artist_folders() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        // Run migrations 1..=5
        run_with(&mut conn, &MIGRATIONS[..5]).unwrap();

        // Populate 0005 DB with artist, release, folder, ignored row, batch and event
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'artist1', 'Artist 1', 'https://artist1.bandcamp.com')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, downloaded, local_status)
             VALUES ('album', 101, 1, 'Album 1', 'Artist 1', 'https://artist1.bandcamp.com/album/1', 1, 'present')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO artist_folders (band_id, path, path_key, added_at, local_only_count)
             VALUES (1, '/music/a1', 'a1', 1000, 3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artist_folder_ignored (band_id, path_key) VALUES (1, 'a1/extra')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO library_batches (id, kind, summary_json) VALUES (1, 'adopt', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO library_events (id, batch_id, item_type, item_id, kind) VALUES (1, 1, 'album', 101, 'adopted')",
            [],
        ).unwrap();

        // Upgrade to migration 6 (and 7)
        run(&mut conn).unwrap();

        // All rows preserved
        let (id, band_id, path, path_key, local_only_count): (i64, i64, String, String, u32) = conn
            .query_row(
                "SELECT id, band_id, path, path_key, local_only_count FROM artist_folders WHERE band_id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert!(id > 0);
        assert_eq!(band_id, 1);
        assert_eq!(path, "/music/a1");
        assert_eq!(path_key, "a1");
        assert_eq!(local_only_count, 3);

        let ignored_key: String = conn
            .query_row(
                "SELECT path_key FROM artist_folder_ignored WHERE band_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ignored_key, "a1/extra");

        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM library_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(event_count, 1);

        // PRAGMA foreign_key_check must be empty
        let fk_violations: Vec<String> = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(fk_violations.is_empty(), "FK violations: {fk_violations:?}");

        // PRAGMA integrity_check must be 'ok'
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");

        // Inserting two folders for one band works
        conn.execute(
            "INSERT INTO artist_folders (band_id, path, path_key, added_at, local_only_count)
             VALUES (1, '/music/a2', 'a2', 2000, 2)",
            [],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artist_folders WHERE band_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);

        // Deleting the artist cascades folders and ignores
        conn.execute("DELETE FROM artists WHERE band_id = 1", [])
            .unwrap();
        let remaining_folders: i64 = conn
            .query_row("SELECT COUNT(*) FROM artist_folders", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining_folders, 0);

        let remaining_ignores: i64 = conn
            .query_row("SELECT COUNT(*) FROM artist_folder_ignored", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(remaining_ignores, 0);
    }

    #[test]
    fn migration_0007_allows_unavailable_and_updates_zero_track_releases() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        run_with(&mut conn, &MIGRATIONS[..6]).unwrap();

        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'artist1', 'Artist 1', 'https://artist1.bandcamp.com')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, track_count, metadata_fetched_at)
             VALUES ('album', 101, 1, 'Zero Track Album', 'Artist 1', 'https://artist1.bandcamp.com/album/1', 'nyp', 0, 1000),
                    ('album', 102, 1, 'Normal Album', 'Artist 1', 'https://artist1.bandcamp.com/album/2', 'free', 4, 1000)",
            [],
        ).unwrap();

        run(&mut conn).unwrap();

        let class101: String = conn
            .query_row(
                "SELECT classification FROM releases WHERE item_id = 101",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(class101, "unavailable");

        let class102: String = conn
            .query_row(
                "SELECT classification FROM releases WHERE item_id = 102",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(class102, "free");

        // Manual update to unavailable works
        conn.execute(
            "UPDATE releases SET classification = 'unavailable' WHERE item_id = 102",
            [],
        )
        .unwrap();
    }

    #[test]
    fn migration_0008_creates_artist_folder_rejected_with_cascades() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        run_with(&mut conn, &MIGRATIONS[..7]).unwrap();

        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'artist1', 'Artist 1', 'https://artist1.bandcamp.com')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, track_count, metadata_fetched_at)
             VALUES ('album', 201, 1, 'Split', 'Artist 1', 'https://artist1.bandcamp.com/album/split-1', 'free', 4, 1000)",
            [],
        ).unwrap();

        run(&mut conn).unwrap();

        conn.execute(
            "INSERT INTO artist_folder_rejected (band_id, item_type, item_id, path_key)
             VALUES (1, 'album', 201, '/music/artist1/wall split action 001')",
            [],
        ).unwrap();

        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM artist_folder_rejected WHERE band_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 1);

        // Deleting the release cascades to artist_folder_rejected
        conn.execute("DELETE FROM releases WHERE item_id = 201", []).unwrap();
        let cnt_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM artist_folder_rejected WHERE band_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt_after, 0);
    }
}
