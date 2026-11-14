use super::TabCtx;
use crate::types::{FormatFallback, OutputLayout};
use crate::ui::theme::pal;
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    ui.label(RichText::new("Downloads & Storage").strong());
    ui.add_space(4.0);
    egui::Grid::new("classic_settings_downloads_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Output Layout:");
            ui.horizontal(|ui| {
                let first = ui.selectable_value(&mut ctx.draft.layout, OutputLayout::Flat, "Flat");
                if ctx.focus_first {
                    first.request_focus();
                }
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
