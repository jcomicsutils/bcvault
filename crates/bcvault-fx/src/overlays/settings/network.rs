use super::TabCtx;
use crate::paint;
use bcvault_core::types::{
    MAX_CONCURRENT_RANGE, PAGE_DELAY_RANGE, PAGE_JITTER_RANGE, QUOTA_HARD_RANGE, QUOTA_SOFT_RANGE,
    Settings,
};
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    let defaults = Settings::default();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("30-DAY QUOTA")
                .monospace()
                .strong()
                .color(th.accent),
        );
        if paint::sharp_small_button(ui, "Reset").clicked() {
            ctx.draft.quota_soft = defaults.quota_soft;
            ctx.draft.quota_hard = defaults.quota_hard;
        }
    });
    ui.add_space(4.0);

    egui::Grid::new("fx_settings_quota_grid")
        .num_columns(3)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Soft Quota Warning (30d):");
            let first = ui.add(egui::Slider::new(
                &mut ctx.draft.quota_soft,
                QUOTA_SOFT_RANGE,
            ));
            if ctx.focus_first {
                first.request_focus();
            }
            ui.label(
                RichText::new(format!("default: {}", defaults.quota_soft))
                    .size(10.5)
                    .color(th.text_dim),
            );
            ui.end_row();

            ui.label("Hard Quota Cap (30d):");
            ui.add(egui::Slider::new(
                &mut ctx.draft.quota_hard,
                QUOTA_HARD_RANGE,
            ));
            ui.label(
                RichText::new(format!("default: {}", defaults.quota_hard))
                    .size(10.5)
                    .color(th.text_dim),
            );
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("NETWORK & PACING")
                .monospace()
                .strong()
                .color(th.accent),
        );
        if paint::sharp_small_button(ui, "Reset").clicked() {
            ctx.draft.page_delay_ms = defaults.page_delay_ms;
            ctx.draft.page_jitter_pct = defaults.page_jitter_pct;
            ctx.draft.max_concurrent_downloads = defaults.max_concurrent_downloads;
            ctx.draft.user_agent = defaults.user_agent.clone();
        }
    });
    ui.add_space(4.0);

    egui::Grid::new("fx_settings_network_grid")
        .num_columns(3)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Page Request Delay (ms):");
            ui.add(egui::Slider::new(
                &mut ctx.draft.page_delay_ms,
                PAGE_DELAY_RANGE,
            ));
            ui.label(
                RichText::new(format!("default: {}", defaults.page_delay_ms))
                    .size(10.5)
                    .color(th.text_dim),
            );
            ui.end_row();

            ui.label("Page Jitter (%):");
            ui.add(egui::Slider::new(
                &mut ctx.draft.page_jitter_pct,
                PAGE_JITTER_RANGE,
            ));
            ui.label(
                RichText::new(format!("default: {}", defaults.page_jitter_pct))
                    .size(10.5)
                    .color(th.text_dim),
            );
            ui.end_row();

            ui.label("Max Concurrent Downloads:");
            ui.add(egui::Slider::new(
                &mut ctx.draft.max_concurrent_downloads,
                MAX_CONCURRENT_RANGE,
            ));
            ui.label(
                RichText::new(format!("default: {}", defaults.max_concurrent_downloads))
                    .size(10.5)
                    .color(th.text_dim),
            );
            ui.end_row();

            ui.label("User-Agent:");
            ui.add(egui::TextEdit::singleline(&mut ctx.draft.user_agent).desired_width(240.0));
            ui.label(
                RichText::new("default: bcvault/...")
                    .size(10.5)
                    .color(th.text_dim),
            );
            ui.end_row();
        });
}
