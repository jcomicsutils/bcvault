pub mod migrations;
pub mod repo;
pub mod seed;

use crate::error::DbError;
use rusqlite::Connection;
use std::path::Path;

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
        let path_buf = path.to_path_buf();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<(), DbError>>(1);
        let (tx, rx) = std::sync::mpsc::channel::<Job>();

        let thread_name = match role {
            Role::Writer => "bcvault-db-writer",
            Role::Reader => "bcvault-db-reader",
        };

        let spawn_res = std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(move || {
                let mut conn = match Self::init_conn(&path_buf, role) {
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

    fn init_conn(path: &Path, role: Role) -> Result<Connection, DbError> {
        let mut conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA temp_store   = MEMORY;
             PRAGMA cache_size   = -20000;",
        )?;

        match role {
            Role::Writer => {
                migrations::run(&mut conn)?;
                repo::releases::reclassify_from_pricing_json(&mut conn)?;
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
