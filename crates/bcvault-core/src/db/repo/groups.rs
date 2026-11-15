use crate::error::DbError;
use crate::groups::{
    ActiveGrouping, ArtistGroup, ArtistGroupSet, GroupError, MAX_GROUP_SETS, MAX_GROUPS, PinTarget,
    validate_group_name,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use tracing::warn;

const ACTIVE_GROUPING_KEY: &str = "active_artist_grouping";

pub fn get_active_grouping(conn: &Connection) -> Result<ActiveGrouping, DbError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![ACTIVE_GROUPING_KEY],
            |r| r.get(0),
        )
        .optional()?;
    let Some(json) = raw else {
        return Ok(ActiveGrouping::None);
    };
    match serde_json::from_str::<ActiveGrouping>(&json) {
        Ok(ActiveGrouping::CustomSet(set_id)) => {
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT id FROM artist_group_sets WHERE id = ?1",
                    params![set_id],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_some() {
                Ok(ActiveGrouping::CustomSet(set_id))
            } else {
                Ok(ActiveGrouping::None)
            }
        }
        Ok(other) => Ok(other),
        Err(err) => {
            warn!("ignoring invalid active_artist_grouping setting: {}", err);
            Ok(ActiveGrouping::None)
        }
    }
}

pub fn set_active_grouping(conn: &Connection, grouping: ActiveGrouping) -> Result<(), DbError> {
    if let ActiveGrouping::CustomSet(set_id) = grouping {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM artist_group_sets WHERE id = ?1",
                params![set_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(GroupError::GroupSetNotFound(set_id).into());
        }
    }
    let json = serde_json::to_string(&grouping)?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![ACTIVE_GROUPING_KEY, json],
    )?;
    Ok(())
}

pub fn list_group_sets(conn: &Connection) -> Result<Vec<ArtistGroupSet>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, pos
         FROM artist_group_sets
         ORDER BY pos ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ArtistGroupSet {
            id: row.get(0)?,
            name: row.get(1)?,
            pos: row.get(2)?,
        })
    })?;
    let mut sets = Vec::new();
    for row in rows {
        sets.push(row?);
    }
    Ok(sets)
}

pub fn create_group_set(conn: &Connection, name: &str) -> Result<ArtistGroupSet, DbError> {
    let name = validate_group_name(name)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM artist_group_sets", [], |r| r.get(0))?;
    if count >= MAX_GROUP_SETS as i64 {
        return Err(GroupError::TooManyGroupSets {
            max: MAX_GROUP_SETS,
        }
        .into());
    }
    let duplicate: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_group_sets WHERE name = ?1",
            params![&name],
            |r| r.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(GroupError::DuplicateSetName(name).into());
    }
    let pos: i32 = conn.query_row(
        "SELECT COALESCE(MAX(pos) + 1, 0) FROM artist_group_sets",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO artist_group_sets (name, pos) VALUES (?1, ?2)",
        params![&name, pos],
    )?;
    let id = conn.last_insert_rowid();
    Ok(ArtistGroupSet { id, name, pos })
}

pub fn rename_group_set(conn: &Connection, set_id: i64, name: &str) -> Result<(), DbError> {
    let name = validate_group_name(name)?;
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_group_sets WHERE id = ?1",
            params![set_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(GroupError::GroupSetNotFound(set_id).into());
    }
    let duplicate: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_group_sets WHERE name = ?1 AND id != ?2",
            params![&name, set_id],
            |r| r.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(GroupError::DuplicateSetName(name).into());
    }
    conn.execute(
        "UPDATE artist_group_sets SET name = ?1 WHERE id = ?2",
        params![&name, set_id],
    )?;
    Ok(())
}

pub fn delete_group_set(conn: &Connection, set_id: i64) -> Result<(), DbError> {
    let changed = conn.execute(
        "DELETE FROM artist_group_sets WHERE id = ?1",
        params![set_id],
    )?;
    if changed == 0 {
        return Err(GroupError::GroupSetNotFound(set_id).into());
    }
    if matches!(get_active_grouping(conn)?, ActiveGrouping::None) {
        // ActiveGrouping::CustomSet(set_id) now resolves to None; persist None cleanly.
        let _ = set_active_grouping(conn, ActiveGrouping::None);
    }
    Ok(())
}

pub fn list_groups(conn: &Connection, set_id: i64) -> Result<Vec<ArtistGroup>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, set_id, name, pos, collapsed
         FROM artist_groups
         WHERE set_id = ?1
         ORDER BY pos ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![set_id], |row| {
        let collapsed: i32 = row.get(4)?;
        Ok(ArtistGroup {
            id: row.get(0)?,
            set_id: row.get(1)?,
            name: row.get(2)?,
            pos: row.get(3)?,
            collapsed: collapsed != 0,
        })
    })?;

    let mut groups = Vec::new();
    for row in rows {
        groups.push(row?);
    }
    Ok(groups)
}

pub fn list_pins(conn: &Connection, set_id: i64) -> Result<HashMap<i64, i64>, DbError> {
    let mut stmt =
        conn.prepare("SELECT band_id, group_id FROM artist_group_pins WHERE set_id = ?1")?;
    let rows = stmt.query_map(params![set_id], |row| {
        let band_id: i64 = row.get(0)?;
        let group_id: i64 = row.get(1)?;
        Ok((band_id, group_id))
    })?;
    let mut pins = HashMap::new();
    for row in rows {
        let (band_id, group_id) = row?;
        pins.insert(band_id, group_id);
    }
    Ok(pins)
}

pub fn create_group(conn: &Connection, set_id: i64, name: &str) -> Result<ArtistGroup, DbError> {
    let set_exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_group_sets WHERE id = ?1",
            params![set_id],
            |r| r.get(0),
        )
        .optional()?;
    if set_exists.is_none() {
        return Err(GroupError::GroupSetNotFound(set_id).into());
    }

    let name = validate_group_name(name)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM artist_groups WHERE set_id = ?1",
        params![set_id],
        |r| r.get(0),
    )?;
    if count >= MAX_GROUPS as i64 {
        return Err(GroupError::TooManyGroups { max: MAX_GROUPS }.into());
    }

    let duplicate: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_groups WHERE set_id = ?1 AND name = ?2",
            params![set_id, &name],
            |r| r.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(GroupError::DuplicateName(name).into());
    }

    let pos: i32 = conn.query_row(
        "SELECT COALESCE(MAX(pos) + 1, 0) FROM artist_groups WHERE set_id = ?1",
        params![set_id],
        |r| r.get(0),
    )?;

    conn.execute(
        "INSERT INTO artist_groups (set_id, name, pos, collapsed) VALUES (?1, ?2, ?3, 0)",
        params![set_id, &name, pos],
    )?;
    let id = conn.last_insert_rowid();
    Ok(ArtistGroup {
        id,
        set_id,
        name,
        pos,
        collapsed: false,
    })
}

pub fn update_group(conn: &Connection, id: i64, name: &str) -> Result<i64, DbError> {
    let set_id: Option<i64> = conn
        .query_row(
            "SELECT set_id FROM artist_groups WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(set_id) = set_id else {
        return Err(GroupError::GroupNotFound(id).into());
    };

    let name = validate_group_name(name)?;
    let duplicate: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_groups WHERE set_id = ?1 AND name = ?2 AND id != ?3",
            params![set_id, &name, id],
            |r| r.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(GroupError::DuplicateName(name).into());
    }

    conn.execute(
        "UPDATE artist_groups SET name = ?1 WHERE id = ?2",
        params![&name, id],
    )?;
    Ok(set_id)
}

pub fn delete_group(conn: &Connection, id: i64) -> Result<(), DbError> {
    let changed = conn.execute("DELETE FROM artist_groups WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(GroupError::GroupNotFound(id).into());
    }
    Ok(())
}

pub fn reorder_groups(conn: &mut Connection, set_id: i64, ids: &[i64]) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    for (pos, &id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE artist_groups SET pos = ?1 WHERE id = ?2 AND set_id = ?3",
            params![pos as i32, id, set_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn set_collapsed(conn: &Connection, id: i64, collapsed: bool) -> Result<(), DbError> {
    let changed = conn.execute(
        "UPDATE artist_groups SET collapsed = ?1 WHERE id = ?2",
        params![i32::from(collapsed), id],
    )?;
    if changed == 0 {
        return Err(GroupError::GroupNotFound(id).into());
    }
    Ok(())
}

pub fn set_pins(
    conn: &mut Connection,
    set_id: i64,
    band_ids: &[i64],
    target: PinTarget,
) -> Result<(), DbError> {
    if band_ids.is_empty() {
        return Ok(());
    }
    if let PinTarget::Group(gid) = target {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM artist_groups WHERE id = ?1 AND set_id = ?2",
                params![gid, set_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(GroupError::GroupNotFound(gid).into());
        }
    }

    let tx = conn.transaction()?;
    for &band_id in band_ids {
        match target {
            PinTarget::Group(gid) => {
                tx.execute(
                    "INSERT INTO artist_group_pins (set_id, band_id, group_id) VALUES (?1, ?2, ?3)
                     ON CONFLICT(set_id, band_id) DO UPDATE SET group_id = excluded.group_id",
                    params![set_id, band_id, gid],
                )?;
            }
            PinTarget::Ungrouped => {
                tx.execute(
                    "DELETE FROM artist_group_pins WHERE set_id = ?1 AND band_id = ?2",
                    params![set_id, band_id],
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn replace_group_members(
    conn: &mut Connection,
    set_id: i64,
    group_id: i64,
    member_band_ids: &[i64],
) -> Result<(), DbError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_groups WHERE id = ?1 AND set_id = ?2",
            params![group_id, set_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(GroupError::GroupNotFound(group_id).into());
    }

    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM artist_group_pins WHERE set_id = ?1 AND group_id = ?2",
        params![set_id, group_id],
    )?;
    for &band_id in member_band_ids {
        tx.execute(
            "INSERT INTO artist_group_pins (set_id, band_id, group_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(set_id, band_id) DO UPDATE SET group_id = excluded.group_id",
            params![set_id, band_id, group_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use crate::groups::BuiltinGrouping;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES
             (1, 'a1', 'Artist 1', 'https://a1.bandcamp.com'),
             (2, 'a2', 'Artist 2', 'https://a2.bandcamp.com')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_group_sets_non_destructive_switching_and_cascades() {
        let mut conn = setup();
        assert_eq!(get_active_grouping(&conn).unwrap(), ActiveGrouping::None);

        let s1 = create_group_set(&conn, "Workflow").unwrap();
        let s2 = create_group_set(&conn, "Labels").unwrap();
        let g1 = create_group(&conn, s1.id, "Finished").unwrap();
        let g2 = create_group(&conn, s2.id, "Finished").unwrap(); // same name allowed in different set

        set_pins(&mut conn, s1.id, &[1], PinTarget::Group(g1.id)).unwrap();
        set_pins(&mut conn, s2.id, &[2], PinTarget::Group(g2.id)).unwrap();

        set_active_grouping(&conn, ActiveGrouping::CustomSet(s1.id)).unwrap();
        assert_eq!(
            get_active_grouping(&conn).unwrap(),
            ActiveGrouping::CustomSet(s1.id)
        );
        set_active_grouping(&conn, ActiveGrouping::Builtin(BuiltinGrouping::FirstLetter)).unwrap();
        assert_eq!(
            get_active_grouping(&conn).unwrap(),
            ActiveGrouping::Builtin(BuiltinGrouping::FirstLetter)
        );

        // Pins and groups in s1 and s2 remain intact
        assert_eq!(
            list_pins(&conn, s1.id).unwrap(),
            HashMap::from([(1, g1.id)])
        );
        assert_eq!(
            list_pins(&conn, s2.id).unwrap(),
            HashMap::from([(2, g2.id)])
        );

        // replace_group_members swaps members cleanly
        replace_group_members(&mut conn, s1.id, g1.id, &[2]).unwrap();
        assert_eq!(
            list_pins(&conn, s1.id).unwrap(),
            HashMap::from([(2, g1.id)])
        );

        // Deleting s1 cascades its groups and pins without touching s2
        delete_group_set(&conn, s1.id).unwrap();
        assert!(list_groups(&conn, s1.id).unwrap().is_empty());
        assert_eq!(list_groups(&conn, s2.id).unwrap().len(), 1);
    }
}
