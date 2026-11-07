use crate::{motion::SpringCfg, paint, services::messages::ToastKind, state::FxState};
use bcvault_core::worker::messages::UiToWorkerCommand;
use egui::{Align2, CornerRadius, Order, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use tokio::sync::mpsc::UnboundedSender;

pub fn show_toasts(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    now_s: f64,
) {
    // Auto-expire non-error toasts after 4.0 seconds
    state
        .toasts
        .retain(|t| t.kind == ToastKind::Error || (now_s - t.created_s) < 4.0);

    if state.toasts.is_empty() {
        return;
    }

    let th = state.active_theme;
    let mut dismiss_id: Option<u64> = None;
    let toasts = state.toasts.clone();

    egui::Area::new(egui::Id::new("fx_toasts_stack"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, [-16.0, -140.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                for t in &toasts {
                    let slide = state
                        .motion
                        .val_from(("toast_in", t.id), 0.0, 1.0, SpringCfg::POP);
                    let col = match t.kind {
                        ToastKind::Info => th.accent,
                        ToastKind::Success => th.ok,
                        ToastKind::Warn => th.warn,
                        ToastKind::Error => th.err,
                    };

                    egui::Frame::NONE
                        .fill(th.bg0)
                        .stroke(Stroke::new(1.0_f32, col))
                        .corner_radius(CornerRadius::ZERO)
                        .inner_margin(8.0_f32)
                        .show(ui, |ui| {
                            ui.set_width(320.0 * slide.clamp(0.2, 1.0));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&t.message).size(12.0).color(th.text),
                                    )
                                    .wrap(),
                                );
                                if let Some(batch_id) = t.undo_batch_id
                                    && paint::sharp_button(ui, "Undo").clicked()
                                {
                                    let _ = core_tx.send(UiToWorkerCommand::UndoBatch { batch_id });
                                    dismiss_id = Some(t.id);
                                }
                                if paint::sharp_button(ui, "×").clicked() {
                                    dismiss_id = Some(t.id);
                                }
                            });

                            if t.kind != ToastKind::Error {
                                let rem =
                                    (1.0 - ((now_s - t.created_s) / 4.0) as f32).clamp(0.0, 1.0);
                                let (bar_r, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 2.0),
                                    Sense::hover(),
                                );
                                let fill_r = Rect::from_min_max(
                                    bar_r.min,
                                    Pos2::new(bar_r.min.x + bar_r.width() * rem, bar_r.max.y),
                                );
                                paint::rect_filled(ui.painter(), fill_r, col);
                                ctx.request_repaint();
                            }
                        });
                }
            });
        });

    if let Some(id) = dismiss_id {
        state.toasts.retain(|t| t.id != id);
    }
}
