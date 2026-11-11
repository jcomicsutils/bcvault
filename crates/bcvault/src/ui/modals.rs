use crate::{
    bandcamp::redeem::validate_redeem_url,
    types::{
        DeletedPolicy, DownloadOpts, FolderState, FormatFallback, MatchDecision, MissingAction,
        MissingDecision, OnMissing, OutputLayout, RememberChoice, UnavailableAction,
        UnavailableDecision, UnavailableRoot,
    },
    ui::{
        state::{LocalAction, MissingItemChoice, ReportTab, UiState},
        theme::pal,
        widgets::{paint_folder_status_dot, sharp_button, sharp_rect, sharp_small_button},
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align2, Color32, Context, CornerRadius, Id, Order, RichText, Sense, Stroke};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
) {
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    show_undo_toast(ctx, state, cmd_tx);

    if state.show_resolve_missing_window && state.resolve_missing_modal.is_some() {
        show_resolve_missing_modal(ctx, state, cmd_tx, rt_handle, esc);
    }
    if state.show_unavailable_window && state.unavailable_modal.is_some() {
        show_unavailable_modal(ctx, state, cmd_tx, rt_handle, esc);
    }

    let any_modal = state.redeem_modal.is_some()
        || state.quota_modal.is_some()
        || state.settings_draft.is_some()
        || state.confirm_remove_artist.is_some()
        || state.rename_artist_modal.is_some()
        || state.link_folder_modal.is_some()
        || state.reconcile_report_modal.is_some();

    if !any_modal {
        return;
    }

    // Dimmed backdrop
    let screen = ctx.content_rect();
    egui::Area::new(Id::new("modal_backdrop"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click());
            sharp_rect(
                ui.painter(),
                rect,
                Color32::from_black_alpha(160),
                Stroke::NONE,
            );
        });

    if state.redeem_modal.is_some() {
        show_redeem_modal(ctx, state, cmd_tx, esc);
    }
    if state.quota_modal.is_some() {
        show_quota_modal(ctx, state, cmd_tx, esc);
    }
    if state.settings_draft.is_some() {
        show_settings_modal(ctx, state, cmd_tx, esc);
    }
    if state.confirm_remove_artist.is_some() {
        show_confirm_remove_modal(ctx, state, cmd_tx, esc);
    }
    if state.rename_artist_modal.is_some() {
        show_rename_artist_modal(ctx, state, cmd_tx, esc);
    }
    if state.link_folder_modal.is_some() {
        show_link_folder_modal(ctx, state, cmd_tx, rt_handle, esc);
    }
    if state.reconcile_report_modal.is_some() {
        show_reconcile_report_modal(ctx, state, cmd_tx, esc);
    }
}

fn show_redeem_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut modal) = state.redeem_modal.take() else {
        return;
    };
    let mut keep_open = !esc;

    egui::Window::new("Redeem Bandcamp Download Link")
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label(RichText::new(&modal.title).strong());
            ui.label(
                RichText::new(
                    "Paste the https://*.bandcamp.com/download?... link received by email:",
                )
                .color(pal::TEXT_DIM),
            );
            ui.add(
                egui::TextEdit::singleline(&mut modal.url_input)
                    .hint_text("https://bandcamp.com/download?...")
                    .desired_width(f32::INFINITY),
            );
            if let Some(ref err) = modal.error {
                ui.label(RichText::new(err).color(pal::ERR));
            }
            ui.separator();
            ui.horizontal(|ui| {
                if sharp_button(ui, "Redeem & Download").clicked() {
                    match validate_redeem_url(&modal.url_input, false) {
                        Ok(valid_url) => {
                            let opts = DownloadOpts {
                                format: state.settings.format,
                                out_dir: state.settings.out_dir.clone(),
                                batch_limit: state.settings.batch_limit,
                            };
                            let _ = cmd_tx.send(UiToWorkerCommand::RedeemUrl {
                                target: modal.target,
                                url: valid_url.to_string(),
                                opts,
                            });
                            keep_open = false;
                        }
                        Err(e) => {
                            modal.error = Some(e.to_string());
                        }
                    }
                }
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.redeem_modal = Some(modal);
    }
}

fn show_quota_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(modal) = state.quota_modal.take() else {
        return;
    };
    let mut keep_open = !esc;

    egui::Window::new("Artist Download Quota Warning")
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.label(
                RichText::new(format!(
                    "Download {} more release(s) for this artist?",
                    modal.requested
                ))
                .strong()
                .color(pal::WARN),
            );
            ui.label(format!(
                "Your downloads (30d): {} used / {} soft warning / {} hard cap",
                modal.used_30d, modal.soft, modal.hard
            ));
            ui.label(
                RichText::new(
                    "Note: Bandcamp's monthly free-download allowance is shared across all fans.",
                )
                .color(pal::TEXT_DIM),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if sharp_button(ui, "Proceed").clicked() {
                    let _ = cmd_tx.send(UiToWorkerCommand::EnqueueDownloads {
                        items: modal.items.clone(),
                        opts: modal.opts.clone(),
                        ack_soft_quota: true,
                    });
                    keep_open = false;
                }
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.quota_modal = Some(modal);
    }
}

fn show_settings_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut draft) = state.settings_draft.take() else {
        return;
    };
    let mut keep_open = !esc;

    egui::Window::new("Settings")
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(520.0);
            egui::ScrollArea::vertical()
                .max_height(480.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Downloads & Storage").strong());
                    egui::Grid::new("settings_grid")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("Output Layout:");
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut draft.layout, OutputLayout::Flat, "Flat");
                                ui.selectable_value(
                                    &mut draft.layout,
                                    OutputLayout::ByArtist,
                                    "By Release Artist",
                                );
                                ui.selectable_value(
                                    &mut draft.layout,
                                    OutputLayout::ByPageArtist,
                                    "By Page Artist",
                                );
                            });
                            ui.end_row();

                            ui.label("Filename Pattern:")
                                .on_hover_text("Tags: {artist}, {page_artist}, {album} / {title}, {year}, {date}, {format}, {type}, {id}\nLeave blank to use default / Bandcamp server filename.");
                            ui.vertical(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut draft.filename_pattern)
                                        .hint_text("Default (e.g. {artist} - {album} or {year} - {artist} - {album})")
                                        .desired_width(280.0),
                                );
                                ui.label(
                                    RichText::new(
                                        "Tags: {artist}, {page_artist}, {album}, {title}, {year}, {date}, {format}, {type}, {id}",
                                    )
                                    .size(11.0)
                                    .color(pal::TEXT_DIM),
                                );
                            });
                            ui.end_row();

                            ui.label("Format Fallback:");
                            ui.horizontal(|ui| {
                                ui.selectable_value(
                                    &mut draft.format_fallback,
                                    FormatFallback::Nearest,
                                    "Nearest Available",
                                );
                                ui.selectable_value(
                                    &mut draft.format_fallback,
                                    FormatFallback::Strict,
                                    "Strict (Fail if missing)",
                                );
                            });
                            ui.end_row();

                            ui.label("Archive Extraction:");
                            ui.checkbox(&mut draft.auto_extract, "Automatically extract album ZIPs");
                            ui.end_row();

                            ui.label("Cleanup ZIP:");
                            ui.add_enabled(
                                draft.auto_extract,
                                egui::Checkbox::new(
                                    &mut draft.delete_zip_after_extract,
                                    "Delete ZIP after successful extraction",
                                ),
                            );
                            ui.end_row();

                            ui.label("Cover Thumbnails:");
                            ui.checkbox(
                                &mut draft.show_thumbnails,
                                "Show 20px cover thumbnails in table",
                            );
                            ui.end_row();

                            ui.label("Page Request Delay (ms):");
                            ui.add(egui::Slider::new(&mut draft.page_delay_ms, 500..=5000));
                            ui.end_row();

                            ui.label("Page Jitter (%):");
                            ui.add(egui::Slider::new(&mut draft.page_jitter_pct, 0..=50));
                            ui.end_row();

                            ui.label("Max Concurrent Downloads:");
                            ui.add(egui::Slider::new(
                                &mut draft.max_concurrent_downloads,
                                1..=4,
                            ));
                            ui.end_row();

                            ui.label("Soft Quota Warning (30d):");
                            ui.add(egui::Slider::new(&mut draft.quota_soft, 25..=200));
                            ui.end_row();

                            ui.label("Hard Quota Cap (30d):");
                            ui.add(egui::Slider::new(&mut draft.quota_hard, 50..=300));
                            ui.end_row();

                            ui.label("User-Agent:");
                            ui.add(egui::TextEdit::singleline(&mut draft.user_agent).desired_width(260.0));
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(RichText::new("Library").strong());

                    egui::Grid::new("library_settings_grid")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("Verify on Sync:");
                            ui.checkbox(&mut draft.verify_on_sync, "Verify files after every sync");
                            ui.end_row();

                            ui.label("Rescan on Sync:");
                            ui.checkbox(&mut draft.rescan_linked_on_sync, "Re-read linked folders on sync");
                            ui.end_row();

                            ui.label("Auto-Relink:");
                            ui.checkbox(&mut draft.auto_relink, "Try to find moved albums automatically");
                            ui.end_row();

                            ui.label("When Files Missing:")
                                .on_hover_text("Ask: show resolve missing dialog on sync\nHandle automatically: apply deleted policy\nNote: an unavailable folder always asks");
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut draft.on_missing, OnMissing::Ask, "Ask");
                                ui.selectable_value(&mut draft.on_missing, OnMissing::Auto, "Handle automatically");
                            });
                            ui.end_row();

                            ui.label("If Deleted (used when handled automatically, and as the default in the dialog):")
                                .on_hover_text("Keep marked: mark as archived and keep downloaded = 1\nMark as not downloaded: clear downloaded mark\n(used when handled automatically, and as the default in the dialog)");
                            ui.horizontal(|ui| {
                                ui.selectable_value(
                                    &mut draft.deleted_policy,
                                    DeletedPolicy::KeepMarked,
                                    "Keep marked as downloaded",
                                );
                                ui.selectable_value(
                                    &mut draft.deleted_policy,
                                    DeletedPolicy::MarkNotDownloaded,
                                    "Mark as not downloaded",
                                );
                            });
                            ui.end_row();

                            ui.label("Auto-Match Min Score:")
                                .on_hover_text("Score threshold (0.50 - 1.00) required to adopt non-exact (fuzzy) matches automatically.");
                            ui.add(
                                egui::Slider::new(&mut draft.auto_match_min, 0.50..=1.00)
                                    .step_by(0.01)
                                    .custom_formatter(|n, _| format!("{:.2}", n)),
                            );
                            ui.end_row();

                            ui.label("Review Min Score:")
                                .on_hover_text("Minimum score threshold (0.50 - 1.00) to propose non-exact (fuzzy) matches for review.");
                            ui.add(
                                egui::Slider::new(&mut draft.review_match_min, 0.50..=1.00)
                                    .step_by(0.01)
                                    .custom_formatter(|n, _| format!("{:.2}", n)),
                            );
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(RichText::new("Folder Naming").strong());
                    ui.label(
                        RichText::new(
                            "Declare how your folders are named using placeholders: {title} (required), \
                            {artist}, {year}, and {noise}. Punctuation/brackets are ignored during matching.",
                        )
                        .color(pal::TEXT_DIM)
                        .size(11.5),
                    );

                    ui.add_space(4.0);
                    ui.label(RichText::new("Templates:").size(12.0));

                    let mut remove_tmpl = None;
                    let can_remove_tmpl = draft.folder_style.templates.len() > 1;
                    for (idx, tmpl) in draft.folder_style.templates.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(tmpl).desired_width(260.0));
                            if can_remove_tmpl && sharp_small_button(ui, "✕").clicked() {
                                remove_tmpl = Some(idx);
                            }
                        });
                    }
                    if let Some(idx) = remove_tmpl {
                        draft.folder_style.templates.remove(idx);
                    }

                    ui.horizontal(|ui| {
                        if draft.folder_style.templates.len() < 8
                            && sharp_small_button(ui, "+ Add Template").clicked()
                        {
                            draft.folder_style.templates.push("{title}".to_string());
                        }
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Presets:").color(pal::TEXT_DIM).size(11.5));
                        let presets = [
                            "{title}",
                            "{artist} - {title}",
                            "{year} - {title}",
                            "{title} ({year})",
                            "{title} ({noise})",
                            "{artist} - {year} - {title}",
                        ];
                        for p in presets {
                            if sharp_small_button(ui, p).clicked()
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
                            .color(pal::TEXT_DIM)
                            .size(11.5),
                    );

                    let mut remove_noise = None;
                    for (idx, entry) in draft.folder_style.noise.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(entry).desired_width(180.0));
                            if sharp_small_button(ui, "✕").clicked() {
                                remove_noise = Some(idx);
                            }
                        });
                    }
                    if let Some(idx) = remove_noise {
                        draft.folder_style.noise.remove(idx);
                    }

                    ui.horizontal(|ui| {
                        if draft.folder_style.noise.len() < 32
                            && sharp_small_button(ui, "+ Add Noise").clicked()
                        {
                            draft.folder_style.noise.push(String::new());
                        }
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Quick-add:").color(pal::TEXT_DIM).size(11.5));
                        let quick_adds = [
                            "FLAC", "MP3", "V0", "320", "WAV", "AAC", "OGG", "ALAC", "24bit",
                        ];
                        for q in quick_adds {
                            if sharp_small_button(ui, q).clicked()
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
                        ui.label(RichText::new(format!("⚠ {}", err)).color(pal::ERR));
                    }
                    for hint in draft.folder_style.hints() {
                        ui.label(RichText::new(format!("ℹ {}", hint)).color(pal::WARN));
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(RichText::new("Linked Artist Folders").strong());

                    if state.folders.is_empty() {
                        ui.label(RichText::new("No artist folders linked.").color(pal::TEXT_DIM));
                    } else {
                        let mut linked: Vec<_> = state.folders.values().flatten().cloned().collect();
                        linked.sort_by(|a, b| {
                            let name_a = state
                                .artists
                                .iter()
                                .find(|x| x.band_id == a.band_id)
                                .map(|x| x.display_name())
                                .unwrap_or("");
                            let name_b = state
                                .artists
                                .iter()
                                .find(|x| x.band_id == b.band_id)
                                .map(|x| x.display_name())
                                .unwrap_or("");
                            name_a
                                .to_lowercase()
                                .cmp(&name_b.to_lowercase())
                                .then_with(|| a.path.cmp(&b.path))
                        });

                        egui::Grid::new("linked_folders_grid")
                            .num_columns(4)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                for folder in linked {
                                    let tip = match folder.state {
                                        FolderState::Ok => "Folder OK and accessible",
                                        FolderState::Missing => "Folder missing on disk",
                                        FolderState::Unreadable => "Folder unreadable or inaccessible",
                                    };
                                    paint_folder_status_dot(ui, folder.state).on_hover_text(tip);

                                    let artist_name = state
                                        .artists
                                        .iter()
                                        .find(|a| a.band_id == folder.band_id)
                                        .map(|a| a.display_name())
                                        .unwrap_or("Unknown Artist");
                                    ui.label(RichText::new(artist_name).strong());

                                    let path_str = folder.path.display().to_string();
                                    ui.label(
                                        RichText::new(&path_str)
                                            .color(pal::TEXT_DIM)
                                            .monospace(),
                                    )
                                    .on_hover_text(&path_str);

                                    if sharp_small_button(ui, "Unlink").clicked() {
                                        let _ = cmd_tx.send(UiToWorkerCommand::UnlinkArtistFolder {
                                            folder_id: folder.id,
                                        });
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                if sharp_button(ui, "Save").clicked() {
                    draft.quota_soft = draft.quota_soft.min(draft.quota_hard);
                    draft.batch_limit = draft.batch_limit.min(draft.quota_hard);
                    draft.review_match_min = draft.review_match_min.min(draft.auto_match_min);
                    draft.folder_style.sanitize();
                    state.settings = draft.clone();
                    let _ = cmd_tx.send(UiToWorkerCommand::UpdateSettings(draft.clone()));
                    keep_open = false;
                }
                if sharp_button(ui, "Copy Settings JSON").clicked()
                    && let Ok(json) = serde_json::to_string_pretty(&draft)
                {
                    ui.ctx().copy_text(json);
                }
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.settings_draft = Some(draft);
    }
}

fn show_confirm_remove_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some((band_id, name)) = state.confirm_remove_artist.take() else {
        return;
    };
    let mut keep_open = !esc;

    egui::Window::new("Remove Artist")
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(340.0);
            ui.label(format!(
                "Remove '{name}' and its release records from the database?"
            ));
            ui.label(
                RichText::new("Downloaded files on disk and quota history are never deleted.")
                    .color(pal::TEXT_DIM),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if sharp_button(ui, "Remove").clicked() {
                    let _ = cmd_tx.send(UiToWorkerCommand::RemoveArtist { band_id });
                    keep_open = false;
                }
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.confirm_remove_artist = Some((band_id, name));
    }
}

fn show_rename_artist_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut modal) = state.rename_artist_modal.take() else {
        return;
    };
    let mut keep_open = !esc;

    egui::Window::new("Rename Page Artist")
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.label(
                RichText::new(format!("Bandcamp page name: {}", modal.original_name))
                    .color(pal::TEXT_DIM),
            );
            ui.label("Custom display & folder name:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut modal.name_input)
                    .hint_text(&modal.original_name)
                    .desired_width(f32::INFINITY),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            ui.separator();
            ui.horizontal(|ui| {
                if sharp_button(ui, "Save").clicked() || enter {
                    let trimmed = modal.name_input.trim();
                    let custom_name = if trimmed.is_empty() || trimmed == modal.original_name {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                    let _ = cmd_tx.send(UiToWorkerCommand::RenameArtist {
                        band_id: modal.band_id,
                        custom_name,
                    });
                    keep_open = false;
                }
                if sharp_button(ui, "Reset to Default").clicked() {
                    let _ = cmd_tx.send(UiToWorkerCommand::RenameArtist {
                        band_id: modal.band_id,
                        custom_name: None,
                    });
                    keep_open = false;
                }
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.rename_artist_modal = Some(modal);
    }
}

fn show_link_folder_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
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

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 48.0).clamp(420.0, 620.0);

    let target_band_id = modal.band_id.or_else(|| {
        state
            .artists
            .iter()
            .find(|a| a.matches_input(&modal.artist_input))
            .map(|a| a.band_id)
    });
    let artist_name = target_band_id
        .and_then(|bid| state.artists.iter().find(|a| a.band_id == bid))
        .map(|a| a.display_name().to_string())
        .unwrap_or_else(|| {
            if !modal.artist_input.is_empty() {
                modal.artist_input.clone()
            } else {
                "Artist".to_string()
            }
        });

    let existing_folders = target_band_id
        .and_then(|bid| state.folders.get(&bid))
        .cloned()
        .unwrap_or_default();

    let is_repoint = modal.repoint_folder_id.is_some();
    let has_links = !existing_folders.is_empty();

    let window_title = if is_repoint {
        format!("Change Folder Location - {artist_name}")
    } else if has_links {
        format!("Add another folder for {artist_name}")
    } else {
        "Link Artist Folder".to_string()
    };

    let desc = if is_repoint {
        "Choose the new location for this artist folder:"
    } else if has_links {
        "Add another local music folder for this artist:"
    } else {
        "Link a local music folder to a Bandcamp artist:"
    };

    egui::Window::new(window_title)
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .max_size([max_w, (screen.height() - 48.0).max(300.0)])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_min_width(ui.available_width().min(460.0));

            ui.label(RichText::new(desc).color(pal::TEXT_DIM));
            ui.add_space(4.0);

            egui::Grid::new("link_folder_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Artist Page:");
                    if let Some(band_id) = modal.band_id {
                        let a_name = state
                            .artists
                            .iter()
                            .find(|a| a.band_id == band_id)
                            .map(|a| a.display_name().to_string())
                            .unwrap_or_else(|| format!("Artist {band_id}"));
                        ui.label(RichText::new(a_name).strong());
                    } else {
                        ui.vertical(|ui| {
                            let mut selected_text = "Select tracked artist...".to_string();
                            if let Some(band_id) = modal.band_id
                                && let Some(a) = state.artists.iter().find(|a| a.band_id == band_id)
                            {
                                selected_text = a.display_name().to_string();
                            }
                            egui::ComboBox::from_id_salt("link_modal_artist_combo")
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

                    if !is_repoint && has_links {
                        ui.label("Existing links:");
                        ui.vertical(|ui| {
                            for f in &existing_folders {
                                ui.label(
                                    RichText::new(f.path.display().to_string())
                                        .color(pal::TEXT_DIM)
                                        .monospace(),
                                );
                            }
                        });
                        ui.end_row();
                    }

                    ui.label("Folder:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut modal.folder_path)
                                .hint_text("/path/to/artist/folder")
                                .desired_width(240.0),
                        );
                        if sharp_small_button(ui, "Browse...").clicked() {
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
            if !is_repoint {
                ui.checkbox(
                    &mut modal.auto_apply_confident,
                    "Adopt confident matches automatically",
                );
            }

            if let Some(ref err) = modal.error {
                ui.add_space(4.0);
                ui.label(RichText::new(err).color(pal::ERR));
            }

            ui.separator();
            ui.horizontal(|ui| {
                let action_label = if is_repoint {
                    "Change Folder"
                } else if has_links {
                    "Add Folder"
                } else {
                    "Link Folder"
                };

                if sharp_button(ui, action_label).clicked() {
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
                        } else if let Some(folder_id) = modal.repoint_folder_id {
                            let _ = cmd_tx.send(UiToWorkerCommand::RepointArtistFolder {
                                folder_id,
                                new_path: path,
                            });
                            keep_open = false;
                        } else {
                            let path_key =
                                bcvault_core::library::link::normalize_path_key(&path);
                            let mut overlap_err = None;
                            for f in &existing_folders {
                                let f_key =
                                    bcvault_core::library::link::normalize_path_key(&f.path);
                                if path_key == f_key || path_key.starts_with(&format!("{f_key}/")) {
                                    overlap_err =
                                        Some(format!("already covered by {}", f.path.display()));
                                    break;
                                } else if f_key.starts_with(&format!("{path_key}/")) {
                                    overlap_err = Some(format!(
                                        "contains {}; unlink that first",
                                        f.path.display()
                                    ));
                                    break;
                                }
                            }

                            if let Some(err_msg) = overlap_err {
                                modal.error = Some(err_msg);
                            } else if let Some(band_id) = target_band_id {
                                let _ = cmd_tx.send(UiToWorkerCommand::LinkArtistFolder {
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
                                    let _ = cmd_tx.send(
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
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.link_folder_modal = Some(modal);
    }
}

fn format_signals(signals: &[crate::library::matcher::Signal]) -> String {
    if signals.is_empty() {
        return "No signals recorded".to_string();
    }
    signals
        .iter()
        .map(|s| match s {
            crate::library::matcher::Signal::UrlTag => "URL tag match (exact)",
            crate::library::matcher::Signal::AlbumTag => "Album tag match",
            crate::library::matcher::Signal::FolderName => "Folder name match",
            crate::library::matcher::Signal::TrackCount => "Track count match",
            crate::library::matcher::Signal::Year => "Year tag match",
        })
        .collect::<Vec<_>>()
        .join("\n")
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

fn effective_catalog(
    state: &UiState,
    report: &bcvault_core::library::ReconcileReport,
) -> Vec<bcvault_core::types::ReleaseRow> {
    if !report.catalog.is_empty() {
        return report.catalog.clone();
    }
    state
        .rows
        .iter()
        .filter(|cr| cr.row.band_id == report.band_id)
        .map(|cr| cr.row.clone())
        .collect()
}

fn render_release_attach_picker(
    ui: &mut egui::Ui,
    id_salt: &str,
    path: &std::path::Path,
    current_key: &mut bcvault_core::types::ItemKey,
    search_map: &mut std::collections::HashMap<std::path::PathBuf, String>,
    catalog: &[bcvault_core::types::ReleaseRow],
    priority_keys: &[bcvault_core::types::ItemKey],
) {
    use bcvault_core::library::{
        filter_releases_for_picker, format_release_picker_label, resolve_release_by_query,
    };

    let mut ordered_catalog = Vec::with_capacity(catalog.len());
    for &pk in priority_keys {
        if let Some(r) = catalog.iter().find(|r| r.key == pk) {
            ordered_catalog.push(r.clone());
        }
    }
    for r in catalog {
        if !priority_keys.contains(&r.key) {
            ordered_catalog.push(r.clone());
        }
    }

    let query = search_map.entry(path.to_path_buf()).or_default();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Search / URL / Slug:").color(pal::TEXT_DIM).size(11.5));
        let resp = ui.add(
            egui::TextEdit::singleline(query)
                .hint_text("Search title, slug, or paste Bandcamp URL...")
                .desired_width(280.0),
        );
        if !query.is_empty() && sharp_small_button(ui, "✕").clicked() {
            query.clear();
        }
        if resp.changed() {
            if let Some(exact) = resolve_release_by_query(query, &ordered_catalog) {
                *current_key = exact;
            } else {
                let filtered = filter_releases_for_picker(query, &ordered_catalog);
                if !filtered.iter().any(|r| r.key == *current_key)
                    && let Some(first) = filtered.first()
                {
                    *current_key = first.key;
                }
            }
        }
    });

    let exact_match = resolve_release_by_query(query, &ordered_catalog);
    let filtered = filter_releases_for_picker(query, &ordered_catalog);

    if !query.trim().is_empty() {
        if exact_match.is_some() {
            ui.label(
                RichText::new("✓ Exact URL/slug match")
                    .color(pal::OK)
                    .size(11.0),
            );
        } else if filtered.is_empty() {
            ui.label(
                RichText::new("⚠ No release on this artist page matches that query/URL")
                    .color(pal::WARN)
                    .size(11.0),
            );
        }
    }

    ui.horizontal(|ui| {
        ui.label("Attach to:");
        let selected_rel = ordered_catalog.iter().find(|r| r.key == *current_key);
        let selected_label = selected_rel
            .map(format_release_picker_label)
            .unwrap_or_else(|| format!("Release #{}", current_key.id));

        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(&selected_label)
            .width(360.0)
            .show_ui(ui, |ui| {
                if filtered.is_empty() {
                    ui.label(RichText::new("No matching releases").color(pal::TEXT_DIM));
                } else {
                    for r in filtered {
                        let lbl = format_release_picker_label(r);
                        if ui
                            .selectable_label(r.key == *current_key, &lbl)
                            .on_hover_text(&r.url)
                            .clicked()
                        {
                            *current_key = r.key;
                        }
                    }
                }
            });

        if let Some(rel) = selected_rel
            && sharp_small_button(ui, "Open")
                .on_hover_text(&rel.url)
                .clicked()
        {
            let _ = open::that_detached(&rel.url);
        }
    });
}

fn show_reconcile_report_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut modal) = state.reconcile_report_modal.take() else {
        return;
    };
    let mut keep_open = !esc;

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
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(true)
        .default_size([700.0, 520.0])
        .max_size([max_w, max_h])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_min_width(ui.available_width().min(580.0));

            // Subtitle
            let folder_paths: Vec<String> = state
                .folders
                .get(&modal.report.band_id)
                .map(|v| v.iter().map(|f| f.path.display().to_string()).collect())
                .unwrap_or_default();
            let folder_summary = if folder_paths.is_empty() {
                "Unknown folder".to_string()
            } else if folder_paths.len() == 1 {
                folder_paths[0].clone()
            } else {
                format!("{} folders linked", folder_paths.len())
            };
            ui.label(
                RichText::new(format!(
                    "Batch #{}  ·  Linked: {}",
                    modal.report.batch_id, folder_summary
                ))
                .size(11.5)
                .color(pal::TEXT_DIM),
            );

            if !modal.report.unavailable_folders.is_empty() {
                ui.add_space(2.0);
                let total = state
                    .folders
                    .get(&modal.report.band_id)
                    .map(|v| v.len())
                    .unwrap_or(modal.report.unavailable_folders.len());
                let banner = format!(
                    "{} of {} folders unavailable; results may be incomplete",
                    modal.report.unavailable_folders.len(),
                    total
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").color(pal::WARN));
                    ui.label(RichText::new(banner).color(pal::WARN));
                });
            }
            ui.add_space(4.0);

            // Tab bar
            ui.horizontal(|ui| {
                let tabs = [
                    (
                        ReportTab::Matched,
                        format!(
                            "Matched ({})",
                            modal.report.matched.len() + modal.report.linked.len()
                        ),
                    ),
                    (
                        ReportTab::Review,
                        format!("Review ({})", modal.report.review.len()),
                    ),
                    (
                        ReportTab::PageOnly,
                        format!("On page only ({})", modal.report.page_only.len()),
                    ),
                    (
                        ReportTab::LocalOnly,
                        format!("Local only ({})", modal.report.local_only.len()),
                    ),
                    (
                        ReportTab::Conflicts,
                        format!("Conflicts ({})", modal.report.conflicts.len()),
                    ),
                    (
                        ReportTab::Restorable,
                        format!("Found again ({})", modal.report.restorable.len()),
                    ),
                ];

                for (tab, label) in tabs {
                    let is_active = modal.active_tab == tab;
                    let text = if is_active {
                        RichText::new(label).strong().color(pal::ACCENT)
                    } else {
                        RichText::new(label).color(pal::TEXT)
                    };
                    if sharp_small_button(ui, text.text()).clicked() {
                        modal.active_tab = tab;
                    }
                }
            });

            ui.separator();

            // Tab contents inside ScrollArea
            egui::ScrollArea::vertical()
                .max_height((max_h - 160.0).max(200.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width());
                    match modal.active_tab {
                    ReportTab::Matched => {
                        if !modal.report.matched.is_empty() {
                            ui.horizontal(|ui| {
                                let all_selected = !modal.report.matched.is_empty()
                                    && modal.selected_matched.len() == modal.report.matched.len();
                                if sharp_small_button(
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
                                    .color(pal::TEXT_DIM),
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
                            let budget = compute_balanced_columns(avail_w, 104.0, title_max, cand_max);

                            egui::Grid::new("matched_tab_grid")
                                .num_columns(4)
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

                                        truncated_cell(
                                            ui,
                                            budget.title_width,
                                            20.0,
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
                                            20.0,
                                            RichText::new(cand_disp)
                                                .color(pal::TEXT_DIM)
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
                                                .color(pal::OK)
                                                .monospace(),
                                        )
                                        .on_hover_text(tip);

                                        ui.end_row();
                                    }
                                });
                        } else if modal.report.linked.is_empty() {
                            ui.label(RichText::new("No matched releases.").color(pal::TEXT_DIM));
                        }

                        if !modal.report.linked.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(format!(
                                    "Already linked ({})",
                                    modal.report.linked.len()
                                ))
                                .strong()
                                .color(pal::TEXT_DIM),
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
                            let budget = compute_balanced_columns(avail_w, 16.0, title_max, cand_max);

                            egui::Grid::new("linked_tab_grid")
                                .num_columns(2)
                                .spacing([8.0, 6.0])
                                .show(ui, |ui| {
                                    for l in &modal.report.linked {
                                        truncated_cell(
                                            ui,
                                            budget.title_width,
                                            20.0,
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
                                            20.0,
                                            RichText::new(name).color(pal::TEXT_DIM).monospace(),
                                            Some(l.path.display().to_string()),
                                        );
                                        ui.end_row();
                                    }
                                });
                        }
                        }
                    ReportTab::Review => {
                        if modal.report.review.is_empty() {
                            ui.label(
                                RichText::new("No items requiring review.").color(pal::TEXT_DIM),
                            );
                        } else {
                            let catalog = effective_catalog(state, &modal.report);
                            ui.horizontal_wrapped(|ui| {
                                if sharp_small_button(ui, "Mark all as wrong (don't ask again)")
                                    .on_hover_text("Never propose these folder pairings again")
                                    .clicked()
                                {
                                    modal.mark_all_review_wrong();
                                }
                                if sharp_small_button(ui, "Mark skipped as wrong")
                                    .on_hover_text("Mark all currently skipped review items as wrong (don't ask again)")
                                    .clicked()
                                {
                                    modal.mark_unselected_review_wrong();
                                }
                                if sharp_small_button(ui, "Skip all for now").clicked() {
                                    modal.skip_all_review();
                                }
                                let adopt_n = modal.selected_review.len();
                                let reject_n = modal.rejected_review.len();
                                let skip_n = modal
                                    .report
                                    .review
                                    .len()
                                    .saturating_sub(adopt_n + reject_n);
                                ui.label(
                                    RichText::new(format!(
                                        "{adopt_n} adopt · {reject_n} wrong · {skip_n} skip"
                                    ))
                                    .color(pal::TEXT_DIM)
                                    .size(11.5),
                                );
                            });
                            ui.add_space(4.0);

                            for r in &modal.report.review {
                                ui.group(|ui| {
                                    let rel_opt = catalog.iter().find(| row | row.key == r.release_key);
                                    ui.horizontal(|ui| {
                                        let label_txt = rel_opt
                                            .map(bcvault_core::library::format_release_picker_label)
                                            .unwrap_or_else(|| r.release_title.clone());
                                        ui.add(
                                            egui::Label::new(RichText::new(&label_txt).strong())
                                                .truncate(),
                                        )
                                        .on_hover_text(
                                            rel_opt
                                                .map(|row| row.url.as_str())
                                                .unwrap_or(&r.release_title),
                                        );
                                        if let Some(rel) = rel_opt
                                            && sharp_small_button(ui, "Open")
                                                .on_hover_text(&rel.url)
                                                .clicked()
                                        {
                                            let _ = open::that_detached(&rel.url);
                                        }
                                    });

                                    let chosen = modal.selected_review.get(&r.release_key).copied();
                                    let is_rejected = modal.rejected_review.contains(&r.release_key);

                                    for (c_idx, c) in r.candidates.iter().enumerate() {
                                        let is_chosen = !is_rejected && chosen == Some(c_idx);
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
                                                modal.rejected_review.remove(&r.release_key);
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
                                                .color(pal::WARN)
                                                .size(11.0),
                                            )
                                            .on_hover_text(tip);
                                        });
                                    }

                                    let is_skipped = chosen.is_none() && !is_rejected;
                                    ui.horizontal(|ui| {
                                        if ui.radio(is_skipped, "Skip for now").clicked() {
                                            modal.selected_review.remove(&r.release_key);
                                            modal.rejected_review.remove(&r.release_key);
                                        }
                                        if ui
                                            .radio(
                                                is_rejected,
                                                RichText::new("Wrong, don't ask again").color(
                                                    if is_rejected {
                                                        pal::WARN
                                                    } else {
                                                        pal::TEXT
                                                    },
                                                ),
                                            )
                                            .on_hover_text(
                                                "Permanently dismiss this pairing so future scans never ask again",
                                            )
                                            .clicked()
                                        {
                                            modal.selected_review.remove(&r.release_key);
                                            modal.rejected_review.insert(r.release_key);
                                        }
                                    });
                                });
                                ui.add_space(4.0);
                            }
                        }
                    }
                    ReportTab::PageOnly => {
                        if modal.report.page_only.is_empty() {
                            ui.label(
                                RichText::new("All releases have corresponding local files.")
                                    .color(pal::TEXT_DIM),
                            );
                        } else {
                            let catalog = effective_catalog(state, &modal.report);
                            ui.label(
                                RichText::new(
                                    "These releases exist on Bandcamp but have no local audio files in the linked folder:",
                                )
                                .color(pal::TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            for key in &modal.report.page_only {
                                let rel_opt = catalog.iter().find(|r| r.key == *key);
                                let label = rel_opt
                                    .map(bcvault_core::library::format_release_picker_label)
                                    .unwrap_or_else(|| format!("Release #{}", key.id));
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("-").color(pal::TEXT_DIM));
                                    ui.add(egui::Label::new(RichText::new(&label).strong()).truncate())
                                        .on_hover_text(
                                            rel_opt.map(|r| r.url.as_str()).unwrap_or(&label),
                                        );
                                    if let Some(rel) = rel_opt
                                        && sharp_small_button(ui, "Open")
                                            .on_hover_text(&rel.url)
                                            .clicked()
                                    {
                                        let _ = open::that_detached(&rel.url);
                                    }
                                });
                            }
                        }
                    }
                    ReportTab::LocalOnly => {
                        if modal.report.local_only.is_empty() {
                            ui.label(
                                RichText::new("No unlinked local folders.").color(pal::TEXT_DIM),
                            );
                        } else {
                            let catalog = effective_catalog(state, &modal.report);
                            ui.label(
                                RichText::new(
                                    "These folders were found locally but do not match any release on the artist's Bandcamp page:",
                                )
                                .color(pal::TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            for orphan in &modal.report.local_only {
                                ui.group(|ui| {
                                    let fmt_str = orphan
                                        .format_guess
                                        .map(|f| format!("[{}]", f.label()))
                                        .unwrap_or_default();
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "{} ({} tracks) {fmt_str}",
                                                orphan.name, orphan.track_count
                                            ))
                                            .strong(),
                                        )
                                        .truncate(),
                                    );
                                    let display_path = if let Some(folders) =
                                        state.folders.get(&modal.report.band_id)
                                        && folders.len() > 1
                                    {
                                        let best = folders
                                            .iter()
                                            .filter(|f| orphan.path.starts_with(&f.path))
                                            .max_by_key(|f| f.path.as_os_str().len());
                                        if let Some(owning) = best {
                                            let folder_name = owning
                                                .path
                                                .file_name()
                                                .map(|n| n.to_string_lossy())
                                                .unwrap_or_default();
                                            if let Ok(rel) = orphan.path.strip_prefix(&owning.path)
                                            {
                                                format!("{} / {}", folder_name, rel.display())
                                            } else {
                                                orphan.path.display().to_string()
                                            }
                                        } else {
                                            orphan.path.display().to_string()
                                        }
                                    } else {
                                        orphan.path.display().to_string()
                                    };

                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&display_path)
                                                .color(pal::TEXT_DIM)
                                                .size(11.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(orphan.path.display().to_string());

                                    let current_decision = modal
                                        .local_decisions
                                        .get(&orphan.path)
                                        .cloned()
                                        .unwrap_or(LocalAction::Leave);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .selectable_label(
                                                matches!(current_decision, LocalAction::Leave),
                                                "Leave",
                                            )
                                            .clicked()
                                        {
                                            modal
                                                .local_decisions
                                                .insert(orphan.path.clone(), LocalAction::Leave);
                                        }
                                        if ui
                                            .selectable_label(
                                                matches!(current_decision, LocalAction::Ignore),
                                                "Ignore in future",
                                            )
                                            .clicked()
                                        {
                                            modal
                                                .local_decisions
                                                .insert(orphan.path.clone(), LocalAction::Ignore);
                                        }
                                        let is_attach =
                                            matches!(current_decision, LocalAction::Attach(_));
                                        if ui
                                            .selectable_label(is_attach, "Attach to release...")
                                            .clicked()
                                            && !is_attach
                                            && let Some(first_rel) = catalog.first()
                                        {
                                            modal.local_decisions.insert(
                                                orphan.path.clone(),
                                                LocalAction::Attach(first_rel.key),
                                            );
                                        }
                                    });

                                    if let LocalAction::Attach(mut target_key) = current_decision {
                                        let combo_id =
                                            format!("attach_combo_{}", orphan.path.display());
                                        render_release_attach_picker(
                                            ui,
                                            &combo_id,
                                            &orphan.path,
                                            &mut target_key,
                                            &mut modal.local_attach_search,
                                            &catalog,
                                            &[],
                                        );
                                        modal
                                            .local_decisions
                                            .insert(orphan.path.clone(), LocalAction::Attach(target_key));
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        }
                    }
                    ReportTab::Conflicts => {
                        if modal.report.conflicts.is_empty() {
                            ui.label(
                                RichText::new("No conflicts or multiple matches found.")
                                    .color(pal::TEXT_DIM),
                            );
                        } else {
                            let catalog = effective_catalog(state, &modal.report);
                            for c in &modal.report.conflicts {
                                ui.group(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&c.description)
                                                .strong()
                                                .color(pal::WARN),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&c.description);

                                    if !c.release_keys.is_empty() {
                                        ui.label(
                                            RichText::new("Colliding Bandcamp releases:")
                                                .color(pal::TEXT_DIM)
                                                .size(11.0),
                                        );
                                        for rk in &c.release_keys {
                                            if let Some(rel) = catalog.iter().find(|r| r.key == *rk)
                                            {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "  • {}",
                                                            bcvault_core::library::format_release_picker_label(rel)
                                                        ))
                                                        .size(11.0),
                                                    )
                                                    .on_hover_text(&rel.url);
                                                    if sharp_small_button(ui, "Open")
                                                        .on_hover_text(&rel.url)
                                                        .clicked()
                                                    {
                                                        let _ = open::that_detached(&rel.url);
                                                    }
                                                });
                                            }
                                        }
                                    }

                                    if !c.candidate_paths.is_empty() {
                                        ui.add_space(2.0);
                                        ui.label(
                                            RichText::new("Resolve local folders:")
                                                .color(pal::TEXT_DIM)
                                                .size(11.0),
                                        );
                                        for p in &c.candidate_paths {
                                            let name = p
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| p.display().to_string());
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(format!("Folder: {name}"))
                                                        .strong()
                                                        .monospace()
                                                        .size(11.5),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(p.display().to_string());

                                            let current_decision = modal
                                                .conflict_decisions
                                                .get(p)
                                                .cloned()
                                                .unwrap_or(LocalAction::Leave);
                                            ui.horizontal(|ui| {
                                                if ui
                                                    .selectable_label(
                                                        matches!(current_decision, LocalAction::Leave),
                                                        "Leave",
                                                    )
                                                    .clicked()
                                                {
                                                    modal
                                                        .conflict_decisions
                                                        .insert(p.clone(), LocalAction::Leave);
                                                }
                                                if ui
                                                    .selectable_label(
                                                        matches!(current_decision, LocalAction::Ignore),
                                                        "Ignore in future",
                                                    )
                                                    .clicked()
                                                {
                                                    modal
                                                        .conflict_decisions
                                                        .insert(p.clone(), LocalAction::Ignore);
                                                }
                                                let is_attach =
                                                    matches!(current_decision, LocalAction::Attach(_));
                                                if ui
                                                    .selectable_label(is_attach, "Match to release...")
                                                    .clicked()
                                                    && !is_attach
                                                {
                                                    let default_key = c
                                                        .release_keys
                                                        .first()
                                                        .copied()
                                                        .or_else(|| catalog.first().map(|r| r.key));
                                                    if let Some(key) = default_key {
                                                        modal.conflict_decisions.insert(
                                                            p.clone(),
                                                            LocalAction::Attach(key),
                                                        );
                                                    }
                                                }
                                            });

                                            if let LocalAction::Attach(mut target_key) =
                                                current_decision
                                            {
                                                let combo_id = format!(
                                                    "conflict_attach_combo_{}",
                                                    p.display()
                                                );
                                                render_release_attach_picker(
                                                    ui,
                                                    &combo_id,
                                                    p,
                                                    &mut target_key,
                                                    &mut modal.local_attach_search,
                                                    &catalog,
                                                    &c.release_keys,
                                                );
                                                modal.conflict_decisions.insert(
                                                    p.clone(),
                                                    LocalAction::Attach(target_key),
                                                );
                                            }
                                            ui.add_space(2.0);
                                        }
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        }
                    }
                    ReportTab::Restorable => {
                        if modal.report.restorable.is_empty() {
                            ui.label(
                                RichText::new("No previously archived releases found on disk.")
                                    .color(pal::TEXT_DIM),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                let all_selected = !modal.report.restorable.is_empty()
                                    && modal.selected_restorable.len()
                                        == modal.report.restorable.len();
                                if sharp_small_button(
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
                                    .color(pal::TEXT_DIM),
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
                            let budget = compute_balanced_columns(avail_w, 48.0, title_max, cand_max);

                            egui::Grid::new("restorable_tab_grid")
                                .num_columns(3)
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
                                        truncated_cell(
                                            ui,
                                            budget.title_width,
                                            20.0,
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
                                            20.0,
                                            RichText::new(cand_disp)
                                                .color(pal::TEXT_DIM)
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

            // Footer
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
                    let decisions = modal.build_decisions();
                    let _ = cmd_tx.send(UiToWorkerCommand::ApplyReconcile {
                        batch_id: modal.report.batch_id,
                        decisions,
                    });
                    keep_open = false;
                }

                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.reconcile_report_modal = Some(modal);
    }
}

fn show_undo_toast(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
) {
    let Some(toast) = state.undo_toast.clone() else {
        return;
    };

    if toast.created_at.elapsed() >= std::time::Duration::from_secs(12) {
        state.undo_toast = None;
        return;
    }

    egui::Area::new(Id::new("undo_toast_area"))
        .order(Order::Tooltip)
        .anchor(Align2::RIGHT_BOTTOM, [-24.0, -24.0])
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .corner_radius(CornerRadius::ZERO)
                .fill(pal::BG1)
                .stroke(Stroke::new(1.0_f32, pal::BORDER_HI))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        paint_folder_status_dot(ui, FolderState::Ok);
                        ui.label(RichText::new(&toast.message).color(pal::TEXT));
                        ui.separator();
                        let batch_id = toast.batch_id;
                        if sharp_small_button(ui, "Undo").clicked() {
                            let _ = cmd_tx.send(UiToWorkerCommand::UndoBatch { batch_id });
                            state.undo_toast = None;
                        }
                        if sharp_small_button(ui, "X").clicked() {
                            state.undo_toast = None;
                        }
                    });
                });
        });
}

fn show_resolve_missing_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
    esc: bool,
) {
    let Some(mut modal) = state.resolve_missing_modal.take() else {
        return;
    };

    // Check asynchronous folder picker completions
    if let Some((key, path)) = modal
        .picked_folder
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        modal.decisions.insert(key, MissingItemChoice::Moved(path));
    }
    if let Some((band_id, new_path)) = modal
        .repoint_picked_folder
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        if let Some(folders) = state.folders.get(&band_id) {
            let target_folder_id = modal
                .items
                .iter()
                .filter(|it| it.band_id == band_id)
                .find_map(|it| {
                    it.last_known_path.as_ref().and_then(|p| {
                        let rel_key = bcvault_core::library::link::normalize_path_key(p);
                        folders
                            .iter()
                            .filter(|f| {
                                let f_key =
                                    bcvault_core::library::link::normalize_path_key(&f.path);
                                rel_key == f_key || rel_key.starts_with(&format!("{f_key}/"))
                            })
                            .max_by_key(|f| f.path.as_os_str().len())
                            .map(|f| f.id)
                    })
                })
                .or_else(|| folders.first().map(|f| f.id));

            if let Some(folder_id) = target_folder_id {
                let _ = cmd_tx.send(UiToWorkerCommand::RepointArtistFolder {
                    folder_id,
                    new_path,
                });
            }
        }
        state.show_resolve_missing_window = false;
        state.resolve_missing_modal = None;
        return;
    }

    if esc {
        state.show_resolve_missing_window = false;
        state.resolve_missing_modal = Some(modal);
        return;
    }

    let mut open = state.show_resolve_missing_window;
    let mut applied = false;
    let mut close_requested = false;

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 48.0).clamp(480.0, 860.0);
    let max_h = (screen.height() - 48.0).clamp(360.0, 720.0);

    egui::Window::new("Resolve Missing Files")
        .order(Order::Foreground)
        .collapsible(true)
        .resizable(true)
        .default_size([720.0, 500.0])
        .max_size([max_w, max_h])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_min_width(ui.available_width().min(600.0));

            // Mass-missing safeguard banner
            if modal.mass_missing {
                egui::Frame::NONE
                    .corner_radius(CornerRadius::ZERO)
                    .fill(pal::WARN.gamma_multiply(0.12))
                    .stroke(Stroke::new(1.0_f32, pal::WARN))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Many albums are missing: is a drive disconnected?")
                                    .color(pal::WARN)
                                    .strong(),
                            );
                        });
                        ui.label(
                            RichText::new(
                                "Automatic policies were halted to prevent unintended deletions. Please verify that your storage drive is mounted and accessible.",
                            )
                            .size(11.0)
                            .color(pal::TEXT_DIM),
                        );
                    });
                ui.add_space(8.0);
            }

            // Bulk actions
            ui.horizontal(|ui| {
                if sharp_small_button(ui, "Mark All as Deleted").clicked() {
                    for it in &modal.items {
                        modal.decisions.insert(it.key, MissingItemChoice::Deleted);
                    }
                }
                if sharp_small_button(ui, "Skip All").clicked() {
                    for it in &modal.items {
                        modal.decisions.insert(it.key, MissingItemChoice::Skip);
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
                    if sharp_small_button(
                        ui,
                        &format!("The artist's folder moved ({artist_label})..."),
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
                            .color(pal::TEXT_DIM),
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
                    egui::ComboBox::from_id_salt("repoint_artist_combo")
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
                    if sharp_small_button(ui, "Browse New Folder...").clicked() {
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

            // Item list
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .show(ui, |ui| {
                    for item in &modal.items {
                        egui::Frame::NONE
                            .corner_radius(CornerRadius::ZERO)
                            .fill(pal::BG1)
                            .stroke(Stroke::new(1.0_f32, pal::BORDER))
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
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
                                            .color(pal::TEXT_DIM),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(last.display().to_string());
                                }
                                if let Some(cand) = &item.best_candidate {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "Candidate in artist folder ({}%): {}",
                                                (cand.score * 100.0) as u32,
                                                cand.path.display()
                                            ))
                                            .color(pal::ACCENT)
                                            .size(11.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(cand.path.display().to_string());
                                }

                                // Decision radios
                                let current_decision = modal
                                    .decisions
                                    .get(&item.key)
                                    .cloned()
                                    .unwrap_or(MissingItemChoice::Skip);
                                ui.horizontal(|ui| {
                                    let is_moved =
                                        matches!(current_decision, MissingItemChoice::Moved(_));
                                    if ui.radio(is_moved, "Moved").clicked() && !is_moved {
                                        if let Some(cand) = &item.best_candidate {
                                            modal.decisions.insert(
                                                item.key,
                                                MissingItemChoice::Moved(cand.path.clone()),
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
                                            MissingItemChoice::Moved(p) => p.display().to_string(),
                                            _ => String::new(),
                                        };
                                        ui.label(
                                            RichText::new(path_str)
                                                .size(11.0)
                                                .color(pal::TEXT_DIM),
                                        );
                                        if sharp_small_button(ui, "Browse...").clicked() {
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
                                        matches!(current_decision, MissingItemChoice::Deleted);
                                    let del_label = match modal.deleted_policy {
                                        DeletedPolicy::KeepMarked => {
                                            "Deleted (Keep marked / Archive)"
                                        }
                                        DeletedPolicy::MarkNotDownloaded => "Deleted (Unmark)",
                                    };
                                    if ui.radio(is_deleted, del_label).clicked() {
                                        modal
                                            .decisions
                                            .insert(item.key, MissingItemChoice::Deleted);
                                    }

                                    let is_skip = matches!(current_decision, MissingItemChoice::Skip);
                                    if ui.radio(is_skip, "Skip for now").clicked() {
                                        modal.decisions.insert(item.key, MissingItemChoice::Skip);
                                    }
                                });
                            });
                        ui.add_space(4.0);
                    }
                });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            // Footer controls
            ui.horizontal(|ui| {
                ui.label(RichText::new("When deleted:").size(12.0));
                let curr_text = match modal.deleted_policy {
                    DeletedPolicy::KeepMarked => "Keep marked as downloaded (archived)",
                    DeletedPolicy::MarkNotDownloaded => "Mark as not downloaded",
                };
                egui::ComboBox::from_id_salt("resolve_deleted_policy_combo")
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
                if sharp_button(ui, &apply_label).clicked() {
                    let mut decisions = Vec::new();
                    for item in &modal.items {
                        let choice = modal
                            .decisions
                            .get(&item.key)
                            .cloned()
                            .unwrap_or(MissingItemChoice::Skip);
                        let action = match choice {
                            MissingItemChoice::Moved(p) => MissingAction::Moved(p),
                            MissingItemChoice::Deleted => {
                                MissingAction::Deleted(modal.deleted_policy)
                            }
                            MissingItemChoice::Skip => MissingAction::Skip,
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

                    let _ = cmd_tx.send(UiToWorkerCommand::ResolveMissing {
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

                    applied = true;
                }

                if sharp_button(ui, "Close").clicked() {
                    close_requested = true;
                }
            });
        });

    if close_requested {
        open = false;
    }

    state.show_resolve_missing_window = open && !applied;
    if applied {
        state.resolve_missing_modal = None;
    } else {
        state.resolve_missing_modal = Some(modal);
    }
}

fn show_unavailable_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
    esc: bool,
) {
    let Some(mut modal) = state.unavailable_modal.take() else {
        return;
    };

    // Check asynchronous folder picker completions
    if let Some((folder_id, new_path)) = modal
        .picked_folder
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        let _ = cmd_tx.send(UiToWorkerCommand::RepointArtistFolder {
            folder_id,
            new_path,
        });
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
        let _ = cmd_tx.send(UiToWorkerCommand::ResolveUnavailable {
            batch_id: modal.batch_id,
            decisions,
        });
        state.show_unavailable_window = false;
        state.unavailable_modal = None;
        return;
    }

    let mut open = state.show_unavailable_window;
    let mut close_requested = false;
    let mut decisions_to_send = Vec::new();
    let mut cards_to_remove = Vec::new();

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 48.0).clamp(480.0, 860.0);
    let max_h = (screen.height() - 48.0).clamp(360.0, 720.0);

    egui::Window::new("Resolve Unavailable Folder")
        .order(Order::Foreground)
        .collapsible(true)
        .resizable(true)
        .default_size([700.0, 480.0])
        .max_size([max_w, max_h])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
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
                                let folder = state.folders.values().flatten().find(|f| f.id == folder_id);
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
                            .fill(pal::BG1)
                            .stroke(Stroke::new(1.0_f32, pal::BORDER))
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.label(RichText::new(&title).strong().color(pal::WARN));
                                ui.label(
                                    RichText::new(state_text)
                                        .size(11.5)
                                        .color(pal::TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new(format!("Path: {}", card.folder.path.display()))
                                        .size(11.0)
                                        .monospace()
                                        .color(pal::TEXT_DIM),
                                );
                                let rel_summary = if count == 0 {
                                    "No active downloads are located in this folder. You can unlink this folder or repoint it to a new location.".to_string()
                                } else {
                                    format!("{count} downloaded releases were last seen here. Their files can't be checked.")
                                };
                                ui.label(RichText::new(rel_summary).size(11.5).color(pal::TEXT_DIM));
                                ui.add_space(6.0);

                                // Choice 1: Drive disconnected
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("The drive is disconnected, I'll reconnect it:").size(12.0));
                                    if sharp_small_button(ui, "Keep as is").clicked() {
                                        decisions_to_send.push(UnavailableDecision {
                                            root: card.folder.root,
                                            keys: card.folder.releases.iter().map(|r| r.key).collect(),
                                            action: UnavailableAction::Keep,
                                        });
                                        cards_to_remove.push(idx);
                                    }
                                });

                                // Choice 2: Moved folder (Linked) or moved files to linked folder (OutDir)
                                match card.folder.root {
                                    UnavailableRoot::Linked { folder_id } => {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("I moved the folder:").size(12.0));
                                            if sharp_small_button(ui, "Choose new folder...").clicked() {
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
                                    UnavailableRoot::OutDir => {
                                        let has_linked_artist = card.folder.releases.iter().any(|r| {
                                            state
                                                .folders
                                                .get(&r.band_id)
                                                .is_some_and(|fs| !fs.is_empty())
                                        });
                                        if has_linked_artist {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new("I moved the files to the linked folder:")
                                                        .size(12.0),
                                                );
                                                if sharp_small_button(ui, "Scan linked folder").clicked() {
                                                    decisions_to_send.push(UnavailableDecision {
                                                        root: card.folder.root,
                                                        keys: card.folder.releases.iter().map(|r| r.key).collect(),
                                                        action: UnavailableAction::MovedToLinked,
                                                    });
                                                    cards_to_remove.push(idx);
                                                }
                                            });
                                        }
                                    }
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
                                    egui::ComboBox::from_id_salt(format!("card_del_policy_{idx}"))
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
                                    if sharp_small_button(ui, "Apply").clicked() {
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
                                            .id_salt(format!("card_rels_scroll_{idx}"))
                                            .show(ui, |ui| {
                                                for rel in &card.folder.releases {
                                                    let path_display = rel
                                                        .last_known_path
                                                        .as_ref()
                                                        .map(|p| p.display().to_string())
                                                        .unwrap_or_else(|| "Unknown path".to_string());
                                                    ui.horizontal(|ui| {
                                                        ui.label(RichText::new(&rel.title).size(11.0).color(pal::TEXT));
                                                        ui.label(
                                                            RichText::new(format!("({path_display})"))
                                                                .size(10.5)
                                                                .color(pal::TEXT_DIM),
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
                if sharp_button(ui, "Close").clicked() {
                    close_requested = true;
                }
            });
        });

    if !decisions_to_send.is_empty() {
        let _ = cmd_tx.send(UiToWorkerCommand::ResolveUnavailable {
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
        let _ = cmd_tx.send(UiToWorkerCommand::ResolveUnavailable {
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
