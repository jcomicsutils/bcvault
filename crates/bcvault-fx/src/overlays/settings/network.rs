use super::TabCtx;
use bcvault_core::types::{QUOTA_HARD_RANGE, QUOTA_SOFT_RANGE};
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    ui.label(
        RichText::new("QUOTA & NETWORK")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    egui::Grid::new("fx_settings_network_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Soft Quota Warning (30d):");
            let first =
                ui.add(egui::DragValue::new(&mut ctx.draft.quota_soft).range(QUOTA_SOFT_RANGE));
            if ctx.focus_first {
                first.request_focus();
            }
            ui.end_row();

            ui.label("Hard Quota Cap (30d):");
            ui.add(egui::DragValue::new(&mut ctx.draft.quota_hard).range(QUOTA_HARD_RANGE));
            ui.end_row();
        });
}
