use crate::{paint, state::FxState};
use egui::{Align2, CornerRadius, Order, RichText, Stroke};

pub fn show_debug_overlay(ctx: &egui::Context, state: &FxState) {
    if !state.show_debug_overlay {
        return;
    }

    let th = state.active_theme;
    egui::Area::new(egui::Id::new("fx_debug_f3"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_TOP, [-12.0, 48.0])
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(th.bg0)
                .stroke(Stroke::new(1.0_f32, th.accent))
                .corner_radius(CornerRadius::ZERO)
                .inner_margin(8.0_f32)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("F3 FX DIAGNOSTICS")
                            .monospace()
                            .strong()
                            .color(th.accent),
                    );
                    let tex_mb = (state.tex_cache.used_bytes() as f64) / (1024.0 * 1024.0);
                    let cap_mb = (state.tex_cache.budget_bytes() as f64) / (1024.0 * 1024.0);
                    ui.label(
                        RichText::new(format!(
                            "frame:       {:5.2} ms\nsprings:     {} active ({} tracked)\ntexture RAM: {:5.1} / {:5.0} MB\nuploads/f:   {} (queue: {})\nrows:        {} ({} sel)\neffects:     {} (reduced: {})\nevent drain: {}/frame",
                            state.last_frame_ms,
                            state.motion.active_springs(),
                            state.motion.entry_count(),
                            tex_mb,
                            cap_mb,
                            state.tex_cache.uploads_last_frame(),
                            state.tex_cache.pending_upload_count(),
                            state.rows.len(),
                            state.selected_keys.len(),
                            state.prefs.effects.label(),
                            state.prefs.reduced_motion,
                            state.last_event_backlog,
                        ))
                        .monospace()
                        .size(11.0)
                        .color(th.text),
                    );
                    if let Some(reason) = &state.shader_disabled_reason {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(ui.available_width(), 2.0), egui::Sense::hover());
                        paint::rect_filled(ui.painter(), r, th.warn);
                        ui.label(
                            RichText::new(format!("shader fallback: {reason}"))
                                .monospace()
                                .size(10.0)
                                .color(th.warn),
                        );
                    }
                });
        });
}
