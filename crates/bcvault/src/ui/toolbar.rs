use crate::{
    types::{AudioFormat, DownloadOpts, EnqueueMode, ReleaseFilter},
    ui::{
        state::{ClassicRemainingTarget, UiState},
        theme::pal,
        widgets::sharp_button,
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{CornerRadius, RichText, Ui};
use tokio::sync::mpsc::UnboundedSender;

const BATCH_LIMITS: [u32; 5] = [25, 50, 100, 150, 200];

pub fn show(
    ui: &mut Ui,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
) {
    // Row 1: Filter chips + Search box + Settings button
    ui.horizontal(|ui| {
        for f in ReleaseFilter::ALL {
            let changed = ui
                .selectable_value(&mut state.filter, f, f.label())
                .changed();
            if changed {
                state.mark_query_immediate();
            }
        }

        ui.separator();

        let search_id = ui.make_persistent_id("release_search_box");
        let search_resp = ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .id(search_id)
                .hint_text("Search releases or artists (Ctrl+F)...")
                .desired_width(240.0),
        );
        if state.focus_search_next_frame {
            search_resp.request_focus();
            state.focus_search_next_frame = false;
        }
        if search_resp.changed() {
            state.mark_query_debounced();
        }
        if !state.search.is_empty() && sharp_button(ui, "X").clicked() {
            state.search.clear();
            state.mark_query_immediate();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if sharp_button(ui, "Settings").clicked() {
                state.open_settings_modal();
            }
            if sharp_button(ui, "Link Folder to Artist...").clicked() {
                state.link_folder_modal = Some(crate::ui::state::LinkFolderModalState::new_empty(
                    state.active_band_id(),
                ));
            }
        });
    });

    ui.separator();

    // Row 2: Format ComboBox, Batch-limit ComboBox, Output folder button, Download Selected, Download All Eligible, Quota readout
    ui.horizontal(|ui| {
        ui.label(RichText::new("Format:").color(pal::TEXT_DIM));
        let prev_fmt = state.settings.format;
        egui::ComboBox::from_id_salt("toolbar_format_combo")
            .selected_text(state.settings.format.label())
            .width(96.0)
            .show_ui(ui, |ui| {
                for fmt in AudioFormat::ALL {
                    ui.selectable_value(&mut state.settings.format, fmt, fmt.label());
                }
            });
        if state.settings.format != prev_fmt {
            let _ = cmd_tx.send(UiToWorkerCommand::UpdateSettings(state.settings.clone()));
        }

        ui.separator();

        ui.label(RichText::new("Batch Limit:").color(pal::TEXT_DIM));
        let prev_batch = state.settings.batch_limit;
        let selected_batch_text = if state.settings.batch_limit > state.settings.quota_soft {
            RichText::new(state.settings.batch_limit.to_string()).color(pal::WARN)
        } else {
            RichText::new(state.settings.batch_limit.to_string()).color(pal::TEXT)
        };
        egui::ComboBox::from_id_salt("toolbar_batch_combo")
            .selected_text(selected_batch_text)
            .width(68.0)
            .show_ui(ui, |ui| {
                for &lim in &BATCH_LIMITS {
                    if lim <= state.settings.quota_hard {
                        let label = if lim > state.settings.quota_soft {
                            RichText::new(lim.to_string()).color(pal::WARN)
                        } else {
                            RichText::new(lim.to_string()).color(pal::TEXT)
                        };
                        ui.selectable_value(&mut state.settings.batch_limit, lim, label);
                    }
                }
            });
        if state.settings.batch_limit != prev_batch {
            let _ = cmd_tx.send(UiToWorkerCommand::UpdateSettings(state.settings.clone()));
        }

        ui.separator();

        let dir_str = state.settings.out_dir.to_string_lossy();
        let short_dir = truncate_left(&dir_str, 28);
        if sharp_button(ui, &format!("Out: {short_dir}"))
            .on_hover_text(dir_str.as_ref())
            .clicked()
        {
            let tx_clone = cmd_tx.clone();
            let mut settings_clone = state.settings.clone();
            rt_handle.spawn(async move {
                if let Some(folder) = rfd::AsyncFileDialog::new()
                    .set_directory(&settings_clone.out_dir)
                    .pick_folder()
                    .await
                {
                    settings_clone.out_dir = folder.path().to_path_buf();
                    let _ = tx_clone.send(UiToWorkerCommand::UpdateSettings(settings_clone));
                }
            });
        }

        ui.separator();

        let stats = state.selection_stats();
        let sel_count = stats.selected;
        if ui
            .add_enabled(
                sel_count > 0,
                egui::Button::new(format!("Download Selected ({sel_count})"))
                    .corner_radius(CornerRadius::ZERO),
            )
            .clicked()
        {
            let items: Vec<_> = state.selected.iter().copied().collect();
            let opts = DownloadOpts {
                format: state.settings.format,
                out_dir: state.settings.out_dir.clone(),
                batch_limit: state.settings.batch_limit,
            };
            let _ = cmd_tx.send(UiToWorkerCommand::EnqueueDownloads {
                items,
                opts,
                ack_soft_quota: false,
                mode: EnqueueMode::New,
            });
        }

        if ui
            .add_enabled(
                stats.remaining_in_view > 0,
                egui::Button::new(format!("Select remaining ({})", stats.remaining_in_view))
                    .corner_radius(CornerRadius::ZERO),
            )
            .clicked()
        {
            state.select_remaining();
        }

        if ui
            .add_enabled(
                sel_count > 0,
                egui::Button::new("Clear").corner_radius(CornerRadius::ZERO),
            )
            .clicked()
        {
            state.clear_selection();
        }

        let rem_view = stats.remaining_in_view;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            if ui
                .add_enabled(
                    rem_view > 0,
                    egui::Button::new(format!("Download remaining ({rem_view})"))
                        .corner_radius(CornerRadius::ZERO),
                )
                .clicked()
            {
                state.trigger_download_remaining(ClassicRemainingTarget::View, cmd_tx);
            }
            let has_artist = !state.selected_artists.is_empty() || state.active_band_id().is_some();
            let has_any_artist = !state.artists.is_empty();
            ui.menu_button("▾", |ui| {
                if ui
                    .add_enabled(rem_view > 0, egui::Button::new("In this view"))
                    .clicked()
                {
                    state.trigger_download_remaining(ClassicRemainingTarget::View, cmd_tx);
                    ui.close();
                }
                if ui
                    .add_enabled(has_artist, egui::Button::new("Selected artists"))
                    .clicked()
                {
                    state.trigger_download_remaining(
                        ClassicRemainingTarget::SelectedArtists,
                        cmd_tx,
                    );
                    ui.close();
                }
                if ui
                    .add_enabled(has_any_artist, egui::Button::new("All artists..."))
                    .clicked()
                {
                    state.trigger_download_remaining(ClassicRemainingTarget::AllArtists, cmd_tx);
                    ui.close();
                }
            });
        });

        if let Some((ref msg, is_warn, expires_at)) = state.enqueue_status {
            if std::time::Instant::now() < expires_at {
                let color = if is_warn { pal::WARN } else { pal::TEXT_DIM };
                ui.label(RichText::new(msg).size(11.5).color(color));
            } else {
                state.enqueue_status = None;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(bid) = state.active_band_id()
                && let Some(artist) = state.artists.iter().find(|a| a.band_id == bid)
            {
                let used = artist.stats.quota_used_30d;
                let hard = state.settings.quota_hard;
                let color = if used >= hard {
                    pal::ERR
                } else if used >= state.settings.quota_soft {
                    pal::WARN
                } else {
                    pal::TEXT_DIM
                };
                ui.label(
                    RichText::new(format!("your downloads (30d): {used}/{hard}"))
                        .monospace()
                        .color(color),
                );
            }
        });
    });
}

fn truncate_left(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let start = chars.len() - max_chars;
        let tail: String = chars[start..].iter().collect();
        format!("...{tail}")
    }
}
