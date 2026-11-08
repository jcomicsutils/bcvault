use crate::{
    actions::{FxAction, SmartView},
    motion::SpringCfg,
    paint,
    services::messages::FxCommand,
    state::{FxLinkFolderModalState, FxState},
};
use bcvault_core::{
    types::{ArtistStats, FolderState, SyncMode, SyncScope},
    worker::messages::UiToWorkerCommand,
};
use egui::{Align, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
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

    if state.sidebar_collapsed {
        ui.vertical_centered(|ui| {
            if paint::sharp_button(ui, "»")
                .on_hover_text("Expand sidebar (Ctrl+B)")
                .clicked()
            {
                pending_actions.push(FxAction::ToggleSidebar);
            }
            ui.separator();
            if paint::sharp_button(ui, "ALL")
                .on_hover_text("All artists")
                .clicked()
            {
                pending_actions.push(FxAction::SelectArtist(None));
            }
            for a in state.artists.iter().take(16) {
                let initials: String = a
                    .display_name()
                    .chars()
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                let is_sel = state.query.base.band_id == Some(a.band_id);
                let btn = egui::Button::new(RichText::new(initials).monospace().color(if is_sel {
                    th.accent
                } else {
                    th.text
                }))
                .corner_radius(CornerRadius::ZERO);
                if ui
                    .add_sized([40.0, 28.0], btn)
                    .on_hover_text(a.display_name())
                    .clicked()
                {
                    pending_actions.push(FxAction::SelectArtist(Some(a.band_id)));
                }
            }
        });
        for act in pending_actions {
            state.dispatch(act, core_tx, fx_tx, now_s);
        }
        return;
    }

    // 1. Add Artist input bar
    let top_row_w = ui.available_width();
    let (collapse_clicked, add_clicked, enter_pressed) = ui
        .allocate_ui_with_layout(
            Vec2::new(top_row_w, 28.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                let collapse = paint::sharp_button(ui, "«")
                    .on_hover_text("Collapse sidebar (Ctrl+B)")
                    .clicked();
                let add = ui
                    .add_enabled(
                        !state.add_artist_pending && !state.add_artist_input.trim().is_empty(),
                        egui::Button::new("Add").corner_radius(CornerRadius::ZERO),
                    )
                    .clicked();
                let te = egui::TextEdit::singleline(&mut state.add_artist_input)
                    .hint_text("Add Artist (URL or Slug)")
                    .desired_width(ui.available_width());
                let resp = ui.add_enabled(!state.add_artist_pending, te);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                (collapse, add, enter)
            },
        )
        .inner;

    if collapse_clicked {
        pending_actions.push(FxAction::ToggleSidebar);
    }
    if (enter_pressed || add_clicked) && !state.add_artist_input.trim().is_empty() {
        let input = state.add_artist_input.trim().to_string();
        state.add_artist_input.clear();
        state.add_artist_pending = true;
        let _ = core_tx.send(UiToWorkerCommand::AddArtist { input });
    }

    // 2. Smart Views & Saved Views
    ui.collapsing(
        RichText::new("SMART VIEWS").size(11.0).color(th.text_dim),
        |ui| {
            for sv in SmartView::ALL {
                let active = state.query.smart == Some(sv);
                let resp = ui.selectable_label(
                    active,
                    RichText::new(sv.label()).size(12.0).color(if active {
                        th.accent
                    } else {
                        th.text
                    }),
                );
                if resp.clicked() {
                    let next = if active { None } else { Some(sv) };
                    pending_actions.push(FxAction::SelectSmartView(next));
                }
            }
        },
    );

    ui.collapsing(
        RichText::new("SAVED VIEWS").size(11.0).color(th.text_dim),
        |ui| {
            let views = state.saved_views.clone();
            for v in views {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(false, RichText::new(&v.name).size(12.0).color(th.text))
                        .clicked()
                    {
                        pending_actions.push(FxAction::ApplySavedView(v.id));
                    }
                    if paint::sharp_button(ui, "×")
                        .on_hover_text("Delete saved view")
                        .clicked()
                    {
                        pending_actions.push(FxAction::DeleteSavedView(v.id));
                    }
                });
            }
            if paint::sharp_button(ui, "+ Save Current View...").clicked() {
                state.save_view_modal = Some(String::new());
            }
        },
    );

    // 3. Artist filter
    ui.add(
        egui::TextEdit::singleline(&mut state.artist_filter)
            .hint_text("Filter artists...")
            .desired_width(f32::INFINITY),
    );

    ui.separator();

    let list_height = (ui.available_height() - 78.0).max(80.0);
    let filter_lower = state.artist_filter.trim().to_lowercase();
    let filtered_indices: Vec<usize> = state
        .artists
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            filter_lower.is_empty()
                || a.display_name().to_lowercase().contains(&filter_lower)
                || a.name.to_lowercase().contains(&filter_lower)
                || a.slug.to_lowercase().contains(&filter_lower)
        })
        .map(|(i, _)| i)
        .collect();

    let mut total_stats = ArtistStats::default();
    for a in &state.artists {
        total_stats.downloaded += a.stats.downloaded;
        total_stats.eligible_downloaded += a.stats.eligible_downloaded;
        total_stats.non_eligible_downloaded += a.stats.non_eligible_downloaded;
        total_stats.eligible += a.stats.eligible;
        total_stats.total += a.stats.total;
        total_stats.email_required += a.stats.email_required;
        total_stats.unknown += a.stats.unknown;
        total_stats.quota_used_30d += a.stats.quota_used_30d;
    }

    let total_rows = 1 + filtered_indices.len();

    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show_rows(ui, 28.0, total_rows, |ui, row_range| {
            for row_idx in row_range {
                if row_idx == 0 {
                    let is_selected = state.query.base.band_id.is_none();
                    let resp = draw_artist_row(
                        ui,
                        state,
                        0,
                        "All artists",
                        total_stats,
                        is_selected,
                        None,
                        now_s,
                    );
                    if resp.clicked() {
                        pending_actions.push(FxAction::SelectArtist(None));
                    }
                } else {
                    let artist_idx = filtered_indices[row_idx - 1];
                    let artist = state.artists[artist_idx].clone();
                    let is_selected = state.query.base.band_id == Some(artist.band_id);
                    let sync_frac = state.active_syncs.get(&artist.band_id).map(|p| {
                        if p.total == 0 {
                            0.15_f32
                        } else {
                            (p.done as f32 / p.total as f32).clamp(0.05, 1.0)
                        }
                    });

                    let resp = draw_artist_row(
                        ui,
                        state,
                        artist.band_id,
                        artist.display_name(),
                        artist.stats,
                        is_selected,
                        sync_frac,
                        now_s,
                    );

                    if resp.clicked() {
                        pending_actions.push(FxAction::SelectArtist(Some(artist.band_id)));
                    }

                    resp.context_menu(|ui| {
                        if paint::sharp_button(ui, "Sync Artist").clicked() {
                            let _ = core_tx.send(UiToWorkerCommand::Sync {
                                scope: SyncScope::Selected(vec![artist.band_id]),
                                mode: state.sync_mode,
                            });
                            ui.close();
                        }
                        if let Some(folder) = state.folders.get(&artist.band_id) {
                            if paint::sharp_button(ui, "Change Folder...").clicked() {
                                state.link_folder_modal = Some(FxLinkFolderModalState::new(
                                    Some(artist.band_id),
                                    folder.path.display().to_string(),
                                ));
                                ui.close();
                            }
                            if paint::sharp_button(ui, "Match Folder").clicked() {
                                let _ = core_tx.send(UiToWorkerCommand::MatchArtistFolder {
                                    band_id: artist.band_id,
                                    auto_apply_confident: true,
                                });
                                ui.close();
                            }
                            if paint::sharp_button(ui, "Verify Files").clicked() {
                                let _ = core_tx.send(UiToWorkerCommand::VerifyFiles {
                                    scope: SyncScope::Selected(vec![artist.band_id]),
                                });
                                ui.close();
                            }
                            if paint::sharp_button(ui, "Unlink Folder").clicked() {
                                let _ = core_tx.send(UiToWorkerCommand::UnlinkArtistFolder {
                                    band_id: artist.band_id,
                                });
                                ui.close();
                            }
                        } else {
                            if paint::sharp_button(ui, "Link Folder...").clicked() {
                                state.link_folder_modal =
                                    Some(FxLinkFolderModalState::new_empty(Some(artist.band_id)));
                                ui.close();
                            }
                        }
                        if paint::sharp_button(ui, "Rename Artist...").clicked() {
                            state.rename_artist_modal = Some((
                                artist.band_id,
                                artist.name.clone(),
                                artist.display_name().to_string(),
                            ));
                            ui.close();
                        }
                        if paint::sharp_button(ui, "Open in Browser").clicked() {
                            let _ = open::that_detached(&artist.url);
                            ui.close();
                        }
                        if paint::sharp_button(ui, "Remove Artist...").clicked() {
                            state.confirm_remove_artist =
                                Some((artist.band_id, artist.display_name().to_string()));
                            ui.close();
                        }
                    });
                }
            }
        });

    ui.separator();

    // 4. Footer: Sync controls
    ui.horizontal(|ui| {
        ui.label(RichText::new("Mode:").color(th.text_dim));
        egui::ComboBox::from_id_salt("fx_sync_mode")
            .selected_text(state.sync_mode.label())
            .width(135.0)
            .show_ui(ui, |ui| {
                for mode in SyncMode::ALL {
                    ui.selectable_value(&mut state.sync_mode, mode, mode.label());
                }
            });
    });

    ui.horizontal(|ui| {
        let has_sel = state.query.base.band_id.is_some();
        if ui
            .add_enabled(
                has_sel,
                egui::Button::new("Sync Selected").corner_radius(CornerRadius::ZERO),
            )
            .clicked()
        {
            pending_actions.push(FxAction::SyncSelected(state.sync_mode));
        }
        if paint::sharp_button(ui, "Sync All").clicked() {
            pending_actions.push(FxAction::SyncAll(state.sync_mode));
        }
        if !state.active_syncs.is_empty() && paint::sharp_button(ui, "Stop").clicked() {
            let _ = core_tx.send(UiToWorkerCommand::CancelSync);
        }
    });

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_artist_row(
    ui: &mut Ui,
    state: &mut FxState,
    band_id: i64,
    name: &str,
    stats: ArtistStats,
    selected: bool,
    sync_frac: Option<f32>,
    now_s: f64,
) -> egui::Response {
    let th = state.active_theme;
    let desired = Vec2::new(ui.available_width(), 26.0);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
    let linked_folder = if band_id > 0 {
        state.folders.get(&band_id).cloned()
    } else {
        None
    };

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bg = if selected || resp.hovered() {
            th.bg2
        } else {
            th.bg1
        };
        paint::rect(painter, rect, bg, Stroke::new(1.0_f32, th.border));

        if selected {
            let bar = Rect::from_min_max(rect.min, Pos2::new(rect.min.x + 3.0, rect.max.y));
            paint::rect_filled(painter, bar, th.accent);
        }

        // 2x2 miniature mosaic square on the left
        let mos_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + 6.0, rect.center().y - 8.0),
            Vec2::splat(16.0),
        );
        paint::geometric_placeholder(painter, mos_rect, band_id, th.bg0, th.accent, th.accent2);

        if let Some(frac) = sync_frac {
            let prog_h = 2.0;
            let fill_w = (rect.width() * frac).min(rect.width());
            let fill_r = Rect::from_min_max(
                Pos2::new(rect.min.x, rect.max.y - prog_h),
                Pos2::new(rect.min.x + fill_w, rect.max.y),
            );
            paint::rect_filled(painter, fill_r, th.accent);
            let sweep_x = rect.min.x + ((now_s as f32 * 1.4).rem_euclid(1.0) * fill_w);
            let sweep_r = Rect::from_min_max(
                Pos2::new(sweep_x, rect.max.y - prog_h),
                Pos2::new((sweep_x + 18.0).min(rect.max.x), rect.max.y),
            );
            paint::rect_filled(painter, sweep_r, th.accent2);
            ui.ctx().request_repaint();
        }

        let el_dl_tween = state
            .motion
            .val(
                ("art_el_dl", band_id),
                stats.eligible_downloaded as f32,
                SpringCfg::SMOOTH,
            )
            .round() as u32;
        let non_el_dl_tween = state
            .motion
            .val(
                ("art_non_el_dl", band_id),
                stats.non_eligible_downloaded as f32,
                SpringCfg::SMOOTH,
            )
            .round() as u32;
        let el_tween = state
            .motion
            .val(
                ("art_el", band_id),
                stats.eligible as f32,
                SpringCfg::SMOOTH,
            )
            .round() as u32;
        let tot_tween = state
            .motion
            .val(("art_tot", band_id), stats.total as f32, SpringCfg::SMOOTH)
            .round() as u32;

        let badge_str = if non_el_dl_tween > 0 {
            format!("{el_dl_tween} (+ {non_el_dl_tween})/{el_tween}/{tot_tween}")
        } else {
            format!("{el_dl_tween}/{el_tween}/{tot_tween}")
        };

        let inner = Rect::from_min_max(
            Pos2::new(mos_rect.max.x + 6.0, rect.min.y + 2.0),
            Pos2::new(rect.max.x - 6.0, rect.max.y - 2.0),
        );

        let badge_galley =
            ui.painter()
                .layout_no_wrap(badge_str, FontId::monospace(11.0), th.text_dim);
        let badge_w = badge_galley.size().x;
        let badge_pos = Pos2::new(
            inner.max.x - badge_w,
            inner.center().y - badge_galley.size().y * 0.5,
        );
        ui.painter().galley(badge_pos, badge_galley, th.text_dim);

        let name_rect = Rect::from_min_max(
            inner.min,
            Pos2::new((inner.max.x - badge_w - 6.0).max(inner.min.x), inner.max.y),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(name_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        if let Some(folder) = &linked_folder {
            let dot_color = match folder.state {
                FolderState::Ok => th.accent,
                FolderState::Missing => th.err,
                FolderState::Unreadable => th.warn,
            };
            let (glyph_rect, _) = child.allocate_exact_size(Vec2::new(14.0, 12.0), Sense::hover());
            let p = child.painter();
            paint::icons::paint_icon(p, glyph_rect, paint::icons::IconKind::Folder, th.text_dim);
            let dot = Rect::from_min_max(
                Pos2::new(glyph_rect.max.x - 4.0, glyph_rect.max.y - 4.0),
                glyph_rect.max,
            );
            paint::rect_filled(p, dot, dot_color);
            child.add_space(3.0);
        }
        child.add(egui::Label::new(RichText::new(name).color(th.text)).truncate());
    }

    let folder_tooltip = match &linked_folder {
        Some(f) => format!(
            "\nLinked folder: {}\nFolder status: {:?}",
            f.path.display(),
            f.state
        ),
        None => String::new(),
    };

    let dl_desc = if stats.non_eligible_downloaded > 0 {
        format!(
            "{} ({} eligible + {} paid/extra)",
            stats.downloaded, stats.eligible_downloaded, stats.non_eligible_downloaded
        )
    } else {
        stats.downloaded.to_string()
    };

    resp.on_hover_text(format!(
        "Downloaded: {}\nEligible (Free + NYP + Email): {}\nTotal releases on page: {}\nEmail-gated: {}\nPending metadata: {}\nYour downloads (30d): {}/{}{}",
        dl_desc,
        stats.eligible,
        stats.total,
        stats.email_required,
        stats.unknown,
        stats.quota_used_30d,
        state.settings.quota_hard,
        folder_tooltip,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::theme::{FxTheme, apply_theme};
    use bcvault_core::types::ArtistView;

    #[test]
    fn sidebar_does_not_overflow_or_leave_gap_before_central_panel() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, &FxTheme::PAPER);
        let mut state = FxState::new();
        state.artists = vec![
            ArtistView {
                band_id: 1,
                slug: "areyoudoneyet".to_string(),
                name: "areyoudoneyet".to_string(),
                custom_name: None,
                url: "https://areyoudoneyet.bandcamp.com".to_string(),
                last_synced_at: None,
                stats: ArtistStats {
                    downloaded: 17,
                    eligible_downloaded: 17,
                    non_eligible_downloaded: 0,
                    eligible: 17,
                    total: 17,
                    email_required: 1,
                    unknown: 0,
                    quota_used_30d: 0,
                    archived: 0,
                    missing_files: 0,
                },
            },
            ArtistView {
                band_id: 2,
                slug: "deafballerina".to_string(),
                name: "Deaf Ballerina".to_string(),
                custom_name: None,
                url: "https://deafballerina.bandcamp.com".to_string(),
                last_synced_at: None,
                stats: ArtistStats {
                    downloaded: 11,
                    eligible_downloaded: 11,
                    non_eligible_downloaded: 0,
                    eligible: 11,
                    total: 11,
                    email_required: 0,
                    unknown: 0,
                    quota_used_30d: 0,
                    archived: 0,
                    missing_files: 0,
                },
            },
        ];

        let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        for frame_idx in 0..5 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 860.0))),
                ..Default::default()
            };
            let mut sb_rect = Rect::NOTHING;
            let mut sb_clip = Rect::NOTHING;
            let mut sb_inner_min = Rect::NOTHING;
            let mut central_rect = Rect::NOTHING;
            let _ = ctx.run(raw, |ctx| {
                state.motion.begin_frame_dt(1.0 / 60.0);
                let sb = egui::SidePanel::left("fx_sidebar_expanded")
                    .default_width(280.0)
                    .min_width(240.0)
                    .max_width(500.0)
                    .resizable(true)
                    .show(ctx, |ui| {
                        sb_clip = ui.clip_rect();
                        ui.set_max_width(ui.available_width());
                        show(ui, &mut state, &core_tx, &fx_tx, frame_idx as f64 * 0.016);
                        sb_inner_min = ui.min_rect();
                    });
                sb_rect = sb.response.rect;
                let cp = egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("central");
                });
                central_rect = cp.response.rect;
                state.motion.end_frame(ctx);
            });

            assert!(
                sb_inner_min.max.x <= sb_clip.max.x,
                "sidebar inner content max.x ({}) exceeded clip_rect max.x ({}) on frame {frame_idx}",
                sb_inner_min.max.x,
                sb_clip.max.x
            );
            assert!(
                (sb_rect.max.x - sb_clip.max.x).abs() <= 1.0,
                "sidebar response rect max.x ({}) != clip_rect max.x ({}) on frame {frame_idx}",
                sb_rect.max.x,
                sb_clip.max.x
            );
            assert!(
                (sb_rect.width() - 280.0).abs() <= 1.0,
                "sidebar width drifted to {} on frame {frame_idx}",
                sb_rect.width()
            );
            assert!(
                (central_rect.min.x - sb_rect.max.x).abs() <= 1.0,
                "gap between sidebar ({}) and central panel ({}) on frame {frame_idx}",
                sb_rect.max.x,
                central_rect.min.x
            );
        }
    }
}
