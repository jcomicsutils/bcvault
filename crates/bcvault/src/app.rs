use crate::{
    types::DownloadOpts,
    ui::{self, state::UiState, theme::pal},
    worker::{
        WorkerHandle,
        messages::{UiToWorkerCommand, WorkerToUiEvent},
    },
};
use egui::{Align2, Order, RichText};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub struct App {
    rt_handle: tokio::runtime::Handle,
    pub state: UiState,
    pub cmd_tx: UnboundedSender<UiToWorkerCommand>,
    pub ev_rx: UnboundedReceiver<WorkerToUiEvent>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        rt_handle: tokio::runtime::Handle,
        worker: WorkerHandle,
    ) -> Self {
        ui::theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut state = UiState::new();
        state.paths = crate::paths::AppPaths::resolve();
        Self {
            rt_handle,
            state,
            cmd_tx: worker.cmd_tx,
            ev_rx: worker.ev_rx,
        }
    }

    pub fn drain_events(&mut self) {
        let mut count = 0;
        while count < 256 {
            match self.ev_rx.try_recv() {
                Ok(ev) => {
                    self.state.apply_event(ev);
                    count += 1;
                }
                Err(_) => break,
            }
        }
        self.state.last_event_backlog = count;
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::F3)) {
            self.state.show_debug_overlay = !self.state.show_debug_overlay;
        }

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            self.state.focus_search_next_frame = true;
        }

        let any_modal = self.state.redeem_modal.is_some()
            || !self.state.quota_warnings.is_empty()
            || self.state.bulk_confirm_modal.is_some()
            || self.state.settings_draft.is_some()
            || self.state.confirm_remove_artist.is_some();

        if !any_modal && !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
                self.state.select_all_downloadable();
            }

            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !self.state.selected.is_empty() {
                self.state.clear_selection();
            }

            if ctx.input(|i| i.key_pressed(egui::Key::Space))
                && let Some(idx) = self.state.last_clicked_row
                && let Some(cr) = self.state.rows.get(idx)
                && cr.row.is_downloadable()
            {
                let key = cr.row.key;
                if !self.state.selected.remove(&key) {
                    self.state.selected.insert(key);
                }
                self.state.mark_selection_dirty();
            }

            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && !self.state.selected.is_empty() {
                let items: Vec<_> = self.state.selected.iter().copied().collect();
                let opts = DownloadOpts {
                    format: self.state.settings.format,
                    out_dir: self.state.settings.out_dir.clone(),
                    batch_limit: self.state.settings.batch_limit,
                };
                let _ = self.cmd_tx.send(UiToWorkerCommand::EnqueueDownloads {
                    items,
                    opts,
                    ack_soft_quota: false,
                    mode: bcvault_core::types::EnqueueMode::New,
                });
            }
        }
    }

    fn draw_debug_overlay(&self, ctx: &egui::Context) {
        if !self.state.show_debug_overlay {
            return;
        }
        let dt_ms = ctx.input(|i| i.unstable_dt) * 1000.0;
        egui::Window::new("F3 Debug Overlay")
            .order(Order::Tooltip)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::RIGHT_TOP, [-12.0, 42.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("unstable_dt: {dt_ms:.2} ms"))
                        .monospace()
                        .color(pal::ACCENT),
                );
                ui.label(
                    RichText::new(format!("rows: {}", self.state.rows.len()))
                        .monospace()
                        .color(pal::TEXT),
                );
                ui.label(
                    RichText::new(format!("drained events: {}", self.state.last_event_backlog))
                        .monospace()
                        .color(pal::TEXT_DIM),
                );
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.handle_shortcuts(ctx);
        self.state.flush_debounced_query(&self.cmd_tx);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui::toolbar::show(ui, &mut self.state, &self.cmd_tx, &self.rt_handle);
        });
        egui::TopBottomPanel::bottom("dock")
            .resizable(true)
            .default_height(180.0)
            .show(ctx, |ui| {
                ui::bottom_dock::show(ui, &mut self.state, &self.cmd_tx);
            });
        egui::SidePanel::left("artists")
            .resizable(true)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui::sidebar::show(ui, &mut self.state, &self.cmd_tx);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui::release_table::show(ui, &mut self.state, &self.cmd_tx, &self.rt_handle);
        });

        ui::modals::show(ctx, &mut self.state, &self.cmd_tx, &self.rt_handle);
        self.draw_debug_overlay(ctx);

        if self.state.cooldown_visible() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cmd_tx.send(UiToWorkerCommand::Shutdown);
    }
}
