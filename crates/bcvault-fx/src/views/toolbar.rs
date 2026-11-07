use crate::{
    actions::{FxAction, FxSortKey, GroupBy, ViewMode},
    motion::SpringCfg,
    paint,
    services::messages::FxCommand,
    state::{CARD_SIZE_LABELS, FxState},
};
use bcvault_core::{
    types::{AudioFormat, ReleaseFilter, SortDir},
    worker::messages::UiToWorkerCommand,
};
use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
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

    // Row 1: Nav Back/Forward | Breadcrumbs | View Mode Segmented | Card Size | Command Palette | Settings
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                state.nav.can_back(),
                egui::Button::new("◀").corner_radius(CornerRadius::ZERO),
            )
            .on_hover_text("Back (Alt+Left)")
            .clicked()
        {
            pending_actions.push(FxAction::NavBack);
        }
        if ui
            .add_enabled(
                state.nav.can_forward(),
                egui::Button::new("▶").corner_radius(CornerRadius::ZERO),
            )
            .on_hover_text("Forward (Alt+Right)")
            .clicked()
        {
            pending_actions.push(FxAction::NavForward);
        }

        ui.separator();

        // Clickable Breadcrumbs
        if paint::sharp_button(ui, "All artists").clicked() {
            pending_actions.push(FxAction::SelectArtist(None));
        }
        if let Some(bid) = state.query.base.band_id
            && let Some(artist) = state.artists.iter().find(|a| a.band_id == bid)
        {
            ui.label(RichText::new("›").color(th.text_dim));
            if paint::sharp_button(ui, artist.display_name()).clicked() {
                pending_actions.push(FxAction::SetFilter(ReleaseFilter::All));
            }
        }
        if state.query.base.filter != ReleaseFilter::All {
            ui.label(RichText::new("›").color(th.text_dim));
            ui.label(
                RichText::new(state.query.base.filter.label())
                    .monospace()
                    .color(th.accent),
            );
        }

        ui.separator();

        // View mode segmented buttons
        for vm in ViewMode::ALL {
            let active = state.view_mode == vm;
            let btn = egui::Button::new(RichText::new(vm.label()).color(if active {
                th.accent
            } else {
                th.text
            }))
            .fill(if active { th.bg2 } else { th.bg1 })
            .stroke(Stroke::new(
                1.0_f32,
                if active { th.accent } else { th.border },
            ))
            .corner_radius(CornerRadius::ZERO);
            if ui.add(btn).clicked() {
                pending_actions.push(FxAction::SetView(vm));
            }
        }

        if state.view_mode == ViewMode::Wall {
            ui.separator();
            for (idx, &lbl) in CARD_SIZE_LABELS.iter().enumerate() {
                let active = state.prefs.card_size_idx == idx;
                let btn = egui::Button::new(RichText::new(lbl).monospace().color(if active {
                    th.accent
                } else {
                    th.text_dim
                }))
                .corner_radius(CornerRadius::ZERO);
                if ui.add(btn).clicked() {
                    let delta = (idx as i8) - (state.prefs.card_size_idx as i8);
                    pending_actions.push(FxAction::CardSize(delta));
                }
            }

            egui::ComboBox::from_id_salt("fx_group_by")
                .selected_text(state.query.group.label())
                .width(110.0)
                .show_ui(ui, |ui| {
                    for g in GroupBy::ALL {
                        let mut curr = state.query.group;
                        if ui.selectable_value(&mut curr, g, g.label()).clicked() {
                            pending_actions.push(FxAction::SetGroup(g));
                        }
                    }
                });

            egui::ComboBox::from_id_salt("fx_wall_sort")
                .selected_text(format!("Sort: {}", state.query.fx_sort.label()))
                .width(110.0)
                .show_ui(ui, |ui| {
                    for sk in FxSortKey::ALL {
                        let mut curr = state.query.fx_sort;
                        if ui.selectable_value(&mut curr, sk, sk.label()).clicked() {
                            pending_actions.push(FxAction::SetSort(sk, state.query.base.dir));
                        }
                    }
                });

            let dir_icon = match state.query.base.dir {
                SortDir::Asc => "↑",
                SortDir::Desc => "↓",
            };
            if paint::sharp_button(ui, dir_icon)
                .on_hover_text(match state.query.base.dir {
                    SortDir::Asc => "Ascending (click for Descending)",
                    SortDir::Desc => "Descending (click for Ascending)",
                })
                .clicked()
            {
                pending_actions.push(FxAction::SetSort(
                    state.query.fx_sort,
                    state.query.base.dir.toggle(),
                ));
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if paint::sharp_button(ui, "Settings").clicked() {
                state.settings_draft = Some(state.settings.clone());
                state.filename_pattern_input = state.settings.filename_pattern.clone();
                state.settings_modal_open = true;
            }
            if paint::sharp_button(ui, "Ctrl+K Palette")
                .on_hover_text("Open Command Palette (Ctrl+K)")
                .clicked()
            {
                pending_actions.push(FxAction::OpenPalette);
            }
        });
    });

    // Row 2: Search | Segmented Facet Chips with tweening counts | 12 Hue Swatches | Download Selected + Quota Gauge
    ui.horizontal(|ui| {
        let search_id = ui.make_persistent_id("fx_toolbar_search");
        let te = egui::TextEdit::singleline(&mut state.query.base.search)
            .id(search_id)
            .hint_text("Search releases (/ or Ctrl+F)...")
            .desired_width(180.0);
        let s_resp = ui.add(te);
        if state.focus_search_next_frame {
            s_resp.request_focus();
            state.focus_search_next_frame = false;
        }
        if s_resp.changed() {
            state.mark_search_debounced();
        }

        ui.separator();

        // Facet chips with tweening counts and sliding active indicator
        let facet_items = [
            (ReleaseFilter::All, "All", state.facets.all),
            (ReleaseFilter::Free, "Free", state.facets.free),
            (ReleaseFilter::Nyp, "NYP", state.facets.nyp),
            (
                ReleaseFilter::EmailRequired,
                "Email",
                state.facets.email_required,
            ),
            (ReleaseFilter::Paid, "Paid", state.facets.paid),
            (
                ReleaseFilter::NotDownloaded,
                "Not DL",
                state.facets.not_downloaded,
            ),
            (ReleaseFilter::Updated, "Updated", state.facets.updated),
        ];

        for (filter, name, raw_count) in facet_items {
            let active = state.query.smart.is_none() && state.query.base.filter == filter;
            let count_tween = state
                .motion
                .val(("facet_cnt", name), raw_count as f32, SpringCfg::SMOOTH)
                .round() as u32;
            let txt = format!("{name} ({count_tween})");
            let resp = ui.add(
                egui::Button::new(RichText::new(txt).size(11.5).color(if active {
                    th.accent
                } else {
                    th.text
                }))
                .fill(if active { th.bg2 } else { th.bg1 })
                .stroke(Stroke::new(
                    1.0_f32,
                    if active { th.accent } else { th.border },
                ))
                .corner_radius(CornerRadius::ZERO),
            );
            if active {
                let ind_x = state
                    .motion
                    .val("facet_indicator_x", resp.rect.min.x, SpringCfg::SNAP);
                let ind_w =
                    state
                        .motion
                        .val("facet_indicator_w", resp.rect.width(), SpringCfg::SNAP);
                let bar = Rect::from_min_size(
                    Pos2::new(ind_x, resp.rect.max.y - 2.0),
                    Vec2::new(ind_w, 2.0),
                );
                paint::rect_filled(ui.painter(), bar, th.accent);
            }
            if resp.clicked() {
                pending_actions.push(FxAction::SetFilter(filter));
            }
        }

        ui.separator();

        // 12 Hue Swatches colour filter
        let swatch_colors: [(u16, u16, Color32); 6] = [
            (0, 45, Color32::from_rgb(230, 55, 65)),
            (45, 95, Color32::from_rgb(235, 165, 35)),
            (95, 160, Color32::from_rgb(55, 200, 95)),
            (160, 220, Color32::from_rgb(30, 195, 210)),
            (220, 290, Color32::from_rgb(85, 110, 240)),
            (290, 360, Color32::from_rgb(215, 70, 210)),
        ];
        for (h_lo, h_hi, col) in swatch_colors {
            let is_active = state.query.hue == Some((h_lo, h_hi));
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::click());
            paint::rect(
                ui.painter(),
                r,
                col,
                Stroke::new(
                    if is_active { 2.0_f32 } else { 1.0_f32 },
                    if is_active { th.text } else { th.border },
                ),
            );
            if resp
                .on_hover_text(format!("Filter hue {h_lo}°–{h_hi}°"))
                .clicked()
            {
                let next = if is_active { None } else { Some((h_lo, h_hi)) };
                pending_actions.push(FxAction::SetHue(next));
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Segmented 30d quota gauge for selected artist
            if let Some(bid) = state.query.base.band_id
                && let Some(artist) = state.artists.iter().find(|a| a.band_id == bid)
            {
                let used = artist.stats.quota_used_30d;
                let hard = state.settings.quota_hard.max(1);
                let (g_rect, g_resp) =
                    ui.allocate_exact_size(Vec2::new(72.0, 16.0), Sense::hover());
                paint::rect(
                    ui.painter(),
                    g_rect,
                    th.bg0,
                    Stroke::new(1.0_f32, th.border),
                );
                let segs = 10_u32;
                let sw = (g_rect.width() - 4.0) / (segs as f32);
                for s_i in 0..segs {
                    let threshold = ((s_i + 1) * hard) / segs;
                    if used >= threshold || (s_i == 0 && used > 0) {
                        let col = if used >= state.settings.quota_hard {
                            th.err
                        } else if used >= state.settings.quota_soft {
                            th.warn
                        } else {
                            th.accent
                        };
                        let sr = Rect::from_min_size(
                            Pos2::new(g_rect.min.x + 2.0 + (s_i as f32) * sw, g_rect.min.y + 2.0),
                            Vec2::new((sw - 1.5).max(2.0), g_rect.height() - 4.0),
                        );
                        paint::rect_filled(ui.painter(), sr, col);
                    }
                }
                g_resp.on_hover_text(format!(
                    "30-day downloads for {}: {} / {} (soft warning at {})",
                    artist.display_name(),
                    used,
                    hard,
                    state.settings.quota_soft
                ));
            }

            let sel_cnt = state.selected_keys.len();
            let dl_label = if sel_cnt > 0 {
                format!("Download ({sel_cnt})")
            } else {
                "Download".to_string()
            };
            if paint::sharp_button(ui, dl_label).clicked() {
                pending_actions.push(FxAction::DownloadSelected);
            }

            egui::ComboBox::from_id_salt("fx_toolbar_fmt")
                .selected_text(state.settings.format.label())
                .width(72.0)
                .show_ui(ui, |ui| {
                    for fmt in AudioFormat::ALL {
                        if ui
                            .selectable_value(&mut state.settings.format, fmt, fmt.label())
                            .changed()
                        {
                            let _ = core_tx
                                .send(UiToWorkerCommand::UpdateSettings(state.settings.clone()));
                        }
                    }
                });
        });
    });

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}
