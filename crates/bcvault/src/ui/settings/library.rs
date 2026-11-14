use super::TabCtx;
use crate::types::{DeletedPolicy, MATCH_MIN_RANGE, OnMissing};
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    ui.label(RichText::new("Library").strong());
    ui.add_space(4.0);
    egui::Grid::new("classic_settings_library_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Verify on Sync:");
            let first = ui.checkbox(
                &mut ctx.draft.verify_on_sync,
                "Verify files after every sync",
            );
            if ctx.focus_first {
                first.request_focus();
            }
            ui.end_row();

            ui.label("Rescan on Sync:");
            ui.checkbox(
                &mut ctx.draft.rescan_linked_on_sync,
                "Re-read linked folders on sync",
            );
            ui.end_row();

            ui.label("Auto-Relink:");
            ui.checkbox(
                &mut ctx.draft.auto_relink,
                "Try to find moved albums automatically",
            );
            ui.end_row();

            ui.label("When Files Missing:")
                .on_hover_text("Ask: show resolve missing dialog on sync\nHandle automatically: apply deleted policy\nNote: an unavailable folder always asks");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut ctx.draft.on_missing, OnMissing::Ask, "Ask");
                ui.selectable_value(
                    &mut ctx.draft.on_missing,
                    OnMissing::Auto,
                    "Handle automatically",
                );
            });
            ui.end_row();

            ui.label("If Deleted:")
                .on_hover_text("Keep marked: mark as archived and keep downloaded = 1\nMark as not downloaded: clear downloaded mark\n(used when handled automatically, and as the default in the dialog)");
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut ctx.draft.deleted_policy,
                    DeletedPolicy::KeepMarked,
                    "Keep marked as downloaded",
                );
                ui.selectable_value(
                    &mut ctx.draft.deleted_policy,
                    DeletedPolicy::MarkNotDownloaded,
                    "Mark as not downloaded",
                );
            });
            ui.end_row();
        });

    ui.add_space(8.0);
    egui::CollapsingHeader::new("Matching thresholds (advanced)")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("classic_settings_thresholds_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Auto-Match Min Score:")
                        .on_hover_text("Score threshold (0.50 - 1.00) required to adopt non-exact (fuzzy) matches automatically.");
                    ui.add(
                        egui::Slider::new(&mut ctx.draft.auto_match_min, MATCH_MIN_RANGE)
                            .step_by(0.01)
                            .custom_formatter(|n, _| format!("{n:.2}")),
                    );
                    ui.end_row();

                    ui.label("Review Min Score:")
                        .on_hover_text("Minimum score threshold (0.50 - 1.00) to propose non-exact (fuzzy) matches for review.");
                    ui.add(
                        egui::Slider::new(&mut ctx.draft.review_match_min, MATCH_MIN_RANGE)
                            .step_by(0.01)
                            .custom_formatter(|n, _| format!("{n:.2}")),
                    );
                    ui.end_row();
                });
        });
}
