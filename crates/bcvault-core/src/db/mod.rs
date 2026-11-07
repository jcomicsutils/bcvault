pub mod migrations;
pub mod repo;
pub mod seed;

use crate::error::DbError;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Writer,
    Reader,
}

#[derive(Clone)]
pub struct Db {
    tx: std::sync::mpsc::Sender<Job>,
}

impl Db {
    /// Spawns the owning thread, applies PRAGMAs for the role, then loops over jobs.
    pub fn open(path: &Path, role: Role) -> Result<Self, DbError> {
        Self::open_with(path, role, "", &[], migrations::MIGRATIONS, true)
    }

    pub fn open_with(
        path: &Path,
        role: Role,
        extra_pragmas: &str,
        attach: &[(PathBuf, String)],
        migrations: &'static [(i32, &'static str)],
        reclassify_main: bool,
    ) -> Result<Self, DbError> {
        let path_buf = path.to_path_buf();
        let extra = extra_pragmas.to_string();
        let attach_owned = attach.to_vec();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<(), DbError>>(1);
        let (tx, rx) = std::sync::mpsc::channel::<Job>();

        let thread_name = match role {
            Role::Writer => "bcvault-db-writer",
            Role::Reader => "bcvault-db-reader",
        };

        let spawn_res = std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(move || {
                let mut conn = match Self::init_conn(
                    &path_buf,
                    role,
                    &extra,
                    &attach_owned,
                    migrations,
                    reclassify_main,
                ) {
                    Ok(c) => {
                        let _ = init_tx.send(Ok(()));
                        c
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
            });

        if spawn_res.is_err() {
            return Err(DbError::Closed);
        }

        init_rx.recv().map_err(|_| DbError::Closed)??;
        Ok(Self { tx })
    }

    fn init_conn(
        path: &Path,
        role: Role,
        extra_pragmas: &str,
        attach: &[(PathBuf, String)],
        migrations: &[(i32, &str)],
        reclassify_main: bool,
    ) -> Result<Connection, DbError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn = Connection::open_with_flags(path, flags)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA temp_store   = MEMORY;
             PRAGMA cache_size   = -20000;",
        )?;

        for (attach_path, schema_alias) in attach {
            let escaped = attach_path.to_string_lossy().replace('\'', "''");
            let uri = format!("file:{escaped}?mode=ro");
            conn.execute(
                "ATTACH DATABASE ?1 AS ?2",
                rusqlite::params![uri, schema_alias],
            )?;
        }

        if !extra_pragmas.trim().is_empty() {
            conn.execute_batch(extra_pragmas)?;
        }

        match role {
            Role::Writer => {
                migrations::run_with(&mut conn, migrations)?;
                if reclassify_main {
                    repo::releases::reclassify_from_pricing_json(&mut conn)?;
                }
            }
            Role::Reader => {
                conn.execute_batch("PRAGMA query_only = ON;")?;
            }
        }

        Ok(conn)
    }

    pub async fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<R, DbError> + Send + 'static,
    ) -> Result<R, DbError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |c| {
                let _ = tx.send(f(c));
            }))
            .map_err(|_| DbError::Closed)?;
        rx.await.map_err(|_| DbError::Closed)?
    }
}

#[derive(Clone)]
pub struct DbPool {
    pub writer: Db,
    pub reader: Db,
}

impl DbPool {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let writer = Db::open(path, Role::Writer)?;
        let reader = Db::open(path, Role::Reader)?;
        Ok(Self { writer, reader })
    }
}
