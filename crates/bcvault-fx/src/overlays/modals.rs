use crate::{
    actions::FxAction,
    motion::EffectsLevel,
    paint,
    services::messages::{FxCommand, Tier},
    state::{FxLocalAction, FxMissingItemChoice, FxReportTab, FxState, paint_cover_slot},
    theme::ThemeId,
};
use bcvault_core::{
    library::matcher::Signal,
    types::{
        AudioFormat, DeletedPolicy, DownloadOpts, FolderState, MatchDecision, MissingAction,
        MissingDecision, OnMissing, OutputLayout, RememberChoice, UnavailableAction,
        UnavailableDecision, UnavailableRoot,
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{CornerRadius, RichText, Stroke, Vec2};
use tokio::sync::mpsc::UnboundedSender;

pub fn show_modals(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    rt_handle: &tokio::runtime::Handle,
    now_s: f64,
) {
    let th = state.active_theme;
    let mut pending_actions: Vec<FxAction> = Vec::new();
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));

    // 1. Settings Modal (FX Preferences + Core Engine Settings)
    if state.settings_modal_open {
        let mut close_modal = esc;
        let mut save_draft = None;

        egui::Window::new("bcvault-fx Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(540.0);

                egui::ScrollArea::vertical()
                    .max_height(540.0)
                    .show(ui, |ui| {
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

                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("LIBRARY BEHAVIOUR")
                            .monospace()
                            .strong()
                            .color(th.accent),
                    );

                    let mut vo = draft.verify_on_sync;
                    if ui
                        .checkbox(&mut vo, "Verify files after every sync")
                        .changed()
                    {
                        draft.verify_on_sync = vo;
                    }

                    let mut ro = draft.rescan_linked_on_sync;
                    if ui
                        .checkbox(
                            &mut ro,
                            "Re-read linked folders on sync (auto-adopt certain matches)",
                        )
                        .changed()
                    {
                        draft.rescan_linked_on_sync = ro;
                    }

                    let mut fm = draft.auto_relink;
                    if ui
                        .checkbox(&mut fm, "Try to find moved albums automatically")
                        .changed()
                    {
                        draft.auto_relink = fm;
                    }

                    ui.horizontal(|ui| {
                        ui.label("When files are missing:")
                            .on_hover_text("Ask: show resolve missing dialog on sync\nHandle automatically: apply deleted policy\nNote: an unavailable folder always asks");
                        for om in [OnMissing::Ask, OnMissing::Auto] {
                            let active = draft.on_missing == om;
                            let lbl = match om {
                                OnMissing::Ask => "Ask",
                                OnMissing::Auto => "Handle automatically",
                            };
                            if ui.selectable_label(active, lbl).clicked() {
                                draft.on_missing = om;
                            }
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("If deleted (used when handled automatically, and as the default in the dialog):")
                            .on_hover_text("Keep marked: mark as archived and keep downloaded = 1\nMark as not downloaded: clear downloaded mark\n(used when handled automatically, and as the default in the dialog)");
                        for dp in [DeletedPolicy::KeepMarked, DeletedPolicy::MarkNotDownloaded] {
                            let active = draft.deleted_policy == dp;
                            let lbl = match dp {
                                DeletedPolicy::KeepMarked => "Keep marked as downloaded (archived)",
                                DeletedPolicy::MarkNotDownloaded => "Mark as not downloaded",
                            };
                            if ui.selectable_label(active, lbl).clicked() {
                                draft.deleted_policy = dp;
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(
                        RichText::new("MATCHING THRESHOLDS")
                            .monospace()
                            .strong()
                            .color(th.accent),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Auto-Match Min Score:")
                            .on_hover_text("Score threshold (0.50 - 1.00) required to adopt non-exact (fuzzy) matches automatically.");
                        ui.add(
                            egui::Slider::new(&mut draft.auto_match_min, 0.50..=1.00)
                                .step_by(0.01)
                                .custom_formatter(|n, _| format!("{:.2}", n)),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Review Min Score:")
                            .on_hover_text("Minimum score threshold (0.50 - 1.00) to propose non-exact (fuzzy) matches for review.");
                        ui.add(
                            egui::Slider::new(&mut draft.review_match_min, 0.50..=1.00)
                                .step_by(0.01)
                                .custom_formatter(|n, _| format!("{:.2}", n)),
                        );
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(
                        RichText::new("FOLDER NAMING")
                            .monospace()
                            .strong()
                            .color(th.accent),
                    );
                    ui.label(
                        RichText::new(
                            "Declare how your folders are named using placeholders: {title} (required), \
                            {artist}, {year}, and {noise}. Punctuation/brackets are ignored during matching.",
                        )
                        .color(th.text_dim)
                        .size(11.5),
                    );

                    ui.add_space(4.0);
                    ui.label(RichText::new("Templates:").size(12.0));

                    let mut remove_tmpl = None;
                    let can_remove_tmpl = draft.folder_style.templates.len() > 1;
                    for (idx, tmpl) in draft.folder_style.templates.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(tmpl).desired_width(260.0));
                            if can_remove_tmpl && paint::sharp_small_button(ui, "✕").clicked() {
                                remove_tmpl = Some(idx);
                            }
                        });
                    }
                    if let Some(idx) = remove_tmpl {
                        draft.folder_style.templates.remove(idx);
                    }

                    ui.horizontal(|ui| {
                        if draft.folder_style.templates.len() < 8
                            && paint::sharp_small_button(ui, "+ Add Template").clicked()
                        {
                            draft.folder_style.templates.push("{title}".to_string());
                        }
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Presets:").color(th.text_dim).size(11.5));
                        let presets = [
                            "{title}",
                            "{artist} - {title}",
                            "{year} - {title}",
                            "{title} ({year})",
                            "{title} ({noise})",
                            "{artist} - {year} - {title}",
                        ];
                        for p in presets {
                            if paint::sharp_small_button(ui, p).clicked()
                                && !draft.folder_style.templates.iter().any(|t| t == p)
                                && draft.folder_style.templates.len() < 8
                            {
                                draft.folder_style.templates.push(p.to_string());
                            }
                        }
                    });

                    ui.add_space(4.0);
                    ui.label(RichText::new("Noise List ({noise} tokens):").size(12.0));
                    ui.label(
                        RichText::new("Noise decorations are only recognized through the {noise} placeholder.")
                            .color(th.text_dim)
                            .size(11.5),
                    );

                    let mut remove_noise = None;
                    for (idx, entry) in draft.folder_style.noise.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(entry).desired_width(180.0));
                            if paint::sharp_small_button(ui, "✕").clicked() {
                                remove_noise = Some(idx);
                            }
                        });
                    }
                    if let Some(idx) = remove_noise {
                        draft.folder_style.noise.remove(idx);
                    }

                    ui.horizontal(|ui| {
                        if draft.folder_style.noise.len() < 32
                            && paint::sharp_small_button(ui, "+ Add Noise").clicked()
                        {
                            draft.folder_style.noise.push(String::new());
                        }
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Quick-add:").color(th.text_dim).size(11.5));
                        let quick_adds = [
                            "FLAC", "MP3", "V0", "320", "WAV", "AAC", "OGG", "ALAC", "24bit",
                        ];
                        for q in quick_adds {
                            if paint::sharp_small_button(ui, q).clicked()
                                && !draft
                                    .folder_style
                                    .noise
                                    .iter()
                                    .any(|n| n.eq_ignore_ascii_case(q))
                                && draft.folder_style.noise.len() < 32
                            {
                                draft.folder_style.noise.push(q.to_string());
                            }
                        }
                    });

                    if let Err(err) = draft.folder_style.validate() {
                        ui.label(RichText::new(format!("⚠ {}", err)).color(th.err));
                    }
                    for hint in draft.folder_style.hints() {
                        ui.label(RichText::new(format!("ℹ {}", hint)).color(th.warn));
                    }

                    if !state.folders.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new("Linked Folders:").monospace().color(th.text_dim));
                        let mut to_unlink = None;
                        for (band_id, folder) in &state.folders {
                            let artist_name = state
                                .artists
                                .iter()
                                .find(|a| a.band_id == *band_id)
                                .map(|a| a.display_name().to_string())
                                .unwrap_or_else(|| format!("Artist {band_id}"));
                            ui.horizontal(|ui| {
                                let dot_color = match folder.state {
                                    FolderState::Ok => th.accent,
                                    FolderState::Missing => th.err,
                                    FolderState::Unreadable => th.warn,
                                };
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                paint::rect_filled(ui.painter(), r, dot_color);
                                ui.label(RichText::new(&artist_name).strong());
                                ui.label(
                                    RichText::new(folder.path.display().to_string())
                                        .color(th.text_dim)
                                        .monospace(),
                                );
                                if paint::sharp_button(ui, "Unlink").clicked() {
                                    to_unlink = Some(*band_id);
                                }
                            });
                        }
                        if let Some(bid) = to_unlink {
                            let _ = core_tx.send(UiToWorkerCommand::UnlinkArtistFolder { band_id: bid });
                        }
                    }
                }
            });

        if let Some(draft) = &mut state.settings_draft {
            ui.separator();

            ui.horizontal(|ui| {
                if paint::sharp_button(ui, "Save").clicked() {
                    draft.quota_soft = draft.quota_soft.min(draft.quota_hard);
                    draft.batch_limit = draft.batch_limit.min(draft.quota_hard);
                    draft.review_match_min = draft.review_match_min.min(draft.auto_match_min);
                    draft.folder_style.sanitize();
                    save_draft = Some(draft.clone());
                    close_modal = true;
                }
                if paint::sharp_button(ui, "Cancel").clicked() {
                    close_modal = true;
                }
                if paint::sharp_button(ui, "Export Settings JSON").clicked()
                    && let Ok(json) = serde_json::to_string_pretty(&draft)
                {
                    crate::services::export_settings_json(
                        rt_handle,
                        draft.out_dir.clone(),
                        json,
                    );
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
        let mut close_redeem = esc;
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
        let mut close_sv = esc;
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
        let mut close_ren = esc;
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
        let mut close_rem = esc;
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

    show_link_folder_modal(ctx, state, core_tx, rt_handle, esc);
    show_reconcile_report_modal(ctx, state, core_tx, esc);
    show_resolve_missing_modal(ctx, state, core_tx, rt_handle, esc);
    show_unavailable_modal(ctx, state, core_tx, rt_handle, esc);

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}

fn format_signals(signals: &[Signal]) -> String {
    if signals.is_empty() {
        return "No signals recorded".to_string();
    }
    signals
        .iter()
        .map(|s| match s {
            Signal::UrlTag => "URL tag match (exact)",
            Signal::AlbumTag => "Album tag match",
            Signal::FolderName => "Folder name match",
            Signal::TrackCount => "Track count match",
            Signal::Year => "Year tag match",
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn show_link_folder_modal(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
    esc: bool,
) {
    let Some(mut modal) = state.link_folder_modal.take() else {
        return;
    };
    let mut keep_open = !esc;

    if let Ok(mut lock) = modal.picked_folder.lock()
        && let Some(p) = lock.take()
    {
        modal.folder_path = p.display().to_string();
    }

    let th = state.active_theme;
    let screen = ctx.content_rect();
    let max_w = (screen.width() - 48.0).clamp(420.0, 620.0);

    egui::Window::new("Link Artist Folder")
        .collapsible(false)
        .resizable(false)
        .max_size([max_w, (screen.height() - 48.0).max(300.0)])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_min_width(ui.available_width().min(460.0));

            ui.label(
                RichText::new("Link a local music folder to a Bandcamp artist:").color(th.text_dim),
            );
            ui.add_space(4.0);

            egui::Grid::new("fx_link_folder_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Artist Page:");
                    if let Some(band_id) = modal.band_id {
                        let artist_name = state
                            .artists
                            .iter()
                            .find(|a| a.band_id == band_id)
                            .map(|a| a.display_name().to_string())
                            .unwrap_or_else(|| format!("Artist {band_id}"));
                        ui.label(RichText::new(artist_name).strong());
                    } else {
                        ui.vertical(|ui| {
                            let mut selected_text = "Select tracked artist...".to_string();
                            if let Some(band_id) = modal.band_id
                                && let Some(a) = state.artists.iter().find(|a| a.band_id == band_id)
                            {
                                selected_text = a.display_name().to_string();
                            }
                            egui::ComboBox::from_id_salt("fx_link_modal_artist_combo")
                                .selected_text(&selected_text)
                                .width(280.0)
                                .show_ui(ui, |ui| {
                                    for a in &state.artists {
                                        if ui
                                            .selectable_label(
                                                modal.band_id == Some(a.band_id),
                                                a.display_name(),
                                            )
                                            .clicked()
                                        {
                                            modal.band_id = Some(a.band_id);
                                            modal.artist_input = a.url.clone();
                                        }
                                    }
                                });

                            let text_resp = ui.add(
                                egui::TextEdit::singleline(&mut modal.artist_input)
                                    .hint_text("or paste Bandcamp URL / slug")
                                    .desired_width(280.0),
                            );
                            if text_resp.changed() {
                                modal.band_id = state
                                    .artists
                                    .iter()
                                    .find(|a| a.matches_input(&modal.artist_input))
                                    .map(|a| a.band_id);
                            }
                        });
                    }
                    ui.end_row();

                    ui.label("Folder:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut modal.folder_path)
                                .hint_text("/path/to/artist/folder")
                                .desired_width(240.0),
                        );
                        if paint::sharp_button(ui, "Browse...").clicked() {
                            let slot = modal.picked_folder.clone();
                            let start_dir = if !modal.folder_path.trim().is_empty() {
                                Some(std::path::PathBuf::from(modal.folder_path.trim()))
                            } else {
                                None
                            };
                            rt_handle.spawn(async move {
                                let mut dialog = rfd::AsyncFileDialog::new();
                                if let Some(ref d) = start_dir {
                                    dialog = dialog.set_directory(d);
                                }
                                if let Some(handle) = dialog.pick_folder().await
                                    && let Ok(mut l) = slot.lock()
                                {
                                    *l = Some(handle.path().to_path_buf());
                                }
                            });
                        }
                    });
                    ui.end_row();
                });

            ui.add_space(4.0);
            ui.checkbox(
                &mut modal.auto_apply_confident,
                "Adopt confident matches automatically",
            );

            if let Some(ref err) = modal.error {
                ui.add_space(4.0);
                ui.label(RichText::new(err).color(th.err));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if paint::sharp_button(ui, "Link Folder").clicked() {
                    let path_str = modal.folder_path.trim();
                    if path_str.is_empty() {
                        modal.error = Some("Folder path cannot be empty.".to_string());
                    } else {
                        let path = std::path::PathBuf::from(path_str);
                        if !path.exists() {
                            modal.error =
                                Some("Selected folder path does not exist on disk.".to_string());
                        } else if !path.is_dir() {
                            modal.error =
                                Some("Selected path is a file, not a directory.".to_string());
                        } else {
                            let target_band_id = modal.band_id.or_else(|| {
                                state
                                    .artists
                                    .iter()
                                    .find(|a| a.matches_input(&modal.artist_input))
                                    .map(|a| a.band_id)
                            });

                            if let Some(band_id) = target_band_id {
                                let _ = core_tx.send(UiToWorkerCommand::LinkArtistFolder {
                                    band_id,
                                    path,
                                    auto_apply_confident: modal.auto_apply_confident,
                                });
                                keep_open = false;
                            } else {
                                let input = modal.artist_input.trim();
                                if !input.is_empty()
                                    && let Ok(norm) =
                                        bcvault_core::bandcamp::input::normalize_artist_input(input)
                                {
                                    let _ = core_tx.send(
                                        UiToWorkerCommand::AddAndLinkArtistFolder {
                                            input: norm.base_url,
                                            path,
                                            auto_apply_confident: modal.auto_apply_confident,
                                        },
                                    );
                                    keep_open = false;
                                } else {
                                    modal.error = Some(
                                        "Please select a tracked artist or enter a valid Bandcamp artist URL / slug."
                                            .to_string(),
                                    );
                                }
                            }
                        }
                    }
                }
                if paint::sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.link_folder_modal = Some(modal);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnBudget {
    pub title_width: f32,
    pub cand_width: f32,
}

pub fn compute_balanced_columns(
    avail_w: f32,
    fixed_w: f32,
    title_max_chars: usize,
    cand_max_chars: usize,
) -> ColumnBudget {
    let rem_w = (avail_w - fixed_w).max(180.0);
    let est_title = (title_max_chars as f32 * 7.5 + 16.0).max(80.0);
    let est_cand = (cand_max_chars as f32 * 7.5 + 16.0).max(80.0);

    // Case 1: Both fit comfortably without truncation
    if est_title + est_cand <= rem_w {
        return ColumnBudget {
            title_width: est_title,
            cand_width: rem_w - est_title,
        };
    }

    // Baseline split: ~46% Title, ~54% Candidate (candidate includes artist + tags)
    let title_share = (rem_w * 0.46).round();
    let cand_share = rem_w - title_share;

    // Case 2: Title is short and does not need its full share
    if est_title < title_share {
        let title_w = est_title.max(80.0);
        return ColumnBudget {
            title_width: title_w,
            cand_width: rem_w - title_w,
        };
    }

    // Case 3: Candidate is short and does not need its full share
    if est_cand < cand_share {
        let cand_w = est_cand.max(80.0);
        return ColumnBudget {
            title_width: rem_w - cand_w,
            cand_width: cand_w,
        };
    }

    // Case 4: Both are long -> Fair middle-ground split
    ColumnBudget {
        title_width: title_share,
        cand_width: cand_share,
    }
}

fn truncated_cell(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    text: impl Into<egui::WidgetText>,
    tooltip: Option<String>,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_max_width(width);
            let mut resp = ui.add(egui::Label::new(text).truncate());
            if let Some(tip) = tooltip {
                resp = resp.on_hover_text(tip);
            }
            resp
        },
    )
    .inner
}

fn show_reconcile_report_modal(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut modal) = state.reconcile_report_modal.take() else {
        return;
    };
    let mut keep_open = !esc;
    let th = state.active_theme;

    let artist_name = state
        .artists
        .iter()
        .find(|a| a.band_id == modal.report.band_id)
        .map(|a| a.display_name().to_string())
        .unwrap_or_else(|| format!("Artist {}", modal.report.band_id));

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 48.0).clamp(480.0, 860.0);
    let max_h = (screen.height() - 48.0).clamp(360.0, 720.0);

    egui::Window::new(format!("Match Report - {artist_name}"))
        .collapsible(false)
        .resizable(true)
        .default_size([720.0, 520.0])
        .max_size([max_w, max_h])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_min_width(ui.available_width().min(600.0));

            let folder_path = state
                .folders
                .get(&modal.report.band_id)
                .map(|f| f.path.display().to_string())
                .unwrap_or_else(|| "Unknown folder".to_string());
            ui.label(
                RichText::new(format!(
                    "Batch #{}  ·  Linked: {}",
                    modal.report.batch_id, folder_path
                ))
                .size(11.5)
                .color(th.text_dim),
            );
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let tabs = [
                    (
                        FxReportTab::Matched,
                        format!(
                            "Matched ({})",
                            modal.report.matched.len() + modal.report.linked.len()
                        ),
                    ),
                    (
                        FxReportTab::Review,
                        format!("Review ({})", modal.report.review.len()),
                    ),
                    (
                        FxReportTab::PageOnly,
                        format!("On page only ({})", modal.report.page_only.len()),
                    ),
                    (
                        FxReportTab::LocalOnly,
                        format!("Local only ({})", modal.report.local_only.len()),
                    ),
                    (
                        FxReportTab::Conflicts,
                        format!("Conflicts ({})", modal.report.conflicts.len()),
                    ),
                    (
                        FxReportTab::Restorable,
                        format!("Found again ({})", modal.report.restorable.len()),
                    ),
                ];

                for (tab, label) in tabs {
                    let is_active = modal.active_tab == tab;
                    let text = if is_active {
                        RichText::new(&label).strong().color(th.accent)
                    } else {
                        RichText::new(&label).color(th.text)
                    };
                    if paint::sharp_button(ui, text).clicked() {
                        modal.active_tab = tab;
                    }
                }
            });

            ui.separator();

            egui::ScrollArea::vertical()
                .max_height((max_h - 160.0).max(200.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width());
                    match modal.active_tab {
                        FxReportTab::Matched => {
                            if !modal.report.matched.is_empty() {
                                ui.horizontal(|ui| {
                                    let all_selected = !modal.report.matched.is_empty()
                                        && modal.selected_matched.len() == modal.report.matched.len();
                                    if paint::sharp_button(
                                        ui,
                                        if all_selected {
                                            "Deselect All"
                                        } else {
                                            "Select All"
                                        },
                                    )
                                    .clicked()
                                    {
                                        if all_selected {
                                            modal.selected_matched.clear();
                                        } else {
                                            modal.selected_matched =
                                                modal.report.matched.iter().map(|m| m.release_key).collect();
                                        }
                                    }
                                    ui.label(
                                        RichText::new(format!(
                                            "{} of {} proposals selected for adoption",
                                            modal.selected_matched.len(),
                                            modal.report.matched.len()
                                        ))
                                        .color(th.text_dim),
                                    );
                                });
                                ui.add_space(4.0);

                                let avail_w = ui.available_width();
                                let title_max = modal
                                    .report
                                    .matched
                                    .iter()
                                    .map(|m| m.release_title.chars().count())
                                    .max()
                                    .unwrap_or(20);
                                let cand_max = modal
                                    .report
                                    .matched
                                    .iter()
                                    .map(|m| m.candidate_name.chars().count())
                                    .max()
                                    .unwrap_or(20);
                                let budget = compute_balanced_columns(avail_w, 140.0, title_max, cand_max);

                                egui::Grid::new("fx_matched_tab_grid")
                                    .num_columns(5)
                                    .spacing([8.0, 6.0])
                                    .show(ui, |ui| {
                                        for m in &modal.report.matched {
                                            let mut is_checked =
                                                modal.selected_matched.contains(&m.release_key);
                                            if ui.checkbox(&mut is_checked, "").changed() {
                                                if is_checked {
                                                    modal.selected_matched.insert(m.release_key);
                                                } else {
                                                    modal.selected_matched.remove(&m.release_key);
                                                }
                                            }

                                            let art_id = state
                                                .rows
                                                .iter()
                                                .find(|r| r.rel.key == m.release_key)
                                                .and_then(|r| r.art_id);
                                            let (cov_r, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                                            paint_cover_slot(
                                                ui.painter(),
                                                state,
                                                art_id,
                                                m.release_key.id,
                                                cov_r,
                                                Tier::T64,
                                                1.0,
                                                Vec2::ZERO,
                                            );

                                            truncated_cell(
                                                ui,
                                                budget.title_width,
                                                26.0,
                                                RichText::new(&m.release_title).strong(),
                                                Some(m.release_title.clone()),
                                            );

                                            let fmt_str = m
                                                .format_guess
                                                .map(|f| format!("[{}]", f.label()))
                                                .unwrap_or_default();
                                            let cand_disp = format!("{} {fmt_str}", m.candidate_name);
                                            truncated_cell(
                                                ui,
                                                budget.cand_width,
                                                26.0,
                                                RichText::new(cand_disp)
                                                    .color(th.text_dim)
                                                    .monospace(),
                                                Some(m.candidate_path.display().to_string()),
                                            );

                                            let tip = format!(
                                                "Score: {:.2}\nSignals:\n{}",
                                                m.score.value,
                                                format_signals(&m.score.signals)
                                            );
                                            ui.label(
                                                RichText::new(format!("{:.0}%", m.score.value * 100.0))
                                                    .color(th.accent)
                                                    .monospace(),
                                            )
                                            .on_hover_text(tip);

                                            ui.end_row();
                                        }
                                    });
                            } else if modal.report.linked.is_empty() {
                                ui.label(RichText::new("No matched releases.").color(th.text_dim));
                            }

                            if !modal.report.linked.is_empty() {
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(format!(
                                        "Already linked ({})",
                                        modal.report.linked.len()
                                    ))
                                    .strong()
                                    .color(th.text_dim),
                                );
                                ui.add_space(4.0);

                                let avail_w = ui.available_width();
                                let title_max = modal
                                    .report
                                    .linked
                                    .iter()
                                    .map(|l| l.release_title.chars().count())
                                    .max()
                                    .unwrap_or(20);
                                let cand_max = modal
                                    .report
                                    .linked
                                    .iter()
                                    .map(|l| {
                                        l.path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().chars().count())
                                            .unwrap_or(20)
                                    })
                                    .max()
                                    .unwrap_or(20);
                                let budget = compute_balanced_columns(avail_w, 50.0, title_max, cand_max);

                                egui::Grid::new("fx_linked_tab_grid")
                                    .num_columns(3)
                                    .spacing([8.0, 6.0])
                                    .show(ui, |ui| {
                                        for l in &modal.report.linked {
                                            let art_id = state
                                                .rows
                                                .iter()
                                                .find(|r| r.rel.key == l.release_key)
                                                .and_then(|r| r.art_id);
                                            let (cov_r, _) = ui.allocate_exact_size(
                                                Vec2::splat(26.0),
                                                egui::Sense::hover(),
                                            );
                                            paint_cover_slot(
                                                ui.painter(),
                                                state,
                                                art_id,
                                                l.release_key.id,
                                                cov_r,
                                                Tier::T64,
                                                1.0,
                                                Vec2::ZERO,
                                            );
                                            truncated_cell(
                                                ui,
                                                budget.title_width,
                                                26.0,
                                                RichText::new(&l.release_title).strong(),
                                                Some(l.release_title.clone()),
                                            );
                                            let name = l
                                                .path
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| l.path.display().to_string());
                                            truncated_cell(
                                                ui,
                                                budget.cand_width,
                                                26.0,
                                                RichText::new(name).color(th.text_dim).monospace(),
                                                Some(l.path.display().to_string()),
                                            );
                                            ui.end_row();
                                        }
                                    });
                            }
                        }
                    FxReportTab::Review => {
                        if modal.report.review.is_empty() {
                            ui.label(
                                RichText::new("No items requiring review.").color(th.text_dim),
                            );
                        } else {
                            for r in &modal.report.review {
                                egui::Frame::NONE
                                    .corner_radius(CornerRadius::ZERO)
                                    .fill(th.bg1)
                                    .stroke(Stroke::new(1.0_f32, th.border))
                                    .inner_margin(6.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            let art_id = state
                                                .rows
                                                .iter()
                                                .find(|row| row.rel.key == r.release_key)
                                                .and_then(|row| row.art_id);
                                            let (cov_r, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                                            paint_cover_slot(
                                                ui.painter(),
                                                state,
                                                art_id,
                                                r.release_key.id,
                                                cov_r,
                                                Tier::T64,
                                                1.0,
                                                Vec2::ZERO,
                                            );
                                            ui.add(egui::Label::new(RichText::new(&r.release_title).strong()).truncate())
                                                .on_hover_text(&r.release_title);
                                        });

                                        let chosen = modal.selected_review.get(&r.release_key).copied();

                                        for (c_idx, c) in r.candidates.iter().enumerate() {
                                            let is_chosen = chosen == Some(c_idx);
                                            let fmt_str = c
                                                .format_guess
                                                .map(|f| format!("[{}]", f.label()))
                                                .unwrap_or_default();
                                            let tip = format!(
                                                "Score: {:.2}\nSignals:\n{}",
                                                c.score.value,
                                                format_signals(&c.score.signals)
                                            );
                                            ui.horizontal(|ui| {
                                                if ui.radio(is_chosen, "").clicked() {
                                                    modal.selected_review.insert(r.release_key, c_idx);
                                                }
                                                let cand_w = (ui.available_width() - 65.0).max(100.0);
                                                truncated_cell(
                                                    ui,
                                                    cand_w,
                                                    20.0,
                                                    RichText::new(format!(
                                                        "{} {fmt_str}",
                                                        c.candidate_name
                                                    ))
                                                    .monospace(),
                                                    Some(c.candidate_path.display().to_string()),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "({:.0}%)",
                                                        c.score.value * 100.0
                                                    ))
                                                    .color(th.warn)
                                                    .size(11.0),
                                                )
                                                .on_hover_text(tip);
                                            });
                                        }

                                        let is_skipped = chosen.is_none();
                                        if ui.radio(is_skipped, "Skip for now").clicked() {
                                            modal.selected_review.remove(&r.release_key);
                                        }
                                    });
                                ui.add_space(4.0);
                            }
                        }
                    }
                    FxReportTab::PageOnly => {
                        if modal.report.page_only.is_empty() {
                            ui.label(
                                RichText::new("All releases have corresponding local files.")
                                    .color(th.text_dim),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "These releases exist on Bandcamp but have no local audio files in the linked folder:",
                                )
                                .color(th.text_dim),
                            );
                            ui.add_space(4.0);
                            for key in &modal.report.page_only {
                                let art_id = state
                                    .rows
                                    .iter()
                                    .find(|r| r.rel.key == *key)
                                    .and_then(|r| r.art_id);
                                let title = state
                                    .rows
                                    .iter()
                                    .find(|r| r.rel.key == *key)
                                    .map(|r| r.rel.title.clone())
                                    .unwrap_or_else(|| format!("Release #{}", key.id));
                                ui.horizontal(|ui| {
                                    let (cov_r, _) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::hover());
                                    paint_cover_slot(
                                        ui.painter(),
                                        state,
                                        art_id,
                                        key.id,
                                        cov_r,
                                        Tier::T64,
                                        1.0,
                                        Vec2::ZERO,
                                    );
                                    ui.add(egui::Label::new(RichText::new(&title).strong()).truncate())
                                        .on_hover_text(&title);
                                });
                            }
                        }
                    }
                    FxReportTab::LocalOnly => {
                        if modal.report.local_only.is_empty() {
                            ui.label(
                                RichText::new("No unassigned local files found.")
                                    .color(th.text_dim),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "These local audio folders could not be matched to any release on the Bandcamp page:",
                                )
                                .color(th.text_dim),
                            );
                            ui.add_space(4.0);

                            for orphan in &modal.report.local_only {
                                egui::Frame::NONE
                                    .corner_radius(CornerRadius::ZERO)
                                    .fill(th.bg1)
                                    .stroke(Stroke::new(1.0_f32, th.border))
                                    .inner_margin(6.0)
                                    .show(ui, |ui| {
                                        let fmt_str = orphan
                                            .format_guess
                                            .map(|f| format!("[{}]", f.label()))
                                            .unwrap_or_default();
                                        let name = orphan
                                            .path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| orphan.path.display().to_string());
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(format!(
                                                    "{name} {fmt_str}"
                                                ))
                                                .strong()
                                                .monospace(),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(orphan.path.display().to_string());

                                        let current_action = modal
                                            .local_decisions
                                            .get(&orphan.path)
                                            .cloned()
                                            .unwrap_or(FxLocalAction::Leave);

                                        ui.horizontal(|ui| {
                                            if ui
                                                .radio(
                                                    matches!(current_action, FxLocalAction::Leave),
                                                    "Leave unmatched",
                                                )
                                                .clicked()
                                            {
                                                modal
                                                    .local_decisions
                                                    .insert(orphan.path.clone(), FxLocalAction::Leave);
                                            }
                                            if ui
                                                .radio(
                                                    matches!(current_action, FxLocalAction::Ignore),
                                                    "Ignore forever",
                                                )
                                                .clicked()
                                            {
                                                modal
                                                    .local_decisions
                                                    .insert(orphan.path.clone(), FxLocalAction::Ignore);
                                            }
                                            let is_attach =
                                                matches!(current_action, FxLocalAction::Attach(_));
                                            if ui.radio(is_attach, "Attach to release...").clicked()
                                                && !is_attach
                                                && let Some(first_rel) = state
                                                    .rows
                                                    .iter()
                                                    .find(|r| r.rel.band_id == modal.report.band_id)
                                            {
                                                modal.local_decisions.insert(
                                                    orphan.path.clone(),
                                                    FxLocalAction::Attach(first_rel.rel.key),
                                                );
                                            }
                                        });

                                        if let FxLocalAction::Attach(ref current_key) = current_action {
                                            ui.horizontal(|ui| {
                                                ui.label("Attach to:");
                                                let current_title = state
                                                    .rows
                                                    .iter()
                                                    .find(|r| r.rel.key == *current_key)
                                                    .map(|r| r.rel.title.clone())
                                                    .unwrap_or_else(|| "Select release".to_string());
                                                egui::ComboBox::from_id_salt(format!(
                                                    "fx_attach_combo_{}",
                                                    orphan.path.display()
                                                ))
                                                .selected_text(&current_title)
                                                .width(280.0)
                                                .show_ui(ui, |ui| {
                                                    for r in state
                                                        .rows
                                                        .iter()
                                                        .filter(|r| r.rel.band_id == modal.report.band_id)
                                                    {
                                                        if ui
                                                            .selectable_label(
                                                                r.rel.key == *current_key,
                                                                &r.rel.title,
                                                            )
                                                            .clicked()
                                                        {
                                                            modal.local_decisions.insert(
                                                                orphan.path.clone(),
                                                                FxLocalAction::Attach(r.rel.key),
                                                            );
                                                        }
                                                    }
                                                });
                                            });
                                        }
                                    });
                                ui.add_space(4.0);
                            }
                        }
                    }
                    FxReportTab::Conflicts => {
                        if modal.report.conflicts.is_empty() {
                            ui.label(
                                RichText::new("No conflicts or multiple matches found.")
                                    .color(th.text_dim),
                            );
                        } else {
                            for c in &modal.report.conflicts {
                                egui::Frame::NONE
                                    .corner_radius(CornerRadius::ZERO)
                                    .fill(th.bg1)
                                    .stroke(Stroke::new(1.0_f32, th.border))
                                    .inner_margin(6.0)
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&c.description)
                                                    .strong()
                                                    .color(th.warn),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(&c.description);

                                        if !c.candidate_paths.is_empty() {
                                            ui.label(
                                                RichText::new("Paths involved:")
                                                    .color(th.text_dim)
                                                    .size(11.0),
                                            );
                                            for p in &c.candidate_paths {
                                                let name = p
                                                    .file_name()
                                                    .map(|n| n.to_string_lossy().to_string())
                                                    .unwrap_or_else(|| p.display().to_string());
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(format!("  - {name}"))
                                                            .monospace()
                                                            .size(11.0),
                                                    )
                                                    .truncate(),
                                                )
                                                .on_hover_text(p.display().to_string());
                                            }
                                        }
                                    });
                                ui.add_space(4.0);
                            }
                        }
                    }
                    FxReportTab::Restorable => {
                        if modal.report.restorable.is_empty() {
                            ui.label(
                                RichText::new("No previously archived releases found on disk.")
                                    .color(th.text_dim),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                let all_selected = !modal.report.restorable.is_empty()
                                    && modal.selected_restorable.len()
                                        == modal.report.restorable.len();
                                if paint::sharp_button(
                                    ui,
                                    if all_selected {
                                        "Deselect All"
                                    } else {
                                        "Select All"
                                    },
                                )
                                .clicked()
                                {
                                    if all_selected {
                                        modal.selected_restorable.clear();
                                    } else {
                                        modal.selected_restorable = modal
                                            .report
                                            .restorable
                                            .iter()
                                            .map(|m| m.release_key)
                                            .collect();
                                    }
                                }
                                ui.label(
                                    RichText::new(
                                        "These releases were previously archived, but their audio files are now available again.",
                                    )
                                    .color(th.text_dim),
                                );
                            });
                            ui.add_space(4.0);

                            let avail_w = ui.available_width();
                            let title_max = modal
                                .report
                                .restorable
                                .iter()
                                .map(|m| m.release_title.chars().count())
                                .max()
                                .unwrap_or(20);
                            let cand_max = modal
                                .report
                                .restorable
                                .iter()
                                .map(|m| m.candidate_name.chars().count())
                                .max()
                                .unwrap_or(20);
                            let budget = compute_balanced_columns(avail_w, 80.0, title_max, cand_max);

                            egui::Grid::new("fx_restorable_tab_grid")
                                .num_columns(4)
                                .spacing([8.0, 6.0])
                                .show(ui, |ui| {
                                    for m in &modal.report.restorable {
                                        let mut is_checked =
                                            modal.selected_restorable.contains(&m.release_key);
                                        if ui.checkbox(&mut is_checked, "").changed() {
                                            if is_checked {
                                                modal.selected_restorable.insert(m.release_key);
                                            } else {
                                                modal.selected_restorable.remove(&m.release_key);
                                            }
                                        }
                                        let art_id = state
                                            .rows
                                            .iter()
                                            .find(|r| r.rel.key == m.release_key)
                                            .and_then(|r| r.art_id);
                                        let (cov_r, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                                        paint_cover_slot(
                                            ui.painter(),
                                            state,
                                            art_id,
                                            m.release_key.id,
                                            cov_r,
                                            Tier::T64,
                                            1.0,
                                            Vec2::ZERO,
                                        );
                                        truncated_cell(
                                            ui,
                                            budget.title_width,
                                            26.0,
                                            RichText::new(&m.release_title).strong(),
                                            Some(m.release_title.clone()),
                                        );
                                        let fmt_str = m
                                            .format_guess
                                            .map(|f| format!("[{}]", f.label()))
                                            .unwrap_or_default();
                                        let cand_disp = format!("{} {fmt_str}", m.candidate_name);
                                        truncated_cell(
                                            ui,
                                            budget.cand_width,
                                            26.0,
                                            RichText::new(cand_disp)
                                                .color(th.text_dim)
                                                .monospace(),
                                            Some(m.candidate_path.display().to_string()),
                                        );
                                        ui.end_row();
                                    }
                                });
                        }
                    }
                }
            });

            ui.separator();

            let selected_total = modal.total_selected_count();
            ui.horizontal(|ui| {
                let apply_label = format!("Apply selected ({selected_total})");
                if ui
                    .add_enabled(
                        selected_total > 0,
                        egui::Button::new(apply_label).corner_radius(CornerRadius::ZERO),
                    )
                    .clicked()
                {
                    let mut decisions = Vec::new();

                    for m in &modal.report.matched {
                        if modal.selected_matched.contains(&m.release_key) {
                            decisions.push(MatchDecision::Adopt {
                                key: m.release_key,
                                path: m.candidate_path.clone(),
                                format: m.format_guess,
                                folder_mtime: None,
                            });
                        }
                    }

                    for r in &modal.report.review {
                        if let Some(cand) = modal
                            .selected_review
                            .get(&r.release_key)
                            .and_then(|&cand_idx| r.candidates.get(cand_idx))
                        {
                            decisions.push(MatchDecision::Adopt {
                                key: r.release_key,
                                path: cand.candidate_path.clone(),
                                format: cand.format_guess,
                                folder_mtime: None,
                            });
                        }
                    }

                    for m in &modal.report.restorable {
                        if modal.selected_restorable.contains(&m.release_key) {
                            decisions.push(MatchDecision::Restore {
                                key: m.release_key,
                                path: m.candidate_path.clone(),
                            });
                        }
                    }

                    for (path, action) in &modal.local_decisions {
                        match action {
                            FxLocalAction::Ignore => {
                                decisions.push(MatchDecision::Ignore {
                                    band_id: modal.report.band_id,
                                    path: path.clone(),
                                });
                            }
                            FxLocalAction::Attach(key) => {
                                let format_guess = modal
                                    .report
                                    .local_only
                                    .iter()
                                    .find(|o| &o.path == path)
                                    .and_then(|o| o.format_guess);
                                decisions.push(MatchDecision::Attach {
                                    key: *key,
                                    path: path.clone(),
                                    format: format_guess,
                                    folder_mtime: None,
                                });
                            }
                            FxLocalAction::Leave => {}
                        }
                    }

                    let _ = core_tx.send(UiToWorkerCommand::ApplyReconcile {
                        batch_id: modal.report.batch_id,
                        decisions,
                    });
                    keep_open = false;
                }

                if paint::sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.reconcile_report_modal = Some(modal);
    }
}

fn show_resolve_missing_modal(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
    esc: bool,
) {
    let Some(mut modal) = state.resolve_missing_modal.take() else {
        return;
    };

    if esc {
        state.show_resolve_missing_window = false;
        state.resolve_missing_modal = Some(modal);
        return;
    }

    if let Some((key, path)) = modal
        .picked_folder
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        modal
            .decisions
            .insert(key, FxMissingItemChoice::Moved(path));
    }
    if let Some((band_id, new_path)) = modal
        .repoint_picked_folder
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        let _ = core_tx.send(UiToWorkerCommand::RepointArtistFolder { band_id, new_path });
        state.show_resolve_missing_window = false;
        state.resolve_missing_modal = None;
        return;
    }

    if !state.show_resolve_missing_window {
        state.resolve_missing_modal = Some(modal);
        return;
    }

    let mut open = state.show_resolve_missing_window;
    let mut close_requested = false;
    let th = state.active_theme;

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 48.0).clamp(480.0, 860.0);
    let max_h = (screen.height() - 48.0).clamp(360.0, 720.0);

    egui::Window::new("Resolve Missing Files")
        .collapsible(true)
        .resizable(true)
        .default_size([720.0, 500.0])
        .max_size([max_w, max_h])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_min_width(ui.available_width().min(600.0));

            if modal.mass_missing {
                egui::Frame::NONE
                    .corner_radius(CornerRadius::ZERO)
                    .fill(th.warn.gamma_multiply(0.12))
                    .stroke(Stroke::new(1.0_f32, th.warn))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Many albums are missing: is a drive disconnected?")
                                    .color(th.warn)
                                    .strong(),
                            );
                        });
                        ui.label(
                            RichText::new(
                                "Automatic policies were halted to prevent unintended deletions. Please verify that your storage drive is mounted and accessible.",
                            )
                            .size(11.0)
                            .color(th.text_dim),
                        );
                    });
                ui.add_space(8.0);
            }

            ui.horizontal(|ui| {
                if paint::sharp_button(ui, "Mark All as Deleted").clicked() {
                    for it in &modal.items {
                        modal.decisions.insert(it.key, FxMissingItemChoice::Deleted);
                    }
                }
                if paint::sharp_button(ui, "Skip All").clicked() {
                    for it in &modal.items {
                        modal.decisions.insert(it.key, FxMissingItemChoice::Skip);
                    }
                }

                ui.separator();

                let mut unique_artists: Vec<i64> = modal
                    .items
                    .iter()
                    .map(|it| it.band_id)
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                unique_artists.sort_unstable();

                if unique_artists.len() == 1 {
                    let b_id = unique_artists[0];
                    let artist_label = modal
                        .items
                        .first()
                        .map(|it| it.artist_name.as_str())
                        .unwrap_or("Artist");
                    if paint::sharp_button(
                        ui,
                        format!("The artist's folder moved ({artist_label})..."),
                    )
                    .clicked()
                    {
                        let picked = modal.repoint_picked_folder.clone();
                        rt_handle.spawn(async move {
                            if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await
                                && let Ok(mut lock) = picked.lock()
                            {
                                *lock = Some((b_id, folder.path().to_path_buf()));
                            }
                        });
                    }
                } else if !unique_artists.is_empty() {
                    ui.label(
                        RichText::new("The artist's folder moved:")
                            .size(11.0)
                            .color(th.text_dim),
                    );
                    let curr_band = modal
                        .repoint_selected_band_id
                        .unwrap_or(unique_artists[0]);
                    let curr_name = modal
                        .items
                        .iter()
                        .find(|it| it.band_id == curr_band)
                        .map(|it| it.artist_name.as_str())
                        .unwrap_or("Artist");
                    egui::ComboBox::from_id_salt("fx_repoint_artist_combo")
                        .selected_text(curr_name)
                        .show_ui(ui, |ui| {
                            for b_id in &unique_artists {
                                let name = modal
                                    .items
                                    .iter()
                                    .find(|it| it.band_id == *b_id)
                                    .map(|it| it.artist_name.as_str())
                                    .unwrap_or("Artist");
                                ui.selectable_value(
                                    &mut modal.repoint_selected_band_id,
                                    Some(*b_id),
                                    name,
                                );
                            }
                        });
                    if paint::sharp_button(ui, "Browse New Folder...").clicked() {
                        let target_band = modal
                            .repoint_selected_band_id
                            .unwrap_or(unique_artists[0]);
                        let picked = modal.repoint_picked_folder.clone();
                        rt_handle.spawn(async move {
                            if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await
                                && let Ok(mut lock) = picked.lock()
                            {
                                *lock = Some((target_band, folder.path().to_path_buf()));
                            }
                        });
                    }
                }
            });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            egui::ScrollArea::vertical()
                .max_height(280.0)
                .show(ui, |ui| {
                    for item in &modal.items {
                        egui::Frame::NONE
                            .corner_radius(CornerRadius::ZERO)
                            .fill(th.bg1)
                            .stroke(Stroke::new(1.0_f32, th.border))
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let art_id = state
                                        .rows
                                        .iter()
                                        .find(|r| r.rel.key == item.key)
                                        .and_then(|r| r.art_id);
                                    let (cov_r, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                                    paint_cover_slot(
                                        ui.painter(),
                                        state,
                                        art_id,
                                        item.key.id,
                                        cov_r,
                                        Tier::T64,
                                        1.0,
                                        Vec2::ZERO,
                                    );
                                    let label_text = format!("{} - {}", item.artist_name, item.title);
                                    ui.add(egui::Label::new(RichText::new(&label_text).strong()).truncate())
                                        .on_hover_text(&label_text);
                                });
                                if let Some(last) = &item.last_known_path {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "Last known: {}",
                                                last.display()
                                            ))
                                            .size(11.0)
                                            .color(th.text_dim),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(last.display().to_string());
                                }
                                if let Some(cand) = &item.best_candidate {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "Candidate in artist folder ({:.0}%): {}",
                                                cand.score * 100.0,
                                                cand.path.display()
                                            ))
                                            .color(th.accent)
                                            .size(11.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(cand.path.display().to_string());
                                }

                                let current_decision = modal
                                    .decisions
                                    .get(&item.key)
                                    .cloned()
                                    .unwrap_or(FxMissingItemChoice::Skip);
                                ui.horizontal(|ui| {
                                    let is_moved =
                                        matches!(current_decision, FxMissingItemChoice::Moved(_));
                                    if ui.radio(is_moved, "Moved").clicked() && !is_moved {
                                        if let Some(cand) = &item.best_candidate {
                                            modal.decisions.insert(
                                                item.key,
                                                FxMissingItemChoice::Moved(cand.path.clone()),
                                            );
                                        } else {
                                            let picked = modal.picked_folder.clone();
                                            let key = item.key;
                                            rt_handle.spawn(async move {
                                                if let Some(folder) =
                                                    rfd::AsyncFileDialog::new().pick_folder().await
                                                    && let Ok(mut lock) = picked.lock()
                                                {
                                                    *lock =
                                                        Some((key, folder.path().to_path_buf()));
                                                }
                                            });
                                        }
                                    }

                                    if is_moved {
                                        let path_str = match &current_decision {
                                            FxMissingItemChoice::Moved(p) => p.display().to_string(),
                                            _ => String::new(),
                                        };
                                        ui.label(
                                            RichText::new(path_str)
                                                .size(11.0)
                                                .color(th.text_dim),
                                        );
                                        if paint::sharp_button(ui, "Browse...").clicked() {
                                            let picked = modal.picked_folder.clone();
                                            let key = item.key;
                                            rt_handle.spawn(async move {
                                                if let Some(folder) =
                                                    rfd::AsyncFileDialog::new().pick_folder().await
                                                    && let Ok(mut lock) = picked.lock()
                                                {
                                                    *lock =
                                                        Some((key, folder.path().to_path_buf()));
                                                }
                                            });
                                        }
                                    }

                                    let is_deleted =
                                        matches!(current_decision, FxMissingItemChoice::Deleted);
                                    let del_label = match modal.deleted_policy {
                                        DeletedPolicy::KeepMarked => {
                                            "Deleted (Keep marked / Archive)"
                                        }
                                        DeletedPolicy::MarkNotDownloaded => "Deleted (Unmark)",
                                    };
                                    if ui.radio(is_deleted, del_label).clicked() {
                                        modal
                                            .decisions
                                            .insert(item.key, FxMissingItemChoice::Deleted);
                                    }

                                    let is_skip = matches!(current_decision, FxMissingItemChoice::Skip);
                                    if ui.radio(is_skip, "Skip for now").clicked() {
                                        modal.decisions.insert(item.key, FxMissingItemChoice::Skip);
                                    }
                                });
                            });
                        ui.add_space(4.0);
                    }
                });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("When deleted:").size(12.0));
                let curr_text = match modal.deleted_policy {
                    DeletedPolicy::KeepMarked => "Keep marked as downloaded (archived)",
                    DeletedPolicy::MarkNotDownloaded => "Mark as not downloaded",
                };
                egui::ComboBox::from_id_salt("fx_resolve_deleted_policy_combo")
                    .selected_text(curr_text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut modal.deleted_policy,
                            DeletedPolicy::KeepMarked,
                            "Keep marked as downloaded (archived)",
                        );
                        ui.selectable_value(
                            &mut modal.deleted_policy,
                            DeletedPolicy::MarkNotDownloaded,
                            "Mark as not downloaded",
                        );
                    });
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut modal.make_default, "Make this my default");
                ui.checkbox(
                    &mut modal.stop_asking,
                    "Stop asking (treat missing as deleted automatically)",
                );
            });

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let apply_label = format!("Apply Decisions ({})", modal.items.len());
                if paint::sharp_button(ui, &apply_label).clicked() {
                    let mut decisions = Vec::new();
                    for item in &modal.items {
                        let choice = modal
                            .decisions
                            .get(&item.key)
                            .cloned()
                            .unwrap_or(FxMissingItemChoice::Skip);
                        let action = match choice {
                            FxMissingItemChoice::Moved(p) => MissingAction::Moved(p),
                            FxMissingItemChoice::Deleted => {
                                MissingAction::Deleted(modal.deleted_policy)
                            }
                            FxMissingItemChoice::Skip => MissingAction::Skip,
                        };
                        decisions.push(MissingDecision {
                            key: item.key,
                            action,
                        });
                    }

                    let remember = if modal.make_default || modal.stop_asking {
                        Some(RememberChoice {
                            set_default_deleted_policy: if modal.make_default {
                                Some(modal.deleted_policy)
                            } else {
                                None
                            },
                            stop_asking: modal.stop_asking,
                        })
                    } else {
                        None
                    };

                    let _ = core_tx.send(UiToWorkerCommand::ResolveMissing {
                        batch_id: modal.batch_id,
                        decisions,
                        remember,
                    });

                    if modal.make_default {
                        state.settings.deleted_policy = modal.deleted_policy;
                    }
                    if modal.stop_asking {
                        state.settings.on_missing = OnMissing::Auto;
                    }

                    close_requested = true;
                }

                if paint::sharp_button(ui, "Close").clicked() {
                    close_requested = true;
                }
            });
        });

    if close_requested {
        open = false;
    }
    state.show_resolve_missing_window = open;
    if open {
        state.resolve_missing_modal = Some(modal);
    }
}

fn show_unavailable_modal(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
    esc: bool,
) {
    let Some(mut modal) = state.unavailable_modal.take() else {
        return;
    };

    if let Some((folder_id, new_path)) = modal
        .picked_folder
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        let band_id = state
            .folders
            .values()
            .find(|f| f.id == folder_id)
            .map(|f| f.band_id)
            .unwrap_or(folder_id);
        let _ = core_tx.send(UiToWorkerCommand::RepointArtistFolder { band_id, new_path });
        modal.cards.retain(|c| match c.folder.root {
            UnavailableRoot::Linked { folder_id: fid } => fid != folder_id,
            UnavailableRoot::OutDir => true,
        });
        if modal.cards.is_empty() {
            state.show_unavailable_window = false;
            state.unavailable_modal = None;
            return;
        }
    }

    if esc {
        let mut decisions = Vec::new();
        for card in &modal.cards {
            decisions.push(UnavailableDecision {
                root: card.folder.root,
                keys: card.folder.releases.iter().map(|r| r.key).collect(),
                action: UnavailableAction::Keep,
            });
        }
        let _ = core_tx.send(UiToWorkerCommand::ResolveUnavailable {
            batch_id: modal.batch_id,
            decisions,
        });
        state.show_unavailable_window = false;
        state.unavailable_modal = None;
        return;
    }

    if !state.show_unavailable_window {
        state.unavailable_modal = Some(modal);
        return;
    }

    let mut open = state.show_unavailable_window;
    let mut close_requested = false;
    let mut decisions_to_send = Vec::new();
    let mut cards_to_remove = Vec::new();
    let th = state.active_theme;

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 48.0).clamp(480.0, 860.0);
    let max_h = (screen.height() - 48.0).clamp(360.0, 720.0);

    egui::Window::new("Resolve Unavailable Folder")
        .collapsible(true)
        .resizable(true)
        .default_size([700.0, 480.0])
        .max_size([max_w, max_h])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_min_width(ui.available_width().min(600.0));

            egui::ScrollArea::vertical()
                .max_height(max_h - 110.0)
                .show(ui, |ui| {
                    for (idx, card) in modal.cards.iter_mut().enumerate() {
                        let title = match card.folder.root {
                            UnavailableRoot::Linked { folder_id } => {
                                let folder = state.folders.values().find(|f| f.id == folder_id);
                                let artist_name = folder
                                    .and_then(|f| state.artists.iter().find(|a| a.band_id == f.band_id))
                                    .map(|a| a.display_name())
                                    .or_else(|| {
                                        card.folder
                                            .releases
                                            .first()
                                            .map(|r| r.artist_name.as_str())
                                    })
                                    .unwrap_or("Artist");
                                format!("{artist_name}: linked folder unavailable")
                            }
                            UnavailableRoot::OutDir => "Download folder unavailable".to_string(),
                        };

                        let state_text = match card.folder.state {
                            FolderState::Missing => "This folder no longer exists.",
                            FolderState::Unreadable => "This folder or its drive can't be read.",
                            FolderState::Ok => "This folder is reachable.",
                        };
                        let count = card.folder.releases.len();

                        egui::Frame::NONE
                            .corner_radius(CornerRadius::ZERO)
                            .fill(th.bg1)
                            .stroke(Stroke::new(1.0_f32, th.border))
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.label(RichText::new(&title).strong().color(th.warn));
                                ui.label(
                                    RichText::new(state_text)
                                        .size(11.5)
                                        .color(th.text_dim),
                                );
                                ui.label(
                                    RichText::new(format!("Path: {}", card.folder.path.display()))
                                        .size(11.0)
                                        .monospace()
                                        .color(th.text_dim),
                                );
                                let rel_summary = if count == 0 {
                                    "No active downloads are located in this folder. You can unlink this folder or repoint it to a new location.".to_string()
                                } else {
                                    format!("{count} downloaded releases were last seen here. Their files can't be checked.")
                                };
                                ui.label(RichText::new(rel_summary).size(11.5).color(th.text_dim));
                                ui.add_space(6.0);

                                // Choice 1: Drive disconnected
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("The drive is disconnected, I'll reconnect it:").size(12.0));
                                    if paint::sharp_small_button(ui, "Keep as is").clicked() {
                                        decisions_to_send.push(UnavailableDecision {
                                            root: card.folder.root,
                                            keys: card.folder.releases.iter().map(|r| r.key).collect(),
                                            action: UnavailableAction::Keep,
                                        });
                                        cards_to_remove.push(idx);
                                    }
                                });

                                // Choice 2: Moved folder (Linked only)
                                if let UnavailableRoot::Linked { folder_id } = card.folder.root {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("I moved the folder:").size(12.0));
                                        if paint::sharp_small_button(ui, "Choose new folder...").clicked() {
                                            let picked = modal.picked_folder.clone();
                                            rt_handle.spawn(async move {
                                                if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await
                                                    && let Ok(mut lock) = picked.lock()
                                                {
                                                    *lock = Some((folder_id, folder.path().to_path_buf()));
                                                }
                                            });
                                        }
                                    });
                                }

                                // Choice 3: Deleted on purpose
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("I deleted it on purpose:").size(12.0));
                                });
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Then:").size(11.5));
                                    let curr_text = match card.policy {
                                        DeletedPolicy::KeepMarked => "Keep marked as downloaded (archived)",
                                        DeletedPolicy::MarkNotDownloaded => "Mark as not downloaded",
                                    };
                                    egui::ComboBox::from_id_salt(format!("fx_card_del_policy_{idx}"))
                                        .selected_text(curr_text)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut card.policy,
                                                DeletedPolicy::KeepMarked,
                                                "Keep marked as downloaded (archived)",
                                            );
                                            ui.selectable_value(
                                                &mut card.policy,
                                                DeletedPolicy::MarkNotDownloaded,
                                                "Mark as not downloaded",
                                            );
                                        });
                                    if matches!(card.folder.root, UnavailableRoot::Linked { .. }) {
                                        ui.checkbox(&mut card.unlink, "Also unlink this folder")
                                            .on_hover_text("Unlinking removes the link to this folder (marks and downloaded status are kept according to policy above; unlinking cannot be undone with Undo)");
                                    }
                                    if paint::sharp_small_button(ui, "Apply").clicked() {
                                        decisions_to_send.push(UnavailableDecision {
                                            root: card.folder.root,
                                            keys: card.folder.releases.iter().map(|r| r.key).collect(),
                                            action: UnavailableAction::Deleted {
                                                policy: card.policy,
                                                unlink: card.unlink,
                                            },
                                        });
                                        cards_to_remove.push(idx);
                                    }
                                });

                                // Expandable list of releases (only if there are releases to show)
                                if count > 0 {
                                    ui.add_space(4.0);
                                    let toggle_label = if card.expanded {
                                        format!("Hide releases ({count})")
                                    } else {
                                        format!("Show releases ({count})")
                                    };
                                    if ui.small_button(toggle_label).clicked() {
                                        card.expanded = !card.expanded;
                                    }
                                    if card.expanded {
                                        egui::ScrollArea::vertical()
                                            .max_height(120.0)
                                            .id_salt(format!("fx_card_rels_scroll_{idx}"))
                                            .show(ui, |ui| {
                                                for rel in &card.folder.releases {
                                                    let path_display = rel
                                                        .last_known_path
                                                        .as_ref()
                                                        .map(|p| p.display().to_string())
                                                        .unwrap_or_else(|| "Unknown path".to_string());
                                                    ui.horizontal(|ui| {
                                                        ui.label(RichText::new(&rel.title).size(11.0).color(th.text));
                                                        ui.label(
                                                            RichText::new(format!("({path_display})"))
                                                                .size(10.5)
                                                                .color(th.text_dim),
                                                        );
                                                    });
                                                }
                                            });
                                    }
                                }
                            });
                        ui.add_space(8.0);
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                if paint::sharp_button(ui, "Close").clicked() {
                    close_requested = true;
                }
            });
        });

    if !decisions_to_send.is_empty() {
        let _ = core_tx.send(UiToWorkerCommand::ResolveUnavailable {
            batch_id: modal.batch_id,
            decisions: decisions_to_send,
        });
    }

    for idx in cards_to_remove.into_iter().rev() {
        if idx < modal.cards.len() {
            modal.cards.remove(idx);
        }
    }

    if close_requested {
        open = false;
    }

    if !open && !modal.cards.is_empty() {
        let mut remaining_decisions = Vec::new();
        for card in &modal.cards {
            remaining_decisions.push(UnavailableDecision {
                root: card.folder.root,
                keys: card.folder.releases.iter().map(|r| r.key).collect(),
                action: UnavailableAction::Keep,
            });
        }
        let _ = core_tx.send(UiToWorkerCommand::ResolveUnavailable {
            batch_id: modal.batch_id,
            decisions: remaining_decisions,
        });
        state.show_unavailable_window = false;
        state.unavailable_modal = None;
    } else if modal.cards.is_empty() {
        state.show_unavailable_window = false;
        state.unavailable_modal = None;
    } else {
        state.show_unavailable_window = open;
        state.unavailable_modal = Some(modal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_balanced_columns_both_fit() {
        let budget = compute_balanced_columns(700.0, 100.0, 10, 15);
        assert!(budget.title_width + budget.cand_width <= 600.0);
        // Both fit without truncation
        assert_eq!(budget.title_width, 91.0);
        assert_eq!(budget.cand_width, 509.0);
    }

    #[test]
    fn test_compute_balanced_columns_both_long_fair_split() {
        let budget = compute_balanced_columns(700.0, 100.0, 200, 200);
        assert!(budget.title_width + budget.cand_width <= 600.0);
        // Middle ground split: ~46% / 54%
        assert_eq!(budget.title_width, 276.0);
        assert_eq!(budget.cand_width, 324.0);
    }

    #[test]
    fn test_compute_balanced_columns_short_title_lends_space() {
        let budget = compute_balanced_columns(700.0, 100.0, 5, 200);
        assert!(budget.title_width + budget.cand_width <= 600.0);
        // Title gets min 80.0, candidate gets 520.0
        assert_eq!(budget.title_width, 80.0);
        assert_eq!(budget.cand_width, 520.0);
    }

    #[test]
    fn test_compute_balanced_columns_short_cand_lends_space() {
        let budget = compute_balanced_columns(700.0, 100.0, 200, 5);
        assert!(budget.title_width + budget.cand_width <= 600.0);
        // Candidate gets min 80.0, title gets 520.0
        assert_eq!(budget.cand_width, 80.0);
        assert_eq!(budget.title_width, 520.0);
    }
}
