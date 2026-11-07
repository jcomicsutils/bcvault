use crate::{
    bandcamp::{BandcampClient, input::normalize_artist_input},
    db::{
        DbPool,
        repo::{
            artists::{self, UpsertArtist},
            releases::{self, DiscoveredRelease, ReleaseMetadata},
        },
    },
    error::{BcError, DbError},
    types::{ItemKey, LogLevel, SyncMode, SyncPhase, SyncProgress},
    worker::{bridge::UiBridge, messages::WorkerToUiEvent},
};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct SyncService {
    pub pool: DbPool,
    pub client: BandcampClient,
    pub bridge: UiBridge,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncOutcome {
    pub band_id: i64,
    pub new_items: u32,
    pub updated: u32,
}

impl SyncService {
    pub fn new(pool: DbPool, client: BandcampClient, bridge: UiBridge) -> Self {
        Self {
            pool,
            client,
            bridge,
        }
    }

    pub async fn add_and_sync_artist(
        &self,
        raw_input: &str,
        cancel: CancellationToken,
    ) -> Result<SyncOutcome, BcError> {
        let norm = normalize_artist_input(raw_input)?;
        self.sync_resolved(
            &norm.base_url,
            &norm.slug,
            None,
            SyncMode::Incremental,
            cancel,
        )
        .await
    }

    pub async fn sync_artist(
        &self,
        band_id: i64,
        mode: SyncMode,
        cancel: CancellationToken,
    ) -> Result<SyncOutcome, BcError> {
        let view_opt = self
            .pool
            .reader
            .call(move |c| artists::get_view(c, band_id))
            .await
            .map_err(|e| BcError::UnexpectedResponse(e.to_string()))?;

        let Some(view) = view_opt else {
            return Err(BcError::BandIdNotFound);
        };

        self.sync_resolved(&view.url, &view.slug, Some(view.band_id), mode, cancel)
            .await
    }

    async fn sync_resolved(
        &self,
        base_url: &str,
        slug: &str,
        known_band_id: Option<i64>,
        mode: SyncMode,
        cancel: CancellationToken,
    ) -> Result<SyncOutcome, BcError> {
        if let Some(bid) = known_band_id {
            self.bridge
                .emit(WorkerToUiEvent::SyncProgress(SyncProgress {
                    band_id: bid,
                    phase: SyncPhase::Resolving,
                    done: 0,
                    total: 0,
                    current: None,
                }));
        }

        if cancel.is_cancelled() {
            return Ok(SyncOutcome {
                band_id: known_band_id.unwrap_or(0),
                new_items: 0,
                updated: 0,
            });
        }

        if let Some(bid) = known_band_id {
            self.bridge
                .emit(WorkerToUiEvent::SyncProgress(SyncProgress {
                    band_id: bid,
                    phase: SyncPhase::Discovering,
                    done: 0,
                    total: 0,
                    current: None,
                }));
        }

        let disc = self.client.fetch_discography(base_url, slug).await?;
        let band_id = known_band_id.unwrap_or(disc.band_id);

        // Upsert artist record immediately
        let base_owned = base_url.to_string();
        let slug_owned = slug.to_string();
        let name_owned = disc.name.clone();
        let cover_owned = disc.cover_url.clone();

        let artist_view = self
            .pool
            .writer
            .call(move |c| {
                artists::upsert(
                    c,
                    &UpsertArtist {
                        band_id,
                        slug: &slug_owned,
                        name: &name_owned,
                        url: &base_owned,
                        cover_url: cover_owned.as_deref(),
                    },
                )?;
                artists::get_view(c, band_id)
            })
            .await
            .map_err(db_to_bc)?;

        if let Some(av) = artist_view {
            self.bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
        }

        self.bridge
            .emit(WorkerToUiEvent::SyncProgress(SyncProgress {
                band_id,
                phase: SyncPhase::Diffing,
                done: 0,
                total: disc.items.len() as u32,
                current: None,
            }));

        let items_clone = disc.items.clone();
        let (new_count, queue, inserted_rows) = self
            .pool
            .writer
            .call(move |c| {
                let known = releases::known_keys(c, band_id)?;
                let mut new_items_vec = Vec::new();
                for item in &items_clone {
                    if !known.contains(&item.key) {
                        new_items_vec.push(item.key);
                    }
                }

                // Upsert all discovered items immediately (new ones become 'unknown', existing keep classification)
                let batch: Vec<DiscoveredRelease<'_>> = items_clone
                    .iter()
                    .map(|it| DiscoveredRelease {
                        key: it.key,
                        band_id,
                        title: &it.title,
                        artist_name: &it.artist_name,
                        url: &it.url,
                        cover_url: it.cover_url.as_deref(),
                        art_id: it.art_id,
                    })
                    .collect();
                releases::upsert_discovered_batch(c, &batch)?;

                let mut inserted_rows = Vec::new();
                for &k in &new_items_vec {
                    if let Some(r) = releases::get_release(c, k)? {
                        inserted_rows.push(r);
                    }
                }

                // Build metadata fetch queue by mode
                let mut q_set = HashSet::new();
                let mut q_vec: Vec<ItemKey> = Vec::new();
                let mut push_unique = |k: ItemKey| {
                    if q_set.insert(k) {
                        q_vec.push(k);
                    }
                };

                match mode {
                    SyncMode::Incremental => {
                        for k in &new_items_vec {
                            push_unique(*k);
                        }
                        for k in releases::unknown_keys(c, band_id)? {
                            push_unique(k);
                        }
                    }
                    SyncMode::CheckDownloaded => {
                        for k in &new_items_vec {
                            push_unique(*k);
                        }
                        for k in releases::unknown_keys(c, band_id)? {
                            push_unique(k);
                        }
                        for k in releases::downloaded_keys(c, band_id)? {
                            push_unique(k);
                        }
                    }
                    SyncMode::FullRefresh => {
                        for it in &items_clone {
                            push_unique(it.key);
                        }
                    }
                }

                Ok::<_, DbError>((new_items_vec.len() as u32, q_vec, inserted_rows))
            })
            .await
            .map_err(db_to_bc)?;

        for r in inserted_rows {
            self.bridge
                .emit(WorkerToUiEvent::ReleaseUpdated(Box::new(r)));
        }

        let total_queue = queue.len() as u32;
        let mut updated_count = 0_u32;

        for (idx, key) in queue.into_iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }

            let row_opt = self
                .pool
                .reader
                .call(move |c| releases::get_release(c, key))
                .await
                .map_err(db_to_bc)?;
            let Some(row) = row_opt else { continue };

            self.bridge
                .emit(WorkerToUiEvent::SyncProgress(SyncProgress {
                    band_id,
                    phase: SyncPhase::FetchingMetadata,
                    done: idx as u32,
                    total: total_queue,
                    current: Some(row.title.clone()),
                }));

            let fetch_res = tokio::select! {
                _ = cancel.cancelled() => break,
                res = self.client.fetch_release(&row.url, Some(band_id)) => res,
            };

            match fetch_res {
                Ok(parsed) => {
                    let updated_row = self
                        .pool
                        .writer
                        .call(move |c| {
                            releases::update_metadata(
                                c,
                                &ReleaseMetadata {
                                    key,
                                    classification: parsed.classification,
                                    pricing_json: Some(&parsed.pricing_json),
                                    is_preorder: parsed.is_preorder,
                                    track_count: Some(parsed.track_count),
                                    published_at: parsed.published_at,
                                    modified_at: parsed.modified_at,
                                    art_id: parsed.art_id,
                                },
                            )?;
                            releases::get_release(c, key)
                        })
                        .await
                        .map_err(db_to_bc)?;

                    if let Some(ur) = updated_row {
                        if ur.has_remote_update {
                            updated_count += 1;
                        }
                        self.bridge
                            .emit(WorkerToUiEvent::ReleaseUpdated(Box::new(ur)));
                    }
                }
                Err(e) => {
                    self.bridge.log(
                        LogLevel::Warn,
                        "sync",
                        format!("Failed to parse release {}: {e}", row.title),
                        Some(key),
                    );
                }
            }
        }

        self.bridge
            .emit(WorkerToUiEvent::SyncProgress(SyncProgress {
                band_id,
                phase: SyncPhase::Finalising,
                done: total_queue,
                total: total_queue,
                current: None,
            }));

        if !cancel.is_cancelled() {
            let (av_opt, stats) = self
                .pool
                .writer
                .call(move |c| {
                    artists::touch_last_synced(c, band_id)?;
                    let av = artists::get_view(c, band_id)?;
                    let st = artists::stats_with_quota(c, band_id)?;
                    Ok::<_, DbError>((av, st))
                })
                .await
                .map_err(db_to_bc)?;

            if let Some(av) = av_opt {
                self.bridge.emit(WorkerToUiEvent::ArtistUpserted(av));
            }
            self.bridge
                .emit(WorkerToUiEvent::ArtistStats { band_id, stats });
        }

        self.bridge.emit(WorkerToUiEvent::SyncFinished {
            band_id,
            new_items: new_count,
            updated: updated_count,
            error: None,
        });

        Ok(SyncOutcome {
            band_id,
            new_items: new_count,
            updated: updated_count,
        })
    }
}

fn db_to_bc(e: DbError) -> BcError {
    BcError::UnexpectedResponse(format!("Database error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>(_: T) {}

    #[allow(dead_code)]
    fn sync_futures_are_send(svc: SyncService) {
        let tok = CancellationToken::new();
        assert_send(svc.add_and_sync_artist("artist", tok.clone()));
        assert_send(svc.sync_artist(1, SyncMode::Incremental, tok));
    }
}
