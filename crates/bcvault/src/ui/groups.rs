use crate::ui::{state::UiState, theme::pal, widgets::sharp_button};
use bcvault_core::{
    groups::{
        ActiveGrouping, ArtistGroup, ArtistGroupSet, GroupError, MAX_GROUP_SETS, MAX_GROUPS,
        artist_matches_filter, validate_group_name,
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align2, Context, Order, RichText};
use std::collections::{BTreeSet, HashMap};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug)]
pub struct GroupEditorState {
    pub editing_id: Option<i64>,
    pub name: String,
    pub artist_filter: String,
    pub selected_members: BTreeSet<i64>,
}

impl GroupEditorState {
    pub fn new_manual(pending_pin_ids: Vec<i64>) -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            artist_filter: String::new(),
            selected_members: pending_pin_ids.into_iter().collect(),
        }
    }

    pub fn from_group(group: &ArtistGroup, current_members: &[i64]) -> Self {
        Self {
            editing_id: Some(group.id),
            name: group.name.clone(),
            artist_filter: String::new(),
            selected_members: current_members.iter().copied().collect(),
        }
    }

    pub fn validate(&self, existing_groups: &[ArtistGroup]) -> Result<String, String> {
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
        Ok(name)
    }
}

#[derive(Clone, Debug)]
pub struct GroupSetModalState {
    pub editing_id: Option<i64>,
    pub name: String,
}

impl GroupSetModalState {
    pub fn new() -> Self {
        Self {
            editing_id: None,
            name: String::new(),
        }
    }

    pub fn rename(set: &ArtistGroupSet) -> Self {
        Self {
            editing_id: Some(set.id),
            name: set.name.clone(),
        }
    }

    pub fn validate(&self, existing_sets: &[ArtistGroupSet]) -> Result<String, String> {
        let name = validate_group_name(&self.name).map_err(|e| e.to_string())?;
        if self.editing_id.is_none() && existing_sets.len() >= MAX_GROUP_SETS {
            return Err(GroupError::TooManyGroupSets {
                max: MAX_GROUP_SETS,
            }
            .to_string());
        }
        let dup = existing_sets
            .iter()
            .any(|s| Some(s.id) != self.editing_id && s.name.eq_ignore_ascii_case(&name));
        if dup {
            return Err(GroupError::DuplicateSetName(name).to_string());
        }
        Ok(name)
    }
}

impl Default for GroupSetModalState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn show_group_editor_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut editor) = state.group_editor_modal.take() else {
        return;
    };
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
        .max_size([540.0, 560.0])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(460.0);

            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.add(
                    egui::TextEdit::singleline(&mut editor.name)
                        .hint_text("Group name (1–40 chars)")
                        .desired_width(280.0),
                );
            });

            ui.separator();

            let group_names: HashMap<i64, &str> = state
                .groups
                .iter()
                .map(|g| (g.id, g.name.as_str()))
                .collect();
            let filter_lower = editor.artist_filter.trim().to_lowercase();
            let visible_ids: Vec<i64> = state
                .artists
                .iter()
                .filter(|a| artist_matches_filter(a, &filter_lower))
                .map(|a| a.band_id)
                .collect();

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Artists ({} selected)",
                        editor.selected_members.len()
                    ))
                    .strong()
                    .color(pal::TEXT),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut editor.artist_filter)
                        .hint_text("Filter artists...")
                        .desired_width(170.0),
                );
                if sharp_button(ui, "Select visible").clicked() {
                    for id in &visible_ids {
                        editor.selected_members.insert(*id);
                    }
                }
                if sharp_button(ui, "Clear").clicked() {
                    editor.selected_members.clear();
                }
            });

            ui.add_space(4.0);
            egui::Frame::NONE
                .fill(pal::BG0)
                .stroke(egui::Stroke::new(1.0_f32, pal::BORDER))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            if state.artists.is_empty() {
                                ui.label(RichText::new("No artists yet.").color(pal::TEXT_DIM));
                            } else if visible_ids.is_empty() {
                                ui.label(
                                    RichText::new("No artists match the filter.")
                                        .color(pal::TEXT_DIM),
                                );
                            } else {
                                for artist in &state.artists {
                                    if !artist_matches_filter(artist, &filter_lower) {
                                        continue;
                                    }
                                    let mut checked =
                                        editor.selected_members.contains(&artist.band_id);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .checkbox(&mut checked, artist.display_name())
                                            .changed()
                                        {
                                            if checked {
                                                editor.selected_members.insert(artist.band_id);
                                            } else {
                                                editor.selected_members.remove(&artist.band_id);
                                            }
                                        }
                                        if let Some(&other_gid) = state.pins.get(&artist.band_id)
                                            && Some(other_gid) != editor.editing_id
                                            && let Some(other_name) = group_names.get(&other_gid)
                                        {
                                            ui.label(
                                                RichText::new(format!("[in {other_name}]"))
                                                    .size(11.0)
                                                    .color(pal::TEXT_DIM),
                                            );
                                        }
                                    });
                                }
                            }
                        });
                });

            let validation = editor.validate(&state.groups);
            if let Err(err) = &validation {
                ui.add_space(4.0);
                ui.label(RichText::new(err).color(pal::ERR));
            }
            if let Some(server_err) = &state.group_error {
                ui.add_space(2.0);
                ui.label(RichText::new(server_err).color(pal::ERR));
            }

            ui.separator();
            ui.horizontal(|ui| {
                let can_save = validation.is_ok();
                if ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .clicked()
                    && let Ok(name) = validation
                {
                    let member_ids: Vec<i64> = editor.selected_members.iter().copied().collect();
                    if let Some(id) = editor.editing_id {
                        let _ = cmd_tx.send(UiToWorkerCommand::UpdateArtistGroup {
                            id,
                            name,
                            member_ids,
                        });
                    } else {
                        let set_id = match state.active_grouping {
                            ActiveGrouping::CustomSet(sid) => Some(sid),
                            _ => None,
                        };
                        let _ = cmd_tx.send(UiToWorkerCommand::CreateArtistGroup {
                            set_id,
                            name,
                            member_ids,
                        });
                    }
                    keep_open = false;
                }
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.group_editor_modal = Some(editor);
    }
}

pub fn show_group_set_modal(
    ctx: &Context,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    esc: bool,
) {
    let Some(mut modal) = state.group_set_modal.take() else {
        return;
    };
    let mut keep_open = !esc;
    let title = if modal.editing_id.is_some() {
        "Rename Grouping Set"
    } else {
        "New Grouping Set"
    };

    egui::Window::new(title)
        .order(Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.horizontal(|ui| {
                ui.label("Set name:");
                ui.add(
                    egui::TextEdit::singleline(&mut modal.name)
                        .hint_text("e.g. Workflow, Favorites")
                        .desired_width(240.0),
                );
            });
            let validation = modal.validate(&state.group_sets);
            if let Err(err) = &validation {
                ui.label(RichText::new(err).color(pal::ERR));
            }
            if let Some(server_err) = &state.group_error {
                ui.label(RichText::new(server_err).color(pal::ERR));
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(validation.is_ok(), egui::Button::new("Save"))
                    .clicked()
                    && let Ok(name) = validation
                {
                    if let Some(id) = modal.editing_id {
                        let _ = cmd_tx.send(UiToWorkerCommand::RenameArtistGroupSet { id, name });
                    } else {
                        let _ = cmd_tx.send(UiToWorkerCommand::CreateArtistGroupSet { name });
                    }
                    keep_open = false;
                }
                if sharp_button(ui, "Cancel").clicked() {
                    keep_open = false;
                }
            });
        });

    if keep_open {
        state.group_set_modal = Some(modal);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_15_classic_group_editor_validation_and_save_gating() {
        let existing = vec![ArtistGroup {
            id: 1,
            set_id: 1,
            name: "Ambient".into(),
            pos: 0,
            collapsed: false,
        }];
        let mut ed = GroupEditorState::new_manual(vec![10, 20]);
        assert!(ed.validate(&existing).is_err());

        ed.name = "ambient".into();
        assert!(ed.validate(&existing).is_err());

        ed.name = "Favorites".into();
        assert_eq!(ed.validate(&existing).unwrap(), "Favorites");
        assert_eq!(ed.selected_members, BTreeSet::from([10, 20]));

        let edit_existing = GroupEditorState::from_group(&existing[0], &[10]);
        assert_eq!(edit_existing.validate(&existing).unwrap(), "Ambient");
    }
}
