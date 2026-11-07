#![cfg(feature = "live-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use bcvault::bandcamp::{BandcampClient, gate::BandcampGate, input::normalize_artist_input};

#[tokio::test]
async fn live_contract_smoke_test() {
    let Ok(slug) = std::env::var("BCVAULT_LIVE_ARTIST") else {
        eprintln!("Skipping live contract test: set BCVAULT_LIVE_ARTIST=<slug> to run");
        return;
    };

    let norm = normalize_artist_input(&slug).unwrap();
    let gate = BandcampGate::new(1500, 25, 1, None);
    let client = BandcampClient::new(gate, "bcvault/0.1.0 (live contract test)").unwrap();

    let page = client
        .fetch_discography(&norm.base_url, &norm.slug)
        .await
        .unwrap();
    assert!(page.band_id > 0);
    assert!(!page.name.is_empty());

    if let Some(first) = page.items.first() {
        let rel = client
            .fetch_release(&first.url, Some(page.band_id))
            .await
            .unwrap();
        assert!(rel.track_count >= 1);
    }
}
