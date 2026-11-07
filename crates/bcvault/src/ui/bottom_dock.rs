use crate::{
    types::{JobState, LogLevel, RateLimitStatus},
    ui::{
        state::{DockTab, UiState},
        theme::pal,
        widgets::{badge, format_bytes, format_speed, sharp_small_button},
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align, CornerRadius, Layout, RichText, Ui};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(ui: &mut Ui, state: &mut UiState, cmd_tx: &UnboundedSender<UiToWorkerCommand>) {
    // Header bar: Tabs (Queue, Errors) + Queue controls + Rate-limit chip
    ui.horizontal(|ui| {
        let q_label = format!("Queue ({})", state.jobs.len());
        ui.selectable_value(&mut state.dock_tab, DockTab::Queue, q_label);

        let err_count = state
            .logs
            .iter()
            .filter(|l| matches!(l.level, LogLevel::Error | LogLevel::Warn))
            .count();
        let e_label = format!("Errors / Log ({err_count})");
        ui.selectable_value(&mut state.dock_tab, DockTab::Errors, e_label);

        ui.separator();

        if state.dock_tab == DockTab::Queue {
            if state.queue_paused {
                if sharp_small_button(ui, "Resume Queue").clicked() {
                    state.queue_paused = false;
                    let _ = cmd_tx.send(UiToWorkerCommand::ResumeQueue);
                }
            } else if sharp_small_button(ui, "Pause Queue").clicked() {
                state.queue_paused = true;
                let _ = cmd_tx.send(UiToWorkerCommand::PauseQueue);
            }

            if sharp_small_button(ui, "Cancel All").clicked() {
                let _ = cmd_tx.send(UiToWorkerCommand::CancelAllJobs);
            }

            if sharp_small_button(ui, "Clear Finished").clicked() {
                state.clear_finished_jobs();
                let _ = cmd_tx.send(UiToWorkerCommand::ClearFinished);
            }
        } else {
            ui.label(RichText::new("Filter:").color(pal::TEXT_DIM));
            if ui
                .selectable_label(state.log_filter.is_none(), "All")
                .clicked()
            {
                state.log_filter = None;
            }
            if ui
                .selectable_label(state.log_filter == Some(LogLevel::Warn), "Warn")
                .clicked()
            {
                state.log_filter = Some(LogLevel::Warn);
            }
            if ui
                .selectable_label(state.log_filter == Some(LogLevel::Error), "Error")
                .clicked()
            {
                state.log_filter = Some(LogLevel::Error);
            }

            if sharp_small_button(ui, "Copy to Clipboard").clicked() {
                let text: String = state
                    .logs
                    .iter()
                    .map(|l| format!("[{:?}] [{}] {}", l.level, l.scope, l.message))
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.ctx().copy_text(text);
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            draw_rate_limit_chip(ui, &state.rate_limit);
            if let Some(modal) = &state.resolve_missing_modal {
                let count = modal.items.len();
                if count > 0 {
                    let label = format!("MISSING {count}");
                    let btn = ui.add(
                        egui::Button::new(egui::RichText::new(&label).size(11.5).color(pal::WARN))
                            .corner_radius(CornerRadius::ZERO),
                    );
                    if btn.clicked() {
                        state.show_resolve_missing_window = true;
                    }
                }
            }
        });
    });

    ui.separator();

    match state.dock_tab {
        DockTab::Queue => show_queue_tab(ui, state, cmd_tx),
        DockTab::Errors => show_errors_tab(ui, state),
    }
}

fn draw_rate_limit_chip(ui: &mut Ui, status: &RateLimitStatus) {
    match status {
        RateLimitStatus::Idle => {
            badge(ui, "GATE: OK", pal::OK, pal::OK);
        }
        RateLimitStatus::Pacing { next_in_ms } => {
            let secs = (*next_in_ms as f64) / 1000.0;
            badge(ui, &format!("PACING {secs:.1} s"), pal::ACCENT, pal::ACCENT);
        }
        RateLimitStatus::Cooldown { until, reason } => {
            let now = chrono::Utc::now().timestamp();
            let rem = (*until - now).max(0);
            let mins = rem / 60;
            let secs = rem % 60;
            badge(
                ui,
                &format!("COOLDOWN {mins:02}:{secs:02} ({reason})"),
                pal::WARN,
                pal::WARN,
            );
        }
    }
}

fn show_queue_tab(ui: &mut Ui, state: &mut UiState, cmd_tx: &UnboundedSender<UiToWorkerCommand>) {
    let total = state.jobs.len();
    if total == 0 {
        ui.label(RichText::new("No active or recent download jobs.").color(pal::TEXT_DIM));
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, 26.0, total, |ui, range| {
            for idx in range {
                let Some(job) = state.jobs.get(idx).cloned() else {
                    continue;
                };
                let prog = state.progress.get(&job.id).copied().unwrap_or_default();

                ui.horizontal(|ui| {
                    ui.add_sized(
                        [260.0, 20.0],
                        egui::Label::new(RichText::new(&job.label).color(pal::TEXT)).truncate(),
                    );

                    let (state_str, state_col) = match &job.state {
                        JobState::Queued => ("QUEUED".to_string(), pal::TEXT_DIM),
                        JobState::Resolving => ("RESOLVING".to_string(), pal::ACCENT),
                        JobState::Downloading => ("DOWNLOADING".to_string(), pal::ACCENT),
                        JobState::Extracting => ("EXTRACTING".to_string(), pal::ACCENT),
                        JobState::Done { .. } => ("DONE".to_string(), pal::OK),
                        JobState::Failed(_) => ("FAILED".to_string(), pal::ERR),
                        JobState::Cancelled => ("CANCELLED".to_string(), pal::TEXT_DIM),
                    };

                    ui.add_sized(
                        [95.0, 20.0],
                        egui::Label::new(RichText::new(state_str).monospace().color(state_col)),
                    );

                    let (frac, bar_text) = match &job.state {
                        JobState::Done { bytes, .. } => {
                            (1.0_f32, format!("{} (complete)", format_bytes(*bytes)))
                        }
                        JobState::Failed(err) => (0.0_f32, err.to_string()),
                        JobState::Cancelled => (0.0_f32, "Cancelled".to_string()),
                        _ => {
                            let f = match prog.total {
                                Some(t) if t > 0 => (prog.bytes as f32 / t as f32).clamp(0.0, 1.0),
                                _ => 0.0_f32,
                            };
                            let total_str = prog
                                .total
                                .map(format_bytes)
                                .unwrap_or_else(|| "?".to_string());
                            let txt = format!(
                                "{} / {}   {}",
                                format_bytes(prog.bytes),
                                total_str,
                                format_speed(prog.bytes_per_sec)
                            );
                            (f, txt)
                        }
                    };

                    ui.add_sized(
                        [320.0, 18.0],
                        egui::ProgressBar::new(frac)
                            .corner_radius(CornerRadius::ZERO)
                            .text(RichText::new(bar_text).monospace().size(11.0)),
                    );

                    match &job.state {
                        JobState::Queued
                        | JobState::Resolving
                        | JobState::Downloading
                        | JobState::Extracting => {
                            if sharp_small_button(ui, "Cancel").clicked() {
                                let _ = cmd_tx.send(UiToWorkerCommand::CancelJob(job.id));
                            }
                        }
                        JobState::Failed(_) | JobState::Cancelled => {
                            if sharp_small_button(ui, "Retry").clicked() {
                                let _ = cmd_tx.send(UiToWorkerCommand::RetryJob(job.id));
                            }
                        }
                        JobState::Done { path, .. } => {
                            if sharp_small_button(ui, "Open").clicked() {
                                let dir = if path.is_dir() {
                                    path.as_path()
                                } else {
                                    path.parent().unwrap_or(path.as_path())
                                };
                                let _ = open::that_detached(dir);
                            }
                        }
                    }
                });
            }
        });
}

fn show_errors_tab(ui: &mut Ui, state: &mut UiState) {
    let filtered: Vec<_> = state
        .logs
        .iter()
        .rev()
        .filter(|l| state.log_filter.is_none_or(|f| l.level == f))
        .collect();

    if filtered.is_empty() {
        ui.label(RichText::new("No log entries matching filter.").color(pal::TEXT_DIM));
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, 20.0, filtered.len(), |ui, range| {
            for idx in range {
                let entry = filtered[idx];
                let col = match entry.level {
                    LogLevel::Info => pal::TEXT_DIM,
                    LogLevel::Warn => pal::WARN,
                    LogLevel::Error => pal::ERR,
                };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("[{:?}]", entry.level))
                            .monospace()
                            .color(col),
                    );
                    ui.label(
                        RichText::new(format!("[{}]", entry.scope))
                            .monospace()
                            .color(pal::TEXT_DIM),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(&entry.message).color(pal::TEXT)).truncate(),
                    );
                });
            }
        });
}
