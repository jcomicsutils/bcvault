use crate::error::DbError;
use rusqlite::Connection;

pub const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../../migrations/0001_init.sql")),
    (2, include_str!("../../migrations/0002_indexes.sql")),
];

pub const MAX_VERSION: i32 = 2;

pub fn run(conn: &mut Connection) -> Result<(), DbError> {
    let current_version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current_version > MAX_VERSION {
        return Err(DbError::TooNew {
            found: current_version,
            max_supported: MAX_VERSION,
        });
    }

    for &(version, sql) in MIGRATIONS {
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
        assert_eq!(v, 2);

        // Re-running is a no-op
        run(&mut conn).unwrap();
        let v2: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 2);

        // Bump to version 99 and verify TooNew
        conn.pragma_update(None, "user_version", 99).unwrap();
        let err = run(&mut conn).unwrap_err();
        assert!(matches!(
            err,
            DbError::TooNew {
                found: 99,
                max_supported: 2
            }
        ));
    }
}
