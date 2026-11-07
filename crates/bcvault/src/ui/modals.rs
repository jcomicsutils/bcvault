use crate::{
    bandcamp::redeem::validate_redeem_url,
    types::{DeletedPolicy, DownloadOpts, FolderState, FormatFallback, OnMissing, OutputLayout},
    ui::{
        state::UiState,
        theme::pal,
        widgets::{paint_folder_status_dot, sharp_button, sharp_rect, sharp_small_button},
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align2, Color32, Context, Id, Order, RichText, Sense, Stroke};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(ctx: &Context, state: &mut UiState, cmd_tx: &UnboundedSender<UiToWorkerCommand>) {
    let any_modal = state.redeem_modal.is_some()
        || state.quota_modal.is_some()
        || state.settings_draft.is_some()
        || state.confirm_remove_artist.is_some()
        || state.rename_artist_modal.is_some();

    if !any_modal {
        return;
    }

    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));

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
                                .on_hover_text("Ask: show resolve missing dialog on sync\nHandle automatically: apply deleted policy");
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut draft.on_missing, OnMissing::Ask, "Ask");
                                ui.selectable_value(&mut draft.on_missing, OnMissing::Auto, "Handle automatically");
                            });
                            ui.end_row();

                            ui.label("If Deleted:")
                                .on_hover_text("Keep marked: mark as archived and keep downloaded = 1\nMark as not downloaded: clear downloaded mark");
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
                                .on_hover_text("Score threshold (0.50 - 1.00) required to adopt matches automatically.");
                            ui.add(
                                egui::Slider::new(&mut draft.auto_match_min, 0.50..=1.00)
                                    .step_by(0.01)
                                    .custom_formatter(|n, _| format!("{:.2}", n)),
                            );
                            ui.end_row();

                            ui.label("Review Min Score:")
                                .on_hover_text("Minimum score threshold (0.50 - 1.00) to propose matches for review.");
                            ui.add(
                                egui::Slider::new(&mut draft.review_match_min, 0.50..=1.00)
                                    .step_by(0.01)
                                    .custom_formatter(|n, _| format!("{:.2}", n)),
                            );
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(RichText::new("Linked Artist Folders").strong());

                    if state.folders.is_empty() {
                        ui.label(RichText::new("No artist folders linked.").color(pal::TEXT_DIM));
                    } else {
                        let mut linked: Vec<_> = state.folders.values().cloned().collect();
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
                            name_a.to_lowercase().cmp(&name_b.to_lowercase())
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
                                            band_id: folder.band_id,
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
