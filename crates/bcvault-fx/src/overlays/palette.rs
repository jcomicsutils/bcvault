use crate::{
    actions::FxAction,
    motion::SpringCfg,
    paint,
    services::messages::{FxCommand, SearchHitKind, SearchScope, Tier},
    state::{FxState, paint_cover_slot},
};
use bcvault_core::worker::messages::UiToWorkerCommand;
use egui::{
    Align2, Color32, CornerRadius, FontId, Order, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use tokio::sync::mpsc::UnboundedSender;

pub fn show_command_palette(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    now_s: f64,
) {
    if !state.palette_open {
        return;
    }

    let th = state.active_theme;
    let screen = ctx.content_rect();
    let mut pending_actions: Vec<FxAction> = Vec::new();

    // Backdrop scrim
    egui::Area::new(egui::Id::new("fx_palette_scrim"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            paint::rect_filled(ui.painter(), r, Color32::from_black_alpha(110));
            if resp.clicked() {
                pending_actions.push(FxAction::ClosePalette);
            }
        });

    let _scale = state
        .motion
        .val_from("palette_scale", 0.97, 1.0, SpringCfg::SNAP);

    egui::Area::new(egui::Id::new("fx_command_palette"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_TOP, [0.0, 72.0])
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(th.bg1)
                .stroke(Stroke::new(1.5_f32, th.accent))
                .corner_radius(CornerRadius::ZERO)
                .inner_margin(10.0_f32)
                .show(ui, |ui| {
                    ui.set_width(640.0);

                    // Scope tabs (All | @ Artists | # Filters | > Commands)
                    ui.horizontal(|ui| {
                        for (sc, lbl) in [
                            (SearchScope::All, "All"),
                            (SearchScope::Artists, "@ Artists"),
                            (SearchScope::Filters, "# Filters"),
                            (SearchScope::Commands, "> Commands"),
                        ] {
                            let active = state.palette_scope == sc;
                            if ui
                                .selectable_label(
                                    active,
                                    RichText::new(lbl).monospace().color(if active {
                                        th.accent
                                    } else {
                                        th.text_dim
                                    }),
                                )
                                .clicked()
                            {
                                state.palette_scope = sc;
                                let req = state.next_request_id;
                                state.next_request_id += 1;
                                state.active_search_req_id = req;
                                let _ = fx_tx.send(FxCommand::Search {
                                    request_id: req,
                                    text: state.palette_query.clone(),
                                    scope: state.palette_scope,
                                });
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new("Tab: cycle scope · Enter: execute · Esc: close")
                                    .monospace()
                                    .size(10.5)
                                    .color(th.text_dim),
                            );
                        });
                    });

                    // Input field
                    let input_id = ui.make_persistent_id("fx_palette_input");
                    let te = egui::TextEdit::singleline(&mut state.palette_query)
                        .id(input_id)
                        .hint_text("Type to search releases, @artists, #filters, or >commands...")
                        .desired_width(f32::INFINITY);
                    let resp = ui.add(te);
                    if state.focus_palette_next_frame {
                        resp.request_focus();
                        state.focus_palette_next_frame = false;
                    }
                    if resp.changed() {
                        let req = state.next_request_id;
                        state.next_request_id += 1;
                        state.active_search_req_id = req;
                        let _ = fx_tx.send(FxCommand::Search {
                            request_id: req,
                            text: state.palette_query.clone(),
                            scope: state.palette_scope,
                        });
                    }

                    // Keyboard handling inside palette
                    if ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                        state.palette_scope = match state.palette_scope {
                            SearchScope::All => SearchScope::Artists,
                            SearchScope::Artists => SearchScope::Filters,
                            SearchScope::Filters => SearchScope::Commands,
                            SearchScope::Commands => SearchScope::All,
                        };
                        let req = state.next_request_id;
                        state.next_request_id += 1;
                        state.active_search_req_id = req;
                        let _ = fx_tx.send(FxCommand::Search {
                            request_id: req,
                            text: state.palette_query.clone(),
                            scope: state.palette_scope,
                        });
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown))
                        && !state.palette_hits.is_empty()
                    {
                        state.palette_sel =
                            (state.palette_sel + 1).min(state.palette_hits.len() - 1);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                        state.palette_sel = state.palette_sel.saturating_sub(1);
                    }
                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

                    ui.separator();

                    let hits = state.palette_hits.clone();
                    egui::ScrollArea::vertical()
                        .max_height(340.0)
                        .auto_shrink([false, false])
                        .show_rows(ui, 30.0, hits.len(), |ui, range| {
                            for idx in range {
                                let hit = &hits[idx];
                                let is_sel = idx == state.palette_sel;
                                let (row_r, row_resp) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 28.0),
                                    Sense::click(),
                                );

                                if is_sel {
                                    let sel_y = state.motion.val(
                                        "palette_sel_y",
                                        row_r.min.y,
                                        SpringCfg::SNAP,
                                    );
                                    let hi_r = Rect::from_min_size(
                                        Pos2::new(row_r.min.x, sel_y),
                                        row_r.size(),
                                    );
                                    paint::rect(
                                        ui.painter(),
                                        hi_r,
                                        th.bg2,
                                        Stroke::new(1.0_f32, th.accent),
                                    );
                                } else if row_resp.hovered() {
                                    paint::rect_filled(ui.painter(), row_r, th.bg2);
                                }

                                // Thumbnail for Release hits
                                let mut text_x = row_r.min.x + 8.0;
                                if let SearchHitKind::Release { key, art_id } = hit.kind {
                                    let thumb_r = Rect::from_min_size(
                                        Pos2::new(text_x, row_r.center().y - 10.0),
                                        Vec2::splat(20.0),
                                    );
                                    paint_cover_slot(
                                        ui.painter(),
                                        state,
                                        art_id,
                                        key.id,
                                        thumb_r,
                                        Tier::T64,
                                        1.0,
                                        Vec2::ZERO,
                                    );
                                    text_x += 26.0;
                                }

                                // Highlight matched characters in title
                                let mut job = egui::text::LayoutJob::default();
                                for (ci, ch) in hit.title.chars().enumerate() {
                                    let matched = hit.match_indices.contains(&ci);
                                    let mut s = [0_u8; 4];
                                    job.append(
                                        ch.encode_utf8(&mut s),
                                        0.0,
                                        egui::TextFormat {
                                            font_id: FontId::proportional(13.0),
                                            color: if matched { th.accent } else { th.text },
                                            ..Default::default()
                                        },
                                    );
                                }
                                let galley = ui.fonts_mut(|f| f.layout_job(job));
                                ui.painter().galley(
                                    Pos2::new(text_x, row_r.center().y - galley.size().y * 0.5),
                                    galley,
                                    th.text,
                                );

                                // Right-aligned subtitle or key hint
                                let right_txt = match &hit.kind {
                                    SearchHitKind::Command(_, hint) if !hint.is_empty() => {
                                        format!("{}  [{}]", hit.subtitle, hint)
                                    }
                                    _ => hit.subtitle.clone(),
                                };
                                ui.painter().text(
                                    Pos2::new(row_r.max.x - 8.0, row_r.center().y),
                                    Align2::RIGHT_CENTER,
                                    right_txt,
                                    FontId::monospace(10.5),
                                    th.text_dim,
                                );

                                if row_resp.clicked() || (is_sel && enter_pressed) {
                                    match &hit.kind {
                                        SearchHitKind::Artist { band_id } => {
                                            pending_actions
                                                .push(FxAction::SelectArtist(Some(*band_id)));
                                        }
                                        SearchHitKind::Release { key, .. } => {
                                            pending_actions.push(FxAction::OpenDrawer(*key));
                                        }
                                        SearchHitKind::Filter(f) => {
                                            pending_actions.push(FxAction::SetFilter(*f));
                                        }
                                        SearchHitKind::Command(act, _) => {
                                            pending_actions.push(act.clone());
                                        }
                                    }
                                    pending_actions.push(FxAction::ClosePalette);
                                }
                            }
                        });
                });
        });

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}
