use super::TabCtx;
use crate::ui::theme::pal;
use crate::ui::widgets::sharp_button;
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    ui.label(RichText::new("Advanced").strong());
    ui.add_space(4.0);

    let btn = sharp_button(ui, "Copy Settings JSON");
    if ctx.focus_first {
        btn.request_focus();
    }
    if btn.clicked()
        && let Ok(json) = serde_json::to_string_pretty(ctx.draft)
    {
        ui.ctx().copy_text(json);
    }

    if let Some(paths) = ctx.paths {
        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("Paths").strong());
        ui.add_space(4.0);
        egui::Grid::new("classic_settings_paths_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Data directory:");
                ui.label(
                    RichText::new(paths.data_dir.display().to_string())
                        .monospace()
                        .color(pal::TEXT_DIM),
                );
                ui.end_row();

                ui.label("Cover cache:");
                ui.label(
                    RichText::new(paths.cover_cache_dir.display().to_string())
                        .monospace()
                        .color(pal::TEXT_DIM),
                );
                ui.end_row();
            });
    }
}
