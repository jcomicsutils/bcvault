#![allow(clippy::unwrap_used, clippy::expect_used)]

use bcvault::{
    bandcamp::{BUY_FOR_NOTHING_PATH, BandcampClient, gate::BandcampGate},
    db::{
        DbPool,
        repo::{
            artists::{self, UpsertArtist},
            history::{self, NewHistoryEntry},
            releases::{self, DiscoveredRelease, ReleaseMetadata},
        },
    },
    error::DownloadError,
    types::{
        AudioFormat, Classification, DownloadOpts, DownloadSource, HistoryStatus, ItemKey,
        ItemType, JobError, JobState, Settings,
    },
    worker::{
        bridge::UiBridge,
        downloads::DownloadManager,
        extract::extract_zip_sync,
        messages::WorkerToUiEvent,
        stream::{StreamRequest, stream_download},
    },
};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

fn build_zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for &(name, data) in entries {
            zip.start_file(name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

async fn seed_test_release(
    pool: &DbPool,
    band_id: i64,
    key: ItemKey,
    artist: &str,
    title: &str,
    class: Classification,
    tracks: u32,
) {
    let artist_o = artist.to_string();
    let title_o = title.to_string();
    let slug = format!("testartist-{band_id}");
    let url = format!("https://{slug}.bandcamp.com");
    let rel_url = format!("{url}/album/test-{}", key.id);
    pool.writer
        .call(move |c| {
            artists::upsert(
                c,
                &UpsertArtist {
                    band_id,
                    slug: &slug,
                    name: &artist_o,
                    url: &url,
                    cover_url: None,
                },
            )?;
            releases::upsert_discovered(
                c,
                &DiscoveredRelease {
                    key,
                    band_id,
                    title: &title_o,
                    artist_name: &artist_o,
                    url: &rel_url,
                    cover_url: None,
                    art_id: Some(777),
                },
            )?;
            releases::update_metadata(
                c,
                &ReleaseMetadata {
                    key,
                    classification: class,
                    pricing_json: Some(r#"{"minimum_price":0.0}"#),
                    is_preorder: false,
                    track_count: Some(tracks),
                    published_at: Some(1_700_000_000),
                    modified_at: Some(1_700_000_100),
                    art_id: Some(777),
                },
            )
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn wiremock_album_end_to_end_and_zip_extraction() {
    let server = MockServer::start().await;
    let zip_bytes = build_zip_bytes(&[
        ("01 - Intro.flac", b"fake-flac-bytes-track-1"),
        ("02 - Outro.flac", b"fake-flac-bytes-track-2"),
    ]);
    let expected_sha = Sha256::digest(&zip_bytes);

    let dl_url = format!("{}/download/album-101.zip", server.uri());
    let bfn_json = serde_json::json!({
        "ok": true,
        "downloads": {
            "flac": { "url": dl_url }
        }
    });

    Mock::given(method("POST"))
        .and(path(BUY_FOR_NOTHING_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(bfn_json))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/download/album-101.zip"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(
                    "Content-Disposition",
                    "attachment; filename*=UTF-8''Sigur%20R%C3%B3s%20-%20%C3%81g%C3%A6tis.zip",
                )
                .set_body_bytes(zip_bytes.clone()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("dl.db");
    let out_dir = dir.path().join("out");
    let pool = DbPool::open(&db_path).unwrap();

    let key = ItemKey::new(ItemType::Album, 101);
    seed_test_release(
        &pool,
        10,
        key,
        "Sigur Rós",
        "Ágætis",
        Classification::Free,
        2,
    )
    .await;

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(ev_tx, None);
    let gate = BandcampGate::with_delays(5, 0, 5, 5, 2, None);
    let client = BandcampClient::new(gate, "bcvault-test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    // Keep zip (auto_extract = false first so we can verify exact zip file name and SHA-256)
    let mut settings = Settings::default_with_dir(out_dir.clone());
    settings.auto_extract = false;

    let root_cancel = CancellationToken::new();
    let mgr = DownloadManager::new(
        pool.clone(),
        client,
        bridge,
        settings.clone(),
        root_cancel.clone(),
    );

    mgr.enqueue_downloads(
        vec![key],
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: out_dir.clone(),
            batch_limit: 150,
        },
        false,
    )
    .await;

    let mut done_path = None;
    while let Some(ev) = ev_rx.recv().await {
        if let WorkerToUiEvent::JobState {
            state: JobState::Done { path, .. },
            ..
        } = ev
        {
            done_path = Some(path);
            break;
        }
    }

    let saved_zip = done_path.unwrap();
    assert_eq!(
        saved_zip.file_name().unwrap().to_str().unwrap(),
        "Sigur Rós - Ágætis.zip"
    );
    let disk_bytes = std::fs::read(&saved_zip).unwrap();
    assert_eq!(Sha256::digest(&disk_bytes), expected_sha);

    let row = pool
        .reader
        .call(move |c| releases::get_release(c, key))
        .await
        .unwrap()
        .unwrap();
    assert!(row.downloaded);
    assert_eq!(row.downloaded_format, Some(AudioFormat::Flac));
    assert_eq!(row.local_path, Some(saved_zip.clone()));
    assert_eq!(row.track_count, Some(2));

    let quota = pool
        .reader
        .call(|c| history::quota_used_30d(c, 10))
        .await
        .unwrap();
    assert_eq!(quota, 1);

    // Also test extract_zip_sync with delete_after_extract = true
    let extracted_dir = extract_zip_sync(&saved_zip, true).unwrap();
    assert!(!saved_zip.exists());
    assert!(extracted_dir.join("01 - Intro.flac").exists());
    assert!(extracted_dir.join("02 - Outro.flac").exists());

    // Zip containing "../evil.txt" is rejected
    let evil_bytes = build_zip_bytes(&[("../evil.txt", b"pwn")]);
    let evil_zip_path = out_dir.join("evil.zip");
    std::fs::write(&evil_zip_path, evil_bytes).unwrap();
    let err = extract_zip_sync(&evil_zip_path, false).unwrap_err();
    assert!(matches!(err, DownloadError::UnsafeZipEntry(_)));

    root_cancel.cancel();
}

#[tokio::test]
async fn wiremock_track_end_to_end_and_cover_404_tolerance() {
    let server = MockServer::start().await;

    let dl_url_1 = format!("{}/download/track-201.flac", server.uri());
    let dl_url_2 = format!("{}/download/track-202.flac", server.uri());

    Mock::given(method("POST"))
        .and(path(BUY_FOR_NOTHING_PATH))
        .respond_with(move |req: &Request| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let id = body["item_id"].as_str().unwrap();
            let u = if id == "201" { &dl_url_1 } else { &dl_url_2 };
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "downloads": { "flac": { "url": u } }
            }))
        })
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/download/track-201.flac"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio-track-201".to_vec()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/download/track-202.flac"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio-track-202".to_vec()))
        .mount(&server)
        .await;

    // Cover for art_id 777 succeeds; cover for art_id 888 returns 404
    Mock::given(method("GET"))
        .and(path("/img/a0000000777_10.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"jpeg-cover-bytes".to_vec()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/img/a0000000888_10.jpg"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let pool = DbPool::open(&dir.path().join("track.db")).unwrap();
    let out_dir = dir.path().join("out");

    let k1 = ItemKey::new(ItemType::Track, 201);
    let k2 = ItemKey::new(ItemType::Track, 202);
    seed_test_release(
        &pool,
        20,
        k1,
        "TrackArtist",
        "SongOne",
        Classification::Free,
        1,
    )
    .await;
    seed_test_release(
        &pool,
        20,
        k2,
        "TrackArtist",
        "SongTwo",
        Classification::Free,
        1,
    )
    .await;
    // Update k2 art_id to 888 (404 cover)
    pool.writer
        .call(move |c| {
            releases::update_metadata(
                c,
                &ReleaseMetadata {
                    key: k2,
                    classification: Classification::Free,
                    pricing_json: None,
                    is_preorder: false,
                    track_count: Some(1),
                    published_at: None,
                    modified_at: None,
                    art_id: Some(888),
                },
            )
        })
        .await
        .unwrap();

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(ev_tx, None);
    let gate = BandcampGate::with_delays(5, 0, 5, 5, 2, None);
    let client = BandcampClient::new(gate, "bcvault-test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    let settings = Settings::default_with_dir(out_dir.clone());
    let root_cancel = CancellationToken::new();
    let mgr = DownloadManager::new(pool.clone(), client, bridge, settings, root_cancel.clone());

    mgr.enqueue_downloads(
        vec![k1, k2],
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: out_dir.clone(),
            batch_limit: 150,
        },
        false,
    )
    .await;

    let mut done_dirs = Vec::new();
    let mut warn_logged = false;
    while let Some(ev) = ev_rx.recv().await {
        match ev {
            WorkerToUiEvent::JobState {
                state: JobState::Done { path, .. },
                ..
            } => {
                done_dirs.push(path);
                if done_dirs.len() == 2 {
                    break;
                }
            }
            WorkerToUiEvent::Log(entry) if entry.message.contains("cover.jpg") => {
                warn_logged = true;
            }
            _ => {}
        }
    }

    let folder1 = out_dir.join("TrackArtist - SongOne");
    assert!(folder1.join("TrackArtist - SongOne.flac").exists());
    assert!(folder1.join("cover.jpg").exists());

    let folder2 = out_dir.join("TrackArtist - SongTwo");
    assert!(folder2.join("TrackArtist - SongTwo.flac").exists());
    assert!(!folder2.join("cover.jpg").exists());
    assert!(warn_logged);

    root_cancel.cancel();
}

#[tokio::test]
async fn cancel_mid_stream_and_truncated_body() {
    let server = MockServer::start().await;

    // Raw TCP server that sends Content-Length: 1000 and closes after 10 bytes
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let trunc_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0_u8; 512];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nContent-Type: application/zip\r\n\r\n0123456789";
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, resp).await;
        }
    });

    // Slow body for cancellation test
    Mock::given(method("GET"))
        .and(path("/slow.zip"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(5))
                .set_body_bytes(b"slow-data".to_vec()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("stream_out");
    let (ev_tx, _ev_rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(ev_tx, None);
    let gate = BandcampGate::with_delays(5, 0, 5, 5, 2, None);
    let client = BandcampClient::new(gate, "bcvault-test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    // 1. Truncated stream fails with Truncated and leaves no final or .part file
    let trunc_url = format!("http://{trunc_addr}/truncated.zip");
    let err = stream_download(
        &client,
        &bridge,
        StreamRequest {
            job_id: 1,
            url: &trunc_url,
            band_id: Some(1),
            dest_dir: &out_dir,
            artist: "Art",
            title: "Trunc",
            kind: ItemType::Album,
            fallback_ext: "zip",
        },
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DownloadError::Truncated { .. }));
    assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 0);

    // 2. Cancelled mid-stream removes .part and returns Cancelled
    let cancel_tok = CancellationToken::new();
    let cancel_c = cancel_tok.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        cancel_c.cancel();
    });

    let slow_url = format!("{}/slow.zip", server.uri());
    let err2 = stream_download(
        &client,
        &bridge,
        StreamRequest {
            job_id: 2,
            url: &slow_url,
            band_id: Some(1),
            dest_dir: &out_dir,
            artist: "Art",
            title: "Slow",
            kind: ItemType::Album,
            fallback_ext: "zip",
        },
        cancel_tok,
    )
    .await
    .unwrap_err();
    assert!(matches!(err2, DownloadError::Cancelled));
    assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 0);
}

#[tokio::test]
async fn quota_enforcement_api_rejections_and_redeem() {
    let server = MockServer::start().await;
    let bfn_hits = Arc::new(AtomicUsize::new(0));
    let bfn_hits_c = bfn_hits.clone();

    let zip_bytes = build_zip_bytes(&[("track.flac", b"audio-bytes")]);
    let dl_url = format!("{}/dl.zip", server.uri());
    let dl_url_c = dl_url.clone();

    Mock::given(method("POST"))
        .and(path(BUY_FOR_NOTHING_PATH))
        .respond_with(move |req: &Request| {
            bfn_hits_c.fetch_add(1, Ordering::SeqCst);
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let id = body["item_id"].as_str().unwrap();
            match id {
                "901" => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error_message": "An email address is required to download."
                })),
                "902" => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "Free download limit exceeded for this artist."
                })),
                _ => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "downloads": { "flac": { "url": dl_url_c } }
                })),
            }
        })
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/dl.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip_bytes))
        .mount(&server)
        .await;

    let redeem_html = format!(
        r#"<!doctype html><html><body><div id="pagedata" data-blob='{{
            "download_items": [{{
                "item_id": 901,
                "item_type": "album",
                "title": "Email Album",
                "artist": "Band",
                "art_id": 777,
                "downloads": {{ "flac": {{ "url": "{dl_url}" }} }}
            }}]
        }}'></div></body></html>"#
    );

    Mock::given(method("GET"))
        .and(path("/download"))
        .respond_with(ResponseTemplate::new(200).set_body_string(redeem_html))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let pool = DbPool::open(&dir.path().join("q.db")).unwrap();
    let out_dir = dir.path().join("out");

    // Seed 5 releases on band 50
    for i in 1..=5 {
        seed_test_release(
            &pool,
            50,
            ItemKey::new(ItemType::Album, 500 + i),
            "QuotaBand",
            &format!("Alb {i}"),
            Classification::Free,
            1,
        )
        .await;
    }

    // Seed 149 counted history rows on band 50
    pool.writer
        .call(|c| {
            for i in 0..149 {
                let hid = history::insert_started(
                    c,
                    &NewHistoryEntry {
                        key: ItemKey::new(ItemType::Album, 10_000 + i),
                        band_id: 50,
                        format: AudioFormat::Flac,
                        source: DownloadSource::Api,
                        counts_against_quota: true,
                    },
                )?;
                history::finish(c, hid, HistoryStatus::Success, Some(10), None)?;
            }
            Ok(())
        })
        .await
        .unwrap();

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(ev_tx, None);
    let gate = BandcampGate::with_delays(5, 0, 5, 5, 2, None);
    let client = BandcampClient::new(gate, "bcvault-test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    let mut settings = Settings::default_with_dir(out_dir.clone());
    settings.auto_extract = false;
    let root_cancel = CancellationToken::new();
    let mgr = DownloadManager::new(pool.clone(), client, bridge, settings, root_cancel.clone());

    let batch_keys: Vec<_> = (1..=5)
        .map(|i| ItemKey::new(ItemType::Album, 500 + i))
        .collect();

    // 1. Crossing 150 without acknowledgement emits QuotaWarning and makes 0 HTTP calls
    mgr.enqueue_downloads(
        batch_keys.clone(),
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: out_dir.clone(),
            batch_limit: 150,
        },
        false,
    )
    .await;

    let mut got_warn = false;
    while let Some(ev) = ev_rx.recv().await {
        if let WorkerToUiEvent::QuotaWarning {
            band_id: 50,
            used_30d: 149,
            requested: 5,
            ..
        } = ev
        {
            got_warn = true;
            break;
        }
    }
    assert!(got_warn);
    assert_eq!(bfn_hits.load(Ordering::SeqCst), 0);

    // 2. Add 50 more history rows so band 50 is at 199 used; batch of 5 with ack=true is truncated to 1
    pool.writer
        .call(|c| {
            for i in 0..50 {
                let hid = history::insert_started(
                    c,
                    &NewHistoryEntry {
                        key: ItemKey::new(ItemType::Album, 20_000 + i),
                        band_id: 50,
                        format: AudioFormat::Flac,
                        source: DownloadSource::Api,
                        counts_against_quota: true,
                    },
                )?;
                history::finish(c, hid, HistoryStatus::Success, Some(10), None)?;
            }
            Ok(())
        })
        .await
        .unwrap();

    mgr.enqueue_downloads(
        batch_keys,
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: out_dir.clone(),
            batch_limit: 150,
        },
        true,
    )
    .await;

    while let Some(ev) = ev_rx.recv().await {
        if let WorkerToUiEvent::JobState {
            state: JobState::Done { .. },
            ..
        } = ev
        {
            break;
        }
    }
    assert_eq!(bfn_hits.load(Ordering::SeqCst), 1); // Only 1 job ran, bringing quota to 200

    // 3. Email rejection on item 901 re-classifies row as EmailRequired; Redeem downloads it
    let k_email = ItemKey::new(ItemType::Album, 901);
    seed_test_release(
        &pool,
        60,
        k_email,
        "Band",
        "Email Album",
        Classification::Free,
        1,
    )
    .await;

    mgr.enqueue_downloads(
        vec![k_email],
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: out_dir.clone(),
            batch_limit: 150,
        },
        true,
    )
    .await;

    while let Some(ev) = ev_rx.recv().await {
        if let WorkerToUiEvent::JobState {
            state: JobState::Failed(JobError::EmailRequired),
            ..
        } = ev
        {
            break;
        }
    }

    let r_email = pool
        .reader
        .call(move |c| releases::get_release(c, k_email))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r_email.classification, Classification::EmailRequired);

    // Foreign host redeem URL is rejected
    mgr.enqueue_redeem(
        k_email,
        "https://evil.example.com/download?id=901".to_string(),
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: out_dir.clone(),
            batch_limit: 150,
        },
    )
    .await;

    while let Some(ev) = ev_rx.recv().await {
        if let WorkerToUiEvent::JobState {
            state: JobState::Failed(JobError::ApiRejected(_)),
            ..
        } = ev
        {
            break;
        }
    }

    // Valid redeem URL downloads and marks row downloaded without counting against quota
    mgr.enqueue_redeem(
        k_email,
        format!("{}/download?id=901", server.uri()),
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: out_dir.clone(),
            batch_limit: 150,
        },
    )
    .await;

    while let Some(ev) = ev_rx.recv().await {
        if let WorkerToUiEvent::JobState {
            state: JobState::Done { .. },
            ..
        } = ev
        {
            break;
        }
    }

    let r_email_after = pool
        .reader
        .call(move |c| releases::get_release(c, k_email))
        .await
        .unwrap()
        .unwrap();
    assert!(r_email_after.downloaded);
    assert_eq!(r_email_after.classification, Classification::EmailRequired);
    let q60 = pool
        .reader
        .call(|c| history::quota_used_30d(c, 60))
        .await
        .unwrap();
    assert_eq!(q60, 0);

    // 4. Quota rejection on item 902 blocks artist 70 for 30 minutes and drains queued jobs
    let k_q1 = ItemKey::new(ItemType::Album, 902);
    let k_q2 = ItemKey::new(ItemType::Album, 903);
    seed_test_release(&pool, 70, k_q1, "Band70", "Q1", Classification::Free, 1).await;
    seed_test_release(&pool, 70, k_q2, "Band70", "Q2", Classification::Free, 1).await;

    mgr.enqueue_downloads(
        vec![k_q1, k_q2],
        DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: Path::new(&out_dir).to_path_buf(),
            batch_limit: 150,
        },
        true,
    )
    .await;

    let mut quota_failures = 0;
    while let Some(ev) = ev_rx.recv().await {
        if let WorkerToUiEvent::JobState {
            state: JobState::Failed(JobError::QuotaExceeded),
            ..
        } = ev
        {
            quota_failures += 1;
            if quota_failures == 2 {
                break;
            }
        }
    }
    assert_eq!(quota_failures, 2);

    root_cancel.cancel();
}
