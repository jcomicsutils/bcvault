#![allow(clippy::unwrap_used, clippy::expect_used)]

use bcvault_core::{
    db::{
        Db, DbPool, Role,
        repo::{
            artists::{self, UpsertArtist},
            history::{self, NewHistoryEntry},
            releases::{self, DiscoveredRelease, ReleaseMetadata},
            settings,
        },
        seed,
    },
    error::DbError,
    types::{
        AudioFormat, Classification, DownloadSource, HistoryStatus, ItemKey, ItemType,
        ReleaseFilter, ReleaseQuery, Settings, SortDir, SortKey, UpdateReason,
    },
};
use rusqlite::params;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[tokio::test]
async fn fresh_db_pragmas_and_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    let pool = DbPool::open(&db_path).unwrap();

    let (uv, jm, fk): (i32, String, i32) = pool
        .writer
        .call(|c| {
            let uv = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let jm: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
            let fk = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
            Ok((uv, jm, fk))
        })
        .await
        .unwrap();

    assert_eq!(uv, 6);
    assert_eq!(jm.to_lowercase(), "wal");
    assert_eq!(fk, 1);

    let (r_jm, r_fk, r_qo): (String, i32, i32) = pool
        .reader
        .call(|c| {
            let jm: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
            let fk = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
            let qo = c.query_row("PRAGMA query_only", [], |r| r.get(0))?;
            Ok((jm, fk, qo))
        })
        .await
        .unwrap();

    assert_eq!(r_jm.to_lowercase(), "wal");
    assert_eq!(r_fk, 1);
    assert_eq!(r_qo, 1);

    // Reopening is a no-op
    let pool2 = DbPool::open(&db_path).unwrap();
    let uv2: i32 = pool2
        .writer
        .call(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(uv2, 6);

    // Bump user_version to 99 and verify TooNew on reopen
    pool2
        .writer
        .call(|c| {
            c.pragma_update(None, "user_version", 99)?;
            Ok(())
        })
        .await
        .unwrap();
    drop(pool);
    drop(pool2);

    let err = Db::open(&db_path, Role::Writer).err().unwrap();
    assert!(matches!(
        err,
        DbError::TooNew {
            found: 99,
            max_supported: 6
        }
    ));
}

#[tokio::test]
async fn writer_transaction_does_not_block_reader() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("concurrency.db");
    let pool = DbPool::open(&db_path).unwrap();

    pool.writer
        .call(|c| {
            seed::seed(c, 500)?;
            Ok(())
        })
        .await
        .unwrap();

    let writer = pool.writer.clone();
    let writer_task = tokio::spawn(async move {
        writer
            .call(|c| {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO artists (band_id, slug, name, url) VALUES (99999, 'slow', 'Slow', 'https://slow.bandcamp.com')",
                    [],
                )?;
                std::thread::sleep(Duration::from_millis(500));
                tx.commit()?;
                Ok(())
            })
            .await
            .unwrap();
    });

    // Wait a moment for the writer transaction to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    let t0 = Instant::now();
    let rows = pool
        .reader
        .call(|c| {
            releases::query_releases(
                c,
                &ReleaseQuery {
                    band_id: Some(1),
                    ..Default::default()
                },
            )
        })
        .await
        .unwrap();
    let elapsed = t0.elapsed();

    assert!(!rows.is_empty());
    assert!(
        elapsed < Duration::from_millis(50),
        "Reader took {elapsed:?} while writer held transaction"
    );

    writer_task.await.unwrap();
}

#[tokio::test]
async fn discovery_upsert_and_remote_update_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("lifecycle.db");
    let pool = DbPool::open(&db_path).unwrap();

    let key = ItemKey::new(ItemType::Album, 1001);

    pool.writer
        .call(move |c| {
            artists::upsert(
                c,
                &UpsertArtist {
                    band_id: 42,
                    slug: "testband",
                    name: "Test Band",
                    url: "https://testband.bandcamp.com",
                    cover_url: None,
                },
            )?;

            releases::upsert_discovered(
                c,
                &DiscoveredRelease {
                    key,
                    band_id: 42,
                    title: "Initial Title",
                    artist_name: "Test Band",
                    url: "https://testband.bandcamp.com/album/initial",
                    cover_url: Some("https://f4.bcbits.com/img/a1_10.jpg"),
                    art_id: Some(1),
                },
            )?;

            releases::update_metadata(
                c,
                &ReleaseMetadata {
                    key,
                    classification: Classification::Free,
                    pricing_json: Some(r#"{"minimum_price":0.0}"#),
                    is_preorder: false,
                    track_count: Some(5),
                    published_at: Some(1700000000),
                    modified_at: Some(1700000100),
                    art_id: Some(1),
                },
            )?;

            let hist_id = history::insert_started(
                c,
                &NewHistoryEntry {
                    key,
                    band_id: 42,
                    format: AudioFormat::Flac,
                    source: DownloadSource::Api,
                    counts_against_quota: true,
                },
            )?;

            releases::commit_download(
                c,
                key,
                AudioFormat::Flac,
                &PathBuf::from("/tmp/album.zip"),
                123456,
                hist_id,
            )?;

            // Discovery upsert on a downloaded row leaves classification, downloaded*, and local_path untouched
            releases::upsert_discovered(
                c,
                &DiscoveredRelease {
                    key,
                    band_id: 42,
                    title: "Updated Grid Title",
                    artist_name: "Test Band",
                    url: "https://testband.bandcamp.com/album/initial",
                    cover_url: None,
                    art_id: None,
                },
            )?;

            let row = releases::get_release(c, key)?.unwrap();
            assert_eq!(row.title, "Updated Grid Title");
            assert_eq!(row.classification, Classification::Free);
            assert!(row.downloaded);
            assert_eq!(row.downloaded_format, Some(AudioFormat::Flac));
            assert_eq!(row.local_path, Some(PathBuf::from("/tmp/album.zip")));
            assert!(!row.has_remote_update);
            assert_eq!(row.remote_update_reason, None);

            // Raising track_count sets has_remote_update = 1 with tracks_added
            releases::update_metadata(
                c,
                &ReleaseMetadata {
                    key,
                    classification: Classification::Free,
                    pricing_json: Some(r#"{"minimum_price":0.0}"#),
                    is_preorder: false,
                    track_count: Some(7),
                    published_at: Some(1700000000),
                    modified_at: Some(1700000200),
                    art_id: Some(1),
                },
            )?;

            let row2 = releases::get_release(c, key)?.unwrap();
            assert!(row2.has_remote_update);
            assert_eq!(row2.remote_update_reason, Some(UpdateReason::TracksAdded));

            // commit_download clears has_remote_update
            let hist_id2 = history::insert_started(
                c,
                &NewHistoryEntry {
                    key,
                    band_id: 42,
                    format: AudioFormat::Flac,
                    source: DownloadSource::Api,
                    counts_against_quota: true,
                },
            )?;
            releases::commit_download(
                c,
                key,
                AudioFormat::Flac,
                &PathBuf::from("/tmp/album_v2.zip"),
                234567,
                hist_id2,
            )?;

            let row3 = releases::get_release(c, key)?.unwrap();
            assert!(!row3.has_remote_update);
            assert_eq!(row3.remote_update_reason, None);

            // Manual mark/unmark downloaded
            releases::set_downloaded_manual(c, key, false)?;
            let row4 = releases::get_release(c, key)?.unwrap();
            assert!(!row4.downloaded);
            assert_eq!(row4.local_path, None);

            releases::set_downloaded_manual(c, key, true)?;
            let row5 = releases::get_release(c, key)?.unwrap();
            assert!(row5.downloaded);

            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn quota_30d_boundary_and_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("quota.db");
    let pool = DbPool::open(&db_path).unwrap();

    pool.writer
        .call(|c| {
            let key = ItemKey::new(ItemType::Track, 1);

            // 1. Recent API download (counts)
            let id1 = history::insert_started(
                c,
                &NewHistoryEntry {
                    key,
                    band_id: 7,
                    format: AudioFormat::Mp3_320,
                    source: DownloadSource::Api,
                    counts_against_quota: true,
                },
            )?;
            history::finish(c, id1, HistoryStatus::Success, Some(1000), None)?;

            // 2. Redeem download (counts_against_quota = 0, ignored)
            let _id2 = history::insert_started(
                c,
                &NewHistoryEntry {
                    key,
                    band_id: 7,
                    format: AudioFormat::Flac,
                    source: DownloadSource::Redeem,
                    counts_against_quota: false,
                },
            )?;

            // 3. Old API download (31 days ago, ignored)
            c.execute(
                "INSERT INTO download_history (item_type, item_id, band_id, started_at, format, status, source, counts_against_quota)
                 VALUES ('track', 2, 7, unixepoch('now', '-31 days'), 'flac', 'success', 'api', 1)",
                [],
            )?;

            // 4. Started row that gets recovered on boot (still counted toward quota because buy_for_nothing was called)
            let _id4 = history::insert_started(
                c,
                &NewHistoryEntry {
                    key,
                    band_id: 7,
                    format: AudioFormat::Flac,
                    source: DownloadSource::Api,
                    counts_against_quota: true,
                },
            )?;

            let used = history::quota_used_30d(c, 7)?;
            assert_eq!(used, 2);

            let recovered = history::recover_interrupted(c)?;
            assert_eq!(recovered, 2); // id2 and id4 were 'started'

            // Verify settings persistence
            let def_s = settings::get_settings(c, PathBuf::from("/tmp/dl"))?;
            assert_eq!(def_s.out_dir, PathBuf::from("/tmp/dl"));
            let mut custom = Settings::default_with_dir(PathBuf::from("/custom/out"));
            custom.batch_limit = 500; // should clamp to quota_hard (200)
            settings::set_settings(c, &custom)?;
            let loaded = settings::get_settings(c, PathBuf::from("/tmp/dl"))?;
            assert_eq!(loaded.out_dir, PathBuf::from("/custom/out"));
            assert_eq!(loaded.batch_limit, 200);

            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn explain_query_plan_and_100k_seed_performance() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("perf.db");
    let pool = DbPool::open(&db_path).unwrap();

    // EXPLAIN QUERY PLAN assertions
    pool.reader
        .call(|c| {
            let (rel_sql, _) = releases::build_query_sql(&ReleaseQuery {
                band_id: Some(1),
                filter: ReleaseFilter::All,
                search: String::new(),
                sort: SortKey::Released,
                dir: SortDir::Desc,
            });
            let explain_rel = format!("EXPLAIN QUERY PLAN {rel_sql}");
            let mut stmt = c.prepare(&explain_rel)?;
            let details: Vec<String> = stmt
                .query_map(params![1_i64], |r| r.get::<_, String>(3))?
                .collect::<Result<Vec<_>, _>>()?;
            let plan_text = details.join(" | ");
            assert!(
                plan_text.contains("idx_releases_band_pub"),
                "Expected idx_releases_band_pub in plan: {plan_text}"
            );
            assert!(
                !plan_text.contains("SCAN releases"),
                "Unexpected SCAN releases in plan: {plan_text}"
            );

            let explain_quota = "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM download_history \
                WHERE band_id = ?1 AND counts_against_quota = 1 AND started_at >= unixepoch('now', '-30 days')";
            let mut qstmt = c.prepare(explain_quota)?;
            let qdetails: Vec<String> = qstmt
                .query_map(params![1_i64], |r| r.get::<_, String>(3))?
                .collect::<Result<Vec<_>, _>>()?;
            let qplan_text = qdetails.join(" | ");
            assert!(
                qplan_text.contains("idx_hist_quota"),
                "Expected idx_hist_quota in plan: {qplan_text}"
            );
            assert!(
                !qplan_text.contains("SCAN download_history"),
                "Unexpected SCAN download_history in plan: {qplan_text}"
            );

            // Verify query plan for library indexes idx_releases_archived and idx_releases_attention
            let explain_arch =
                "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM releases WHERE band_id = ?1 AND local_status = 'archived'";
            let mut arch_stmt = c.prepare(explain_arch)?;
            let arch_details: Vec<String> = arch_stmt
                .query_map(params![1_i64], |r| r.get::<_, String>(3))?
                .collect::<Result<Vec<_>, _>>()?;
            let arch_plan_text = arch_details.join(" | ");
            assert!(
                arch_plan_text.contains("idx_releases_archived"),
                "Expected idx_releases_archived in plan: {arch_plan_text}"
            );
            assert!(
                !arch_plan_text.contains("SCAN releases"),
                "Unexpected SCAN releases in plan: {arch_plan_text}"
            );

            let explain_miss =
                "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM releases WHERE band_id = ?1 AND local_status = 'missing'";
            let mut miss_stmt = c.prepare(explain_miss)?;
            let miss_details: Vec<String> = miss_stmt
                .query_map(params![1_i64], |r| r.get::<_, String>(3))?
                .collect::<Result<Vec<_>, _>>()?;
            let miss_plan_text = miss_details.join(" | ");
            assert!(
                miss_plan_text.contains("idx_releases_attention"),
                "Expected idx_releases_attention in plan: {miss_plan_text}"
            );
            assert!(
                !miss_plan_text.contains("SCAN releases"),
                "Unexpected SCAN releases in plan: {miss_plan_text}"
            );

            Ok(())
        })
        .await
        .unwrap();

    // Seed 100k rows and benchmark per-artist query and stats query
    pool.writer
        .call(|c| {
            seed::seed(c, 100_000)?;
            Ok(())
        })
        .await
        .unwrap();

    pool.reader
        .call(|c| {
            let t0 = Instant::now();
            let rows = releases::query_releases(
                c,
                &ReleaseQuery {
                    band_id: Some(1),
                    filter: ReleaseFilter::All,
                    search: String::new(),
                    sort: SortKey::Released,
                    dir: SortDir::Desc,
                },
            )?;
            let q_elapsed = t0.elapsed();
            assert_eq!(rows.len(), 500);
            assert!(
                q_elapsed < Duration::from_millis(100),
                "100k per-artist query took {q_elapsed:?} (limit 100ms)"
            );

            let t1 = Instant::now();
            let stats = releases::all_artist_stats(c)?;
            let s_elapsed = t1.elapsed();
            assert_eq!(stats.len(), 200);
            assert!(
                s_elapsed < Duration::from_millis(250),
                "100k all_artist_stats query took {s_elapsed:?} (limit 250ms)"
            );

            Ok(())
        })
        .await
        .unwrap();
}
