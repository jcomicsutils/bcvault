use crate::{
    actions::FxAction, motion::EffectsLevel, paint, services::messages::FxCommand, state::FxState,
    theme::ThemeId,
};
use bcvault_core::{
    types::{AudioFormat, DownloadOpts, OutputLayout},
    worker::messages::UiToWorkerCommand,
};
use egui::{CornerRadius, RichText};
use tokio::sync::mpsc::UnboundedSender;

pub fn show_modals(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    now_s: f64,
) {
    let th = state.active_theme;
    let mut pending_actions: Vec<FxAction> = Vec::new();

    // 1. Settings Modal (FX Preferences + Core Engine Settings)
    if state.settings_modal_open {
        let mut close_modal = false;
        let mut save_draft = None;

        egui::Window::new("bcvault-fx Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(540.0);

                ui.label(
                    RichText::new("VISUALS & MOTION")
                        .monospace()
                        .strong()
                        .color(th.accent),
                );
                ui.horizontal(|ui| {
                    ui.label("Theme:");
                    for tid in ThemeId::ALL {
                        let active = state.prefs.theme == tid;
                        if ui
                            .selectable_label(active, tid.label())
                            .clicked()
                        {
                            pending_actions.push(FxAction::SetTheme(tid));
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Effects Level:");
                    for lvl in EffectsLevel::ALL {
                        let active = state.prefs.effects == lvl;
                        if ui
                            .selectable_label(active, lvl.label())
                            .clicked()
                        {
                            pending_actions.push(FxAction::SetEffects(lvl));
                        }
                    }
                });

                ui.horizontal(|ui| {
                    let mut ad = state.prefs.adaptive_accent;
                    if ui
                        .checkbox(&mut ad, "Adaptive accent from hovered cover")
                        .changed()
                    {
                        pending_actions.push(FxAction::SetAdaptiveAccent(ad));
                    }
                    let mut rm = state.prefs.reduced_motion;
                    if ui.checkbox(&mut rm, "Reduced motion").changed() {
                        pending_actions.push(FxAction::SetReducedMotion(rm));
                    }
                });

                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut state.prefs.animated_grain, "Animated grain (30 fps)")
                        .changed()
                    {
                        let _ = fx_tx.send(FxCommand::SetPref {
                            key: "animated_grain".to_string(),
                            value: if state.prefs.animated_grain { "1" } else { "0" }.to_string(),
                        });
                    }
                    if ui
                        .checkbox(&mut state.prefs.scanlines, "CRT Scanlines")
                        .changed()
                    {
                        let _ = fx_tx.send(FxCommand::SetPref {
                            key: "scanlines".to_string(),
                            value: if state.prefs.scanlines { "1" } else { "0" }.to_string(),
                        });
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Texture VRAM Budget (MB):");
                    if ui
                        .add(
                            egui::DragValue::new(&mut state.prefs.tex_budget_mb)
                                .range(64..=1024),
                        )
                        .changed()
                    {
                        state.tex_cache.set_budget_mb(state.prefs.tex_budget_mb);
                        let _ = fx_tx.send(FxCommand::SetPref {
                            key: "tex_budget_mb".to_string(),
                            value: state.prefs.tex_budget_mb.to_string(),
                        });
                    }
                    let cache_mb = (state.cover_cache_bytes as f64) / (1024.0 * 1024.0);
                    ui.label(
                        RichText::new(format!("Disk cache: {cache_mb:.1} MB"))
                            .monospace()
                            .color(th.text_dim),
                    );
                    if paint::sharp_button(ui, "Purge Covers").clicked() {
                        pending_actions.push(FxAction::PurgeCovers);
                    }
                });

                ui.separator();

                ui.label(
                    RichText::new("DOWNLOADS & QUOTA")
                        .monospace()
                        .strong()
                        .color(th.accent),
                );
                if let Some(draft) = &mut state.settings_draft {
                    egui::Grid::new("fx_settings_grid")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("Output Directory:");
                            ui.horizontal(|ui| {
                                let mut p_str = draft.out_dir.to_string_lossy().into_owned();
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut p_str).desired_width(300.0),
                                    )
                                    .changed()
                                {
                                    draft.out_dir = std::path::PathBuf::from(p_str);
                                }
                            });
                            ui.end_row();

                            ui.label("Output Layout:");
                            egui::ComboBox::from_id_salt("fx_set_layout")
                                .selected_text(draft.layout.label())
                                .show_ui(ui, |ui| {
                                    for l in OutputLayout::ALL {
                                        ui.selectable_value(&mut draft.layout, l, l.label());
                                    }
                                });
                            ui.end_row();

                            ui.label("Filename Pattern:");
                            ui.vertical(|ui| {
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(
                                            &mut state.filename_pattern_input,
                                        )
                                        .hint_text("{artist} - {album}")
                                        .desired_width(300.0),
                                    )
                                    .changed()
                                {
                                    draft.filename_pattern =
                                        state.filename_pattern_input.clone();
                                }
                                ui.label(
                                    RichText::new(
                                        "Tags: {artist}, {page_artist}, {album}, {year}, {date}, {format}, {type}, {id}",
                                    )
                                    .size(10.5)
                                    .color(th.text_dim),
                                );
                            });
                            ui.end_row();

                            ui.label("Preferred Format:");
                            egui::ComboBox::from_id_salt("fx_set_fmt")
                                .selected_text(draft.format.label())
                                .show_ui(ui, |ui| {
                                    for f in AudioFormat::ALL {
                                        ui.selectable_value(&mut draft.format, f, f.label());
                                    }
                                });
                            ui.end_row();

                            ui.label("Batch Limit / Soft / Hard (30d):");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut draft.batch_limit).range(1..=100),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut draft.quota_soft).range(1..=200),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut draft.quota_hard).range(1..=200),
                                );
                            });
                            ui.end_row();
                        });

                    ui.separator();

                    ui.horizontal(|ui| {
                        if paint::sharp_button(ui, "Save").clicked() {
                            save_draft = Some(draft.clone());
                            close_modal = true;
                        }
                        if paint::sharp_button(ui, "Cancel").clicked() {
                            close_modal = true;
                        }
                        if paint::sharp_button(ui, "Export Settings JSON").clicked() {
                            let export_path = draft.out_dir.join("bcvault-settings.json");
                            if let Ok(json) = serde_json::to_string_pretty(&draft) {
                                let _ = std::fs::create_dir_all(&draft.out_dir);
                                let _ = std::fs::write(export_path, json);
                            }
                        }
                    });
                }
            });

        if let Some(d) = save_draft {
            let _ = core_tx.send(UiToWorkerCommand::UpdateSettings(d));
        }
        if close_modal {
            state.settings_modal_open = false;
            state.settings_draft = None;
        }
    }

    // 2. Redeem URL Modal
    if let Some((target_key, title, mut url_input)) = state.redeem_modal.clone() {
        let mut close_redeem = false;
        egui::Window::new("Redeem Bandcamp Download URL")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("Target: {title}")).strong());
                ui.add(
                    egui::TextEdit::singleline(&mut url_input)
                        .hint_text("https://bandcamp.com/download?...")
                        .desired_width(400.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !url_input.trim().is_empty(),
                            egui::Button::new("Redeem & Download")
                                .corner_radius(CornerRadius::ZERO),
                        )
                        .clicked()
                    {
                        let opts = DownloadOpts {
                            out_dir: state.settings.out_dir.clone(),
                            format: state.settings.format,
                            batch_limit: state.settings.batch_limit,
                        };
                        let _ = core_tx.send(UiToWorkerCommand::RedeemUrl {
                            target: target_key,
                            url: url_input.trim().to_string(),
                            opts,
                        });
                        close_redeem = true;
                    }
                    if paint::sharp_button(ui, "Cancel").clicked() {
                        close_redeem = true;
                    }
                });
            });
        if close_redeem {
            state.redeem_modal = None;
        } else {
            state.redeem_modal = Some((target_key, title, url_input));
        }
    }

    // 3. Save View Modal
    if let Some(mut name_input) = state.save_view_modal.clone() {
        let mut close_sv = false;
        egui::Window::new("Save Current View")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut name_input)
                        .hint_text("View name...")
                        .desired_width(280.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !name_input.trim().is_empty(),
                            egui::Button::new("Save").corner_radius(CornerRadius::ZERO),
                        )
                        .clicked()
                    {
                        pending_actions
                            .push(FxAction::SaveCurrentView(name_input.trim().to_string()));
                        close_sv = true;
                    }
                    if paint::sharp_button(ui, "Cancel").clicked() {
                        close_sv = true;
                    }
                });
            });
        if close_sv {
            state.save_view_modal = None;
        } else {
            state.save_view_modal = Some(name_input);
        }
    }

    // 4. Rename Artist Modal
    if let Some((band_id, orig_name, mut name_input)) = state.rename_artist_modal.clone() {
        let mut close_ren = false;
        egui::Window::new("Rename Page Artist")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("Bandcamp name: {orig_name}"))
                        .size(11.5)
                        .color(th.text_dim),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut name_input)
                        .hint_text(&orig_name)
                        .desired_width(300.0),
                );
                ui.horizontal(|ui| {
                    if paint::sharp_button(ui, "Save").clicked() {
                        let trimmed = name_input.trim();
                        let custom_name = if trimmed.is_empty() || trimmed == orig_name {
                            None
                        } else {
                            Some(trimmed.to_string())
                        };
                        let _ = core_tx.send(UiToWorkerCommand::RenameArtist {
                            band_id,
                            custom_name,
                        });
                        close_ren = true;
                    }
                    if paint::sharp_button(ui, "Reset to Default").clicked() {
                        let _ = core_tx.send(UiToWorkerCommand::RenameArtist {
                            band_id,
                            custom_name: None,
                        });
                        close_ren = true;
                    }
                    if paint::sharp_button(ui, "Cancel").clicked() {
                        close_ren = true;
                    }
                });
            });
        if close_ren {
            state.rename_artist_modal = None;
        } else {
            state.rename_artist_modal = Some((band_id, orig_name, name_input));
        }
    }

    // 5. Confirm Remove Artist Modal
    if let Some((band_id, name)) = state.confirm_remove_artist.clone() {
        let mut close_rem = false;
        egui::Window::new("Remove Artist")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Remove '{name}' from tracked artists?"));
                ui.horizontal(|ui| {
                    if paint::sharp_button(ui, "Remove").clicked() {
                        let _ = core_tx.send(UiToWorkerCommand::RemoveArtist { band_id });
                        close_rem = true;
                    }
                    if paint::sharp_button(ui, "Cancel").clicked() {
                        close_rem = true;
                    }
                });
            });
        if close_rem {
            state.confirm_remove_artist = None;
        }
    }

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}
