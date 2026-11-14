use super::TabCtx;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    ui.label(egui::RichText::new("Appearance").strong());
    ui.add_space(4.0);
    egui::Grid::new("classic_settings_appearance_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Cover Thumbnails:");
            let resp = ui.checkbox(
                &mut ctx.draft.show_thumbnails,
                "Show 20px cover thumbnails in table",
            );
            if ctx.focus_first {
                resp.request_focus();
            }
            ui.end_row();
        });
}
