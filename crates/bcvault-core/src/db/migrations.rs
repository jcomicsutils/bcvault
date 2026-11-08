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
];

pub const MAX_VERSION: i32 = 5;

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
        assert_eq!(v, 5);

        // Re-running is a no-op
        run(&mut conn).unwrap();
        let v2: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 5);

        // Bump to version 99 and verify TooNew
        conn.pragma_update(None, "user_version", 99).unwrap();
        let err = run(&mut conn).unwrap_err();
        assert!(matches!(
            err,
            DbError::TooNew {
                found: 99,
                max_supported: 5
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
}
