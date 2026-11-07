use crate::{
    bandcamp::redeem::validate_redeem_url,
    types::{
        DeletedPolicy, DownloadOpts, FolderState, FormatFallback, MatchDecision, OnMissing,
        OutputLayout,
    },
    ui::{
        state::{LocalAction, ReportTab, UiState},
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
    show_undo_toast(ctx, state, cmd_tx);

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

    egui::Window::new("Link Artist Folder")
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(460.0);

            ui.label(
                RichText::new("Link a local music folder to a Bandcamp artist:")
                    .color(pal::TEXT_DIM),
            );
            ui.add_space(4.0);

            egui::Grid::new("link_folder_grid")
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

                            ui.add(
                                egui::TextEdit::singleline(&mut modal.artist_input)
                                    .hint_text("or paste Bandcamp URL / slug")
                                    .desired_width(280.0),
                            );
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
            ui.checkbox(
                &mut modal.auto_apply_confident,
                "Adopt confident matches automatically",
            );

            if let Some(ref err) = modal.error {
                ui.add_space(4.0);
                ui.label(RichText::new(err).color(pal::ERR));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if sharp_button(ui, "Link Folder").clicked() {
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
                                let input = modal.artist_input.trim();
                                if input.is_empty() {
                                    None
                                } else {
                                    let lower = input.to_lowercase();
                                    state
                                        .artists
                                        .iter()
                                        .find(|a| {
                                            a.url.to_lowercase().contains(&lower)
                                                || a.slug.to_lowercase() == lower
                                                || a.name.to_lowercase() == lower
                                                || a.display_name().to_lowercase() == lower
                                        })
                                        .map(|a| a.band_id)
                                }
                            });

                            if let Some(band_id) = target_band_id {
                                let _ = cmd_tx.send(UiToWorkerCommand::LinkArtistFolder {
                                    band_id,
                                    path,
                                    auto_apply_confident: modal.auto_apply_confident,
                                });
                                keep_open = false;
                            } else {
                                modal.error = Some(
                                    "Please select or specify a valid tracked artist.".to_string(),
                                );
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

    egui::Window::new(format!("Match Report - {artist_name}"))
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(true)
        .default_size([700.0, 520.0])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(580.0);

            // Subtitle
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
                .color(pal::TEXT_DIM),
            );
            ui.add_space(4.0);

            // Tab bar
            ui.horizontal(|ui| {
                let tabs = [
                    (
                        ReportTab::Matched,
                        format!("Matched ({})", modal.report.matched.len()),
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
                .max_height(360.0)
                .auto_shrink([false, false])
                .show(ui, |ui| match modal.active_tab {
                    ReportTab::Matched => {
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

                                    ui.label(RichText::new(&m.release_title).strong());

                                    let fmt_str = m
                                        .format_guess
                                        .map(|f| format!("[{}]", f.label()))
                                        .unwrap_or_default();
                                    ui.label(
                                        RichText::new(format!("{} {fmt_str}", m.candidate_name))
                                            .color(pal::TEXT_DIM)
                                            .monospace(),
                                    )
                                    .on_hover_text(m.candidate_path.display().to_string());

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
                    }
                    ReportTab::Review => {
                        if modal.report.review.is_empty() {
                            ui.label(
                                RichText::new("No items requiring review.").color(pal::TEXT_DIM),
                            );
                        } else {
                            for r in &modal.report.review {
                                ui.group(|ui| {
                                    ui.label(RichText::new(&r.release_title).strong());
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
                                            ui.label(
                                                RichText::new(format!(
                                                    "{} {fmt_str}",
                                                    c.candidate_name
                                                ))
                                                .monospace(),
                                            )
                                            .on_hover_text(
                                                c.candidate_path.display().to_string(),
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

                                    let is_skipped = chosen.is_none();
                                    if ui.radio(is_skipped, "Skip for now").clicked() {
                                        modal.selected_review.remove(&r.release_key);
                                    }
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
                            ui.label(
                                RichText::new(
                                    "These releases exist on Bandcamp but have no local audio files in the linked folder:",
                                )
                                .color(pal::TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            for key in &modal.report.page_only {
                                let title = state
                                    .rows
                                    .iter()
                                    .find(|cr| cr.row.key == *key)
                                    .map(|cr| cr.row.title.clone())
                                    .unwrap_or_else(|| format!("Release #{}", key.id));
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("-").color(pal::TEXT_DIM));
                                    ui.label(RichText::new(title).strong());
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
                                    ui.label(
                                        RichText::new(format!(
                                            "{} ({} tracks) {fmt_str}",
                                            orphan.name, orphan.track_count
                                        ))
                                        .strong(),
                                    );
                                    ui.label(
                                        RichText::new(orphan.path.display().to_string())
                                            .color(pal::TEXT_DIM)
                                            .size(11.0),
                                    );

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
                                        {
                                            let target = state
                                                .rows
                                                .iter()
                                                .find(|cr| cr.row.band_id == modal.report.band_id)
                                                .map(|cr| cr.row.key);
                                            if let Some(key) = target {
                                                modal
                                                    .local_decisions
                                                    .insert(orphan.path.clone(), LocalAction::Attach(key));
                                            }
                                        }
                                    });

                                    if let LocalAction::Attach(ref current_key) = current_decision {
                                        ui.horizontal(|ui| {
                                            ui.label("Attach to:");
                                            let current_title = state
                                                .rows
                                                .iter()
                                                .find(|cr| cr.row.key == *current_key)
                                                .map(|cr| cr.row.title.clone())
                                                .unwrap_or_else(|| "Select release".to_string());
                                            egui::ComboBox::from_id_salt(format!(
                                                "attach_combo_{}",
                                                orphan.path.display()
                                            ))
                                            .selected_text(&current_title)
                                            .width(280.0)
                                            .show_ui(ui, |ui| {
                                                for cr in state
                                                    .rows
                                                    .iter()
                                                    .filter(|cr| cr.row.band_id == modal.report.band_id)
                                                {
                                                    if ui
                                                        .selectable_label(
                                                            cr.row.key == *current_key,
                                                            &cr.row.title,
                                                        )
                                                        .clicked()
                                                    {
                                                        modal.local_decisions.insert(
                                                            orphan.path.clone(),
                                                            LocalAction::Attach(cr.row.key),
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
                    ReportTab::Conflicts => {
                        if modal.report.conflicts.is_empty() {
                            ui.label(
                                RichText::new("No conflicts or multiple matches found.")
                                    .color(pal::TEXT_DIM),
                            );
                        } else {
                            for c in &modal.report.conflicts {
                                ui.group(|ui| {
                                    ui.label(
                                        RichText::new(&c.description)
                                            .strong()
                                            .color(pal::WARN),
                                    );
                                    if !c.candidate_paths.is_empty() {
                                        ui.label(
                                            RichText::new("Paths involved:")
                                                .color(pal::TEXT_DIM)
                                                .size(11.0),
                                        );
                                        for p in &c.candidate_paths {
                                            ui.label(
                                                RichText::new(format!("  - {}", p.display()))
                                                    .monospace()
                                                    .size(11.0),
                                            );
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
                                        ui.label(RichText::new(&m.release_title).strong());
                                        ui.label(
                                            RichText::new(m.candidate_path.display().to_string())
                                                .color(pal::TEXT_DIM)
                                                .monospace(),
                                        );
                                        ui.end_row();
                                    }
                                });
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
                    let mut decisions = Vec::new();

                    // Matched decisions
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

                    // Review decisions
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

                    // Restorable decisions
                    for m in &modal.report.restorable {
                        if modal.selected_restorable.contains(&m.release_key) {
                            decisions.push(MatchDecision::Restore {
                                key: m.release_key,
                                path: m.candidate_path.clone(),
                            });
                        }
                    }

                    // Local-only decisions (Ignore / Attach)
                    for (path, action) in &modal.local_decisions {
                        match action {
                            LocalAction::Ignore => {
                                decisions.push(MatchDecision::Ignore {
                                    band_id: modal.report.band_id,
                                    path: path.clone(),
                                });
                            }
                            LocalAction::Attach(key) => {
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
                            LocalAction::Leave => {}
                        }
                    }

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
