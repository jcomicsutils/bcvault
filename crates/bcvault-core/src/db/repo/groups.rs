use crate::error::DbError;
use crate::groups::{
    ArtistGroup, GroupError, GroupRule, MAX_GROUPS, PinTarget, deserialize_rule_json,
    serialize_rule_json, validate_group_name,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use tracing::warn;

pub fn list_groups(conn: &Connection) -> Result<Vec<ArtistGroup>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, pos, collapsed, rule_json
         FROM artist_groups
         ORDER BY pos ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let pos: i32 = row.get(2)?;
        let collapsed: i32 = row.get(3)?;
        let rule_json: Option<String> = row.get(4)?;
        Ok((id, name, pos, collapsed != 0, rule_json))
    })?;

    let mut groups = Vec::new();
    for row in rows {
        let (id, name, pos, collapsed, rule_json) = row?;
        let rule = match rule_json {
            Some(json) => match deserialize_rule_json(&json) {
                Ok(r) => Some(r),
                Err(err) => {
                    warn!(group_id = id, "ignoring invalid group rule_json: {}", err);
                    None
                }
            },
            None => None,
        };
        groups.push(ArtistGroup {
            id,
            name,
            pos,
            collapsed,
            rule,
        });
    }
    Ok(groups)
}

pub fn list_pins(conn: &Connection) -> Result<HashMap<i64, Option<i64>>, DbError> {
    let mut stmt = conn.prepare("SELECT band_id, group_id FROM artist_group_pins")?;
    let rows = stmt.query_map([], |row| {
        let band_id: i64 = row.get(0)?;
        let group_id: Option<i64> = row.get(1)?;
        Ok((band_id, group_id))
    })?;
    let mut pins = HashMap::new();
    for row in rows {
        let (band_id, group_id) = row?;
        pins.insert(band_id, group_id);
    }
    Ok(pins)
}

pub fn create_group(
    conn: &Connection,
    name: &str,
    rule: Option<&GroupRule>,
) -> Result<ArtistGroup, DbError> {
    let name = validate_group_name(name)?;
    if let Some(r) = rule {
        r.validate()?;
    }

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM artist_groups", [], |r| r.get(0))?;
    if count >= MAX_GROUPS as i64 {
        return Err(GroupError::TooManyGroups { max: MAX_GROUPS }.into());
    }

    let duplicate: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_groups WHERE name = ?1",
            params![&name],
            |r| r.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(GroupError::DuplicateName(name).into());
    }

    let rule_json = rule.map(serialize_rule_json).transpose()?;
    let pos: i32 = conn.query_row(
        "SELECT COALESCE(MAX(pos) + 1, 0) FROM artist_groups",
        [],
        |r| r.get(0),
    )?;

    conn.execute(
        "INSERT INTO artist_groups (name, pos, collapsed, rule_json) VALUES (?1, ?2, 0, ?3)",
        params![&name, pos, rule_json],
    )?;
    let id = conn.last_insert_rowid();

    Ok(ArtistGroup {
        id,
        name,
        pos,
        collapsed: false,
        rule: rule.cloned(),
    })
}

pub fn update_group(
    conn: &Connection,
    id: i64,
    name: &str,
    rule: Option<&GroupRule>,
) -> Result<ArtistGroup, DbError> {
    let name = validate_group_name(name)?;
    if let Some(r) = rule {
        r.validate()?;
    }

    let existing: Option<(i32, i32)> = conn
        .query_row(
            "SELECT pos, collapsed FROM artist_groups WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((pos, collapsed)) = existing else {
        return Err(GroupError::GroupNotFound(id).into());
    };

    let duplicate: Option<i64> = conn
        .query_row(
            "SELECT id FROM artist_groups WHERE name = ?1 AND id != ?2",
            params![&name, id],
            |r| r.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(GroupError::DuplicateName(name).into());
    }

    let rule_json = rule.map(serialize_rule_json).transpose()?;
    conn.execute(
        "UPDATE artist_groups SET name = ?1, rule_json = ?2 WHERE id = ?3",
        params![&name, rule_json, id],
    )?;

    Ok(ArtistGroup {
        id,
        name,
        pos,
        collapsed: collapsed != 0,
        rule: rule.cloned(),
    })
}

pub fn delete_group(conn: &Connection, id: i64) -> Result<(), DbError> {
    let changed = conn.execute("DELETE FROM artist_groups WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(GroupError::GroupNotFound(id).into());
    }
    Ok(())
}

pub fn reorder_groups(conn: &mut Connection, ids: &[i64]) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    let mut existing_ids = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT id FROM artist_groups ORDER BY pos ASC, id ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        for row in rows {
            existing_ids.push(row?);
        }
    }

    let mut next_pos: i32 = 0;
    let mut seen = Vec::with_capacity(existing_ids.len());
    for &id in ids {
        if existing_ids.contains(&id) && !seen.contains(&id) {
            tx.execute(
                "UPDATE artist_groups SET pos = ?1 WHERE id = ?2",
                params![next_pos, id],
            )?;
            next_pos += 1;
            seen.push(id);
        }
    }
    for id in existing_ids {
        if !seen.contains(&id) {
            tx.execute(
                "UPDATE artist_groups SET pos = ?1 WHERE id = ?2",
                params![next_pos, id],
            )?;
            next_pos += 1;
        }
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

pub fn set_pins(conn: &mut Connection, band_ids: &[i64], target: PinTarget) -> Result<(), DbError> {
    if band_ids.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    match target {
        PinTarget::Auto => {
            let mut stmt = tx.prepare("DELETE FROM artist_group_pins WHERE band_id = ?1")?;
            for &band_id in band_ids {
                stmt.execute(params![band_id])?;
            }
        }
        PinTarget::Ungrouped => {
            let mut stmt = tx.prepare(
                "INSERT INTO artist_group_pins (band_id, group_id)
                 VALUES (?1, NULL)
                 ON CONFLICT(band_id) DO UPDATE SET group_id = NULL",
            )?;
            for &band_id in band_ids {
                stmt.execute(params![band_id])?;
            }
        }
        PinTarget::Group(gid) => {
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT id FROM artist_groups WHERE id = ?1",
                    params![gid],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(GroupError::GroupNotFound(gid).into());
            }
            let mut stmt = tx.prepare(
                "INSERT INTO artist_group_pins (band_id, group_id)
                 VALUES (?1, ?2)
                 ON CONFLICT(band_id) DO UPDATE SET group_id = excluded.group_id",
            )?;
            for &band_id in band_ids {
                stmt.execute(params![band_id, gid])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::migrations::{MIGRATIONS, run, run_with};
    use crate::groups::{Condition, ConditionKind, MatchMode};

    fn setup_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run(&mut conn).unwrap();
        conn
    }

    fn insert_artist(conn: &Connection, band_id: i64, name: &str) {
        conn.execute(
            "INSERT INTO artists (band_id, slug, name, url) VALUES (?1, ?2, ?3, ?4)",
            params![
                band_id,
                format!("slug-{band_id}"),
                name,
                format!("https://slug-{band_id}.bandcamp.com")
            ],
        )
        .unwrap();
    }

    // Test 7: Migration on a populated database
    #[test]
    fn test_7_migration_on_populated_db_and_fk_check() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_with(&mut conn, &MIGRATIONS[..8]).unwrap();

        insert_artist(&conn, 42, "Existing Artist");
        conn.execute(
            "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url)
             VALUES ('album', 100, 42, 'Existing Release', 'Existing Artist', 'https://slug-42.bandcamp.com/album/a')",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let artist_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0))
            .unwrap();
        assert_eq!(artist_count, 1);

        let fk_violations: Vec<String> = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(fk_violations.is_empty());
        assert!(list_groups(&conn).unwrap().is_empty());
        assert!(list_pins(&conn).unwrap().is_empty());
    }

    // Test 8: Cascades and case-insensitive uniqueness
    #[test]
    fn test_8_cascades_and_case_insensitive_unique_names() {
        let mut conn = setup_db();
        insert_artist(&conn, 1, "Artist One");
        insert_artist(&conn, 2, "Artist Two");

        let g = create_group(&conn, "Ambient", None).unwrap();
        let dup = create_group(&conn, "ambient", None).unwrap_err();
        assert!(matches!(dup, DbError::Group(GroupError::DuplicateName(_))));

        set_pins(&mut conn, &[1, 2], PinTarget::Group(g.id)).unwrap();
        assert_eq!(list_pins(&conn).unwrap().len(), 2);

        // Deleting artist removes its pin
        conn.execute("DELETE FROM artists WHERE band_id = 1", [])
            .unwrap();
        assert_eq!(list_pins(&conn).unwrap().len(), 1);
        assert!(!list_pins(&conn).unwrap().contains_key(&1));

        // Deleting group removes remaining pin via ON DELETE CASCADE
        delete_group(&conn, g.id).unwrap();
        assert!(list_pins(&conn).unwrap().is_empty());
    }

    // Test 9: Bad rule_json loads with rule = None and no panic
    #[test]
    fn test_9_bad_rule_json_loads_as_none() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO artist_groups (name, pos, collapsed, rule_json) VALUES ('Broken', 0, 0, '{not valid json')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artist_groups (name, pos, collapsed, rule_json) VALUES ('Future', 1, 0, '{\"v\":99,\"mode\":\"All\",\"conditions\":[]}')",
            [],
        )
        .unwrap();

        let groups = list_groups(&conn).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rule, None);
        assert_eq!(groups[1].rule, None);
    }

    // Test 10: Validation, 64-group cap, reorder, collapsed, set_pins for all three targets
    #[test]
    fn test_10_repo_validation_reorder_collapsed_and_pins() {
        let mut conn = setup_db();
        insert_artist(&conn, 1, "A");
        insert_artist(&conn, 2, "B");

        // Name length validation
        assert!(matches!(
            create_group(&conn, "   ", None),
            Err(DbError::Group(GroupError::EmptyName))
        ));
        let long_name = "a".repeat(41);
        assert!(matches!(
            create_group(&conn, &long_name, None),
            Err(DbError::Group(GroupError::NameTooLong { max: 40 }))
        ));

        let rule = GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind: ConditionKind::NeverSynced,
            }],
        };

        let g1 = create_group(&conn, "First", Some(&rule)).unwrap();
        let g2 = create_group(&conn, "Second", None).unwrap();

        // Update
        let updated = update_group(&conn, g2.id, "Renamed", Some(&rule)).unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.rule, Some(rule));

        // Reorder
        reorder_groups(&mut conn, &[g2.id, g1.id]).unwrap();
        let listed = list_groups(&conn).unwrap();
        assert_eq!(listed[0].id, g2.id);
        assert_eq!(listed[1].id, g1.id);

        // Collapsed
        set_collapsed(&conn, g2.id, true).unwrap();
        assert!(list_groups(&conn).unwrap()[0].collapsed);

        // Pins for all three targets
        set_pins(&mut conn, &[1, 2], PinTarget::Group(g1.id)).unwrap();
        let pins = list_pins(&conn).unwrap();
        assert_eq!(pins.get(&1), Some(&Some(g1.id)));
        assert_eq!(pins.get(&2), Some(&Some(g1.id)));

        set_pins(&mut conn, &[1], PinTarget::Ungrouped).unwrap();
        assert_eq!(list_pins(&conn).unwrap().get(&1), Some(&None));

        set_pins(&mut conn, &[1], PinTarget::Auto).unwrap();
        assert!(!list_pins(&conn).unwrap().contains_key(&1));

        // 64-group cap (already have 2)
        for i in 2..64 {
            create_group(&conn, &format!("G{i}"), None).unwrap();
        }
        assert!(matches!(
            create_group(&conn, "Overflow", None),
            Err(DbError::Group(GroupError::TooManyGroups { max: 64 }))
        ));
    }
}
