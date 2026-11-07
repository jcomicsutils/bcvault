use crate::{
    actions::FxAction,
    motion::SpringCfg,
    paint,
    services::messages::{FxCommand, Tier},
    state::{FxState, paint_cover_slot},
    views::wall::class_color,
};
use bcvault_core::worker::messages::UiToWorkerCommand;
use egui::{Color32, CornerRadius, Order, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use tokio::sync::mpsc::UnboundedSender;

pub fn show_drawer(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    now_s: f64,
) {
    let is_open = state.drawer_key.is_some();
    let slide = state.motion.val(
        "drawer_slide",
        if is_open { 1.0 } else { 0.0 },
        SpringCfg::SMOOTH,
    );
    if slide < 0.005 && !is_open {
        return;
    }

    let screen = ctx.content_rect();
    let th = state.active_theme;
    let mut pending_actions: Vec<FxAction> = Vec::new();

    // Scrim fading to 35%
    let scrim_alpha = (slide * 0.35).clamp(0.0, 0.35);
    egui::Area::new(egui::Id::new("fx_drawer_scrim"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            paint::rect_filled(
                ui.painter(),
                r,
                Color32::from_black_alpha((scrim_alpha * 255.0) as u8),
            );
            if resp.clicked() {
                pending_actions.push(FxAction::CloseDrawer);
            }
        });

    let drawer_w = 460.0_f32.min(screen.width());
    let x = screen.max.x - drawer_w * slide;
    let drawer_rect = Rect::from_min_max(
        Pos2::new(x, screen.min.y),
        Pos2::new(x + drawer_w, screen.max.y),
    );

    if let Some(key) = state.drawer_key
        && let Some(row) = state.rows.iter().find(|r| r.rel.key == key).cloned()
    {
        egui::Area::new(egui::Id::new("fx_detail_drawer"))
            .order(Order::Foreground)
            .fixed_pos(drawer_rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(drawer_rect.size());
                ui.set_max_size(drawer_rect.size());

                egui::Frame::NONE
                    .fill(th.bg1)
                    .stroke(Stroke::new(1.0_f32, th.border_hi))
                    .corner_radius(CornerRadius::ZERO)
                    .inner_margin(12.0_f32)
                    .show(ui, |ui| {
                        // Palette-tinted mesh gradient header
                        let hdr_rect = Rect::from_min_size(
                            ui.max_rect().min,
                            Vec2::new(ui.available_width(), 44.0),
                        );
                        let (c_tl, c_tr) = if let Some(aid) = row.art_id
                            && let Some(pal) = state.palettes.get(&aid)
                        {
                            (
                                Color32::from_rgb(
                                    pal.swatches[0][0],
                                    pal.swatches[0][1],
                                    pal.swatches[0][2],
                                )
                                .gamma_multiply(0.32),
                                Color32::from_rgb(
                                    pal.swatches[1][0],
                                    pal.swatches[1][1],
                                    pal.swatches[1][2],
                                )
                                .gamma_multiply(0.24),
                            )
                        } else {
                            (
                                th.accent.gamma_multiply(0.25),
                                th.accent2.gamma_multiply(0.18),
                            )
                        };
                        paint::glow_rect(ui.painter(), hdr_rect, c_tl, c_tr, th.bg1, th.bg1);

                        ui.horizontal(|ui| {
                            if paint::sharp_button(ui, "◀ Prev")
                                .on_hover_text("Previous release (Left)")
                                .clicked()
                            {
                                pending_actions.push(FxAction::DrawerStep(-1));
                            }
                            if paint::sharp_button(ui, "Next ▶")
                                .on_hover_text("Next release (Right)")
                                .clicked()
                            {
                                pending_actions.push(FxAction::DrawerStep(1));
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if paint::sharp_button(ui, "Close (Esc)").clicked() {
                                        pending_actions.push(FxAction::CloseDrawer);
                                    }
                                },
                            );
                        });

                        ui.separator();

                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                // Hero cover (340x340 square inside 436px drawer)
                                let hero_side = (ui.available_width() - 8.0).min(340.0);
                                let (cov_r, _) =
                                    ui.allocate_exact_size(Vec2::splat(hero_side), Sense::hover());
                                paint_cover_slot(
                                    ui.painter(),
                                    state,
                                    row.art_id,
                                    key.id,
                                    cov_r,
                                    Tier::Hero,
                                    1.0,
                                    Vec2::ZERO,
                                );

                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(&row.rel.title)
                                        .size(17.0)
                                        .strong()
                                        .color(th.text),
                                );
                                ui.label(
                                    RichText::new(&row.rel.artist_name)
                                        .size(13.5)
                                        .color(th.text_dim),
                                );

                                ui.horizontal(|ui| {
                                    let b_col = class_color(row.rel.classification, &th);
                                    ui.label(
                                        RichText::new(row.rel.classification.label())
                                            .monospace()
                                            .strong()
                                            .color(b_col),
                                    );
                                    if row.rel.is_preorder {
                                        ui.label(
                                            RichText::new("PRE-ORDER").monospace().color(th.warn),
                                        );
                                    }
                                    if row.rel.downloaded {
                                        let fmt_s = row
                                            .rel
                                            .downloaded_format
                                            .map(|f| f.label())
                                            .unwrap_or("MANUAL");
                                        ui.label(
                                            RichText::new(format!("DOWNLOADED ({fmt_s})"))
                                                .monospace()
                                                .color(th.ok),
                                        );
                                    }
                                });

                                ui.add_space(6.0);

                                // Action buttons
                                ui.horizontal_wrapped(|ui| {
                                    if row.rel.classification.auto_downloadable() {
                                        let lbl = if row.rel.has_remote_update {
                                            "Update Download"
                                        } else if row.rel.downloaded {
                                            "Re-download"
                                        } else {
                                            "Download"
                                        };
                                        if paint::sharp_button(ui, lbl).clicked() {
                                            pending_actions.push(FxAction::DownloadKey(key));
                                        }
                                    }
                                    if row.rel.local_path.is_some()
                                        && paint::sharp_button(ui, "Open Folder").clicked()
                                    {
                                        pending_actions.push(FxAction::OpenFolder(key));
                                    }
                                    if paint::sharp_button(ui, "Open in Browser").clicked() {
                                        pending_actions.push(FxAction::OpenInBrowser(key));
                                    }
                                    if paint::sharp_button(ui, "Redeem URL...").clicked() {
                                        pending_actions.push(FxAction::RedeemDialog(key));
                                    }
                                    if row.rel.downloaded {
                                        if paint::sharp_button(ui, "Unmark Downloaded").clicked() {
                                            pending_actions
                                                .push(FxAction::SetReleaseDownloaded(key, false));
                                        }
                                    } else if paint::sharp_button(ui, "Mark Downloaded").clicked() {
                                        pending_actions
                                            .push(FxAction::SetReleaseDownloaded(key, true));
                                    }
                                });

                                if let Some(lp) = &row.rel.local_path {
                                    ui.label(
                                        RichText::new(format!("Local: {}", lp.display()))
                                            .monospace()
                                            .size(10.5)
                                            .color(th.text_dim),
                                    );
                                }

                                ui.separator();

                                // Lazy-loaded Tracklist, Tags, About, Credits
                                if let Some(detail) = state.details_cache.get(&key).cloned() {
                                    let dur_hdr = detail
                                        .duration_s
                                        .map(|s| format!(" ({}:{:02})", s / 60, s % 60))
                                        .unwrap_or_default();
                                    ui.label(
                                        RichText::new(format!(
                                            "TRACKLIST ({}{dur_hdr})",
                                            detail.tracks.len()
                                        ))
                                        .monospace()
                                        .strong()
                                        .color(th.text_dim),
                                    );

                                    for trk in &detail.tracks {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!("{:02}.", trk.track_num))
                                                    .monospace()
                                                    .color(th.text_dim),
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&trk.title).color(th.text),
                                                )
                                                .truncate(),
                                            );
                                            if let Some(ds) = trk.duration_s {
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{}:{:02}",
                                                                ds / 60,
                                                                ds % 60
                                                            ))
                                                            .monospace()
                                                            .size(11.0)
                                                            .color(th.text_dim),
                                                        );
                                                    },
                                                );
                                            }
                                        });
                                    }

                                    if !detail.tags.is_empty() {
                                        ui.add_space(8.0);
                                        ui.horizontal_wrapped(|ui| {
                                            for tag in &detail.tags {
                                                ui.label(
                                                    RichText::new(format!("#{tag}"))
                                                        .monospace()
                                                        .size(11.0)
                                                        .color(th.accent),
                                                );
                                            }
                                        });
                                    }

                                    if let Some(about) = &detail.about {
                                        ui.add_space(8.0);
                                        ui.collapsing(
                                            RichText::new("ABOUT").monospace().color(th.text_dim),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(about).size(12.0).color(th.text),
                                                );
                                            },
                                        );
                                    }

                                    if let Some(credits) = &detail.credits {
                                        ui.collapsing(
                                            RichText::new("CREDITS").monospace().color(th.text_dim),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(credits)
                                                        .size(11.5)
                                                        .color(th.text_dim),
                                                );
                                            },
                                        );
                                    }
                                } else {
                                    ui.horizontal(|ui| {
                                        let (sq_r, _) = ui
                                            .allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                                        paint::busy_squares(
                                            ui.painter(),
                                            sq_r,
                                            (now_s * 1.6) as f32,
                                            th.accent,
                                        );
                                        ui.label(
                                            RichText::new("Tracklist & release details")
                                                .color(th.text_dim),
                                        );
                                        if paint::sharp_button(ui, "Fetch Now").clicked() {
                                            state.drawer_detail_requested = true;
                                            let _ = fx_tx.send(FxCommand::EnsureDetail { key });
                                        }
                                    });
                                }
                            });
                    });
            });
    }

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}
