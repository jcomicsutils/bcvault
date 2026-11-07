use crate::{
    bandcamp::redeem::validate_redeem_url,
    types::{DownloadOpts, FormatFallback, OutputLayout},
    ui::{
        state::UiState,
        theme::pal,
        widgets::{sharp_button, sharp_rect},
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align2, Color32, Context, Id, Order, RichText, Sense, Stroke};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(ctx: &Context, state: &mut UiState, cmd_tx: &UnboundedSender<UiToWorkerCommand>) {
    let any_modal = state.redeem_modal.is_some()
        || state.quota_modal.is_some()
        || state.settings_draft.is_some()
        || state.confirm_remove_artist.is_some();

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
            ui.set_min_width(440.0);
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
                            "By Artist Folder",
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

            ui.separator();
            ui.horizontal(|ui| {
                if sharp_button(ui, "Save").clicked() {
                    draft.quota_soft = draft.quota_soft.min(draft.quota_hard);
                    draft.batch_limit = draft.batch_limit.min(draft.quota_hard);
                    state.settings = draft.clone();
                    let _ = cmd_tx.send(UiToWorkerCommand::UpdateSettings(draft.clone()));
                    keep_open = false;
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
