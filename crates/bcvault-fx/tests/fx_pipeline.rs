#![allow(clippy::unwrap_used, clippy::expect_used)]

use bcvault_core::{
    bandcamp::{BandcampClient, gate::BandcampGate},
    db::DbPool,
    types::{ItemKey, ItemType},
};
use bcvault_fx::{
    db::{FxDb, get_release_detail},
    services::{details::DetailService, messages::FxEvent},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn detail_service_deduplicates_concurrent_requests_and_caches_in_fx_sqlite() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let main_db_path = dir.path().join("bcvault.db");
    let fx_db_path = dir.path().join("fx.sqlite");

    let main_pool = DbPool::open(&main_db_path).unwrap();
    let release_url = format!("{}/album/synth-odyssey", server.uri());
    let release_url_db = release_url.clone();
    main_pool
        .writer
        .call(move |c| {
            c.execute(
                "INSERT INTO artists (band_id, slug, name, url) VALUES (1, 'artist', 'Artist', 'https://artist.bandcamp.com')",
                [],
            )?;
            c.execute(
                "INSERT INTO releases (item_type, item_id, band_id, title, artist_name, url, classification, is_preorder, first_seen_at, last_seen_at, downloaded, has_remote_update)
                 VALUES ('album', 9001, 1, 'Synth Odyssey', 'Artist', ?1, 'free', 0, 100, 100, 0, 0)",
                [release_url_db],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let fx_db = FxDb::open(&fx_db_path, &main_db_path).unwrap();

    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = hits.clone();
    let tralbum_html = r#"<!doctype html><html><head>
<script data-tralbum='{"id":9001,"item_type":"album","artist":"Artist","current":{"title":"Synth Odyssey","type":"album","minimum_price":0.0,"new_date":"01 Jan 2025 00:00:00 GMT","about":"Lush modular synth album.","credits":"Recorded in 2025."},"trackinfo":[{"track_num":1,"title":"Dawn Horizon","duration":194.2},{"track_num":2,"title":"Night Pulse","duration":245.8}],"freeDownloadPage":"https://bandcamp.com/download?id=9001"}'></script>
</head><body><a class="tag">ambient</a><a class="tag">modular</a></body></html>"#;

    struct SlowPage(Arc<AtomicUsize>, String);
    impl Respond for SlowPage {
        fn respond(&self, _req: &Request) -> ResponseTemplate {
            self.0.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(40))
                .set_body_string(self.1.clone())
        }
    }

    Mock::given(method("GET"))
        .and(path("/album/synth-odyssey"))
        .respond_with(SlowPage(hits_clone, tralbum_html.to_string()))
        .mount(&server)
        .await;

    let gate = BandcampGate::with_delays(0, 0, 0, 0, 2, None);
    let client = BandcampClient::new(gate, "test")
        .unwrap()
        .with_base_urls(server.uri(), server.uri());

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let svc = DetailService::new(fx_db.clone(), main_pool, client, ev_tx, None);
    let key = ItemKey {
        kind: ItemType::Album,
        id: 9001,
    };

    // Issue two rapid concurrent requests for the same key -> only 1 HTTP call
    svc.ensure(key);
    svc.ensure(key);

    let ev = tokio::time::timeout(Duration::from_secs(5), ev_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match ev {
        FxEvent::DetailLoaded {
            key: k,
            detail: det,
        } => {
            assert_eq!(k, key);
            assert_eq!(det.tracks.len(), 2);
            assert_eq!(det.tracks[0].title, "Dawn Horizon");
            assert_eq!(det.about.as_deref(), Some("Lush modular synth album."));
            assert!(det.tags.contains(&"ambient".to_string()));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    // Subsequent ensure() serves directly from fx.sqlite with zero additional HTTP calls
    svc.ensure(key);
    let ev2 = tokio::time::timeout(Duration::from_secs(5), ev_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(ev2, FxEvent::DetailLoaded { .. }));
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let cached = fx_db
        .reader
        .call(move |c| get_release_detail(c, key))
        .await
        .unwrap();
    assert!(cached.is_some());
}
