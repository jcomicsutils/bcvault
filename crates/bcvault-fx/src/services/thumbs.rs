use crate::{
    db::{self, FxDb, Palette},
    motion::oklab::OkLab,
    services::messages::{FxEvent, Tier},
};
use bcvault_core::{
    bandcamp::{BandcampClient, gate::GateClass},
    paths::to_os_safe_path,
    worker::bridge::RepaintHook,
};
use egui::{ColorImage, TextureHandle, TextureOptions};
use image::{DynamicImage, GenericImageView, ImageFormat, imageops::FilterType};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Semaphore, mpsc::UnboundedSender};

/// In-tree Texture LRU cache keyed by `(art_id, Tier)` with a configurable byte budget and visible-set pinning.
pub struct TextureCache {
    entries: HashMap<(i64, Tier), (TextureHandle, usize)>,
    order: VecDeque<(i64, Tier)>,
    pinned: HashSet<(i64, Tier)>,
    used_bytes: usize,
    budget_bytes: usize,
    pending_uploads: VecDeque<(i64, Tier, Arc<ColorImage>)>,
    uploads_this_frame: usize,
}

impl TextureCache {
    pub fn new(budget_mb: u32) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            pinned: HashSet::new(),
            used_bytes: 0,
            budget_bytes: (budget_mb as usize) * 1024 * 1024,
            pending_uploads: VecDeque::new(),
            uploads_this_frame: 0,
        }
    }

    pub fn set_budget_mb(&mut self, budget_mb: u32) {
        self.budget_bytes = (budget_mb.clamp(32, 1024) as usize) * 1024 * 1024;
        self.evict_to_budget();
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    pub fn uploads_last_frame(&self) -> usize {
        self.uploads_this_frame
    }

    pub fn pending_upload_count(&self) -> usize {
        self.pending_uploads.len()
    }

    pub fn set_pinned(&mut self, visible: HashSet<(i64, Tier)>) {
        self.pinned = visible;
    }

    pub fn enqueue_upload(&mut self, art_id: i64, tier: Tier, image: Arc<ColorImage>) {
        if self.entries.contains_key(&(art_id, tier)) {
            return;
        }
        if !self
            .pending_uploads
            .iter()
            .any(|(id, t, _)| *id == art_id && *t == tier)
        {
            self.pending_uploads.push_back((art_id, tier, image));
        }
    }

    /// Processes up to 4 texture uploads per frame (~2 ms budget).
    pub fn flush_frame_uploads(&mut self, ctx: &egui::Context) -> Vec<(i64, Tier)> {
        self.uploads_this_frame = 0;
        let mut uploaded = Vec::new();
        while self.uploads_this_frame < 4 {
            let Some((art_id, tier, img)) = self.pending_uploads.pop_front() else {
                break;
            };
            if self.entries.contains_key(&(art_id, tier)) {
                continue;
            }
            let bytes = img.pixels.len() * 4;
            let tex = ctx.load_texture(
                format!("fx_cov_{art_id}_{}", tier.suffix()),
                (*img).clone(),
                TextureOptions::LINEAR,
            );
            self.insert_handle((art_id, tier), tex, bytes);
            self.uploads_this_frame += 1;
            uploaded.push((art_id, tier));
        }
        if !self.pending_uploads.is_empty() {
            ctx.request_repaint();
        }
        uploaded
    }

    pub fn insert_handle(&mut self, key: (i64, Tier), handle: TextureHandle, bytes: usize) {
        if let Some((_, old_b)) = self.entries.remove(&key) {
            self.used_bytes = self.used_bytes.saturating_sub(old_b);
            self.order.retain(|k| *k != key);
        }
        self.used_bytes += bytes;
        self.entries.insert(key, (handle, bytes));
        self.order.push_back(key);
        self.evict_to_budget();
    }

    pub fn get(&mut self, key: (i64, Tier)) -> Option<&TextureHandle> {
        if self.entries.contains_key(&key) {
            self.order.retain(|k| *k != key);
            self.order.push_back(key);
            self.entries.get(&key).map(|(h, _)| h)
        } else {
            None
        }
    }

    pub fn contains(&self, key: (i64, Tier)) -> bool {
        self.entries.contains_key(&key)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.pending_uploads.clear();
        self.used_bytes = 0;
    }

    fn evict_to_budget(&mut self) {
        let mut scanned = 0;
        let max_scan = self.order.len();
        while self.used_bytes > self.budget_bytes && scanned < max_scan {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            scanned += 1;
            if self.pinned.contains(&oldest) {
                self.order.push_back(oldest);
                continue;
            }
            if let Some((_, b)) = self.entries.remove(&oldest) {
                self.used_bytes = self.used_bytes.saturating_sub(b);
            }
        }
    }
}

pub fn crop_and_resize_square(img: &DynamicImage, edge: u32) -> ColorImage {
    let (w, h) = img.dimensions();
    let side = w.min(h).max(1);
    let x0 = (w.saturating_sub(side)) / 2;
    let y0 = (h.saturating_sub(side)) / 2;
    let cropped = img.crop_imm(x0, y0, side, side);
    let resized = cropped.resize_exact(edge, edge, FilterType::Lanczos3);
    let rgba = resized.to_rgba8();
    ColorImage::from_rgba_unmultiplied([edge as usize, edge as usize], rgba.as_raw())
}

pub fn color_image_to_jpeg(img: &ColorImage) -> Result<Vec<u8>, String> {
    let w = img.size[0] as u32;
    let h = img.size[1] as u32;
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for p in &img.pixels {
        let [r, g, b, _] = p.to_srgba_unmultiplied();
        rgb.push(r);
        rgb.push(g);
        rgb.push(b);
    }
    let buf: image::RgbImage =
        image::RgbImage::from_raw(w, h, rgb).ok_or_else(|| "invalid rgb buffer".to_string())?;
    let mut out = Vec::new();
    let mut cursor = Cursor::new(&mut out);
    DynamicImage::ImageRgb8(buf)
        .write_to(&mut cursor, ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn decode_color_image(bytes: &[u8]) -> Result<ColorImage, String> {
    let dyn_img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let (w, h) = dyn_img.dimensions();
    let rgba = dyn_img.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        rgba.as_raw(),
    ))
}

/// Extracts `tiny8` (8x8 RGB = 192 bytes) and a 5-swatch median-cut palette in OKLab from a `T64` image.
pub fn extract_palette(art_id: i64, img64: &ColorImage) -> Palette {
    let w = img64.size[0].max(1);
    let h = img64.size[1].max(1);

    // 1. Build 8x8 tiny8 mosaic by averaging each 8x8 block
    let mut tiny8 = [0_u8; 192];
    for gy in 0..8 {
        for gx in 0..8 {
            let x_start = (gx * w) / 8;
            let x_end = (((gx + 1) * w) / 8).max(x_start + 1);
            let y_start = (gy * h) / 8;
            let y_end = (((gy + 1) * h) / 8).max(y_start + 1);
            let mut r_sum = 0_u32;
            let mut g_sum = 0_u32;
            let mut b_sum = 0_u32;
            let mut cnt = 0_u32;
            for py in y_start..y_end.min(h) {
                for px in x_start..x_end.min(w) {
                    let [r, g, b, _] = img64.pixels[py * w + px].to_srgba_unmultiplied();
                    r_sum += u32::from(r);
                    g_sum += u32::from(g);
                    b_sum += u32::from(b);
                    cnt += 1;
                }
            }
            let cnt = cnt.max(1);
            let base = (gy * 8 + gx) * 3;
            tiny8[base] = (r_sum / cnt) as u8;
            tiny8[base + 1] = (g_sum / cnt) as u8;
            tiny8[base + 2] = (b_sum / cnt) as u8;
        }
    }

    // 2. Median cut into up to 5 buckets in OKLab space
    let samples: Vec<OkLab> = img64
        .pixels
        .iter()
        .map(|c| OkLab::from_color32(*c))
        .collect();

    let mut buckets: Vec<Vec<OkLab>> = vec![samples];
    while buckets.len() < 5 {
        let Some((split_idx, _)) = buckets
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() >= 4)
            .max_by_key(|(_, b)| b.len())
        else {
            break;
        };
        let mut bucket = buckets.swap_remove(split_idx);
        let (mut l_min, mut l_max) = (f32::MAX, f32::MIN);
        let (mut a_min, mut a_max) = (f32::MAX, f32::MIN);
        let (mut b_min, mut b_max) = (f32::MAX, f32::MIN);
        for p in &bucket {
            l_min = l_min.min(p.l);
            l_max = l_max.max(p.l);
            a_min = a_min.min(p.a);
            a_max = a_max.max(p.a);
            b_min = b_min.min(p.b);
            b_max = b_max.max(p.b);
        }
        let dl = l_max - l_min;
        let da = a_max - a_min;
        let db = b_max - b_min;
        if dl >= da && dl >= db {
            bucket.sort_by(|x, y| x.l.total_cmp(&y.l));
        } else if da >= db {
            bucket.sort_by(|x, y| x.a.total_cmp(&y.a));
        } else {
            bucket.sort_by(|x, y| x.b.total_cmp(&y.b));
        }
        let mid = bucket.len() / 2;
        let right = bucket.split_off(mid);
        buckets.push(bucket);
        buckets.push(right);
    }

    buckets.sort_by_key(|b| std::cmp::Reverse(b.len()));

    let mut swatches = [[0_u8; 3]; 5];
    let mut best_accent = [0x00_u8, 0xD1, 0xB2];
    let mut best_chroma = -1.0_f32;
    let mut dom_lch = OkLab::from_rgb([128, 128, 128], 1.0).to_lch();

    for (i, b) in buckets.iter().take(5).enumerate() {
        if b.is_empty() {
            continue;
        }
        let inv = 1.0 / (b.len() as f32);
        let avg = OkLab {
            l: b.iter().map(|p| p.l).sum::<f32>() * inv,
            a: b.iter().map(|p| p.a).sum::<f32>() * inv,
            b: b.iter().map(|p| p.b).sum::<f32>() * inv,
            alpha: 1.0,
        };
        let rgb = avg.to_rgb();
        swatches[i] = rgb;
        let lch = avg.to_lch();
        if i == 0 {
            dom_lch = lch;
        }
        let score = lch.c * (1.0 - (lch.l - 0.62).abs());
        if score > best_chroma {
            best_chroma = score;
            best_accent = rgb;
            if lch.c > dom_lch.c {
                dom_lch = lch;
            }
        }
    }
    for i in buckets.len()..5 {
        swatches[i] = swatches[0];
    }

    let hue = if dom_lch.c >= 0.025 {
        Some((dom_lch.h_deg.round() as u16) % 360)
    } else {
        None
    };
    let chroma = ((dom_lch.c * 300.0).round().clamp(0.0, 100.0)) as u8;
    let luma = ((dom_lch.l * 100.0).round().clamp(0.0, 100.0)) as u8;

    Palette {
        art_id,
        tiny8,
        swatches,
        accent: best_accent,
        hue,
        chroma,
        luma,
    }
}

pub fn tier_cache_path(cover_dir: &Path, art_id: i64, tier: Tier) -> PathBuf {
    let bucket = (art_id.unsigned_abs() % 256) as u8;
    let path = cover_dir
        .join(format!("{bucket:02x}"))
        .join(format!("{art_id}.{}.jpg", tier.suffix()));
    to_os_safe_path(&path)
}

#[derive(Clone)]
pub struct FxThumbService {
    fx_db: FxDb,
    client: BandcampClient,
    cover_dir: PathBuf,
    ev_tx: UnboundedSender<FxEvent>,
    repaint_hook: Option<RepaintHook>,
    generation: Arc<AtomicU64>,
    in_flight: Arc<Mutex<HashSet<(i64, Tier)>>>,
    decode_sem: Arc<Semaphore>,
}

impl FxThumbService {
    pub fn new(
        fx_db: FxDb,
        client: BandcampClient,
        cover_dir: PathBuf,
        ev_tx: UnboundedSender<FxEvent>,
        repaint_hook: Option<RepaintHook>,
    ) -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).max(1))
            .unwrap_or(2);
        Self {
            fx_db,
            client,
            cover_dir,
            ev_tx,
            repaint_hook,
            generation: Arc::new(AtomicU64::new(0)),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
            decode_sem: Arc::new(Semaphore::new(cpus)),
        }
    }

    fn emit(&self, ev: FxEvent) {
        let _ = self.ev_tx.send(ev);
        if let Some(h) = &self.repaint_hook {
            h();
        }
    }

    pub fn ensure_thumbs(&self, generation: u64, mut items: Vec<(i64, Tier, u32)>) {
        self.generation.store(generation, Ordering::SeqCst);
        items.sort_by_key(|(_, _, prio)| *prio);

        for (art_id, tier, _) in items {
            if art_id <= 0 {
                continue;
            }
            {
                let mut g = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
                if !g.insert((art_id, tier)) {
                    continue;
                }
            }
            let svc = self.clone();
            tokio::spawn(async move {
                if svc.generation.load(Ordering::SeqCst) == generation {
                    svc.process_one(art_id, tier, generation).await;
                }
                let mut g = svc.in_flight.lock().unwrap_or_else(|e| e.into_inner());
                g.remove(&(art_id, tier));
            });
        }
    }

    pub fn ensure_hero(&self, art_id: i64) {
        if art_id <= 0 {
            return;
        }
        let current_gen = self.generation.load(Ordering::SeqCst);
        {
            let mut g = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
            if !g.insert((art_id, Tier::Hero)) {
                return;
            }
        }
        let svc = self.clone();
        tokio::spawn(async move {
            svc.process_one(art_id, Tier::Hero, current_gen).await;
            let mut g = svc.in_flight.lock().unwrap_or_else(|e| e.into_inner());
            g.remove(&(art_id, Tier::Hero));
        });
    }

    async fn process_one(&self, art_id: i64, tier: Tier, req_gen: u64) {
        if tier != Tier::Hero && self.generation.load(Ordering::SeqCst) != req_gen {
            return;
        }

        let target_path = tier_cache_path(&self.cover_dir, art_id, tier);
        if target_path.exists()
            && let Ok(bytes) = tokio::fs::read(&target_path).await
        {
            let _permit = self.decode_sem.acquire().await;
            if let Ok(Ok(img)) =
                tokio::task::spawn_blocking(move || decode_color_image(&bytes)).await
            {
                self.emit(FxEvent::ThumbReady {
                    art_id,
                    tier,
                    image: Arc::new(img),
                });
                return;
            }
        }

        // Check negative cache before network fetch
        if let Ok(true) = self
            .fx_db
            .reader
            .call(move |c| db::is_cover_in_negative_cache(c, art_id))
            .await
        {
            return;
        }

        if tier != Tier::Hero && self.generation.load(Ordering::SeqCst) != req_gen {
            return;
        }

        let url = self.client.cover_url_for_art_id(art_id);
        let bytes = match self.client.fetch_bytes_gated(GateClass::Cdn, &url).await {
            Ok(b) => b,
            Err(_) => {
                let _ = self
                    .fx_db
                    .writer
                    .call(move |c| db::record_cover_failure(c, art_id))
                    .await;
                self.emit(FxEvent::ThumbFailed { art_id });
                return;
            }
        };

        let cover_dir = self.cover_dir.clone();
        let _permit = self.decode_sem.acquire().await;
        let derived =
            tokio::task::spawn_blocking(move || -> Result<(ColorImage, Palette), String> {
                let dyn_img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
                let img64 = crop_and_resize_square(&dyn_img, Tier::T64.edge_px());
                let img192 = crop_and_resize_square(&dyn_img, Tier::T192.edge_px());
                let img384 = crop_and_resize_square(&dyn_img, Tier::T384.edge_px());

                for (t, im) in [
                    (Tier::T64, &img64),
                    (Tier::T192, &img192),
                    (Tier::T384, &img384),
                ] {
                    let p = tier_cache_path(&cover_dir, art_id, t);
                    if let Some(parent) = p.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Ok(jpg) = color_image_to_jpeg(im) {
                        let part = p.with_extension("jpg.part");
                        if std::fs::write(&part, &jpg).is_ok() {
                            let _ = std::fs::rename(&part, &p);
                        }
                    }
                }

                let requested_img = match tier {
                    Tier::T64 => img64.clone(),
                    Tier::T192 => img192,
                    Tier::T384 => img384,
                    Tier::Hero => {
                        let hero = crop_and_resize_square(&dyn_img, Tier::Hero.edge_px());
                        let p = tier_cache_path(&cover_dir, art_id, Tier::Hero);
                        if let Some(parent) = p.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Ok(jpg) = color_image_to_jpeg(&hero) {
                            let part = p.with_extension("jpg.part");
                            if std::fs::write(&part, &jpg).is_ok() {
                                let _ = std::fs::rename(&part, &p);
                            }
                        }
                        hero
                    }
                };

                let palette = extract_palette(art_id, &img64);
                Ok((requested_img, palette))
            })
            .await;

        if let Ok(Ok((req_img, palette))) = derived {
            let pal_for_db = palette.clone();
            let _ = self
                .fx_db
                .writer
                .call(move |c| db::upsert_cover_meta(c, &pal_for_db))
                .await;
            self.emit(FxEvent::PaletteReady { art_id, palette });
            self.emit(FxEvent::ThumbReady {
                art_id,
                tier,
                image: Arc::new(req_img),
            });
        } else {
            let _ = self
                .fx_db
                .writer
                .call(move |c| db::record_cover_failure(c, art_id))
                .await;
            self.emit(FxEvent::ThumbFailed { art_id });
        }
    }
}

pub fn compute_dir_size(path: &Path) -> u64 {
    let mut total = 0_u64;
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total += compute_dir_size(&p);
        } else if let Ok(m) = entry.metadata() {
            total += m.len();
        }
    }
    total
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use bcvault_core::{bandcamp::gate::BandcampGate, db::DbPool};
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::mpsc;
    use wiremock::{
        Mock, MockServer, Request, Respond, ResponseTemplate,
        matchers::{method, path},
    };

    fn make_solid_jpeg(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut buf = image::RgbImage::new(w, h);
        for p in buf.pixels_mut() {
            *p = image::Rgb(rgb);
        }
        let mut out = Vec::new();
        DynamicImage::ImageRgb8(buf)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
            .unwrap();
        out
    }

    #[test]
    fn crop_and_resize_exact_dimensions_and_palette() {
        for (w, h) in [(400, 400), (600, 300), (250, 500), (32, 32)] {
            let bytes = make_solid_jpeg(w, h, [220, 40, 60]);
            let dyn_img = image::load_from_memory(&bytes).unwrap();
            let t64 = crop_and_resize_square(&dyn_img, 64);
            let t192 = crop_and_resize_square(&dyn_img, 192);
            assert_eq!(t64.size, [64, 64]);
            assert_eq!(t192.size, [192, 192]);

            let pal = extract_palette(1, &t64);
            assert!(pal.hue.is_some());
            assert!(pal.accent[0] > 180);
        }
    }

    #[test]
    fn lru_respects_budget_and_pins() {
        let ctx = egui::Context::default();
        let mut cache = TextureCache::new(1); // 1 MB = 1_048_576 bytes
        let img = ColorImage::filled([256, 256], egui::Color32::RED);

        // Pin item 1 so item 2 gets evicted when budget is exceeded
        let mut pinned = HashSet::new();
        pinned.insert((1, Tier::T192));
        cache.set_pinned(pinned);

        for id in 1..=3 {
            let h = ctx.load_texture(format!("t{id}"), img.clone(), TextureOptions::LINEAR);
            cache.insert_handle((id, Tier::T192), h, 300_000);
        }

        let h4 = ctx.load_texture("t4", img, TextureOptions::LINEAR);
        cache.insert_handle((4, Tier::T192), h4, 300_000);

        assert!(cache.used_bytes() <= cache.budget_bytes());
        assert!(cache.contains((1, Tier::T192)), "pinned entry must survive");
        assert!(
            !cache.contains((2, Tier::T192)),
            "oldest unpinned must evict"
        );
    }

    #[tokio::test]
    async fn wiremock_fetch_once_and_disk_cache_hit() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let main_db = dir.path().join("bcvault.db");
        let fx_db_path = dir.path().join("fx.sqlite");
        let cover_dir = dir.path().join("covers");

        let _main_pool = DbPool::open(&main_db).unwrap();
        let fx_db = FxDb::open(&fx_db_path, &main_db).unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = hits.clone();
        let jpeg_bytes = make_solid_jpeg(400, 400, [30, 200, 140]);

        struct CountingCover(Arc<AtomicUsize>, Vec<u8>);
        impl Respond for CountingCover {
            fn respond(&self, _req: &Request) -> ResponseTemplate {
                self.0.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_bytes(self.1.clone())
            }
        }

        Mock::given(method("GET"))
            .and(path("/img/a0000000777_10.jpg"))
            .respond_with(CountingCover(hits_clone, jpeg_bytes))
            .mount(&server)
            .await;

        let gate = BandcampGate::with_delays(0, 0, 0, 0, 2, None);
        let client = BandcampClient::new(gate, "test")
            .unwrap()
            .with_base_urls(server.uri(), server.uri());

        let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
        let svc = FxThumbService::new(fx_db, client, cover_dir, ev_tx, None);

        // First request -> network fetch + derives all tiers
        svc.process_one(777, Tier::T64, 0).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // Second request (even for T192!) -> served from disk with zero HTTP calls
        svc.process_one(777, Tier::T192, 0).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let mut ready_tiers = Vec::new();
        while let Ok(ev) = ev_rx.try_recv() {
            if let FxEvent::ThumbReady { tier, .. } = ev {
                ready_tiers.push(tier);
            }
        }
        assert!(ready_tiers.contains(&Tier::T64));
        assert!(ready_tiers.contains(&Tier::T192));
    }
}
