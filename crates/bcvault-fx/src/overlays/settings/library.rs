use super::TabCtx;
use bcvault_core::types::{DeletedPolicy, MATCH_MIN_RANGE, OnMissing};
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    ui.label(
        RichText::new("LIBRARY BEHAVIOUR")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    let first = ui.checkbox(
        &mut ctx.draft.verify_on_sync,
        "Verify files after every sync",
    );
    if ctx.focus_first {
        first.request_focus();
    }

    ui.checkbox(
        &mut ctx.draft.rescan_linked_on_sync,
        "Re-read linked folders on sync (auto-adopt certain matches)",
    );

    ui.checkbox(
        &mut ctx.draft.auto_relink,
        "Try to find moved albums automatically",
    );

    ui.horizontal(|ui| {
        ui.label("When files are missing:")
            .on_hover_text("Ask: show resolve missing dialog on sync\nHandle automatically: apply deleted policy\nNote: an unavailable folder always asks");
        for om in [OnMissing::Ask, OnMissing::Auto] {
            let active = ctx.draft.on_missing == om;
            let lbl = match om {
                OnMissing::Ask => "Ask",
                OnMissing::Auto => "Handle automatically",
            };
            if ui.selectable_label(active, lbl).clicked() {
                ctx.draft.on_missing = om;
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("If deleted:")
            .on_hover_text("Keep marked: mark as archived and keep downloaded = 1\nMark as not downloaded: clear downloaded mark\n(used when handled automatically, and as the default in the dialog)");
        for dp in [DeletedPolicy::KeepMarked, DeletedPolicy::MarkNotDownloaded] {
            let active = ctx.draft.deleted_policy == dp;
            let lbl = match dp {
                DeletedPolicy::KeepMarked => "Keep marked as downloaded (archived)",
                DeletedPolicy::MarkNotDownloaded => "Mark as not downloaded",
            };
            if ui.selectable_label(active, lbl).clicked() {
                ctx.draft.deleted_policy = dp;
            }
        }
    });

    ui.add_space(8.0);
    egui::CollapsingHeader::new("Matching thresholds (advanced)")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("fx_settings_thresholds_grid")
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
