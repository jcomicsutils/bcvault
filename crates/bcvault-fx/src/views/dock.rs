use crate::{
    motion::{EffectsLevel, SpringCfg},
    paint,
    services::messages::Tier,
    state::{FxState, paint_cover_slot},
};
use bcvault_core::{
    types::{JobState, RateLimitStatus},
    worker::messages::UiToWorkerCommand,
};
use egui::{Align, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(
    ui: &mut Ui,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    now_s: f64,
) {
    let th = state.active_theme;

    // Header row: Queue / Logs tabs + Speed Sparkline + Gate Status Chip + Queue Controls
    ui.horizontal(|ui| {
        if ui
            .selectable_label(
                state.dock_tab == 0,
                RichText::new(format!("QUEUE ({})", state.jobs.len()))
                    .monospace()
                    .strong(),
            )
            .clicked()
        {
            state.dock_tab = 0;
        }
        if ui
            .selectable_label(
                state.dock_tab == 1,
                RichText::new(format!("LOGS ({})", state.logs.len()))
                    .monospace()
                    .strong(),
            )
            .clicked()
        {
            state.dock_tab = 1;
        }

        ui.separator();

        if paint::sharp_button(ui, "Pause").clicked() {
            let _ = core_tx.send(UiToWorkerCommand::PauseQueue);
        }
        if paint::sharp_button(ui, "Resume").clicked() {
            let _ = core_tx.send(UiToWorkerCommand::ResumeQueue);
        }
        if paint::sharp_button(ui, "Cancel All").clicked() {
            let _ = core_tx.send(UiToWorkerCommand::CancelAllJobs);
        }
        if paint::sharp_button(ui, "Clear Finished").clicked() {
            let _ = core_tx.send(UiToWorkerCommand::ClearFinished);
            state
                .jobs
                .retain(|j| !matches!(j.view.state, JobState::Done { .. } | JobState::Cancelled));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Gate chip: OK / PACING / COOLDOWN 00:41
            draw_gate_chip(ui, state, now_s);

            ui.separator();

            // 30s stepped square speed sparkline
            let (spark_rect, _) = ui.allocate_exact_size(Vec2::new(120.0, 20.0), Sense::hover());
            paint::rect(
                ui.painter(),
                spark_rect,
                th.bg0,
                Stroke::new(1.0_f32, th.border),
            );
            let max_speed = state
                .speed_samples
                .iter()
                .copied()
                .fold(1024.0_f64, f64::max);
            let n = state.speed_samples.len().max(1);
            let bw = (spark_rect.width() - 4.0) / (n as f32);
            for (i, &spd) in state.speed_samples.iter().enumerate() {
                let frac = ((spd / max_speed) as f32).clamp(0.0, 1.0);
                let bh =
                    (frac * (spark_rect.height() - 4.0)).max(if spd > 0.0 { 2.0 } else { 0.0 });
                if bh > 0.0 {
                    let bar = Rect::from_min_max(
                        Pos2::new(
                            spark_rect.min.x + 2.0 + (i as f32) * bw,
                            spark_rect.max.y - 2.0 - bh,
                        ),
                        Pos2::new(
                            spark_rect.min.x + 2.0 + ((i + 1) as f32) * bw - 1.0,
                            spark_rect.max.y - 2.0,
                        ),
                    );
                    paint::rect_filled(ui.painter(), bar, th.accent);
                }
            }

            let cur_bps: f64 = state.jobs.iter().map(|j| j.speed_bps).sum();
            let mb_s = cur_bps / (1024.0 * 1024.0);
            ui.label(
                RichText::new(format!("{mb_s:4.2} MB/s"))
                    .monospace()
                    .size(11.0)
                    .color(th.text_dim),
            );
        });
    });

    ui.separator();

    if state.dock_tab == 0 {
        let jobs = state.jobs.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 26.0, jobs.len(), |ui, range| {
                for idx in range {
                    let job = &jobs[idx];
                    ui.horizontal(|ui| {
                        let (cov_r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        let art_id = state
                            .rows
                            .iter()
                            .find(|r| r.rel.key == job.view.key)
                            .and_then(|r| r.art_id);
                        paint_cover_slot(
                            ui.painter(),
                            state,
                            art_id,
                            job.view.key.id,
                            cov_r,
                            Tier::T64,
                            1.0,
                            Vec2::ZERO,
                        );

                        ui.add_sized(
                            [220.0, 22.0],
                            egui::Label::new(
                                RichText::new(&job.view.label).size(12.0).color(th.text),
                            )
                            .truncate(),
                        );

                        // Progress bar with SMOOTH spring and moving 45° hatch shimmer
                        let is_done = matches!(job.view.state, JobState::Done { .. });
                        let is_failed = matches!(job.view.state, JobState::Failed(_));
                        let real_frac = match job.bytes_total {
                            Some(tot) if tot > 0 => {
                                (job.bytes_done as f32 / tot as f32).clamp(0.0, 1.0)
                            }
                            _ if is_done => 1.0,
                            _ => 0.0,
                        };
                        let disp_frac = state.motion.val(
                            ("job_prog", job.view.id),
                            real_frac,
                            SpringCfg::SMOOTH,
                        );

                        let (bar_r, _) =
                            ui.allocate_exact_size(Vec2::new(220.0, 16.0), Sense::hover());
                        paint::rect(ui.painter(), bar_r, th.bg0, Stroke::new(1.0_f32, th.border));
                        let fill_w = (bar_r.width() * disp_frac).min(bar_r.width());
                        if fill_w > 0.5 {
                            let fill_r = Rect::from_min_max(
                                bar_r.min,
                                Pos2::new(bar_r.min.x + fill_w, bar_r.max.y),
                            );
                            let fill_col = if is_done {
                                th.ok
                            } else if is_failed {
                                th.err
                            } else {
                                th.accent
                            };
                            paint::rect_filled(ui.painter(), fill_r, fill_col.gamma_multiply(0.45));

                            if matches!(
                                job.view.state,
                                JobState::Downloading | JobState::Extracting
                            ) && state.prefs.effects != EffectsLevel::Off
                                && !state.prefs.reduced_motion
                            {
                                paint::hatch(
                                    ui.painter(),
                                    fill_r,
                                    (now_s * 1.5) as f32,
                                    10.0,
                                    Stroke::new(1.0_f32, th.accent),
                                );
                                ui.ctx().request_repaint();
                            }
                        }

                        let done_mb = (job.bytes_done as f64) / (1024.0 * 1024.0);
                        let tot_mb = job
                            .bytes_total
                            .map(|t| format!("{:.1}", (t as f64) / (1024.0 * 1024.0)))
                            .unwrap_or_else(|| "?".to_string());
                        let spd_mb = job.speed_bps / (1024.0 * 1024.0);
                        ui.label(
                            RichText::new(format!(
                                "{done_mb:5.1} / {tot_mb} MB  {spd_mb:4.2} MB/s"
                            ))
                            .monospace()
                            .size(11.0)
                            .color(th.text_dim),
                        );

                        if is_done {
                            let (chk_r, _) =
                                ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                            let p = state.motion.val_from(
                                ("job_chk", job.view.id),
                                0.0,
                                1.0,
                                SpringCfg::POP,
                            );
                            paint::drawn_check(ui.painter(), chk_r, p, Stroke::new(1.8_f32, th.ok));
                        } else if is_failed {
                            if paint::sharp_button(ui, "Retry").clicked() {
                                let _ = core_tx.send(UiToWorkerCommand::RetryJob(job.view.id));
                            }
                        } else if paint::sharp_button(ui, "Cancel").clicked() {
                            let _ = core_tx.send(UiToWorkerCommand::CancelJob(job.view.id));
                        }
                    });
                }
            });
    } else {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 20.0, state.logs.len(), |ui, range| {
                for idx in range {
                    if let Some(entry) = state.logs.get(idx) {
                        ui.label(
                            RichText::new(format!("[{}] {}", entry.scope, entry.message))
                                .monospace()
                                .size(11.0)
                                .color(th.text_dim),
                        );
                    }
                }
            });
    }
}

fn draw_gate_chip(ui: &mut Ui, state: &FxState, _now_s: f64) {
    let th = state.active_theme;
    let (label, col, frac) = match &state.rate_limit {
        RateLimitStatus::Idle => ("GATE: OK".to_string(), th.ok, 1.0_f32),
        RateLimitStatus::Pacing { next_in_ms } => {
            (format!("GATE: PACING {}ms", next_in_ms), th.accent, 0.5_f32)
        }
        RateLimitStatus::Cooldown { until, reason } => {
            let now_ts = chrono::Utc::now().timestamp();
            let rem = (*until - now_ts).max(0);
            let mins = rem / 60;
            let secs = rem % 60;
            ui.ctx().request_repaint();
            (
                format!("COOLDOWN {mins:02}:{secs:02} ({reason})"),
                th.warn,
                (rem as f32 / 60.0).clamp(0.05, 1.0),
            )
        }
    };

    let (r, _) = ui.allocate_exact_size(Vec2::new(150.0, 20.0), Sense::hover());
    paint::rect(ui.painter(), r, th.bg0, Stroke::new(1.0_f32, col));
    let bar = Rect::from_min_max(
        Pos2::new(r.min.x, r.max.y - 2.0),
        Pos2::new(r.min.x + r.width() * frac, r.max.y),
    );
    paint::rect_filled(ui.painter(), bar, col);
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::monospace(10.5),
        col,
    );
}
