pub mod advanced;
pub mod appearance;
pub mod downloads;
pub mod folders;
pub mod library;
pub mod network;

use crate::{
    actions::FxAction,
    db::Prefs,
    motion::SpringCfg,
    paint,
    services::{messages::FxCommand, thumbs::TextureCache},
    state::FxState,
    theme::{FxTheme, registry::ThemeRegistry},
};
use bcvault_core::{
    types::{ArtistFolder, ArtistView, Settings},
    worker::messages::UiToWorkerCommand,
};
use egui::{Color32, CornerRadius, Key, RichText, Stroke};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SettingsTab {
    Appearance,
    #[default]
    Downloads,
    Network,
    Library,
    Folders,
    Advanced,
}

impl SettingsTab {
    pub const ALL: &[Self] = &[
        Self::Appearance,
        Self::Downloads,
        Self::Network,
        Self::Library,
        Self::Folders,
        Self::Advanced,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Downloads => "Downloads",
            Self::Network => "Network & Quota",
            Self::Library => "Library",
            Self::Folders => "Folders",
            Self::Advanced => "Advanced",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Downloads => "downloads",
            Self::Network => "network",
            Self::Library => "library",
            Self::Folders => "folders",
            Self::Advanced => "advanced",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "appearance" => Self::Appearance,
            "downloads" => Self::Downloads,
            "network" => Self::Network,
            "library" => Self::Library,
            "folders" => Self::Folders,
            "advanced" => Self::Advanced,
            _ => Self::Downloads,
        }
    }

    pub const fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Appearance => &[],
            Self::Downloads => &[
                "out_dir",
                "layout",
                "filename_pattern",
                "format",
                "format_fallback",
                "auto_extract",
                "delete_zip_after_extract",
                "batch_limit",
            ],
            Self::Network => &[
                "quota_soft",
                "quota_hard",
                "page_delay_ms",
                "page_jitter_pct",
                "max_concurrent_downloads",
                "user_agent",
            ],
            Self::Library => &[
                "verify_on_sync",
                "rescan_linked_on_sync",
                "auto_relink",
                "on_missing",
                "deleted_policy",
                "auto_match_min",
                "review_match_min",
            ],
            Self::Folders => &["folder_style"],
            Self::Advanced => &[],
        }
    }

    pub const NOT_SHOWN: &'static [(&'static str, &'static str)] = &[(
        "show_thumbnails",
        "bcvault-fx always renders covers in the wall and table views; only the classic UI uses this toggle",
    )];

    pub fn cycle(self, delta: isize) -> Self {
        let len = Self::ALL.len() as isize;
        let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0) as isize;
        let next = (idx + delta).rem_euclid(len) as usize;
        Self::ALL[next]
    }
}

pub struct TabCtx<'a> {
    pub draft: &'a mut Settings,
    pub filename_pattern_input: &'a mut String,
    pub prefs: &'a mut Prefs,
    pub tex_cache: &'a mut TextureCache,
    pub cover_cache_bytes: u64,
    pub theme: FxTheme,
    pub themes: &'a ThemeRegistry,
    pub artists: &'a [ArtistView],
    pub folders: &'a HashMap<i64, Vec<ArtistFolder>>,
    pub folder_filter: &'a mut String,
    pub picked_out_dir: &'a std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
    pub core_tx: &'a UnboundedSender<UiToWorkerCommand>,
    pub fx_tx: &'a UnboundedSender<FxCommand>,
    pub rt_handle: &'a tokio::runtime::Handle,
    pub pending_actions: &'a mut Vec<FxAction>,
    pub focus_first: bool,
}

pub fn is_tab_dirty(tab: SettingsTab, draft: &Settings, saved: &Settings) -> bool {
    let fields = tab.fields();
    if fields.is_empty() {
        return false;
    }
    let Ok(draft_val) = serde_json::to_value(draft) else {
        return false;
    };
    let Ok(saved_val) = serde_json::to_value(saved) else {
        return false;
    };
    fields
        .iter()
        .any(|key| draft_val.get(*key) != saved_val.get(*key))
}

pub fn is_any_dirty(draft: &Settings, saved: &Settings) -> bool {
    SettingsTab::ALL
        .iter()
        .any(|&tab| is_tab_dirty(tab, draft, saved))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsEscOutcome {
    Close,
    ShowConfirm,
    KeepEditing,
}

pub fn handle_settings_esc(
    draft: &Settings,
    saved: &Settings,
    confirm_open: &mut bool,
) -> SettingsEscOutcome {
    if !is_any_dirty(draft, saved) {
        *confirm_open = false;
        SettingsEscOutcome::Close
    } else if *confirm_open {
        *confirm_open = false;
        SettingsEscOutcome::KeepEditing
    } else {
        *confirm_open = true;
        SettingsEscOutcome::ShowConfirm
    }
}

pub fn save_draft(
    draft: &mut Settings,
    saved: &Settings,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
) {
    if draft.out_dir.to_string_lossy().trim().is_empty() {
        draft.out_dir = saved.out_dir.clone();
    }
    draft.sanitize();
    let _ = core_tx.send(UiToWorkerCommand::UpdateSettings(draft.clone()));
}

fn switch_tab(state: &mut FxState, fx_tx: &UnboundedSender<FxCommand>, tab: SettingsTab) {
    if state.prefs.settings_tab != tab {
        let old_idx = SettingsTab::ALL
            .iter()
            .position(|&t| t == state.prefs.settings_tab)
            .unwrap_or(0);
        let new_idx = SettingsTab::ALL.iter().position(|&t| t == tab).unwrap_or(0);
        state.settings_tab_anim_dir = if new_idx >= old_idx { 1.0 } else { -1.0 };
        state.settings_tab_anim_gen = state.settings_tab_anim_gen.wrapping_add(1);
        state.prefs.settings_tab = tab;
        state.settings_focus_first = true;
        let _ = fx_tx.send(FxCommand::SetPref {
            key: "settings_tab".to_string(),
            value: tab.as_str().to_string(),
        });
    }
}

pub fn show_settings_modal(
    ctx: &egui::Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    rt_handle: &tokio::runtime::Handle,
    pending_actions: &mut Vec<FxAction>,
) {
    if !state.settings_modal_open {
        return;
    }

    let Some(mut draft) = state.settings_draft.take() else {
        state.settings_modal_open = false;
        return;
    };

    if let Some(picked) = state
        .settings_picked_out_dir
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        draft.out_dir = picked;
    }

    let th = state.active_theme;
    let mut keep_open = true;
    let mut trigger_save = false;

    let tab_delta = ctx.input(|i| {
        let ctrl = i.modifiers.command || i.modifiers.ctrl;
        if ctrl && (i.key_pressed(Key::PageDown) || (i.key_pressed(Key::Tab) && !i.modifiers.shift))
        {
            1
        } else if ctrl
            && (i.key_pressed(Key::PageUp) || (i.key_pressed(Key::Tab) && i.modifiers.shift))
        {
            -1
        } else {
            0
        }
    });
    if tab_delta != 0 {
        let next = state.prefs.settings_tab.cycle(tab_delta);
        switch_tab(state, fx_tx, next);
    }

    let ctrl_s = ctx.input(|i| (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(Key::S));
    if ctrl_s {
        trigger_save = true;
    }

    let screen = ctx.content_rect();
    let max_body_h = (screen.height() * 0.80 - 130.0).max(220.0);

    egui::Window::new("bcvault-fx Settings")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(680.0)
        .show(ctx, |ui| {
            ui.set_min_width(680.0);
            ui.set_max_width(680.0);

            let mut active_tab_rect: Option<egui::Rect> = None;
            ui.horizontal_wrapped(|ui| {
                let mut tab_resps = Vec::with_capacity(SettingsTab::ALL.len());
                for &tab in SettingsTab::ALL {
                    let selected = state.prefs.settings_tab == tab;
                    let dirty = is_tab_dirty(tab, &draft, &state.settings);
                    let label = if dirty {
                        format!("{} •", tab.label())
                    } else {
                        tab.label().to_string()
                    };
                    let btn = egui::Button::new(
                        RichText::new(label)
                            .color(if selected { th.text } else { th.text_dim })
                            .strong(),
                    )
                    .fill(if selected { th.bg2 } else { th.bg1 })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if selected { th.accent } else { th.border },
                    ))
                    .corner_radius(CornerRadius::ZERO);

                    let resp = ui.add(btn);
                    if selected {
                        active_tab_rect = Some(resp.rect);
                    }
                    if resp.clicked() && !selected {
                        switch_tab(state, fx_tx, tab);
                    }
                    tab_resps.push((tab, resp));
                }

                for (idx, (_, resp)) in tab_resps.iter().enumerate() {
                    if resp.has_focus() {
                        if ui.input(|i| i.key_pressed(Key::ArrowRight)) {
                            let next = (idx + 1) % tab_resps.len();
                            tab_resps[next].1.request_focus();
                            let t = tab_resps[next].0;
                            switch_tab(state, fx_tx, t);
                        } else if ui.input(|i| i.key_pressed(Key::ArrowLeft)) {
                            let prev = (idx + tab_resps.len() - 1) % tab_resps.len();
                            tab_resps[prev].1.request_focus();
                            let t = tab_resps[prev].0;
                            switch_tab(state, fx_tx, t);
                        }
                        break;
                    }
                }
            });

            if let Some(target_rect) = active_tab_rect {
                let ind_x =
                    state
                        .motion
                        .val("settings_tab_ind_x", target_rect.min.x, SpringCfg::SNAP);
                let ind_w =
                    state
                        .motion
                        .val("settings_tab_ind_w", target_rect.width(), SpringCfg::SNAP);
                let ind_rect = egui::Rect::from_min_size(
                    egui::pos2(ind_x, target_rect.max.y - 2.0),
                    egui::vec2(ind_w.max(4.0), 2.0),
                );
                paint::rect_filled(ui.painter(), ind_rect, th.accent);
            }

            ui.separator();

            let focus_first = std::mem::take(&mut state.settings_focus_first);
            let active_tab = state.prefs.settings_tab;
            let enter = state.motion.val_from(
                ("settings_tab_enter", state.settings_tab_anim_gen),
                0.0,
                1.0,
                SpringCfg::SMOOTH,
            );
            let slide_x = (1.0 - enter) * 18.0 * state.settings_tab_anim_dir;

            egui::ScrollArea::vertical()
                .id_salt("fx_settings_tab_scroll")
                .auto_shrink([false, false])
                .max_height(max_body_h)
                .show(ui, |ui| {
                    let base_rect = ui.available_rect_before_wrap();
                    let shifted_rect = base_rect.translate(egui::vec2(slide_x, 0.0));
                    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(shifted_rect));
                    child_ui.set_clip_rect(ui.clip_rect());
                    child_ui.set_opacity(enter.clamp(0.0, 1.0));
                    let mut tab_ctx = TabCtx {
                        draft: &mut draft,
                        filename_pattern_input: &mut state.filename_pattern_input,
                        prefs: &mut state.prefs,
                        tex_cache: &mut state.tex_cache,
                        cover_cache_bytes: state.cover_cache_bytes,
                        theme: th,
                        themes: &state.themes,
                        artists: &state.artists,
                        folders: &state.folders,
                        folder_filter: &mut state.settings_folders_filter,
                        picked_out_dir: &state.settings_picked_out_dir,
                        core_tx,
                        fx_tx,
                        rt_handle,
                        pending_actions,
                        focus_first,
                    };
                    match active_tab {
                        SettingsTab::Appearance => appearance::show(&mut child_ui, &mut tab_ctx),
                        SettingsTab::Downloads => downloads::show(&mut child_ui, &mut tab_ctx),
                        SettingsTab::Network => network::show(&mut child_ui, &mut tab_ctx),
                        SettingsTab::Library => library::show(&mut child_ui, &mut tab_ctx),
                        SettingsTab::Folders => folders::show(&mut child_ui, &mut tab_ctx),
                        SettingsTab::Advanced => advanced::show(&mut child_ui, &mut tab_ctx),
                    }
                    let used_h = child_ui.min_rect().height();
                    ui.allocate_space(egui::vec2(base_rect.width(), used_h));
                });

            ui.separator();

            if state.settings_confirm_discard {
                let focus_keep = std::mem::take(&mut state.settings_focus_keep_editing);
                egui::Frame::new()
                    .fill(Color32::from_rgb(56, 36, 20))
                    .stroke(Stroke::new(1.0_f32, th.warn))
                    .corner_radius(CornerRadius::ZERO)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Unsaved changes:").color(th.warn).strong());
                            if paint::sharp_button(ui, "Save").clicked() {
                                trigger_save = true;
                            }
                            if paint::sharp_button(ui, "Discard").clicked() {
                                state.settings_confirm_discard = false;
                                keep_open = false;
                            }
                            let keep_btn = paint::sharp_button(ui, "Keep editing");
                            if focus_keep {
                                keep_btn.request_focus();
                            }
                            if keep_btn.clicked() {
                                state.settings_confirm_discard = false;
                            }
                        });
                    });
                ui.add_space(4.0);
            }

            ui.horizontal(|ui| {
                if paint::sharp_button(ui, "Save").clicked() {
                    trigger_save = true;
                }
                if paint::sharp_button(ui, "Cancel").clicked() {
                    state.settings_confirm_discard = false;
                    keep_open = false;
                }
            });
        });

    if trigger_save {
        save_draft(&mut draft, &state.settings, core_tx);
        state.settings_confirm_discard = false;
        keep_open = false;
    }

    if keep_open {
        state.settings_draft = Some(draft);
    } else {
        state.settings_modal_open = false;
        state.settings_draft = None;
        state.settings_confirm_discard = false;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn settings_tab_parse_and_cycle_roundtrip() {
        for &tab in SettingsTab::ALL {
            assert_eq!(SettingsTab::parse(tab.as_str()), tab);
        }
        assert_eq!(SettingsTab::parse("unknown_tab"), SettingsTab::Downloads);
        assert_eq!(SettingsTab::Downloads.cycle(1), SettingsTab::Network);
        assert_eq!(SettingsTab::Downloads.cycle(-1), SettingsTab::Appearance);
    }

    #[test]
    fn dirty_marker_and_esc_state_machine() {
        let saved = Settings::default();
        let mut draft = saved.clone();
        assert!(!is_any_dirty(&draft, &saved));

        draft.batch_limit = 50;
        assert!(is_tab_dirty(SettingsTab::Downloads, &draft, &saved));
        assert!(!is_tab_dirty(SettingsTab::Network, &draft, &saved));
        assert!(is_any_dirty(&draft, &saved));

        let mut confirm = false;
        assert_eq!(
            handle_settings_esc(&saved, &saved, &mut confirm),
            SettingsEscOutcome::Close
        );
        assert_eq!(
            handle_settings_esc(&draft, &saved, &mut confirm),
            SettingsEscOutcome::ShowConfirm
        );
        assert!(confirm);
        assert_eq!(
            handle_settings_esc(&draft, &saved, &mut confirm),
            SettingsEscOutcome::KeepEditing
        );
        assert!(!confirm);
    }

    #[test]
    fn save_draft_sanitizes_and_sends_update_settings() {
        let saved = Settings::default();
        let mut draft = Settings {
            quota_soft: 250,
            quota_hard: 120,
            batch_limit: 150,
            ..saved.clone()
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        save_draft(&mut draft, &saved, &tx);
        match rx.try_recv() {
            Ok(UiToWorkerCommand::UpdateSettings(sent)) => {
                assert_eq!(sent.quota_soft, 120);
                assert_eq!(sent.batch_limit, 100);
            }
            other => panic!("expected UpdateSettings, got {other:?}"),
        }
    }

    #[test]
    fn settings_tabs_and_not_shown_cover_all_settings_fields() {
        use std::collections::BTreeSet;

        let json = serde_json::to_value(Settings::default()).unwrap();
        let obj = json.as_object().unwrap();
        let actual: BTreeSet<&str> = obj.keys().map(String::as_str).collect();

        let mut declared = BTreeSet::new();
        for &tab in SettingsTab::ALL {
            for &field in tab.fields() {
                assert!(
                    declared.insert(field),
                    "duplicate Settings field {field:?} in SettingsTab"
                );
            }
        }
        for &(field, reason) in SettingsTab::NOT_SHOWN {
            assert!(
                !reason.trim().is_empty(),
                "NOT_SHOWN reason must not be empty"
            );
            assert!(
                declared.insert(field),
                "duplicate Settings field {field:?} in NOT_SHOWN"
            );
        }

        let missing: Vec<_> = actual.difference(&declared).copied().collect();
        let unknown: Vec<_> = declared.difference(&actual).copied().collect();
        assert!(
            missing.is_empty() && unknown.is_empty(),
            "Settings coverage mismatch: missing={missing:?}, unknown={unknown:?}"
        );
    }

    #[test]
    fn switch_tab_updates_anim_dir_and_gen_and_respects_reduced_motion() {
        let mut state = FxState::new();
        let (fx_tx, _fx_rx) = mpsc::unbounded_channel();
        assert_eq!(state.prefs.settings_tab, SettingsTab::Downloads);
        assert_eq!(state.settings_tab_anim_gen, 0);

        // Moving forward: Downloads (1) -> Library (3) => dir = +1.0
        switch_tab(&mut state, &fx_tx, SettingsTab::Library);
        assert_eq!(state.prefs.settings_tab, SettingsTab::Library);
        assert_eq!(state.settings_tab_anim_dir, 1.0);
        assert_eq!(state.settings_tab_anim_gen, 1);

        // Moving backward: Library (3) -> Appearance (0) => dir = -1.0
        switch_tab(&mut state, &fx_tx, SettingsTab::Appearance);
        assert_eq!(state.prefs.settings_tab, SettingsTab::Appearance);
        assert_eq!(state.settings_tab_anim_dir, -1.0);
        assert_eq!(state.settings_tab_anim_gen, 2);

        // Motion enters from 0.0 -> 1.0 normally, and snaps to 1.0 when reduced_motion = true
        state.motion.begin_frame_dt(1.0 / 60.0);
        let v_anim = state.motion.val_from(
            ("settings_tab_enter", state.settings_tab_anim_gen),
            0.0,
            1.0,
            SpringCfg::SMOOTH,
        );
        assert!(v_anim < 1.0);

        state.motion.reduced_motion = true;
        let v_snap =
            state
                .motion
                .val_from(("settings_tab_enter", 99_u64), 0.0, 1.0, SpringCfg::SMOOTH);
        assert_eq!(v_snap, 1.0);
    }
}
