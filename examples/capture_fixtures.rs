use bcvault::bandcamp::{
    BandcampClient,
    gate::{BandcampGate, GateClass},
    input::normalize_artist_input,
};
use std::{path::PathBuf, time::Duration};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!(
            "Usage: cargo run --example capture_fixtures -- <artist_slug_or_url> [<artist2> ...]"
        );
        return Ok(());
    }

    let out_dir = PathBuf::from("tests/fixtures/captured");
    std::fs::create_dir_all(&out_dir)?;

    let gate = BandcampGate::new(1500, 25, 1, None);
    let client = BandcampClient::new(gate, "bcvault/0.1.0 (fixture capture tool)")?;

    for raw in args {
        let norm = normalize_artist_input(&raw)?;
        tracing::info!(
            "Capturing discography for {} ({})",
            norm.slug,
            norm.base_url
        );

        let music_url = format!("{}/music", norm.base_url);
        let (html, final_url) = client
            .get_text_gated(&music_url, GateClass::Page, None)
            .await?;
        let sanitized_html = sanitize_text(&html);
        let disc_file = out_dir.join(format!("{}_music.html", norm.slug));
        std::fs::write(&disc_file, &sanitized_html)?;

        let page = bcvault::bandcamp::discography::parse_discography_html(
            &html,
            &norm.base_url,
            Some(&final_url),
            &norm.slug,
        )?;
        tracing::info!(
            "Parsed band_id={} ({}) with {} discovered items",
            page.band_id,
            page.name,
            page.items.len()
        );

        if let Ok(details_raw) = client
            .post_json_gated(
                &format!(
                    "{}{}",
                    client.api_base_url,
                    bcvault::bandcamp::BAND_DETAILS_PATH
                ),
                &serde_json::json!({ "band_id": page.band_id }),
                GateClass::Api,
                Some(page.band_id),
            )
            .await
        {
            let details_file = out_dir.join(format!("{}_band_details.json", norm.slug));
            std::fs::write(&details_file, sanitize_text(&details_raw))?;
        }

        // Capture up to 3 sample releases per artist
        for item in page.items.iter().take(3) {
            let (rel_html, _) = client
                .get_text_gated(&item.url, GateClass::Page, Some(page.band_id))
                .await?;
            let rel_parsed = bcvault::bandcamp::tralbum::parse_release_html(&rel_html)?;
            tracing::info!(
                "  Item {:?}: class={:?} tracks={}",
                item.key,
                rel_parsed.classification,
                rel_parsed.track_count
            );
            let rel_file = out_dir.join(format!(
                "{}_{}_{}.html",
                norm.slug,
                item.key.kind.as_str(),
                item.key.id
            ));
            std::fs::write(&rel_file, sanitize_text(&rel_html))?;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    Ok(())
}

fn sanitize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        let mut clean = line.to_string();
        if let Some(idx) = clean.find("sig=") {
            let after = &clean[idx + 4..];
            let end = after.find(['&', '"', '\'', ' ']).unwrap_or(after.len());
            clean.replace_range(idx + 4..idx + 4 + end, "SANITIZED");
        }
        out.push_str(&clean);
        out.push('\n');
    }
    out
}
