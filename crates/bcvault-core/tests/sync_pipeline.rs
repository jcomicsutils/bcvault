#![allow(clippy::unwrap_used, clippy::expect_used)]

use bcvault_core::{
    bandcamp::{
        BandcampClient, discography::parse_discography_html,
        download_api::parse_buy_for_nothing_json, gate::BandcampGate, redeem::parse_redeem_html,
        tralbum::parse_release_html,
    },
    db::{
        DbPool,
        repo::{
            artists,
            history::{self, NewHistoryEntry},
            library as lib_repo, releases,
        },
    },
    error::BcError,
    types::{
        AudioFormat, Classification, DownloadSource, ItemKey, ItemType, LocalStatus,
        RateLimitStatus, ReleaseQuery, SyncMode, UpdateReason,
    },
    worker::{
        bridge::UiBridge, library::LibraryService, messages::WorkerToUiEvent, sync::SyncService,
    },
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

#[test]
fn fixtures_parse_to_manifest_expectations() {
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    // 1. Discography fixtures
    let disc_cases = [
        (
            "music_grid_overflow.html",
            "https://synthlabel.bandcamp.com",
            "synthlabel",
            440011_i64,
            "Synth Label Collective",
            4_usize,
        ),
        (
            "band_id_meta_props.html",
            "https://metaprops.bandcamp.com",
            "metaprops",
            880022_i64,
            "Meta Props Artist",
            1_usize,
        ),
        (
            "band_id_script_databand.html",
            "https://databand.bandcamp.com",
            "databand",
            990033_i64,
            "DataBand Project",
            1_usize,
        ),
    ];

    for (file, base_url, slug, exp_band_id, exp_name, exp_count) in disc_cases {
        let html = std::fs::read_to_string(fixtures_dir.join(file)).unwrap();
        let parsed = parse_discography_html(&html, base_url, None, slug).unwrap();
        assert_eq!(parsed.band_id, exp_band_id, "file={file}");
        assert_eq!(parsed.name, exp_name, "file={file}");
        assert_eq!(parsed.items.len(), exp_count, "file={file}");
    }

    // Verify overflow fixture deduplicates item 1002 and preserves label artist override
    let overflow_html =
        std::fs::read_to_string(fixtures_dir.join("music_grid_overflow.html")).unwrap();
    let overflow = parse_discography_html(
        &overflow_html,
        "https://synthlabel.bandcamp.com",
        None,
        "synthlabel",
    )
    .unwrap();
    assert_eq!(overflow.items[0].title, "Zero Voltage");
    assert_eq!(overflow.items[0].artist_name, "Sub Artist One");
    assert_eq!(overflow.items[1].key, ItemKey::new(ItemType::Album, 1002));
    assert_eq!(overflow.items[2].key, ItemKey::new(ItemType::Album, 1003));
    assert_eq!(overflow.items[3].key, ItemKey::new(ItemType::Track, 2001));

    // 2. Release fixtures
    let release_cases = [
        (
            "release_free.html",
            1001_i64,
            ItemType::Album,
            440011_i64,
            Classification::Free,
            4_u32,
            1_629_417_600_i64,
        ),
        (
            "release_nyp.html",
            1002_i64,
            ItemType::Album,
            440011_i64,
            Classification::Nyp,
            3_u32,
            1_663_200_000_i64,
        ),
        (
            "release_email.html",
            1003_i64,
            ItemType::Album,
            440011_i64,
            Classification::EmailRequired,
            2_u32,
            1_672_531_200_i64,
        ),
        (
            "release_paid.html",
            1004_i64,
            ItemType::Album,
            440011_i64,
            Classification::Paid,
            5_u32,
            1_704_067_200_i64,
        ),
        (
            "release_track.html",
            2001_i64,
            ItemType::Track,
            440011_i64,
            Classification::Free,
            1_u32,
            1_706_745_600_i64,
        ),
    ];

    for (file, exp_id, exp_type, exp_band, exp_class, exp_tracks, exp_pub) in release_cases {
        let html = std::fs::read_to_string(fixtures_dir.join(file)).unwrap();
        let parsed = parse_release_html(&html).unwrap();
        assert_eq!(parsed.item_id, Some(exp_id), "file={file}");
        assert_eq!(parsed.item_type, Some(exp_type), "file={file}");
        assert_eq!(parsed.band_id, Some(exp_band), "file={file}");
        assert_eq!(parsed.classification, exp_class, "file={file}");
        assert_eq!(parsed.track_count, exp_tracks, "file={file}");
        assert_eq!(parsed.published_at, Some(exp_pub), "file={file}");
        assert!(!parsed.is_preorder, "file={file}");
    }

    // 3. Redeem & buy_for_nothing fixtures
    let redeem_html = std::fs::read_to_string(fixtures_dir.join("redeem_valid.html")).unwrap();
    let redeem_items = parse_redeem_html(&redeem_html).unwrap();
    assert_eq!(redeem_items.len(), 1);
    assert_eq!(redeem_items[0].key, ItemKey::new(ItemType::Album, 1003));
    assert!(redeem_items[0].downloads.contains_key(&AudioFormat::Flac));

    let bfn_ok = std::fs::read_to_string(fixtures_dir.join("buy_for_nothing_ok.json")).unwrap();
    let dl_map = parse_buy_for_nothing_json(&bfn_ok).unwrap();
    assert!(dl_map.contains_key(&AudioFormat::Flac));
    assert!(dl_map.contains_key(&AudioFormat::Mp3_320));

    let bfn_email =
        std::fs::read_to_string(fixtures_dir.join("buy_for_nothing_email.json")).unwrap();
    assert!(matches!(
        parse_buy_for_nothing_json(&bfn_email),
        Err(BcError::EmailRequired)
    ));

    let bfn_quota =
        std::fs::read_to_string(fixtures_dir.join("buy_for_nothing_quota.json")).unwrap();
    assert!(matches!(
        parse_buy_for_nothing_json(&bfn_quota),
        Err(BcError::QuotaExceeded)
    ));

    let bfn_rej =
        std::fs::read_to_string(fixtures_dir.join("buy_for_nothing_rejected.json")).unwrap();
    assert!(matches!(
        parse_buy_for_nothing_json(&bfn_rej),
        Err(BcError::ApiRejected(_))
    ));
}

fn make_release_html(id: i64, class_min_price: f64, tracks: usize) -> String {
    let track_objs: Vec<String> = (1..=tracks)
        .map(|i| format!(r#"{{"title":"Track {i}"}}"#))
        .collect();
    format!(
        r#"<!doctype html><html><head><script data-tralbum='{{
            "id": {id},
            "item_type": "album",
            "freeDownloadPage": "https://example.bandcamp.com/download?id={id}",
            "art_id": 100,
            "artist": "Mock Band",
            "trackinfo": [{tracks_json}],
            "current": {{
                "id": {id},
                "type": "album",
                "band_id": 777,
                "title": "Album {id}",
                "minimum_price": {class_min_price},
                "set_price": 0.0,
                "is_set_price": 1,
                "require_email": 0,
                "publish_date": "20 Aug 2021 00:00:00 GMT",
                "mod_date": "21 Aug 2021 00:00:00 GMT",
                "is_preorder": 0
            }}
        }}'></script></head><body></body></html>"#,
        tracks_json = track_objs.join(",")
    )
}

fn make_grid_html(count: usize) -> String {
    let mut lis = String::new();
    for i in 1..=count {
        lis.push_str(&format!(
            r#"<li class="music-grid-item" data-item-id="album-{i}" data-band-id="777">
                <a href="/album/rel-{i}"><p class="title">Album {i}</p></a>
            </li>"#
        ));
    }
    format!(
        r#"<!doctype html><html><head><meta property="og:site_name" content="Mock Band"></head>
        <body><ol id="music-grid">{lis}</ol></body></html>"#
    )
}

#[tokio::test]
async fn wiremock_20_release_sync_and_incremental_diff() {
    let server = MockServer::start().await;

    let req_times: Arc<Mutex<Vec<Instant>>> = Arc::new(Mutex::new(Vec::new()));
    let total_reqs = Arc::new(AtomicUsize::new(0));
    let current_grid_count = Arc::new(AtomicUsize::new(20));

    struct GridResponder {
        times: Arc<Mutex<Vec<Instant>>>,
        total: Arc<AtomicUsize>,
        count: Arc<AtomicUsize>,
    }
    impl Respond for GridResponder {
        fn respond(&self, _req: &Request) -> ResponseTemplate {
            self.times.lock().unwrap().push(Instant::now());
            self.total.fetch_add(1, Ordering::SeqCst);
            let n = self.count.load(Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_string(make_grid_html(n))
        }
    }

    Mock::given(method("GET"))
        .and(path("/music"))
        .respond_with(GridResponder {
            times: req_times.clone(),
            total: total_reqs.clone(),
            count: current_grid_count.clone(),
        })
        .mount(&server)
        .await;

    for i in 1..=21 {
        let times_c = req_times.clone();
        let total_c = total_reqs.clone();
        let body = make_release_html(i as i64, if i % 2 == 0 { 0.0 } else { 5.0 }, 3);
        Mock::given(method("GET"))
            .and(path(format!("/album/rel-{i}")))
            .respond_with(move |_req: &Request| {
                times_c.lock().unwrap().push(Instant::now());
                total_c.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_string(body.clone())
            })
            .mount(&server)
            .await;
    }

    let dir = tempfile::tempdir().unwrap();
    let pool = DbPool::open(&dir.path().join("sync.db")).unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(tx, None);

    // Use 15 ms page delay with 20% jitter (min gap >= 12 ms) so 21 requests complete quickly in tests
    let gate = BandcampGate::with_delays(15, 20, 10, 10, 2, None);
    let client = BandcampClient::new(gate, "bcvault-test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    let svc = SyncService::new(pool.clone(), client, bridge);

    let outcome = svc
        .add_and_sync_artist(&server.uri(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(outcome.band_id, 777);
    assert_eq!(outcome.new_items, 20);
    assert_eq!(total_reqs.load(Ordering::SeqCst), 21); // 1 grid + 20 release pages

    // Verify all 20 rows are classified
    let rows = pool
        .reader
        .call(|c| releases::query_releases(c, &ReleaseQuery::default()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 20);
    assert!(
        rows.iter()
            .all(|r| r.classification != Classification::Unknown)
    );

    // Verify page requests are spaced by at least the minimum jittered gap (15ms * 0.8 = 12ms; allow 10ms clock tolerance)
    {
        let recorded = req_times.lock().unwrap();
        assert_eq!(recorded.len(), 21);
        for w in recorded.windows(2) {
            let delta = w[1].duration_since(w[0]);
            assert!(
                delta >= Duration::from_millis(10),
                "Expected gap >= 10ms, got {delta:?}"
            );
        }
    }

    // Second Incremental sync issues exactly 1 request (the /music page)
    total_reqs.store(0, Ordering::SeqCst);
    let outcome2 = svc
        .sync_artist(777, SyncMode::Incremental, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(outcome2.new_items, 0);
    assert_eq!(total_reqs.load(Ordering::SeqCst), 1);

    // Mock adds 1 release (now 21); next Incremental sync issues exactly 2 requests (/music + /album/rel-21)
    current_grid_count.store(21, Ordering::SeqCst);
    total_reqs.store(0, Ordering::SeqCst);
    let outcome3 = svc
        .sync_artist(777, SyncMode::Incremental, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(outcome3.new_items, 1);
    assert_eq!(total_reqs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn wiremock_429_retry_cancel_resume_and_check_downloaded() {
    let server = MockServer::start().await;

    let music_attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = music_attempts.clone();

    Mock::given(method("GET"))
        .and(path("/music"))
        .respond_with(move |_req: &Request| {
            let n = attempts_clone.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                ResponseTemplate::new(429).insert_header("Retry-After", "2")
            } else {
                ResponseTemplate::new(200).set_body_string(make_grid_html(2))
            }
        })
        .mount(&server)
        .await;

    let rel1_tracks = Arc::new(AtomicUsize::new(3));
    let rel1_tracks_c = rel1_tracks.clone();
    Mock::given(method("GET"))
        .and(path("/album/rel-1"))
        .respond_with(move |_req: &Request| {
            let t = rel1_tracks_c.load(Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_string(make_release_html(1, 0.0, t))
        })
        .mount(&server)
        .await;

    let cancel_tok = CancellationToken::new();
    let cancel_on_rel1 = cancel_tok.clone();
    let rel2_hits = Arc::new(AtomicUsize::new(0));
    let rel2_hits_c = rel2_hits.clone();
    Mock::given(method("GET"))
        .and(path("/album/rel-2"))
        .respond_with(move |_req: &Request| {
            rel2_hits_c.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_string(make_release_html(2, 0.0, 4))
        })
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let pool = DbPool::open(&dir.path().join("retry.db")).unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(tx.clone(), None);

    let tx_rate = tx.clone();
    let gate = BandcampGate::with_delays(
        10,
        0,
        10,
        10,
        2,
        Some(Arc::new(move |st| {
            let _ = tx_rate.send(WorkerToUiEvent::RateLimit(st));
        })),
    );
    let client = BandcampClient::new(gate, "bcvault-test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    let svc = SyncService::new(pool.clone(), client, bridge);

    // Spawn a watcher that cancels the sync right after Album 1 is updated
    let watcher_cancel = cancel_on_rel1.clone();
    let rate_events = Arc::new(Mutex::new(Vec::new()));
    let rate_events_c = rate_events.clone();
    let ev_task = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                WorkerToUiEvent::RateLimit(st) => {
                    rate_events_c.lock().unwrap().push(st);
                }
                WorkerToUiEvent::ReleaseUpdated(row)
                    if row.key.id == 1 && row.classification != Classification::Unknown =>
                {
                    watcher_cancel.cancel();
                }
                _ => {}
            }
        }
    });

    let t0 = Instant::now();
    let _ = svc
        .add_and_sync_artist(&server.uri(), cancel_tok)
        .await
        .unwrap();
    let elapsed = t0.elapsed();

    // Must have waited at least 2s for Retry-After: 2
    assert!(
        elapsed >= Duration::from_millis(1950),
        "Expected >= 2s cooldown wait, got {elapsed:?}"
    );
    assert!(
        rate_events
            .lock()
            .unwrap()
            .iter()
            .any(|st| matches!(st, RateLimitStatus::Cooldown { .. }))
    );

    // Because we cancelled right after Album 1, Album 2 was still 'unknown' and never fetched
    assert_eq!(rel2_hits.load(Ordering::SeqCst), 0);
    let r2 = pool
        .reader
        .call(|c| releases::get_release(c, ItemKey::new(ItemType::Album, 2)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r2.classification, Classification::Unknown);

    // Re-running Incremental sync processes only the remaining unknown row (Album 2)
    let _ = svc
        .sync_artist(777, SyncMode::Incremental, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(rel2_hits.load(Ordering::SeqCst), 1);
    let r2_after = pool
        .reader
        .call(|c| releases::get_release(c, ItemKey::new(ItemType::Album, 2)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r2_after.classification, Classification::Free);

    // Mark Album 1 as downloaded with 3 tracks, then grow trackinfo to 5 and run CheckDownloaded
    pool.writer
        .call(|c| {
            let key = ItemKey::new(ItemType::Album, 1);
            let hid = history::insert_started(
                c,
                &NewHistoryEntry {
                    key,
                    band_id: 777,
                    format: AudioFormat::Flac,
                    source: DownloadSource::Api,
                    counts_against_quota: true,
                },
            )?;
            releases::commit_download(
                c,
                key,
                AudioFormat::Flac,
                &PathBuf::from("/tmp/1.zip"),
                100,
                hid,
            )
        })
        .await
        .unwrap();

    rel1_tracks.store(5, Ordering::SeqCst);
    let check_out = svc
        .sync_artist(777, SyncMode::CheckDownloaded, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(check_out.updated, 1);

    let r1_updated = pool
        .reader
        .call(|c| releases::get_release(c, ItemKey::new(ItemType::Album, 1)))
        .await
        .unwrap()
        .unwrap();
    assert!(r1_updated.has_remote_update);
    assert_eq!(
        r1_updated.remote_update_reason,
        Some(UpdateReason::TracksAdded)
    );

    ev_task.abort();
}

#[tokio::test]
async fn wiremock_sync_rescan_auto_adopts_certain_match_and_updates_stats() {
    let server = MockServer::start().await;

    // 1 grid page with 1 album, 1 album page
    Mock::given(method("GET"))
        .and(path("/music"))
        .respond_with(ResponseTemplate::new(200).set_body_string(make_grid_html(1)))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/album/rel-1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(make_release_html(1, 0.0, 3)))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sync_rescan.db");
    let pool = DbPool::open(&db_path).unwrap();

    let artist_dir = dir.path().join("Artist 777");
    let album_dir = artist_dir.join("Album 1");
    std::fs::create_dir_all(&album_dir).unwrap();
    std::fs::write(album_dir.join("01.mp3"), b"audio").unwrap();
    std::fs::write(album_dir.join("02.mp3"), b"audio").unwrap();
    std::fs::write(album_dir.join("03.mp3"), b"audio").unwrap();

    let out_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&out_dir).unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(tx, None);

    let gate = BandcampGate::with_delays(10, 10, 5, 5, 2, None);
    let client = BandcampClient::new(gate, "bcvault-test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    let lib_svc = LibraryService::new(pool.clone(), bridge.clone());
    let svc = SyncService::new(pool.clone(), client, bridge).with_library(lib_svc);

    // Link artist folder for band_id 777 before sync (so it will be queued until sync discovers artist 777)
    // Or link folder directly
    let a_dir = artist_dir.clone();
    let o_dir = out_dir.clone();
    let s_uri = server.uri();
    pool.writer
        .call(move |c| {
            artists::upsert(
                c,
                &artists::UpsertArtist {
                    band_id: 777,
                    slug: "artist-777",
                    name: "Artist 777",
                    url: &s_uri,
                    cover_url: None,
                },
            )?;
            lib_repo::link_folder(c, 777, &a_dir, &o_dir)?;
            Ok::<_, bcvault_core::error::DbError>(())
        })
        .await
        .unwrap();

    let outcome = svc
        .sync_artist(777, SyncMode::Incremental, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(outcome.band_id, 777);
    assert_eq!(outcome.new_items, 1);
    assert_eq!(outcome.adopted, 1);

    // Verify release is marked downloaded in DB
    let rel = pool
        .reader
        .call(|c| releases::get_release(c, ItemKey::new(ItemType::Album, 1)))
        .await
        .unwrap()
        .unwrap();
    assert!(rel.downloaded);
    assert_eq!(rel.local_status, Some(LocalStatus::Present));

    // Verify artist stats reflect downloaded = 1
    let stats = pool
        .reader
        .call(|c| artists::stats_with_quota(c, 777))
        .await
        .unwrap();
    assert_eq!(stats.downloaded, 1);

    // Collect events and check SyncFinished
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    let sync_finished = events.iter().find_map(|e| match e {
        WorkerToUiEvent::SyncFinished {
            band_id, adopted, ..
        } if *band_id == 777 => Some(*adopted),
        _ => None,
    });
    assert_eq!(sync_finished, Some(1));
}
