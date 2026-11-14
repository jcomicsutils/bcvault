use super::TabCtx;
use crate::paint;
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    ui.label(
        RichText::new("ADVANCED")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    let btn = paint::sharp_button(ui, "Export Settings JSON");
    if ctx.focus_first {
        btn.request_focus();
    }
    if btn.clicked()
        && let Ok(json) = serde_json::to_string_pretty(ctx.draft)
    {
        crate::services::export_settings_json(ctx.rt_handle, ctx.draft.out_dir.clone(), json);
    }

    if let Some(paths) = bcvault_core::paths::AppPaths::resolve() {
        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("PATHS").monospace().strong().color(th.accent));
        ui.add_space(4.0);
        egui::Grid::new("fx_settings_paths_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Data directory:");
                ui.label(
                    RichText::new(paths.data_dir.display().to_string())
                        .monospace()
                        .color(th.text_dim),
                );
                ui.end_row();

                ui.label("Cover cache:");
                ui.label(
                    RichText::new(paths.cover_cache_dir.display().to_string())
                        .monospace()
                        .color(th.text_dim),
                );
                ui.end_row();
            });
    }
}
