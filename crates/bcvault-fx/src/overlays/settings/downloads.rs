use super::TabCtx;
use crate::paint;
use bcvault_core::types::{AudioFormat, BATCH_LIMIT_PRESETS, FormatFallback, OutputLayout};
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    ui.label(
        RichText::new("DOWNLOADS")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    egui::Grid::new("fx_settings_downloads_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Output Directory:");
            ui.horizontal(|ui| {
                let mut p_str = ctx.draft.out_dir.to_string_lossy().into_owned();
                let resp = ui.add(egui::TextEdit::singleline(&mut p_str).desired_width(280.0));
                if ctx.focus_first {
                    resp.request_focus();
                }
                if resp.changed() {
                    ctx.draft.out_dir = std::path::PathBuf::from(p_str);
                }
                if paint::sharp_button(ui, "Browse...").clicked() {
                    let slot = ctx.picked_out_dir.clone();
                    ctx.rt_handle.spawn(async move {
                        if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await
                            && let Ok(mut lock) = slot.lock()
                        {
                            *lock = Some(folder.path().to_path_buf());
                        }
                    });
                }
            });
            ui.end_row();

            ui.label("Output Layout:");
            egui::ComboBox::from_id_salt("fx_set_layout")
                .selected_text(ctx.draft.layout.label())
                .show_ui(ui, |ui| {
                    for l in OutputLayout::ALL {
                        ui.selectable_value(&mut ctx.draft.layout, l, l.label());
                    }
                });
            ui.end_row();

            ui.label("Filename Pattern:");
            ui.vertical(|ui| {
                if ui
                    .add(
                        egui::TextEdit::singleline(ctx.filename_pattern_input)
                            .hint_text("{artist} - {album}")
                            .desired_width(320.0),
                    )
                    .changed()
                {
                    ctx.draft.filename_pattern = ctx.filename_pattern_input.clone();
                }
                ui.label(
                    RichText::new(
                        "Tags: {artist}, {page_artist}, {album}, {title}, {year}, {date}, {format}, {type}, {id}",
                    )
                    .size(10.5)
                    .color(th.text_dim),
                );
            });
            ui.end_row();

            ui.label("Preferred Format:");
            egui::ComboBox::from_id_salt("fx_set_fmt")
                .selected_text(ctx.draft.format.label())
                .show_ui(ui, |ui| {
                    for f in AudioFormat::ALL {
                        ui.selectable_value(&mut ctx.draft.format, f, f.label());
                    }
                });
            ui.end_row();

            ui.label("Format Fallback:");
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut ctx.draft.format_fallback,
                    FormatFallback::Nearest,
                    "Nearest Available",
                );
                ui.selectable_value(
                    &mut ctx.draft.format_fallback,
                    FormatFallback::Strict,
                    "Strict (Fail if missing)",
                );
            });
            ui.end_row();

            ui.label("Batch Limit:");
            egui::ComboBox::from_id_salt("fx_set_batch_limit")
                .selected_text(ctx.draft.batch_limit.to_string())
                .show_ui(ui, |ui| {
                    for &lim in &BATCH_LIMIT_PRESETS {
                        if lim <= ctx.draft.quota_hard {
                            ui.selectable_value(&mut ctx.draft.batch_limit, lim, lim.to_string());
                        }
                    }
                });
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    ui.label(
        RichText::new("ZIP FILES")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    egui::Grid::new("fx_settings_zip_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Archive Extraction:");
            ui.vertical(|ui| {
                ui.checkbox(
                    &mut ctx.draft.auto_extract,
                    "Extract downloaded ZIP archives",
                );
                ui.label(
                    RichText::new(
                        "Bandcamp album downloads arrive as .zip files. When extraction is off, the ZIP itself is kept as your download.",
                    )
                    .size(10.5)
                    .color(th.text_dim),
                );
            });
            ui.end_row();

            ui.label("Cleanup ZIP:");
            ui.vertical(|ui| {
                ui.add_enabled(
                    ctx.draft.auto_extract,
                    egui::Checkbox::new(
                        &mut ctx.draft.delete_zip_after_extract,
                        "Delete the ZIP after successful extraction",
                    ),
                );
                ui.label(
                    RichText::new(
                        "Only the downloaded .zip is removed, and only after every file was extracted successfully. Off by default.",
                    )
                    .size(10.5)
                    .color(th.text_dim),
                );
                if !ctx.draft.auto_extract {
                    ui.label(
                        RichText::new("Enable ZIP extraction above to configure cleanup.")
                            .size(10.5)
                            .color(th.text_dim),
                    );
                }
            });
            ui.end_row();
        });
}
