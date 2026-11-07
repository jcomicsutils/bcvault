use crate::{error::DbError, types::Settings};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;

const KEY_APP_SETTINGS: &str = "app_settings";

pub fn get_settings(conn: &Connection, default_out_dir: PathBuf) -> Result<Settings, DbError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![KEY_APP_SETTINGS],
            |r| r.get(0),
        )
        .optional()?;

    match raw {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => {
            let mut s = Settings::default_with_dir(default_out_dir);
            s.batch_limit = s.batch_limit.min(s.quota_hard);
            Ok(s)
        }
    }
}

pub fn set_settings(conn: &Connection, settings: &Settings) -> Result<(), DbError> {
    let mut clamped = settings.clone();
    clamped.batch_limit = clamped.batch_limit.min(clamped.quota_hard);
    let json = serde_json::to_string(&clamped)?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![KEY_APP_SETTINGS, json],
    )?;
    Ok(())
}
