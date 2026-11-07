use crate::{
    actions::{FxAction, ViewMode},
    debug::show_debug_overlay,
    motion::{EffectsLevel, SpringCfg},
    overlays::{
        drawer::show_drawer, modals::show_modals, palette::show_command_palette,
        toasts::show_toasts,
    },
    paint,
    services::{
        FxWorkerHandle,
        messages::{FxCommand, FxEvent},
    },
    shader::{GRAIN_VIGNETTE_FRAG_GLSL, show_atmosphere_overlay, validate_atmosphere_shader},
    state::FxState,
    theme::{FxTheme, apply_theme, install_fonts},
    views,
};
use bcvault_core::worker::{
    WorkerHandle,
    messages::{UiToWorkerCommand, WorkerToUiEvent},
};
use egui::{Pos2, Rect, RichText, Stroke, Vec2};
use std::time::Instant;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub struct FxApp {
    pub state: FxState,
    pub core_tx: UnboundedSender<UiToWorkerCommand>,
    pub core_rx: UnboundedReceiver<WorkerToUiEvent>,
    pub fx_tx: UnboundedSender<FxCommand>,
    pub fx_rx: UnboundedReceiver<FxEvent>,
    pub boot_instant: Instant,
    pub rt_handle: tokio::runtime::Handle,
    last_applied_theme: FxTheme,
}

impl FxApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        core_worker: WorkerHandle,
        fx_worker: FxWorkerHandle,
        rt_handle: tokio::runtime::Handle,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        let mut state = FxState::new();
        if let Err(reason) = validate_atmosphere_shader(GRAIN_VIGNETTE_FRAG_GLSL) {
            state.shader_disabled_reason = Some(reason);
        }
        apply_theme(&cc.egui_ctx, &state.active_theme);

        Self {
            last_applied_theme: state.active_theme,
            state,
            core_tx: core_worker.cmd_tx,
            core_rx: core_worker.ev_rx,
            fx_tx: fx_worker.cmd_tx,
            fx_rx: fx_worker.ev_rx,
            boot_instant: Instant::now(),
            rt_handle,
        }
    }

    pub fn drain_events(&mut self, ctx: &egui::Context, now_s: f64) {
        let mut count = 0;
        while count < 256 {
            match self.core_rx.try_recv() {
                Ok(ev) => {
                    self.state.apply_core_event(ev, &self.fx_tx, now_s);
                    count += 1;
                }
                Err(_) => break,
            }
        }
        let mut fx_count = 0;
        while fx_count < 256 {
            match self.fx_rx.try_recv() {
                Ok(ev) => {
                    self.state.apply_fx_event(ev, now_s);
                    fx_count += 1;
                }
                Err(_) => break,
            }
        }
        self.state.last_event_backlog = count + fx_count;
        let _uploaded = self.state.tex_cache.flush_frame_uploads(ctx);
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context, now_s: f64) {
        let mut actions: Vec<FxAction> = Vec::new();

        // Ctrl+K or Ctrl+P -> Command Palette
        if ctx.input(|i| {
            (i.modifiers.ctrl || i.modifiers.command)
                && (i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::P))
        }) {
            if self.state.palette_open {
                actions.push(FxAction::ClosePalette);
            } else {
                actions.push(FxAction::OpenPalette);
            }
        }

        // Ctrl+B -> Toggle Sidebar
        if ctx.input(|i| (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::B)) {
            actions.push(FxAction::ToggleSidebar);
        }

        // F3 -> Debug overlay
        if ctx.input(|i| i.key_pressed(egui::Key::F3)) {
            actions.push(FxAction::ToggleDebug);
        }

        // F11 -> Fullscreen toggle
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            let is_fs = ctx.input(|i| i.viewport().fullscreen).unwrap_or(false);
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fs));
        }

        // Alt+Left / Alt+Right -> NavBack / NavForward
        if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft)) {
            actions.push(FxAction::NavBack);
        }
        if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight)) {
            actions.push(FxAction::NavForward);
        }

        // Esc layering: Modal -> Toast -> Palette -> Drawer -> Artist Dive -> Selection
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.state.close_top_modal() {
                // Top modal closed
            } else if !self.state.toasts.is_empty() {
                self.state.toasts.pop();
            } else if self.state.palette_open {
                actions.push(FxAction::ClosePalette);
            } else if self.state.drawer_key.is_some() {
                actions.push(FxAction::CloseDrawer);
            } else if self.state.artist_dive_banner.take().is_some() {
                actions.push(FxAction::SetView(ViewMode::Artists));
            } else if !self.state.selected_keys.is_empty() {
                actions.push(FxAction::ClearSelection);
            }
        }

        let any_modal = self.state.has_open_modal() || self.state.palette_open;

        if !any_modal && !ctx.wants_keyboard_input() {
            if ctx.input(|i| {
                i.key_pressed(egui::Key::Slash)
                    || ((i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::F))
            }) {
                actions.push(FxAction::FocusSearch);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Num1)) {
                actions.push(FxAction::SetView(ViewMode::Wall));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Num2)) {
                actions.push(FxAction::SetView(ViewMode::Table));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Num3)) {
                actions.push(FxAction::SetView(ViewMode::Artists));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Num4)) {
                actions.push(FxAction::SetView(ViewMode::Stats));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
                actions.push(FxAction::CardSize(1));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Minus)) {
                actions.push(FxAction::CardSize(-1));
            }

            // Previous / Next artist (`[` and `]`)
            if ctx.input(|i| i.key_pressed(egui::Key::OpenBracket))
                && !self.state.artists.is_empty()
            {
                let cur_pos = self
                    .state
                    .query
                    .base
                    .band_id
                    .and_then(|bid| self.state.artists.iter().position(|a| a.band_id == bid));
                let next_bid = match cur_pos {
                    Some(0) | None => None,
                    Some(idx) => Some(self.state.artists[idx - 1].band_id),
                };
                actions.push(FxAction::SelectArtist(next_bid));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::CloseBracket))
                && !self.state.artists.is_empty()
            {
                let cur_pos = self
                    .state
                    .query
                    .base
                    .band_id
                    .and_then(|bid| self.state.artists.iter().position(|a| a.band_id == bid));
                let next_bid = match cur_pos {
                    None => Some(self.state.artists[0].band_id),
                    Some(idx) if idx + 1 < self.state.artists.len() => {
                        Some(self.state.artists[idx + 1].band_id)
                    }
                    Some(idx) => Some(self.state.artists[idx].band_id),
                };
                actions.push(FxAction::SelectArtist(next_bid));
            }

            // Focus navigation (Arrows or H/J/K/L)
            let mut move_delta: isize = 0;
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::H)) {
                if self.state.drawer_key.is_some() {
                    actions.push(FxAction::DrawerStep(-1));
                } else {
                    move_delta = -1;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::L)) {
                if self.state.drawer_key.is_some() {
                    actions.push(FxAction::DrawerStep(1));
                } else {
                    move_delta = 1;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::K)) {
                move_delta = -4;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::J)) {
                move_delta = 4;
            }
            if move_delta != 0 && !self.state.rows.is_empty() {
                let cur_idx = self
                    .state
                    .focused_key
                    .and_then(|fk| self.state.rows.iter().position(|r| r.rel.key == fk))
                    .unwrap_or(0);
                let next_idx = (cur_idx as isize + move_delta)
                    .clamp(0, (self.state.rows.len() - 1) as isize)
                    as usize;
                self.state.focused_key = Some(self.state.rows[next_idx].rel.key);
            }

            if let Some(fk) = self.state.focused_key {
                if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    actions.push(FxAction::OpenDrawer(fk));
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                    actions.push(FxAction::ToggleSelect(fk));
                }
                if ctx.input(|i| i.key_pressed(egui::Key::D)) {
                    actions.push(FxAction::DownloadSelected);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::O)) {
                    actions.push(FxAction::OpenInBrowser(fk));
                }
            }
        }

        for act in actions {
            self.state.dispatch(act, &self.core_tx, &self.fx_tx, now_s);
        }
    }
}

impl eframe::App for FxApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.state.active_theme.bg1.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let t_start = Instant::now();
        let now_s = self.boot_instant.elapsed().as_secs_f64();

        self.state.window_focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        self.state.motion.begin_frame(ctx);
        self.drain_events(ctx, now_s);
        self.state.flush_debounced_queries(&self.fx_tx);
        self.state.check_lazy_drawer_detail(&self.fx_tx);
        self.state.sample_download_speed();
        self.handle_keyboard(ctx, now_s);

        // Theme + optional adaptive accent animated through OKLab springs
        let hovered_swatch = if self.state.prefs.adaptive_accent {
            self.state
                .hovered_art_id
                .and_then(|id| self.state.palettes.get(&id).map(|p| p.accent))
        } else {
            None
        };
        let target_theme =
            FxTheme::for_id(self.state.prefs.theme).with_adaptive_accent(hovered_swatch);
        let animated_theme = target_theme.animate_through(&mut self.state.motion);
        self.state.active_theme = animated_theme;
        if self.last_applied_theme != animated_theme {
            apply_theme(ctx, &animated_theme);
            self.last_applied_theme = animated_theme;
        }

        if let Some(err) = &self.state.schema_error {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(120.0);
                    ui.label(
                        RichText::new("SCHEMA MISMATCH — UPDATE BCVAULT-FX")
                            .size(18.0)
                            .strong()
                            .color(animated_theme.err),
                    );
                    ui.label(
                        RichText::new(err)
                            .monospace()
                            .color(animated_theme.text_dim),
                    );
                });
            });
            self.state.motion.end_frame(ctx);
            return;
        }

        // 1. Top Toolbar
        egui::TopBottomPanel::top("fx_top_toolbar")
            .exact_height(64.0)
            .show(ctx, |ui| {
                views::toolbar::show(ui, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
            });

        // 2. Bottom Dock
        egui::TopBottomPanel::bottom("fx_bottom_dock")
            .resizable(true)
            .default_height(132.0)
            .min_height(84.0)
            .max_height(320.0)
            .show(ctx, |ui| {
                views::dock::show(ui, &mut self.state, &self.core_tx, now_s);
            });

        // 3. Left Sidebar (resizable when expanded, 56px fixed rail when collapsed)
        let sb_panel = if self.state.sidebar_collapsed {
            egui::SidePanel::left("fx_sidebar_collapsed")
                .exact_width(56.0)
                .resizable(false)
        } else {
            egui::SidePanel::left("fx_sidebar_expanded")
                .default_width(280.0)
                .min_width(240.0)
                .max_width(500.0)
                .resizable(true)
        };
        sb_panel.show(ctx, |ui| {
            ui.set_max_width(ui.available_width());
            views::sidebar::show(ui, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
        });

        // 4. Central View with hard wipe transition
        egui::CentralPanel::default().show(ctx, |ui| {
            let wipe_prog = self.state.motion.val_from(
                ("view_wipe", self.state.view_switch_seq),
                if self.state.prefs.effects == EffectsLevel::Off {
                    1.0
                } else {
                    0.0
                },
                1.0,
                SpringCfg::SMOOTH,
            );

            match self.state.view_mode {
                ViewMode::Wall => {
                    views::wall::show(ui, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
                }
                ViewMode::Table => {
                    views::table::show(ui, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
                }
                ViewMode::Artists => {
                    views::artists::show(ui, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
                }
                ViewMode::Stats => {
                    views::stats::show(ui, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
                }
            }

            if wipe_prog < 0.995 {
                let r = ui.max_rect();
                let edge_x = r.min.x + r.width() * wipe_prog;
                let line_r =
                    Rect::from_min_size(Pos2::new(edge_x, r.min.y), Vec2::new(2.0, r.height()));
                paint::rect_filled(ui.painter(), line_r, animated_theme.accent);
                let mask_r = Rect::from_min_max(Pos2::new(edge_x + 2.0, r.min.y), r.max);
                paint::rect_filled(
                    ui.painter(),
                    mask_r,
                    animated_theme.bg0.gamma_multiply(1.0 - wipe_prog),
                );
            }

            // Shared-element artist banner dive overlay
            if let Some((bid, banner_rect)) = self.state.artist_dive_banner {
                let dive_t = self.state.motion.val_from(
                    ("dive_hero", bid, self.state.view_switch_seq),
                    0.0,
                    1.0,
                    SpringCfg::SOFT,
                );
                if dive_t < 0.98 {
                    let target = ui.max_rect();
                    let cur = Rect::from_min_max(
                        banner_rect.min.lerp(target.min, dive_t),
                        banner_rect.max.lerp(target.max, dive_t),
                    );
                    paint::rect_outline(
                        ui.painter(),
                        cur,
                        Stroke::new(2.0_f32, animated_theme.accent.gamma_multiply(1.0 - dive_t)),
                    );
                }
            }
        });

        // 5. Overlays: Detail Drawer, Command Palette, Modals, Toasts, Atmosphere, F3 Debug
        show_drawer(ctx, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
        show_command_palette(ctx, &mut self.state, &self.core_tx, &self.fx_tx, now_s);
        show_modals(
            ctx,
            &mut self.state,
            &self.core_tx,
            &self.fx_tx,
            &self.rt_handle,
            now_s,
        );
        show_toasts(ctx, &mut self.state, &self.core_tx, now_s);
        show_atmosphere_overlay(ctx, &mut self.state, now_s);
        show_debug_overlay(ctx, &self.state);

        self.state.last_frame_ms = t_start.elapsed().as_secs_f32() * 1000.0;
        self.state.motion.end_frame(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.fx_tx.send(FxCommand::Shutdown);
        let _ = self.core_tx.send(UiToWorkerCommand::Shutdown);
    }
}
