use crate::{
    db::{self, FxDb, ReleaseDetail, TrackEntry},
    services::messages::FxEvent,
};
use bcvault_core::{
    bandcamp::BandcampClient,
    db::{DbPool, repo::releases},
    types::ItemKey,
    worker::bridge::RepaintHook,
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub struct DetailService {
    fx_db: FxDb,
    main_db: DbPool,
    client: BandcampClient,
    ev_tx: UnboundedSender<FxEvent>,
    repaint_hook: Option<RepaintHook>,
    in_flight: Arc<Mutex<HashSet<ItemKey>>>,
}

impl DetailService {
    pub fn new(
        fx_db: FxDb,
        main_db: DbPool,
        client: BandcampClient,
        ev_tx: UnboundedSender<FxEvent>,
        repaint_hook: Option<RepaintHook>,
    ) -> Self {
        Self {
            fx_db,
            main_db,
            client,
            ev_tx,
            repaint_hook,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn emit(&self, ev: FxEvent) {
        let _ = self.ev_tx.send(ev);
        if let Some(h) = &self.repaint_hook {
            h();
        }
    }

    pub fn ensure(&self, key: ItemKey) {
        {
            let mut g = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
            if !g.insert(key) {
                return;
            }
        }
        let svc = self.clone();
        tokio::spawn(async move {
            svc.fetch_or_load(key).await;
            let mut g = svc.in_flight.lock().unwrap_or_else(|e| e.into_inner());
            g.remove(&key);
        });
    }

    async fn fetch_or_load(&self, key: ItemKey) {
        if let Ok(Some(cached)) = self
            .fx_db
            .reader
            .call(move |c| db::get_release_detail(c, key))
            .await
        {
            self.emit(FxEvent::DetailLoaded {
                key,
                detail: Box::new(cached),
            });
            return;
        }

        let Ok(Some(rel)) = self
            .main_db
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
        else {
            return;
        };

        let Ok(raw) = self
            .client
            .fetch_release_raw(&rel.url, Some(rel.band_id))
            .await
        else {
            return;
        };

        let detail = parse_raw_detail(&raw);
        let detail_for_db = detail.clone();
        let _ = self
            .fx_db
            .writer
            .call(move |c| db::upsert_release_detail(c, key, &detail_for_db))
            .await;

        self.emit(FxEvent::DetailLoaded {
            key,
            detail: Box::new(detail),
        });
    }
}

pub fn parse_raw_detail(raw: &serde_json::Value) -> ReleaseDetail {
    let mut tracks = Vec::new();
    let mut total_dur = 0_u32;

    if let Some(arr) = raw.get("trackinfo").and_then(|v| v.as_array()) {
        for (idx, item) in arr.iter().enumerate() {
            let track_num = item
                .get("track_num")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or((idx + 1) as u32);
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Untitled")
                .to_string();
            let duration_s = item
                .get("duration")
                .and_then(|v| v.as_f64())
                .map(|d| d.round().max(0.0) as u32);
            if let Some(d) = duration_s {
                total_dur += d;
            }
            let stream_url = item
                .get("file")
                .and_then(|f| f.get("mp3-128"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            tracks.push(TrackEntry {
                track_num,
                title,
                duration_s,
                stream_url,
            });
        }
    }

    let current = raw.get("current");
    let about = current
        .and_then(|c| c.get("about"))
        .and_then(|v| v.as_str())
        .or_else(|| raw.get("_dom_about").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let credits = current
        .and_then(|c| c.get("credits"))
        .and_then(|v| v.as_str())
        .or_else(|| raw.get("_dom_credits").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let tags: Vec<String> = raw
        .get("_dom_tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    ReleaseDetail {
        fetched_at: chrono::Utc::now().timestamp(),
        tracks,
        about,
        credits,
        tags,
        duration_s: if total_dur > 0 { Some(total_dur) } else { None },
    }
}
