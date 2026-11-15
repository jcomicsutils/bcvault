use crate::{
    actions::{FxAction, GroupBy},
    db::WallRow,
    motion::{EffectsLevel, SpringCfg, stagger_delay_s},
    paint,
    services::messages::{FxCommand, Tier},
    state::{FxState, paint_cover_slot},
};
use bcvault_core::{
    types::{Classification, ItemKey},
    worker::messages::UiToWorkerCommand,
};
use chrono::{Datelike, TimeZone, Utc};
use egui::{Align, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::{collections::HashSet, time::Duration};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug, PartialEq)]
pub struct WallGroup {
    pub label: String,
    pub header_y: f32,
    pub start_idx: usize,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WallLayout {
    pub cols: usize,
    pub card_w: f32,
    pub card_h: f32,
    pub gap: f32,
    pub label_h: f32,
    pub total_height: f32,
    pub card_positions: Vec<Pos2>,
    pub groups: Vec<WallGroup>,
}

impl WallLayout {
    pub fn compute(avail_w: f32, card_w: f32, rows: &[WallRow], group_by: GroupBy) -> Self {
        let gap = 12.0_f32;
        let label_h = 52.0_f32;
        let card_h = card_w + label_h;
        let usable_w = (avail_w - 28.0).max(card_w);
        let cols = (((usable_w + gap) / (card_w + gap)).floor() as usize).max(1);

        let mut card_positions = Vec::with_capacity(rows.len());
        let mut groups = Vec::new();
        let mut cursor_y = 8.0_f32;

        if group_by == GroupBy::None || rows.is_empty() {
            for idx in 0..rows.len() {
                let col = idx % cols;
                let row = idx / cols;
                let x = 8.0 + (col as f32) * (card_w + gap);
                let y = cursor_y + (row as f32) * (card_h + gap);
                card_positions.push(Pos2::new(x, y));
            }
            let num_rows = rows.len().div_ceil(cols);
            cursor_y += (num_rows as f32) * (card_h + gap);
        } else {
            let mut idx = 0;
            while idx < rows.len() {
                let g_label = group_label_for(&rows[idx], group_by);
                let start_idx = idx;
                while idx < rows.len() && group_label_for(&rows[idx], group_by) == g_label {
                    idx += 1;
                }
                let count = idx - start_idx;
                let header_y = cursor_y;
                cursor_y += 32.0;
                for local_i in 0..count {
                    let col = local_i % cols;
                    let row = local_i / cols;
                    let x = 8.0 + (col as f32) * (card_w + gap);
                    let y = cursor_y + (row as f32) * (card_h + gap);
                    card_positions.push(Pos2::new(x, y));
                }
                let num_rows = count.div_ceil(cols);
                cursor_y += (num_rows as f32) * (card_h + gap) + 8.0;
                groups.push(WallGroup {
                    label: g_label,
                    header_y,
                    start_idx,
                    count,
                });
            }
        }

        Self {
            cols,
            card_w,
            card_h,
            gap,
            label_h,
            total_height: cursor_y.max(120.0),
            card_positions,
            groups,
        }
    }

    pub fn pos(&self, index: usize) -> Option<Pos2> {
        self.card_positions.get(index).copied()
    }

    pub fn card_rect(&self, index: usize, origin: Pos2) -> Option<Rect> {
        let p = self.pos(index)?;
        Some(Rect::from_min_size(
            Pos2::new(origin.x + p.x, origin.y + p.y),
            Vec2::new(self.card_w, self.card_h),
        ))
    }

    pub fn visible_range(&self, viewport_min_y: f32, viewport_max_y: f32) -> (usize, usize) {
        if self.card_positions.is_empty() {
            return (0, 0);
        }
        let start = self
            .card_positions
            .partition_point(|p| p.y + self.card_h < viewport_min_y);
        let end = self
            .card_positions
            .partition_point(|p| p.y <= viewport_max_y)
            .min(self.card_positions.len());
        (start.min(end), end)
    }

    pub fn index_at(&self, local_pt: Pos2) -> Option<usize> {
        let (lo, hi) = self.visible_range(local_pt.y - 4.0, local_pt.y + 4.0);
        for idx in lo..hi {
            let p = self.card_positions[idx];
            let r = Rect::from_min_size(p, Vec2::new(self.card_w, self.card_h));
            if r.contains(local_pt) {
                return Some(idx);
            }
        }
        None
    }
}

fn group_label_for(row: &WallRow, group_by: GroupBy) -> String {
    match group_by {
        GroupBy::None => String::new(),
        GroupBy::Year => row
            .rel
            .published_at
            .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
            .map(|dt| dt.year().to_string())
            .unwrap_or_else(|| "Unknown Year".to_string()),
        GroupBy::Artist => row.rel.artist_name.clone(),
        GroupBy::Classification => row.rel.classification.label().to_string(),
        GroupBy::Initial => row
            .rel
            .title
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "#".to_string()),
    }
}

pub fn show(
    ui: &mut Ui,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    now_s: f64,
) {
    let th = state.active_theme;
    let avail = ui.available_rect_before_wrap();

    let card_w = state
        .motion
        .val("wall_card_w", state.card_size(), SpringCfg::SOFT);
    let layout = WallLayout::compute(avail.width(), card_w, &state.rows, state.query.group);
    let ppp = ui.ctx().pixels_per_point();
    let tier = Tier::for_display_px(card_w * ppp);

    let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(target_y) = state.pending_scroll_to_y.take() {
        scroll_area = scroll_area.vertical_scroll_offset(target_y);
    }

    let mut pending_actions: Vec<FxAction> = Vec::new();
    let mut new_hovered_key: Option<ItemKey> = None;
    let mut new_hovered_art: Option<i64> = None;

    let scroll_out = scroll_area.show_viewport(ui, |ui, viewport| {
        state.scroll_y = viewport.min.y;
        let (content_rect, bg_resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 20.0, layout.total_height),
            Sense::click_and_drag(),
        );
        let origin = content_rect.min;

        if bg_resp.drag_started() {
            state.marquee_start = bg_resp.interact_pointer_pos();
            state.marquee_current = state.marquee_start;
        }
        if bg_resp.dragged() {
            state.marquee_current = bg_resp.interact_pointer_pos();
        }
        if bg_resp.drag_stopped() {
            if let (Some(p0), Some(p1)) = (state.marquee_start.take(), state.marquee_current.take())
            {
                let sel_rect = Rect::from_two_pos(p0, p1);
                if sel_rect.width() > 4.0 || sel_rect.height() > 4.0 {
                    if !ui.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                        state.selected_keys.clear();
                    }
                    let (v_lo, v_hi) = layout.visible_range(
                        viewport.min.y - layout.card_h,
                        viewport.max.y + layout.card_h,
                    );
                    for idx in v_lo..v_hi {
                        if let Some(cr) = layout.card_rect(idx, origin)
                            && sel_rect.intersects(cr)
                        {
                            state.selected_keys.insert(state.rows[idx].rel.key);
                        }
                    }
                    state.mark_selection_dirty();
                }
            }
        } else if bg_resp.clicked() {
            pending_actions.push(FxAction::ClearSelection);
        }

        let (vis_lo, vis_hi) = layout.visible_range(viewport.min.y - 40.0, viewport.max.y + 40.0);
        let (pref_lo, pref_hi) = layout.visible_range(
            viewport.min.y - viewport.height(),
            viewport.max.y + viewport.height(),
        );

        // Pin visible covers in TextureCache and schedule debounced thumbnail requests
        let mut pinned = HashSet::new();
        for row in &state.rows[vis_lo..vis_hi] {
            if let Some(art_id) = row.art_id {
                pinned.insert((art_id, tier));
            }
        }
        state.tex_cache.set_pinned(pinned);

        if state.last_thumb_req_at.elapsed() >= Duration::from_millis(80) {
            state.last_thumb_req_at = std::time::Instant::now();
            let vp_center_y = viewport.center().y;
            let mut req_items = Vec::new();
            for idx in pref_lo..pref_hi {
                if let Some(art_id) = state.rows[idx].art_id
                    && !state.tex_cache.contains((art_id, tier))
                {
                    let card_y = layout.pos(idx).map(|p| p.y).unwrap_or(0.0);
                    let dist = ((card_y - vp_center_y).abs()) as u32;
                    req_items.push((art_id, tier, dist));
                }
            }
            if !req_items.is_empty() {
                let _ = fx_tx.send(FxCommand::EnsureThumbs {
                    generation: state.thumb_generation,
                    items: req_items,
                });
            }
        }

        let painter = ui.painter();

        // Draw group headers
        for g in &layout.groups {
            let hy = origin.y + g.header_y;
            if hy + 32.0 >= content_rect.min.y + viewport.min.y - 40.0
                && hy <= content_rect.min.y + viewport.max.y + 40.0
            {
                let h_rect = Rect::from_min_size(
                    Pos2::new(origin.x + 8.0, hy),
                    Vec2::new(content_rect.width() - 16.0, 26.0),
                );
                paint::rect_filled(painter, h_rect, th.bg0);
                let accent_bar = Rect::from_min_size(h_rect.min, Vec2::new(3.0, h_rect.height()));
                paint::rect_filled(painter, accent_bar, th.accent);
                painter.text(
                    Pos2::new(h_rect.min.x + 10.0, h_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{} ({})", g.label, g.count),
                    FontId::proportional(14.0),
                    th.text,
                );
            }
        }

        // Draw fading ghost cards (up to 40) from FLIP
        state.flip_ghosts.retain(|(_, _, _, t0)| now_s - *t0 < 0.12);
        let current_keys: HashSet<ItemKey> = state.rows.iter().map(|r| r.rel.key).collect();
        let ghosts_snapshot = state.flip_ghosts.clone();
        for (g_key, g_art, g_rect, t0) in ghosts_snapshot {
            if !current_keys.contains(&g_key) {
                let alpha = (1.0 - ((now_s - t0) as f32 / 0.12)).clamp(0.0, 1.0);
                paint::rect_filled(painter, g_rect, th.bg2.gamma_multiply(alpha * 0.5));
                if let Some(aid) = g_art
                    && let Some(pal) = state.palettes.get(&aid)
                {
                    let cov = Rect::from_min_size(g_rect.min, Vec2::splat(g_rect.width()));
                    paint::tiny8_mosaic(painter, cov, &pal.tiny8, alpha * 0.5);
                }
            }
        }

        let seed_flip = std::mem::take(&mut state.flip_seed_pending);
        state.last_card_rects.clear();
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());

        for idx in vis_lo..vis_hi {
            let row = state.rows[idx].clone();
            let key = row.rel.key;
            let Some(target_rect) = layout.card_rect(idx, origin) else {
                continue;
            };

            if seed_flip {
                let start_min = if let Some(old_r) = state.flip_origins.get(&key).copied() {
                    let delta = old_r.min - target_rect.min;
                    let dist = delta.length();
                    if dist < 1.0 {
                        state.flip_origins.remove(&key);
                        target_rect.min
                    } else {
                        let clamped = if dist > 420.0 {
                            delta * (420.0 / dist)
                        } else {
                            delta
                        };
                        target_rect.min + clamped
                    }
                } else {
                    let fallback = target_rect.min + Vec2::new(0.0, 56.0);
                    state
                        .flip_origins
                        .insert(key, Rect::from_min_size(fallback, target_rect.size()));
                    fallback
                };
                state
                    .motion
                    .set_vec2(("flip_pos", key), start_min.to_vec2());
            }

            // FLIP position spring if card moved from a prior layout
            let animated_min = if state.flip_origins.contains_key(&key) {
                let v = state.motion.vec2(
                    ("flip_pos", key),
                    target_rect.min.to_vec2(),
                    SpringCfg::SMOOTH,
                );
                if (v - target_rect.min.to_vec2()).length_sq() < 0.25 {
                    state.flip_origins.remove(&key);
                    target_rect.min
                } else {
                    Pos2::new(v.x, v.y)
                }
            } else {
                target_rect.min
            };

            let base_rect = Rect::from_min_size(animated_min, target_rect.size());
            state.last_card_rects.insert(key, base_rect);

            let is_selected = state.selected_keys.contains(&key);
            let show_sel_chk = is_selected || !state.selected_keys.is_empty();
            let sel_chk_base = Rect::from_min_size(
                Pos2::new(base_rect.max.x - 24.0, base_rect.min.y + 6.0),
                Vec2::new(18.0, 18.0),
            );
            let sel_chk_resp = ui.interact(
                sel_chk_base,
                ui.id().with(("wall_sel_chk", key)),
                Sense::click(),
            );
            let card_resp =
                ui.interact(base_rect, ui.id().with(("wall_card", key)), Sense::click());
            let is_hovered = card_resp.hovered() || sel_chk_resp.hovered();
            let is_focused = state.focused_key == Some(key);

            if is_hovered {
                new_hovered_key = Some(key);
                new_hovered_art = row.art_id;
            }

            // Entrance stagger scale
            let stagger_s = stagger_delay_s(idx.saturating_sub(vis_lo), 12, 240);
            let _ = stagger_s;
            let lift = state.motion.val(
                ("card_lift", key),
                if is_hovered && state.prefs.effects != EffectsLevel::Off {
                    4.0
                } else {
                    0.0
                },
                SpringCfg::SNAP,
            );
            let draw_rect = base_rect.translate(Vec2::new(0.0, -lift));

            let card_accent = row
                .accent
                .map(|[r, g, b]| Color32::from_rgb(r, g, b))
                .unwrap_or(th.accent);

            if lift > 0.2 {
                paint::hard_shadow(
                    painter,
                    draw_rect,
                    Vec2::new(lift, lift),
                    card_accent.gamma_multiply(0.45),
                );
            }

            let border_col = if is_selected || is_hovered {
                card_accent
            } else {
                th.border
            };
            let border_w = if is_selected { 2.0_f32 } else { 1.0_f32 };
            paint::rect(
                painter,
                draw_rect,
                th.bg1,
                Stroke::new(border_w, border_col),
            );

            // Square cover region
            let cov_rect =
                Rect::from_min_size(draw_rect.min, Vec2::splat(layout.card_w)).shrink(1.0);
            let zoom = state.motion.val(
                ("cov_zoom", key),
                if is_hovered && state.prefs.effects != EffectsLevel::Off {
                    1.04
                } else {
                    1.0
                },
                SpringCfg::SNAP,
            );
            let uv_shift = if is_hovered
                && state.prefs.effects != EffectsLevel::Off
                && let Some(mp) = pointer_pos
            {
                let rel_x = ((mp.x - cov_rect.center().x) / cov_rect.width()).clamp(-0.5, 0.5);
                let rel_y = ((mp.y - cov_rect.center().y) / cov_rect.height()).clamp(-0.5, 0.5);
                Vec2::new(rel_x * 0.03, rel_y * 0.03)
            } else {
                Vec2::ZERO
            };

            paint_cover_slot(
                painter,
                state,
                row.art_id,
                row.rel.key.id,
                cov_rect,
                tier,
                zoom,
                uv_shift,
            );

            // Top-left classification badge on cover
            let badge_col = class_color(row.rel.classification, &th);
            let badge_txt = row.rel.classification.label();
            let badge_w = if badge_txt.len() > 6 {
                (badge_txt.len() as f32 * 6.2 + 10.0).ceil()
            } else {
                42.0
            };
            let badge_r = Rect::from_min_size(
                Pos2::new(cov_rect.min.x + 6.0, cov_rect.min.y + 6.0),
                Vec2::new(badge_w, 16.0),
            );
            paint::rect(
                painter,
                badge_r,
                th.bg0.gamma_multiply(0.88),
                Stroke::new(1.0_f32, badge_col),
            );
            painter.text(
                badge_r.center(),
                egui::Align2::CENTER_CENTER,
                badge_txt,
                FontId::monospace(10.0),
                badge_col,
            );

            // Status badge below classification badge on cover (UPDATE / MISSING / ARCHIVED / DOWNLOADED)
            let status_badge = if row.rel.has_remote_update {
                Some(("UPDATE", 46.0_f32, th.warn, th.warn))
            } else if row.rel.downloaded {
                let is_archived =
                    row.rel.local_status == Some(bcvault_core::types::LocalStatus::Archived);
                let is_folder_missing = !is_archived
                    && (row.rel.local_status == Some(bcvault_core::types::LocalStatus::Missing)
                        || if let Some(folders) = state.folders.get(&row.rel.band_id) {
                            match &row.rel.local_path {
                                Some(p) => {
                                    let rel_key =
                                        bcvault_core::library::link::normalize_path_key(p);
                                    let owning = folders
                                        .iter()
                                        .filter(|f| {
                                            let folder_key =
                                                bcvault_core::library::link::normalize_path_key(
                                                    &f.path,
                                                );
                                            rel_key == folder_key
                                                || rel_key.starts_with(&format!("{folder_key}/"))
                                        })
                                        .max_by_key(|f| f.path.as_os_str().len());
                                    owning
                                        .map(|f| f.state != bcvault_core::types::FolderState::Ok)
                                        .unwrap_or(false)
                                }
                                None => folders
                                    .iter()
                                    .any(|f| f.state != bcvault_core::types::FolderState::Ok),
                            }
                        } else {
                            false
                        });

                if is_folder_missing {
                    let border_col = if is_selected { th.accent } else { th.warn };
                    Some(("MISSING", 46.0_f32, border_col, th.warn))
                } else if is_archived {
                    let border_col = if is_selected { th.accent } else { th.border };
                    Some(("ARCHIVED", 54.0_f32, border_col, th.text_dim))
                } else {
                    let border_col = if is_selected { th.accent } else { th.ok };
                    Some(("DOWNLOADED", 72.0_f32, border_col, th.ok))
                }
            } else {
                None
            };

            if let Some((label, width, border_col, text_col)) = status_badge {
                let status_r = Rect::from_min_size(
                    Pos2::new(cov_rect.min.x + 6.0, cov_rect.min.y + 26.0),
                    Vec2::new(width, 16.0),
                );
                paint::rect(
                    painter,
                    status_r,
                    th.bg0.gamma_multiply(0.88),
                    Stroke::new(1.0_f32, border_col),
                );
                painter.text(
                    status_r.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    FontId::monospace(9.5),
                    text_col,
                );
            }

            if show_sel_chk || is_hovered {
                let box_r = Rect::from_min_size(
                    Pos2::new(cov_rect.max.x - 22.0, cov_rect.min.y + 6.0),
                    Vec2::new(16.0, 16.0),
                );
                let fill = if is_selected { th.accent } else { th.bg0 };
                let stroke_col = if is_selected || sel_chk_resp.hovered() {
                    th.accent
                } else {
                    th.border_hi
                };
                paint::rect(painter, box_r, fill, Stroke::new(1.5_f32, stroke_col));
                if is_selected {
                    paint::drawn_check(painter, box_r, 1.0, Stroke::new(1.8_f32, th.bg0));
                }
            }

            // Title + artist label strip below cover
            let text_rect = Rect::from_min_max(
                Pos2::new(draw_rect.min.x + 6.0, draw_rect.min.y + layout.card_w + 4.0),
                Pos2::new(draw_rect.max.x - 6.0, draw_rect.max.y - 4.0),
            );
            let clipped = painter.with_clip_rect(text_rect);
            let mark_inv = state.settings.mark_invisible_chars;
            if mark_inv && bcvault_core::text::has_invisible_chars(&row.rel.title) {
                let job = paint::display_layout_job(
                    &row.rel.title,
                    true,
                    FontId::proportional(12.5),
                    th.text,
                    th.warn,
                );
                let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
                clipped.galley(text_rect.left_top(), galley, th.text);
            } else {
                clipped.text(
                    text_rect.left_top(),
                    egui::Align2::LEFT_TOP,
                    &row.rel.title,
                    FontId::proportional(12.5),
                    th.text,
                );
            }
            if mark_inv && bcvault_core::text::has_invisible_chars(&row.rel.artist_name) {
                let job = paint::display_layout_job(
                    &row.rel.artist_name,
                    true,
                    FontId::proportional(11.0),
                    th.text_dim,
                    th.warn,
                );
                let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
                clipped.galley(
                    Pos2::new(text_rect.min.x, text_rect.min.y + 20.0),
                    galley,
                    th.text_dim,
                );
            } else {
                clipped.text(
                    Pos2::new(text_rect.min.x, text_rect.min.y + 20.0),
                    egui::Align2::LEFT_TOP,
                    &row.rel.artist_name,
                    FontId::proportional(11.0),
                    th.text_dim,
                );
            }

            if is_focused {
                let focus_pos =
                    state
                        .motion
                        .vec2("wall_focus_frame", draw_rect.min.to_vec2(), SpringCfg::SNAP);
                let focus_r =
                    Rect::from_min_size(Pos2::new(focus_pos.x, focus_pos.y), draw_rect.size())
                        .expand(2.0);
                paint::rect_outline(painter, focus_r, Stroke::new(1.5_f32, th.accent2));
            }

            let mods = ui.input(|i| i.modifiers);
            if sel_chk_resp.clicked() {
                pending_actions.push(FxAction::ToggleSelect(key));
            } else if card_resp.double_clicked() {
                pending_actions.push(FxAction::OpenDrawer(key));
            } else if card_resp.clicked() {
                if mods.shift {
                    pending_actions.push(FxAction::SelectRange(key));
                } else if mods.ctrl || mods.command {
                    pending_actions.push(FxAction::ToggleSelect(key));
                } else {
                    state.selected_keys.clear();
                    state.selected_keys.insert(key);
                    state.last_clicked_key = Some(key);
                    state.focused_key = Some(key);
                    state.mark_selection_dirty();
                }
            }

            card_resp.context_menu(|ui| {
                if paint::sharp_button(ui, "Inspect Details").clicked() {
                    pending_actions.push(FxAction::OpenDrawer(key));
                    ui.close();
                }
                if row.rel.is_downloadable() {
                    if row.rel.downloaded {
                        if paint::sharp_button(ui, "Re-download").clicked() {
                            pending_actions.push(FxAction::Redownload(key));
                            ui.close();
                        }
                    } else if paint::sharp_button(ui, "Download").clicked() {
                        pending_actions.push(FxAction::DownloadKey(key));
                        ui.close();
                    }
                }
                if paint::sharp_button(ui, "Open in Browser").clicked() {
                    pending_actions.push(FxAction::OpenInBrowser(key));
                    ui.close();
                }
                if paint::sharp_button(ui, "Relink to Folder...").clicked() {
                    pending_actions.push(FxAction::RelinkReleaseModal(key));
                    ui.close();
                }
                if row.rel.downloaded {
                    if paint::sharp_button(ui, "Unmark Downloaded").clicked() {
                        pending_actions.push(FxAction::SetReleaseDownloaded(key, false));
                        ui.close();
                    }
                } else if paint::sharp_button(ui, "Mark Downloaded").clicked() {
                    pending_actions.push(FxAction::SetReleaseDownloaded(key, true));
                    ui.close();
                }
                if row.rel.classification == Classification::Unavailable {
                    if paint::sharp_button(ui, "Unmark Unavailable").clicked() {
                        pending_actions.push(FxAction::SetReleaseUnavailable(key, false));
                        ui.close();
                    }
                } else if paint::sharp_button(ui, "Mark as Unavailable").clicked() {
                    pending_actions.push(FxAction::SetReleaseUnavailable(key, true));
                    ui.close();
                }
            });
        }

        for idx in 0..state.rows.len().min(500) {
            let key = state.rows[idx].rel.key;
            if !state.last_card_rects.contains_key(&key)
                && let Some(r) = layout.card_rect(idx, origin)
            {
                state.last_card_rects.insert(key, r);
            }
        }

        // Marquee rectangle overlay
        if let (Some(p0), Some(p1)) = (state.marquee_start, state.marquee_current) {
            let mr = Rect::from_two_pos(p0, p1);
            paint::rect(
                painter,
                mr,
                th.accent.gamma_multiply(0.14),
                Stroke::new(1.0_f32, th.accent),
            );
        }
    });

    // Right-edge scrub rail (years or initials)
    let rail_rect = Rect::from_min_max(
        Pos2::new(avail.max.x - 16.0, avail.min.y + 8.0),
        Pos2::new(avail.max.x - 2.0, avail.max.y - 8.0),
    );
    let rail_resp = ui.interact(
        rail_rect,
        ui.id().with("wall_scrub_rail"),
        Sense::click_and_drag(),
    );
    paint::rect_filled(ui.painter(), rail_rect, th.bg0);
    let ticks = 16_usize;
    for t_i in 0..ticks {
        let fy = (t_i as f32 + 0.5) / (ticks as f32);
        let y = rail_rect.min.y + fy * rail_rect.height();
        ui.painter().line_segment(
            [
                Pos2::new(rail_rect.min.x + 3.0, y),
                Pos2::new(rail_rect.max.x - 3.0, y),
            ],
            Stroke::new(1.0_f32, th.border),
        );
    }
    if (rail_resp.clicked() || rail_resp.dragged())
        && let Some(mp) = rail_resp.interact_pointer_pos()
        && !state.rows.is_empty()
    {
        let frac = ((mp.y - rail_rect.min.y) / rail_rect.height()).clamp(0.0, 1.0);
        state.pending_scroll_to_y = Some(frac * (layout.total_height - avail.height()).max(0.0));
        let row_idx = ((frac * (state.rows.len().saturating_sub(1) as f32)).round() as usize)
            .min(state.rows.len() - 1);
        let scrub_label = group_label_for(&state.rows[row_idx], GroupBy::Year);
        let tip_rect = Rect::from_min_size(
            Pos2::new(
                rail_rect.min.x - 76.0,
                (mp.y - 14.0).clamp(avail.min.y, avail.max.y - 28.0),
            ),
            Vec2::new(68.0, 26.0),
        );
        paint::rect(
            ui.painter(),
            tip_rect,
            th.bg0,
            Stroke::new(1.0_f32, th.accent),
        );
        ui.painter().text(
            tip_rect.center(),
            egui::Align2::CENTER_CENTER,
            scrub_label,
            FontId::monospace(12.0),
            th.accent,
        );
    }

    let _ = scroll_out;
    state.hovered_key = new_hovered_key;
    state.hovered_art_id = new_hovered_art;

    // Empty state
    if state.rows.is_empty() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(avail), |ui| {
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.add_space(avail.height() * 0.35);
                ui.label(
                    RichText::new("No releases match the active filter")
                        .size(15.0)
                        .color(th.text_dim),
                );
            });
        });
    }

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}

pub fn class_color(c: Classification, th: &crate::theme::FxTheme) -> Color32 {
    match c {
        Classification::Free => th.ok,
        Classification::Nyp => th.accent,
        Classification::EmailRequired => th.warn,
        Classification::Paid => th.err,
        Classification::Unavailable | Classification::Unknown => th.text_dim,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use bcvault_core::types::{ItemType, ReleaseRow};

    fn make_rows(n: usize) -> Vec<WallRow> {
        (0..n)
            .map(|i| WallRow {
                rel: ReleaseRow {
                    key: ItemKey {
                        kind: ItemType::Album,
                        id: i as i64,
                    },
                    band_id: 1,
                    title: format!("Rel {i}"),
                    artist_name: "Artist".to_string(),
                    url: "https://a.bandcamp.com/album/x".to_string(),
                    cover_url: None,
                    art_id: Some(i as i64),
                    classification: Classification::Free,
                    is_preorder: false,
                    track_count: Some(5),
                    published_at: Some(1_700_000_000),
                    modified_at: None,
                    downloaded: false,
                    downloaded_at: None,
                    downloaded_format: None,
                    local_path: None,
                    local_status: None,
                    origin: None,
                    missing_ack_at: None,
                    has_remote_update: false,
                    remote_update_reason: None,
                },
                art_id: Some(i as i64),
                hue: Some(180),
                accent: None,
                added_at: 1_700_000_000,
            })
            .collect()
    }

    #[test]
    fn wall_layout_pos_visible_range_and_hit_test() {
        let rows = make_rows(50);
        let layout = WallLayout::compute(1000.0, 168.0, &rows, GroupBy::None);
        assert!(layout.cols >= 4);
        assert_eq!(layout.card_positions.len(), 50);

        let p0 = layout.pos(0).unwrap();
        assert_eq!(
            layout.index_at(Pos2::new(p0.x + 10.0, p0.y + 10.0)),
            Some(0)
        );

        let (lo, hi) = layout.visible_range(0.0, 250.0);
        assert_eq!(lo, 0);
        assert!(hi >= layout.cols);
    }

    #[test]
    fn wall_sort_change_triggers_flip_spring_animation() {
        use crate::{actions::FxSortKey, services::messages::FxEvent};
        use bcvault_core::types::SortDir;

        let ctx = egui::Context::default();
        let mut state = FxState::new();
        state.prefs.card_size_idx = 3; // XL cards
        state.rows = make_rows(28);

        let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        // Frame 1: initial layout
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
            ..Default::default()
        };
        let _ = ctx.run(raw.clone(), |ctx| {
            state.motion.begin_frame_dt(1.0 / 60.0);
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &core_tx, &fx_tx, 0.0);
            });
            state.motion.end_frame(ctx);
        });
        assert_eq!(state.last_card_rects.len(), 28);

        // User dispatches SetSort in toolbar during Frame 2 before WallLoaded arrives
        let _ = ctx.run(raw.clone(), |ctx| {
            state.motion.begin_frame_dt(1.0 / 60.0);
            state.dispatch(
                FxAction::SetSort(FxSortKey::Title, SortDir::Asc),
                &core_tx,
                &fx_tx,
                0.016,
            );
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &core_tx, &fx_tx, 0.016);
            });
            state.motion.end_frame(ctx);
        });

        // Async WallLoaded arrives with reversed rows on Frame 3
        let mut reversed = make_rows(28);
        reversed.reverse();
        let req_id = state.active_wall_req_id;
        state.apply_fx_event(
            FxEvent::WallLoaded {
                request_id: req_id,
                chunk: 0,
                last: true,
                rows: reversed,
            },
            0.032,
        );
        assert!(state.flip_seed_pending);

        let _ = ctx.run(raw, |ctx| {
            state.motion.begin_frame_dt(1.0 / 60.0);
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &core_tx, &fx_tx, 0.032);
            });
            state.motion.end_frame(ctx);
        });

        // Cards must be actively animating via FLIP springs, not teleporting
        assert!(!state.flip_origins.is_empty());
        assert!(state.motion.active_springs() > 0);
    }

    #[test]
    fn wall_selected_card_does_not_render_download_checkmark_when_undownloaded() {
        let ctx = egui::Context::default();
        let mut state = FxState::new();
        state.rows = make_rows(1);
        let key = state.rows[0].rel.key;
        state.selected_keys.insert(key);

        let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
            ..Default::default()
        };

        let has_text = |out: &egui::FullOutput, needle: &str| {
            out.shapes.iter().any(|s| {
                matches!(
                    &s.shape,
                    egui::epaint::Shape::Text(t) if t.galley.text() == needle
                )
            })
        };

        // Render with downloaded = false
        let out_undl = ctx.run(raw.clone(), |ctx| {
            state.motion.begin_frame_dt(1.0 / 60.0);
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &core_tx, &fx_tx, 0.0);
            });
            state.motion.end_frame(ctx);
        });

        assert!(!has_text(&out_undl, "DOWNLOADED"));

        // Now set downloaded = true and re-render
        state.rows[0].rel.downloaded = true;
        let out_dl = ctx.run(raw, |ctx| {
            state.motion.begin_frame_dt(1.0 / 60.0);
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &core_tx, &fx_tx, 0.016);
            });
            state.motion.end_frame(ctx);
        });

        assert!(has_text(&out_dl, "DOWNLOADED"));
    }

    #[test]
    fn wall_selected_downloaded_card_with_missing_folder_shows_missing_not_checkmark() {
        use bcvault_core::types::{ArtistFolder, FolderState};
        use std::path::PathBuf;

        let ctx = egui::Context::default();
        let mut state = FxState::new();
        state.rows = make_rows(1);
        state.rows[0].rel.downloaded = true;
        let key = state.rows[0].rel.key;
        let band_id = state.rows[0].rel.band_id;
        state.selected_keys.insert(key);

        state.folders.insert(
            band_id,
            vec![ArtistFolder {
                id: 1,
                band_id,
                path: PathBuf::from("/music/Artist"),
                state: FolderState::Missing,
            }],
        );

        let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
            ..Default::default()
        };

        let out = ctx.run(raw, |ctx| {
            state.motion.begin_frame_dt(1.0 / 60.0);
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &core_tx, &fx_tx, 0.0);
            });
            state.motion.end_frame(ctx);
        });

        let has_text = |needle: &str| {
            out.shapes.iter().any(|s| {
                matches!(
                    &s.shape,
                    egui::epaint::Shape::Text(t) if t.galley.text() == needle
                )
            })
        };
        assert!(has_text("MISSING"));
        assert!(!has_text("DOWNLOADED"));
        assert!(!state.motion.has(("dl_check", key)));
    }
}
