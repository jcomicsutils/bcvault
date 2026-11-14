use super::TabCtx;
use crate::types::{AudioFormat, BATCH_LIMIT_PRESETS, FormatFallback, OutputLayout};
use crate::ui::theme::pal;
use crate::ui::widgets::sharp_button;
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    ui.label(RichText::new("Downloads & Storage").strong());
    ui.add_space(4.0);
    egui::Grid::new("classic_settings_downloads_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Output Folder:");
            ui.horizontal(|ui| {
                let mut p_str = ctx.draft.out_dir.to_string_lossy().into_owned();
                let resp = ui.add(egui::TextEdit::singleline(&mut p_str).desired_width(280.0));
                if ctx.focus_first {
                    resp.request_focus();
                }
                if resp.changed() {
                    ctx.draft.out_dir = std::path::PathBuf::from(p_str);
                }
                if sharp_button(ui, "Browse...").clicked() {
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
            ui.horizontal(|ui| {
                ui.selectable_value(&mut ctx.draft.layout, OutputLayout::Flat, "Flat");
                ui.selectable_value(
                    &mut ctx.draft.layout,
                    OutputLayout::ByArtist,
                    "By Release Artist",
                );
                ui.selectable_value(
                    &mut ctx.draft.layout,
                    OutputLayout::ByPageArtist,
                    "By Page Artist",
                );
            });
            ui.end_row();

            ui.label("Filename Pattern:")
                .on_hover_text("Tags: {artist}, {page_artist}, {album} / {title}, {year}, {date}, {format}, {type}, {id}\nLeave blank to use default / Bandcamp server filename.");
            ui.vertical(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut ctx.draft.filename_pattern)
                        .hint_text("Default (e.g. {artist} - {album} or {year} - {artist} - {album})")
                        .desired_width(300.0),
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

            ui.label("Preferred Format:");
            egui::ComboBox::from_id_salt("classic_settings_format_combo")
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
            egui::ComboBox::from_id_salt("classic_settings_batch_limit_combo")
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
    ui.label(RichText::new("ZIP files").strong());
    ui.add_space(4.0);
    egui::Grid::new("classic_settings_zip_grid")
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
                    .size(11.0)
                    .color(pal::TEXT_DIM),
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
                    .size(11.0)
                    .color(pal::TEXT_DIM),
                );
                if !ctx.draft.auto_extract {
                    ui.label(
                        RichText::new("Enable ZIP extraction above to configure cleanup.")
                            .size(10.5)
                            .color(pal::TEXT_DIM),
                    );
                }
            });
            ui.end_row();
        });
}
