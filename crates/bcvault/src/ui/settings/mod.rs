pub mod advanced;
pub mod appearance;
pub mod downloads;
pub mod folders;
pub mod library;
pub mod network;

use crate::paths::AppPaths;
use crate::types::{ArtistFolder, ArtistView, Settings};
use crate::ui::state::UiState;
use crate::ui::theme::pal;
use crate::ui::widgets::sharp_button;
use crate::worker::messages::UiToWorkerCommand;
use egui::{Align2, Color32, Context, CornerRadius, Key, Order, RichText, Stroke};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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

    pub const fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Appearance => &["show_thumbnails", "mark_invisible_chars"],
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

    pub const NOT_SHOWN: &'static [(&'static str, &'static str)] = &[];

    pub fn cycle(self, delta: isize) -> Self {
        let len = Self::ALL.len() as isize;
        let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0) as isize;
        let next = (idx + delta).rem_euclid(len) as usize;
        Self::ALL[next]
    }
}

pub struct TabCtx<'a> {
    pub draft: &'a mut Settings,
    pub artists: &'a [ArtistView],
    pub folders: &'a HashMap<i64, Vec<ArtistFolder>>,
    pub folder_filter: &'a mut String,
    pub picked_out_dir: &'a Arc<Mutex<Option<std::path::PathBuf>>>,
    pub cmd_tx: &'a UnboundedSender<UiToWorkerCommand>,
    pub rt_handle: &'a tokio::runtime::Handle,
    pub paths: Option<&'a AppPaths>,
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
    saved: &mut Settings,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
) {
    if draft.out_dir.to_string_lossy().trim().is_empty() {
        draft.out_dir = saved.out_dir.clone();
    }
    draft.sanitize();
    *saved = draft.clone();
    let _ = cmd_tx.send(UiToWorkerCommand::UpdateSettings(draft.clone()));
}

pub fn show_settings_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
    esc: bool,
) {
    let Some(mut draft) = state.settings_draft.take() else {
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
        state.settings_tab = state.settings_tab.cycle(tab_delta);
        state.settings_focus_first = true;
    }

    let ctrl_s = ctx.input(|i| (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(Key::S));
    if ctrl_s {
        trigger_save = true;
    }

    if esc {
        match handle_settings_esc(&draft, &state.settings, &mut state.settings_confirm_discard) {
            SettingsEscOutcome::Close => keep_open = false,
            SettingsEscOutcome::ShowConfirm => {
                state.settings_focus_keep_editing = true;
            }
            SettingsEscOutcome::KeepEditing => {}
        }
    }

    let screen = ctx.content_rect();
    let max_body_h = (screen.height() * 0.80 - 130.0).max(220.0);

    egui::Window::new("Settings")
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(680.0)
        .show(ctx, |ui| {
            ui.set_min_width(680.0);
            ui.set_max_width(680.0);

            // Tab strip
            ui.horizontal_wrapped(|ui| {
                let mut tab_ids = Vec::with_capacity(SettingsTab::ALL.len());
                for &tab in SettingsTab::ALL {
                    let selected = state.settings_tab == tab;
                    let dirty = is_tab_dirty(tab, &draft, &state.settings);
                    let label = if dirty {
                        format!("{} •", tab.label())
                    } else {
                        tab.label().to_string()
                    };
                    let btn = egui::Button::new(
                        RichText::new(label)
                            .color(if selected { pal::TEXT } else { pal::TEXT_DIM })
                            .strong(),
                    )
                    .fill(if selected { pal::BG2 } else { pal::BG1 })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if selected { pal::ACCENT } else { pal::BORDER },
                    ))
                    .corner_radius(CornerRadius::ZERO);

                    let resp = ui.add(btn);
                    if resp.clicked() && !selected {
                        state.settings_tab = tab;
                        state.settings_focus_first = true;
                    }
                    tab_ids.push((tab, resp));
                }

                // Left / Right move between tab buttons when strip has focus
                for (idx, (_, resp)) in tab_ids.iter().enumerate() {
                    if resp.has_focus() {
                        if ui.input(|i| i.key_pressed(Key::ArrowRight)) {
                            let next = (idx + 1) % tab_ids.len();
                            tab_ids[next].1.request_focus();
                            state.settings_tab = tab_ids[next].0;
                        } else if ui.input(|i| i.key_pressed(Key::ArrowLeft)) {
                            let prev = (idx + tab_ids.len() - 1) % tab_ids.len();
                            tab_ids[prev].1.request_focus();
                            state.settings_tab = tab_ids[prev].0;
                        }
                        break;
                    }
                }
            });

            ui.separator();

            let focus_first = std::mem::take(&mut state.settings_focus_first);
            egui::ScrollArea::vertical()
                .id_salt("classic_settings_tab_scroll")
                .max_height(max_body_h)
                .show(ui, |ui| {
                    let mut tab_ctx = TabCtx {
                        draft: &mut draft,
                        artists: &state.artists,
                        folders: &state.folders,
                        folder_filter: &mut state.settings_folders_filter,
                        picked_out_dir: &state.settings_picked_out_dir,
                        cmd_tx,
                        rt_handle,
                        paths: state.paths.as_ref(),
                        focus_first,
                    };
                    match state.settings_tab {
                        SettingsTab::Appearance => appearance::show(ui, &mut tab_ctx),
                        SettingsTab::Downloads => downloads::show(ui, &mut tab_ctx),
                        SettingsTab::Network => network::show(ui, &mut tab_ctx),
                        SettingsTab::Library => library::show(ui, &mut tab_ctx),
                        SettingsTab::Folders => folders::show(ui, &mut tab_ctx),
                        SettingsTab::Advanced => advanced::show(ui, &mut tab_ctx),
                    }
                });

            ui.separator();

            if state.settings_confirm_discard {
                let focus_keep = std::mem::take(&mut state.settings_focus_keep_editing);
                egui::Frame::new()
                    .fill(Color32::from_rgb(56, 36, 20))
                    .stroke(Stroke::new(1.0_f32, pal::WARN))
                    .corner_radius(CornerRadius::ZERO)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Unsaved changes:").color(pal::WARN).strong());
                            if sharp_button(ui, "Save").clicked() {
                                trigger_save = true;
                            }
                            if sharp_button(ui, "Discard").clicked() {
                                state.settings_confirm_discard = false;
                                keep_open = false;
                            }
                            let keep_btn = sharp_button(ui, "Keep editing");
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
                if sharp_button(ui, "Save").clicked() {
                    trigger_save = true;
                }
                if sharp_button(ui, "Cancel").clicked() {
                    state.settings_confirm_discard = false;
                    keep_open = false;
                }
            });
        });

    if trigger_save {
        save_draft(&mut draft, &mut state.settings, cmd_tx);
        state.settings_confirm_discard = false;
        keep_open = false;
    }

    if keep_open {
        state.settings_draft = Some(draft);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn tab_switching_dirty_marker_and_esc_state_machine() {
        let saved = Settings::default();
        let mut draft = saved.clone();
        assert!(!is_any_dirty(&draft, &saved));

        // Cycle tabs forward and backward wrapping around
        let t0 = SettingsTab::Downloads;
        assert_eq!(t0.cycle(1), SettingsTab::Network);
        assert_eq!(t0.cycle(-1), SettingsTab::Appearance);
        assert_eq!(SettingsTab::Advanced.cycle(1), SettingsTab::Appearance);

        // Dirty marker is per-tab
        draft.auto_extract = !draft.auto_extract;
        assert!(is_tab_dirty(SettingsTab::Downloads, &draft, &saved));
        assert!(!is_tab_dirty(SettingsTab::Network, &draft, &saved));
        assert!(is_any_dirty(&draft, &saved));

        // Esc when clean closes immediately
        let mut confirm = false;
        assert_eq!(
            handle_settings_esc(&saved, &saved, &mut confirm),
            SettingsEscOutcome::Close
        );
        assert!(!confirm);

        // First Esc when dirty shows confirm bar; second Esc means KeepEditing
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
    fn open_save_and_cancel_settings_draft_semantics() {
        let mut state = UiState::new();
        state.open_settings_modal();
        assert!(state.settings_draft.is_some());
        assert!(!state.settings_confirm_discard);

        let (tx, mut rx) = mpsc::unbounded_channel();

        // Cancel leaves state.settings untouched and sends nothing
        state.settings_draft.as_mut().unwrap().quota_soft = 50;
        state.settings_draft = None;
        assert!(rx.try_recv().is_err());
        assert_eq!(state.settings.quota_soft, 150);

        // Save sanitizes and sends UpdateSettings
        state.open_settings_modal();
        let mut draft = state.settings_draft.take().unwrap();
        draft.quota_soft = 250;
        draft.quota_hard = 120;
        draft.batch_limit = 150;
        save_draft(&mut draft, &mut state.settings, &tx);

        assert_eq!(state.settings.quota_soft, 120);
        assert_eq!(state.settings.batch_limit, 100);
        match rx.try_recv() {
            Ok(UiToWorkerCommand::UpdateSettings(sent)) => {
                assert_eq!(sent.quota_soft, 120);
                assert_eq!(sent.batch_limit, 100);
            }
            other => panic!("expected UpdateSettings, got {other:?}"),
        }
    }

    #[test]
    fn cleanup_zip_checkbox_disabled_when_extraction_off() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let artists = Vec::new();
        let folders = HashMap::new();
        let picked = Arc::new(Mutex::new(None));

        for auto_extract in [false, true] {
            let mut draft = Settings {
                auto_extract,
                ..Settings::default()
            };
            let mut filter = String::new();
            let mut saw_cleanup_enabled = None;

            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut tctx = TabCtx {
                        draft: &mut draft,
                        artists: &artists,
                        folders: &folders,
                        folder_filter: &mut filter,
                        picked_out_dir: &picked,
                        cmd_tx: &tx,
                        rt_handle: rt.handle(),
                        paths: None,
                        focus_first: false,
                    };
                    downloads::show(ui, &mut tctx);
                    // Verify enabled state inside a matching add_enabled scope
                    let probe = ui.add_enabled(
                        tctx.draft.auto_extract,
                        egui::Checkbox::new(&mut tctx.draft.delete_zip_after_extract, "probe"),
                    );
                    saw_cleanup_enabled = Some(probe.enabled());
                });
            });

            assert_eq!(saw_cleanup_enabled, Some(auto_extract));
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
}
