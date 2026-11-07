use crate::{
    types::{ItemKey, Settings},
    ui::theme::pal,
    ui::widgets::sharp_rect,
    worker::messages::UiToWorkerCommand,
};
use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Ui, Vec2};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

pub struct ThumbCache {
    paths: HashMap<ItemKey, String>, // "file://..." URI
    requested: HashSet<ItemKey>,
    pending_visible: HashSet<ItemKey>,
    last_visible_change: Option<Instant>,
}

impl Default for ThumbCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbCache {
    pub fn new() -> Self {
        Self {
            paths: HashMap::new(),
            requested: HashSet::new(),
            pending_visible: HashSet::new(),
            last_visible_change: None,
        }
    }

    pub fn on_thumb_ready(&mut self, key: ItemKey, path: PathBuf) {
        let uri = format!("file://{}", path.to_string_lossy());
        self.paths.insert(key, uri);
    }

    pub fn mark_visible(&mut self, key: ItemKey) {
        if !self.paths.contains_key(&key)
            && !self.requested.contains(&key)
            && self.pending_visible.insert(key)
        {
            self.last_visible_change = Some(Instant::now());
        }
    }

    pub fn flush_debounced(
        &mut self,
        settings: &Settings,
        cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    ) {
        if !settings.show_thumbnails || self.pending_visible.is_empty() {
            return;
        }
        if let Some(t) = self.last_visible_change
            && t.elapsed() >= Duration::from_millis(120)
        {
            let keys: Vec<ItemKey> = self.pending_visible.drain().collect();
            for &k in &keys {
                self.requested.insert(k);
            }
            self.last_visible_change = None;
            let _ = cmd_tx.send(UiToWorkerCommand::RequestThumbs { keys });
        }
    }

    pub fn draw_thumb(&mut self, ui: &mut Ui, key: ItemKey) {
        if let Some(uri) = self.paths.get(&key) {
            ui.add(
                egui::Image::new(uri.as_str())
                    .fit_to_exact_size(Vec2::splat(20.0))
                    .corner_radius(CornerRadius::ZERO),
            );
        } else {
            self.mark_visible(key);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
            if ui.is_rect_visible(rect) {
                sharp_rect(
                    ui.painter(),
                    Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
                    Color32::from_rgb(0x16, 0x18, 0x1B),
                    Stroke::new(1.0_f32, pal::BORDER),
                );
            }
        }
    }
}
