use crate::{
    bandcamp::{BandcampClient, gate::GateClass},
    db::{DbPool, repo::releases},
    types::ItemKey,
    worker::{bridge::UiBridge, messages::WorkerToUiEvent},
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct ThumbService {
    pool: DbPool,
    client: BandcampClient,
    bridge: UiBridge,
    cache_dir: PathBuf,
    in_flight: Arc<Mutex<HashSet<ItemKey>>>,
}

impl ThumbService {
    pub fn new(pool: DbPool, client: BandcampClient, bridge: UiBridge, cache_dir: PathBuf) -> Self {
        Self {
            pool,
            client,
            bridge,
            cache_dir,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn request_thumbs(&self, keys: Vec<ItemKey>) {
        for key in keys {
            let cached = self.thumb_path(key);
            if cached.exists() {
                self.bridge
                    .emit(WorkerToUiEvent::ThumbReady { key, path: cached });
                continue;
            }

            {
                let mut guard = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
                if !guard.insert(key) {
                    continue;
                }
            }

            let svc = self.clone();
            tokio::spawn(async move {
                let _ = svc.fetch_one(key).await;
                let mut guard = svc.in_flight.lock().unwrap_or_else(|e| e.into_inner());
                guard.remove(&key);
            });
        }
    }

    pub fn thumb_path(&self, key: ItemKey) -> PathBuf {
        self.cache_dir
            .join(format!("{}_{}.jpg", key.kind.as_str(), key.id))
    }

    async fn fetch_one(&self, key: ItemKey) -> Option<()> {
        let row = self
            .pool
            .reader
            .call(move |c| releases::get_release(c, key))
            .await
            .ok()??;

        let url = row
            .cover_url
            .or_else(|| row.art_id.map(|id| self.client.cover_url_for_art_id(id)))?;

        let dest = self.thumb_path(key);
        if fetch_cover_to_path(&self.client, &url, &dest).await.is_ok() {
            self.bridge
                .emit(WorkerToUiEvent::ThumbReady { key, path: dest });
        }
        Some(())
    }
}

pub async fn fetch_cover_to_path(
    client: &BandcampClient,
    url: &str,
    dest: &Path,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    let _permit = client.gate.acquire(GateClass::Cdn, None).await;
    let resp = client
        .page_client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    client.gate.report(status, None);
    let resp = resp.error_for_status().map_err(|e| e.to_string())?;
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;

    let part = dest.with_extension("jpg.part");
    tokio::fs::write(&part, &bytes)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&part, dest)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
