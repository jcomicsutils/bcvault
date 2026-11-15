use crate::{paint, state::FxState};
use bcvault_core::{
    groups::{
        ArtistGroup, Condition, ConditionKind, DownloadState, GroupError, GroupRule, LibraryState,
        MAX_GROUPS, MAX_RULE_CONDITIONS, MatchMode, RulePreset, preview_rule, validate_group_name,
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align2, Context, Order, RichText, Ui};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug)]
pub struct FxGroupEditorState {
    pub editing_id: Option<i64>,
    pub name: String,
    pub is_automatic: bool,
    pub rule: GroupRule,
    pub pending_pin_ids: Vec<i64>,
}

impl FxGroupEditorState {
    pub fn new_manual(pending_pin_ids: Vec<i64>) -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            is_automatic: false,
            rule: GroupRule {
                mode: MatchMode::All,
                conditions: vec![Condition {
                    negate: false,
                    kind: ConditionKind::NameContains(String::new()),
                }],
            },
            pending_pin_ids,
        }
    }

    pub fn from_group(group: &ArtistGroup) -> Self {
        let is_automatic = group.rule.is_some();
        let rule = group.rule.clone().unwrap_or_else(|| GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind: ConditionKind::NameContains(String::new()),
            }],
        });
        Self {
            editing_id: Some(group.id),
            name: group.name.clone(),
            is_automatic,
            rule,
            pending_pin_ids: Vec::new(),
        }
    }

    pub fn validate(
        &self,
        existing_groups: &[ArtistGroup],
    ) -> Result<(String, Option<GroupRule>), String> {
        let name = validate_group_name(&self.name).map_err(|e| e.to_string())?;
        if self.editing_id.is_none() && existing_groups.len() >= MAX_GROUPS {
            return Err(GroupError::TooManyGroups { max: MAX_GROUPS }.to_string());
        }
        let dup = existing_groups
            .iter()
            .any(|g| Some(g.id) != self.editing_id && g.name.eq_ignore_ascii_case(&name));
        if dup {
            return Err(GroupError::DuplicateName(name).to_string());
        }
        let rule = if self.is_automatic {
            self.rule.validate().map_err(|e| e.to_string())?;
            Some(self.rule.clone())
        } else {
            None
        };
        Ok((name, rule))
    }
}

pub fn show_fx_group_editor_modal(
    ctx: &Context,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut editor) = state.group_editor.take() else {
        return;
    };
    let th = state.active_theme;
    let mut keep_open = !esc;
    let title = if editor.editing_id.is_some() {
        "Edit Group"
    } else {
        "New Group"
    };

    egui::Window::new(title)
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .max_size([620.0, 560.0])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(520.0);

            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.add(
                    egui::TextEdit::singleline(&mut editor.name)
                        .hint_text("Group name (1–40 chars)")
                        .desired_width(280.0),
                );
            });

            ui.horizontal(|ui| {
                ui.label("Kind:");
                if ui.radio(!editor.is_automatic, "Manual").clicked() {
                    editor.is_automatic = false;
                }
                if ui.radio(editor.is_automatic, "Automatic (rules)").clicked() {
                    editor.is_automatic = true;
                }
            });

            if !editor.pending_pin_ids.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "{} selected artist(s) will be placed in this group.",
                        editor.pending_pin_ids.len()
                    ))
                    .color(th.text_dim),
                );
            }

            ui.separator();

            if editor.is_automatic {
                ui.horizontal(|ui| {
                    ui.label("Match:");
                    ui.selectable_value(&mut editor.rule.mode, MatchMode::All, "All conditions");
                    ui.selectable_value(&mut editor.rule.mode, MatchMode::Any, "Any condition");

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("Presets", |ui| {
                            for preset in RulePreset::ALL {
                                if paint::sharp_button(ui, preset.label()).clicked() {
                                    let (preset_name, preset_rule) = preset.build();
                                    if editor.name.trim().is_empty() {
                                        editor.name = preset_name.to_string();
                                    }
                                    editor.rule = preset_rule;
                                    ui.close();
                                }
                            }
                        });
                    });
                });

                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        let mut remove_idx: Option<usize> = None;
                        for idx in 0..editor.rule.conditions.len() {
                            ui.horizontal(|ui| {
                                draw_fx_condition_row(ui, idx, &mut editor.rule.conditions[idx]);
                                if paint::sharp_button(ui, "x").clicked() {
                                    remove_idx = Some(idx);
                                }
                            });
                        }
                        if let Some(idx) = remove_idx {
                            editor.rule.conditions.remove(idx);
                        }
                    });

                ui.horizontal(|ui| {
                    let can_add = editor.rule.conditions.len() < MAX_RULE_CONDITIONS;
                    if ui
                        .add_enabled(
                            can_add,
                            egui::Button::new("+ Add condition")
                                .corner_radius(egui::CornerRadius::ZERO),
                        )
                        .clicked()
                    {
                        editor.rule.conditions.push(Condition {
                            negate: false,
                            kind: ConditionKind::NameContains(String::new()),
                        });
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}/{} conditions",
                            editor.rule.conditions.len(),
                            MAX_RULE_CONDITIONS
                        ))
                        .size(11.0)
                        .color(th.text_dim),
                    );
                });

                ui.separator();

                let now = chrono::Utc::now().timestamp();
                let preview = preview_rule(
                    &editor.rule,
                    &state.artists,
                    &state.pins,
                    &state.folders,
                    now,
                    editor.editing_id,
                );
                let preview_line = if preview.pinned_elsewhere > 0 {
                    format!(
                        "Matches {} artists ({} are placed elsewhere by hand)",
                        preview.total_matching, preview.pinned_elsewhere
                    )
                } else {
                    format!("Matches {} artists", preview.total_matching)
                };
                ui.label(RichText::new(preview_line).strong());
                if !preview.sample_names.is_empty() {
                    ui.label(
                        RichText::new(preview.sample_names.join(", "))
                            .size(11.0)
                            .color(th.text_dim),
                    );
                }
            } else {
                ui.label(
                    RichText::new("Manual groups only contain artists you move into them by hand.")
                        .color(th.text_dim),
                );
            }

            ui.add_space(4.0);
            ui.label(
                RichText::new("Artists you move by hand always stay where you put them. Groups higher in the list win when several rules match.")
                    .size(11.0)
                    .color(th.text_dim),
            );

            let validation = editor.validate(&state.groups);
            if let Err(err) = &validation {
                ui.colored_label(th.err, err);
            }
            if let Some(err) = &state.group_error {
                ui.colored_label(th.err, err);
            }

            ui.separator();
            ui.horizontal(|ui| {
                let valid = validation.is_ok();
                if ui
                    .add_enabled(
                        valid,
                        egui::Button::new("Save").corner_radius(egui::CornerRadius::ZERO),
                    )
                    .clicked()
                    && let Ok((name, rule)) = validation
                {
                    if let Some(id) = editor.editing_id {
                        let _ = core_tx.send(UiToWorkerCommand::UpdateArtistGroup {
                            id,
                            name,
                            rule,
                        });
                    } else {
                        if !editor.pending_pin_ids.is_empty() {
                            state.pending_group_pins =
                                Some((name.clone(), editor.pending_pin_ids.clone()));
                        }
                        let _ = core_tx.send(UiToWorkerCommand::CreateArtistGroup { name, rule });
                    }
                    keep_open = false;
                }
                if paint::sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.group_editor = Some(editor);
    }
}

fn draw_fx_condition_row(ui: &mut Ui, idx: usize, cond: &mut Condition) {
    ui.checkbox(&mut cond.negate, "not");

    let current_label = cond.kind.label();
    egui::ComboBox::from_id_salt(("fx_cond_kind", idx))
        .selected_text(current_label)
        .width(160.0)
        .show_ui(ui, |ui| {
            let variants: [(&str, ConditionKind); 9] = [
                ("Name contains", ConditionKind::NameContains(String::new())),
                (
                    "Name starts with",
                    ConditionKind::NameStartsWith(String::new()),
                ),
                (
                    "Name initial range",
                    ConditionKind::NameInitial { from: 'a', to: 'f' },
                ),
                (
                    "Downloads",
                    ConditionKind::Downloads(DownloadState::HasRemaining),
                ),
                ("Library", ConditionKind::Library(LibraryState::NoFolder)),
                (
                    "Synced older than (days)",
                    ConditionKind::SyncedOlderThanDays(30),
                ),
                ("Never synced", ConditionKind::NeverSynced),
                (
                    "Total releases",
                    ConditionKind::TotalReleases {
                        at_least: Some(1),
                        at_most: None,
                    },
                ),
                ("Custom domain", ConditionKind::CustomDomain(true)),
            ];
            for (label, default_kind) in variants {
                let selected =
                    std::mem::discriminant(&cond.kind) == std::mem::discriminant(&default_kind);
                if ui.selectable_label(selected, label).clicked() && !selected {
                    cond.kind = default_kind;
                }
            }
        });

    match &mut cond.kind {
        ConditionKind::NameContains(s) | ConditionKind::NameStartsWith(s) => {
            ui.add(egui::TextEdit::singleline(s).desired_width(150.0));
        }
        ConditionKind::NameInitial { from, to } => {
            let mut is_hash = *from == '#' && *to == '#';
            if ui.checkbox(&mut is_hash, "# (0-9/sym)").changed() {
                if is_hash {
                    *from = '#';
                    *to = '#';
                } else {
                    *from = 'a';
                    *to = 'f';
                }
            }
            if !is_hash {
                let mut f_str = from.to_string();
                let mut t_str = to.to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut f_str).desired_width(24.0))
                    .changed()
                    && let Some(c) = f_str.chars().next()
                {
                    *from = c;
                }
                ui.label("to");
                if ui
                    .add(egui::TextEdit::singleline(&mut t_str).desired_width(24.0))
                    .changed()
                    && let Some(c) = t_str.chars().next()
                {
                    *to = c;
                }
            }
        }
        ConditionKind::Downloads(dl) => {
            egui::ComboBox::from_id_salt(("fx_cond_dl", idx))
                .selected_text(dl.label())
                .width(150.0)
                .show_ui(ui, |ui| {
                    for variant in DownloadState::ALL {
                        ui.selectable_value(dl, variant, variant.label());
                    }
                });
        }
        ConditionKind::Library(lib) => {
            egui::ComboBox::from_id_salt(("fx_cond_lib", idx))
                .selected_text(lib.label())
                .width(150.0)
                .show_ui(ui, |ui| {
                    for variant in LibraryState::ALL {
                        ui.selectable_value(lib, variant, variant.label());
                    }
                });
        }
        ConditionKind::SyncedOlderThanDays(days) => {
            ui.add(egui::DragValue::new(days).range(1..=3650).suffix(" d"));
        }
        ConditionKind::NeverSynced => {}
        ConditionKind::TotalReleases { at_least, at_most } => {
            let mut min_v = at_least.unwrap_or(0);
            let mut has_min = at_least.is_some();
            if ui.checkbox(&mut has_min, "min").changed() {
                *at_least = if has_min { Some(min_v) } else { None };
            }
            if has_min
                && ui
                    .add(egui::DragValue::new(&mut min_v).range(0..=10000))
                    .changed()
            {
                *at_least = Some(min_v);
            }
            let mut max_v = at_most.unwrap_or(10);
            let mut has_max = at_most.is_some();
            if ui.checkbox(&mut has_max, "max").changed() {
                *at_most = if has_max { Some(max_v) } else { None };
            }
            if has_max
                && ui
                    .add(egui::DragValue::new(&mut max_v).range(0..=10000))
                    .changed()
            {
                *at_most = Some(max_v);
            }
        }
        ConditionKind::CustomDomain(val) => {
            ui.checkbox(val, "is custom domain");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_15_fx_group_editor_validation_and_save_gating() {
        let existing = vec![ArtistGroup {
            id: 1,
            name: "Ambient".into(),
            pos: 0,
            collapsed: false,
            rule: None,
        }];

        let mut ed = FxGroupEditorState::new_manual(vec![]);
        assert!(ed.validate(&existing).is_err());

        ed.name = "ambient".into();
        assert!(ed.validate(&existing).is_err());

        ed.name = "Electronic".into();
        assert!(ed.validate(&existing).is_ok());

        ed.is_automatic = true;
        ed.rule = GroupRule {
            mode: MatchMode::All,
            conditions: vec![Condition {
                negate: false,
                kind: ConditionKind::NameInitial { from: 'z', to: 'a' },
            }],
        };
        assert!(ed.validate(&existing).is_err());

        ed.rule.conditions[0].kind = ConditionKind::NameInitial { from: 'a', to: 'f' };
        assert!(ed.validate(&existing).is_ok());
    }
}
