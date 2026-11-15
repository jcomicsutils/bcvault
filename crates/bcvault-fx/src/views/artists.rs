use crate::{
    actions::{FxAction, ViewMode},
    motion::SpringCfg,
    paint,
    services::messages::{FxCommand, Tier},
    state::{FxState, paint_cover_slot},
    views::stats::paint_odometer_u32,
};
use bcvault_core::worker::messages::UiToWorkerCommand;
use chrono::{TimeZone, Utc};
use egui::{FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(
    ui: &mut Ui,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    now_s: f64,
) {
    let th = state.active_theme;
    let mut pending_actions: Vec<FxAction> = Vec::new();
    let artists = state.artists.clone();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, 92.0, artists.len(), |ui, range| {
            for idx in range {
                let artist = &artists[idx];
                let desired = Vec2::new(ui.available_width() - 12.0, 84.0);
                let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());

                if ui.is_rect_visible(rect) {
                    let is_hovered = resp.hovered();
                    let lift = state.motion.val(
                        ("art_banner_lift", artist.band_id),
                        if is_hovered { 3.0 } else { 0.0 },
                        SpringCfg::SNAP,
                    );
                    let draw_rect = rect.translate(Vec2::new(0.0, -lift));
                    if lift > 0.2 {
                        paint::hard_shadow(
                            ui.painter(),
                            draw_rect,
                            Vec2::new(lift, lift),
                            th.accent.gamma_multiply(0.35),
                        );
                    }
                    let border_col = if is_hovered { th.accent } else { th.border };
                    paint::rect(
                        ui.painter(),
                        draw_rect,
                        th.bg1,
                        Stroke::new(1.0_f32, border_col),
                    );

                    // Cover mosaic strip on the left (up to 4 square 64px tiles)
                    let mut art_ids: Vec<Option<i64>> = state
                        .rows
                        .iter()
                        .filter(|r| r.rel.band_id == artist.band_id)
                        .map(|r| r.art_id)
                        .take(4)
                        .collect();
                    while art_ids.len() < 4 {
                        art_ids.push(None);
                    }

                    let tile_s = 68.0_f32;
                    for (t_i, aid) in art_ids.iter().enumerate() {
                        let tr = Rect::from_min_size(
                            Pos2::new(
                                draw_rect.min.x + 8.0 + (t_i as f32) * (tile_s + 4.0),
                                draw_rect.center().y - tile_s * 0.5,
                            ),
                            Vec2::splat(tile_s),
                        );
                        paint_cover_slot(
                            ui.painter(),
                            state,
                            *aid,
                            artist.band_id.wrapping_add(t_i as i64),
                            tr,
                            Tier::T192,
                            1.0,
                            Vec2::ZERO,
                        );
                    }

                    let info_x = draw_rect.min.x + 8.0 + 4.0 * (tile_s + 4.0) + 12.0;
                    let name_disp = bcvault_core::text::format_display_text(
                        artist.display_name(),
                        state.settings.mark_invisible_chars,
                    );
                    ui.painter().text(
                        Pos2::new(info_x, draw_rect.min.y + 14.0),
                        egui::Align2::LEFT_TOP,
                        name_disp.as_ref(),
                        FontId::proportional(17.0),
                        th.text,
                    );
                    let sync_str = artist
                        .last_synced_at
                        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
                        .map(|dt| format!("Last synced {}", dt.format("%Y-%m-%d %H:%M")))
                        .unwrap_or_else(|| "Not yet synced".to_string());
                    ui.painter().text(
                        Pos2::new(info_x, draw_rect.min.y + 42.0),
                        egui::Align2::LEFT_TOP,
                        sync_str,
                        FontId::monospace(11.5),
                        th.text_dim,
                    );

                    // Right-aligned odometer counters (Downloaded / Eligible / Total)
                    let odo_rect = Rect::from_min_size(
                        Pos2::new(draw_rect.max.x - 240.0, draw_rect.min.y + 16.0),
                        Vec2::new(224.0, 52.0),
                    );
                    paint::rect(
                        ui.painter(),
                        odo_rect,
                        th.bg0,
                        Stroke::new(1.0_f32, th.border),
                    );
                    let col_w = odo_rect.width() / 3.0;
                    for (ci, (lbl, val, col)) in [
                        ("DL", artist.stats.downloaded, th.ok),
                        ("ELIG", artist.stats.eligible, th.accent),
                        ("TOTAL", artist.stats.total, th.text),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let cell = Rect::from_min_size(
                            Pos2::new(odo_rect.min.x + (ci as f32) * col_w, odo_rect.min.y),
                            Vec2::new(col_w, odo_rect.height()),
                        );
                        ui.painter().text(
                            Pos2::new(cell.center().x, cell.min.y + 6.0),
                            egui::Align2::CENTER_TOP,
                            lbl,
                            FontId::monospace(9.5),
                            th.text_dim,
                        );
                        let num_r = Rect::from_center_size(
                            Pos2::new(cell.center().x, cell.min.y + 32.0),
                            Vec2::new(col_w - 8.0, 22.0),
                        );
                        paint_odometer_u32(
                            ui.painter(),
                            state,
                            ("art_odo", artist.band_id, lbl),
                            num_r,
                            val,
                            col,
                        );
                    }
                }

                if resp.clicked() {
                    // Shared-element dive into this artist's wall
                    state.artist_dive_banner = Some((artist.band_id, rect));
                    pending_actions.push(FxAction::SelectArtist(Some(artist.band_id)));
                    pending_actions.push(FxAction::SetView(ViewMode::Wall));
                }
            }
        });

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}
