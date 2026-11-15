use crate::{
    actions::{FxAction, SmartView},
    motion::SpringCfg,
    paint,
    services::messages::FxCommand,
    state::{FxLinkFolderModalState, FxState},
};
use bcvault_core::{
    types::{ArtistStats, FolderState, SyncMode, SyncScope},
    worker::messages::UiToWorkerCommand,
};
use egui::{Align, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(
    ui: &mut Ui,
    state: &mut FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    fx_tx: &UnboundedSender<FxCommand>,
    now_s: f64,
) {
    let th = state.active_theme;
    let mut pending_actions: Vec<FxAction> = Vec::new();

    if state.sidebar_collapsed {
        ui.vertical_centered(|ui| {
            if paint::sharp_button(ui, "»")
                .on_hover_text("Expand sidebar (Ctrl+B)")
                .clicked()
            {
                pending_actions.push(FxAction::ToggleSidebar);
            }
            ui.separator();
            if paint::sharp_button(ui, "ALL")
                .on_hover_text("All artists")
                .clicked()
            {
                pending_actions.push(FxAction::SelectArtist(None));
            }
            for a in state.artists.iter().take(16) {
                let initials: String = a
                    .display_name()
                    .chars()
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                let is_sel = state.query.base.band_id == Some(a.band_id);
                let btn = egui::Button::new(RichText::new(initials).monospace().color(if is_sel {
                    th.accent
                } else {
                    th.text
                }))
                .corner_radius(CornerRadius::ZERO);
                if ui
                    .add_sized([40.0, 28.0], btn)
                    .on_hover_text(a.display_name())
                    .clicked()
                {
                    pending_actions.push(FxAction::SelectArtist(Some(a.band_id)));
                }
            }
        });
        for act in pending_actions {
            state.dispatch(act, core_tx, fx_tx, now_s);
        }
        return;
    }

    // 1. Add Artist input bar
    let top_row_w = ui.available_width();
    let (collapse_clicked, add_clicked, enter_pressed) = ui
        .allocate_ui_with_layout(
            Vec2::new(top_row_w, 28.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                let collapse = paint::sharp_button(ui, "«")
                    .on_hover_text("Collapse sidebar (Ctrl+B)")
                    .clicked();
                let add = ui
                    .add_enabled(
                        !state.add_artist_pending && !state.add_artist_input.trim().is_empty(),
                        egui::Button::new("Add").corner_radius(CornerRadius::ZERO),
                    )
                    .clicked();
                let te = egui::TextEdit::singleline(&mut state.add_artist_input)
                    .hint_text("Add Artist (URL or Slug)")
                    .desired_width(ui.available_width());
                let resp = ui.add_enabled(!state.add_artist_pending, te);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                (collapse, add, enter)
            },
        )
        .inner;

    if collapse_clicked {
        pending_actions.push(FxAction::ToggleSidebar);
    }
    if (enter_pressed || add_clicked) && !state.add_artist_input.trim().is_empty() {
        let input = state.add_artist_input.trim().to_string();
        state.add_artist_input.clear();
        state.add_artist_pending = true;
        let _ = core_tx.send(UiToWorkerCommand::AddArtist { input });
    }

    // 2. Smart Views & Saved Views
    ui.collapsing(
        RichText::new("SMART VIEWS").size(11.0).color(th.text_dim),
        |ui| {
            for sv in SmartView::ALL {
                let active = state.query.smart == Some(sv);
                let resp = ui.selectable_label(
                    active,
                    RichText::new(sv.label()).size(12.0).color(if active {
                        th.accent
                    } else {
                        th.text
                    }),
                );
                if resp.clicked() {
                    let next = if active { None } else { Some(sv) };
                    pending_actions.push(FxAction::SelectSmartView(next));
                }
            }
        },
    );

    ui.collapsing(
        RichText::new("SAVED VIEWS").size(11.0).color(th.text_dim),
        |ui| {
            let views = state.saved_views.clone();
            for v in views {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(false, RichText::new(&v.name).size(12.0).color(th.text))
                        .clicked()
                    {
                        pending_actions.push(FxAction::ApplySavedView(v.id));
                    }
                    if paint::sharp_button(ui, "×")
                        .on_hover_text("Delete saved view")
                        .clicked()
                    {
                        pending_actions.push(FxAction::DeleteSavedView(v.id));
                    }
                });
            }
            if paint::sharp_button(ui, "+ Save Current View...").clicked() {
                state.save_view_modal = Some(String::new());
            }
        },
    );

    // 3. Artist filter
    ui.add(
        egui::TextEdit::singleline(&mut state.artist_filter)
            .hint_text("Filter artists...")
            .desired_width(f32::INFINITY),
    );

    show_fx_grouping_bar(ui, state, &mut pending_actions);

    ui.separator();

    let list_height = (ui.available_height() - 78.0).max(80.0);
    state.ensure_sidebar_rows_at(chrono::Utc::now().timestamp(), now_s);
    let rows = state.sidebar_rows.clone();

    let mut total_stats = ArtistStats::default();
    for a in &state.artists {
        total_stats.downloaded += a.stats.downloaded;
        total_stats.eligible_downloaded += a.stats.eligible_downloaded;
        total_stats.non_eligible_downloaded += a.stats.non_eligible_downloaded;
        total_stats.eligible += a.stats.eligible;
        total_stats.total += a.stats.total;
        total_stats.email_required += a.stats.email_required;
        total_stats.unknown += a.stats.unknown;
        total_stats.quota_used_30d += a.stats.quota_used_30d;
    }

    let total_rows = rows.len();

    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show_rows(ui, 28.0, total_rows, |ui, row_range| {
            let content_x = ui.cursor().min.x;
            let content_w = ui.available_width();
            let content_origin_y = ui.cursor().min.y - (row_range.start as f32 * 28.0);
            paint_sidebar_ghosts(ui, state, content_origin_y, content_x, content_w, now_s);

            for row_idx in row_range {
                let row_key = crate::state::SidebarRowKey::from_row(&rows[row_idx]);
                let (draw_offset, row_alpha) = row_flip_and_alpha(state, row_key, row_idx);
                match &rows[row_idx] {
                    bcvault_core::groups::SidebarRow::AllArtists => {
                        let is_selected = state.selected_artists.is_empty()
                            && state.query.base.band_id.is_none()
                            && state.query.base.band_ids.is_empty();
                        let resp = draw_artist_row(
                            ui,
                            state,
                            0,
                            "All artists",
                            total_stats,
                            is_selected,
                            None,
                            now_s,
                            draw_offset,
                            row_alpha,
                        );
                        if resp.clicked() {
                            pending_actions.push(FxAction::SelectArtist(None));
                        }
                        resp.context_menu(|ui| {
                            if paint::sharp_button(ui, "New group...").clicked() {
                                pending_actions.push(FxAction::NewGroup);
                                ui.close();
                            }
                            if paint::sharp_button(ui, "New grouping set...").clicked() {
                                pending_actions.push(FxAction::NewGroupSet);
                                ui.close();
                            }
                            if !state.groups.is_empty() || !state.builtin_groups.is_empty() {
                                ui.separator();
                                if paint::sharp_button(ui, "Collapse all groups").clicked() {
                                    pending_actions.push(FxAction::CollapseAllGroups);
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Expand all groups").clicked() {
                                    pending_actions.push(FxAction::ExpandAllGroups);
                                    ui.close();
                                }
                            }
                        });
                    }
                    bcvault_core::groups::SidebarRow::GroupHeader {
                        id,
                        name,
                        count,
                        collapsed,
                    } => {
                        let member_ids = state.group_member_ids(*id);
                        let (dl, elig) = sum_fx_member_stats(state, &member_ids);
                        let resp = draw_fx_group_header_row(
                            ui,
                            state,
                            row_key,
                            name,
                            *count,
                            *collapsed,
                            dl,
                            elig,
                            draw_offset,
                            row_alpha,
                        );
                        if matches!(
                            state.active_grouping,
                            bcvault_core::groups::ActiveGrouping::CustomSet(_)
                        ) && *id > 0
                        {
                            if resp.dnd_hover_payload::<Vec<i64>>().is_some() {
                                paint::rect(
                                    ui.painter(),
                                    resp.rect,
                                    egui::Color32::TRANSPARENT,
                                    egui::Stroke::new(2.0_f32, state.active_theme.accent),
                                );
                            }
                            if let Some(dragged_ids) = resp.dnd_release_payload::<Vec<i64>>() {
                                pending_actions.push(FxAction::MoveArtistsToGroup(
                                    (*dragged_ids).clone(),
                                    bcvault_core::groups::PinTarget::Group(*id),
                                ));
                            }
                        }
                        if resp.clicked() {
                            pending_actions.push(FxAction::ToggleGroup(*id));
                        }
                        let group_id = *id;
                        let group_name = name.clone();
                        resp.context_menu(|ui| {
                            show_fx_group_context_menu(
                                ui,
                                state,
                                core_tx,
                                &mut pending_actions,
                                group_id,
                                &group_name,
                                &member_ids,
                            );
                        });
                    }
                    bcvault_core::groups::SidebarRow::UngroupedHeader { count, collapsed } => {
                        let member_ids = state.group_assignment.ungrouped.clone();
                        let (dl, elig) = sum_fx_member_stats(state, &member_ids);
                        let resp = draw_fx_group_header_row(
                            ui,
                            state,
                            row_key,
                            "Ungrouped",
                            *count,
                            *collapsed,
                            dl,
                            elig,
                            draw_offset,
                            row_alpha,
                        );
                        if matches!(
                            state.active_grouping,
                            bcvault_core::groups::ActiveGrouping::CustomSet(_)
                        ) {
                            if resp.dnd_hover_payload::<Vec<i64>>().is_some() {
                                paint::rect(
                                    ui.painter(),
                                    resp.rect,
                                    egui::Color32::TRANSPARENT,
                                    egui::Stroke::new(2.0_f32, state.active_theme.accent),
                                );
                            }
                            if let Some(dragged_ids) = resp.dnd_release_payload::<Vec<i64>>() {
                                pending_actions.push(FxAction::MoveArtistsToGroup(
                                    (*dragged_ids).clone(),
                                    bcvault_core::groups::PinTarget::Ungrouped,
                                ));
                            }
                        }
                        if resp.clicked() {
                            pending_actions.push(FxAction::ToggleUngrouped);
                        }
                        resp.context_menu(|ui| {
                            if paint::sharp_button(ui, "Sync ungrouped").clicked() {
                                let _ = core_tx.send(UiToWorkerCommand::Sync {
                                    scope: SyncScope::Selected(member_ids.clone()),
                                    mode: state.sync_mode,
                                });
                                ui.close();
                            }
                            if paint::sharp_button(ui, "Download remaining for ungrouped").clicked()
                            {
                                state.trigger_download_remaining_artists(
                                    "Download Remaining for Ungrouped".to_string(),
                                    member_ids.clone(),
                                    core_tx,
                                );
                                ui.close();
                            }
                        });
                    }
                    bcvault_core::groups::SidebarRow::Artist { band_id } => {
                        let Some(artist) = state
                            .artists
                            .iter()
                            .find(|a| a.band_id == *band_id)
                            .cloned()
                        else {
                            continue;
                        };
                        let is_selected = state.selected_artists.contains(&artist.band_id)
                            || state.query.base.band_id == Some(artist.band_id);
                        let sync_frac = state.active_syncs.get(&artist.band_id).map(|p| {
                            if p.total == 0 {
                                0.15_f32
                            } else {
                                (p.done as f32 / p.total as f32).clamp(0.05, 1.0)
                            }
                        });

                        let name_disp = bcvault_core::text::format_display_text(
                            artist.display_name(),
                            state.settings.mark_invisible_chars,
                        );
                        let resp = draw_artist_row(
                            ui,
                            state,
                            artist.band_id,
                            name_disp.as_ref(),
                            artist.stats,
                            is_selected,
                            sync_frac,
                            now_s,
                            draw_offset,
                            row_alpha,
                        );
                        if resp.drag_started() {
                            let payload: Vec<i64> =
                                if is_selected && state.selected_artists.len() > 1 {
                                    state.selected_artists.iter().copied().collect()
                                } else {
                                    vec![artist.band_id]
                                };
                            resp.dnd_set_drag_payload(payload);
                        }

                        let modifiers = ui.input(|i| i.modifiers);
                        if resp.clicked() {
                            if modifiers.shift
                                && let Some(anchor_id) = state.last_clicked_artist
                            {
                                let visible_ids: Vec<i64> = rows
                                    .iter()
                                    .filter_map(|r| match r {
                                        bcvault_core::groups::SidebarRow::Artist { band_id } => {
                                            Some(*band_id)
                                        }
                                        _ => None,
                                    })
                                    .collect();
                                let pos_a = visible_ids.iter().position(|&id| id == anchor_id);
                                let pos_b = visible_ids.iter().position(|&id| id == artist.band_id);
                                if let (Some(a), Some(b)) = (pos_a, pos_b) {
                                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                                    if !modifiers.ctrl && !modifiers.command {
                                        state.selected_artists.clear();
                                    }
                                    for &bid in &visible_ids[lo..=hi] {
                                        state.selected_artists.insert(bid);
                                    }
                                    let ids: Vec<i64> =
                                        state.selected_artists.iter().copied().collect();
                                    pending_actions.push(FxAction::SelectArtists(ids));
                                } else {
                                    pending_actions
                                        .push(FxAction::SelectArtist(Some(artist.band_id)));
                                }
                            } else if modifiers.ctrl || modifiers.command {
                                if state.selected_artists.contains(&artist.band_id) {
                                    state.selected_artists.remove(&artist.band_id);
                                } else {
                                    state.selected_artists.insert(artist.band_id);
                                }
                                state.last_clicked_artist = Some(artist.band_id);
                                let ids: Vec<i64> =
                                    state.selected_artists.iter().copied().collect();
                                pending_actions.push(FxAction::SelectArtists(ids));
                            } else {
                                pending_actions.push(FxAction::SelectArtist(Some(artist.band_id)));
                            }
                        }

                        resp.context_menu(|ui| {
                            if state.selected_artists.contains(&artist.band_id)
                                && state.selected_artists.len() > 1
                            {
                                let sel_ids: Vec<i64> =
                                    state.selected_artists.iter().copied().collect();
                                if paint::sharp_button(
                                    ui,
                                    format!("Sync {} Selected Artists", sel_ids.len()),
                                )
                                .clicked()
                                {
                                    let _ = core_tx.send(UiToWorkerCommand::Sync {
                                        scope: SyncScope::Selected(sel_ids),
                                        mode: state.sync_mode,
                                    });
                                    ui.close();
                                }
                            } else if paint::sharp_button(ui, "Sync Artist").clicked() {
                                let _ = core_tx.send(UiToWorkerCommand::Sync {
                                    scope: SyncScope::Selected(vec![artist.band_id]),
                                    mode: state.sync_mode,
                                });
                                ui.close();
                            }
                            show_fx_move_to_group_submenu(
                                ui,
                                state,
                                &mut pending_actions,
                                artist.band_id,
                            );
                            let artist_folders = state
                                .folders
                                .get(&artist.band_id)
                                .cloned()
                                .unwrap_or_default();
                            if artist_folders.is_empty() {
                                if paint::sharp_button(ui, "Link Folder...").clicked() {
                                    state.link_folder_modal = Some(
                                        FxLinkFolderModalState::new_empty(Some(artist.band_id)),
                                    );
                                    ui.close();
                                }
                            } else if artist_folders.len() == 1 {
                                let folder = &artist_folders[0];
                                if folder.state != FolderState::Ok
                                    && paint::sharp_button(ui, "Resolve unavailable folder...")
                                        .clicked()
                                {
                                    let _ = core_tx.send(UiToWorkerCommand::VerifyFiles {
                                        scope: SyncScope::Selected(vec![artist.band_id]),
                                    });
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Add Folder...").clicked() {
                                    state.link_folder_modal = Some(
                                        FxLinkFolderModalState::new_empty(Some(artist.band_id)),
                                    );
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Change Folder...").clicked() {
                                    state.link_folder_modal =
                                        Some(FxLinkFolderModalState::for_repoint(
                                            artist.band_id,
                                            folder.id,
                                            folder.path.display().to_string(),
                                        ));
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Match Folder").clicked() {
                                    let _ = core_tx.send(UiToWorkerCommand::MatchArtistFolder {
                                        band_id: artist.band_id,
                                        auto_apply_confident: true,
                                    });
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Verify Files").clicked() {
                                    let _ = core_tx.send(UiToWorkerCommand::VerifyFiles {
                                        scope: SyncScope::Selected(vec![artist.band_id]),
                                    });
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Unlink Folder").clicked() {
                                    let _ = core_tx.send(UiToWorkerCommand::UnlinkArtistFolder {
                                        folder_id: folder.id,
                                    });
                                    ui.close();
                                }
                            } else {
                                if artist_folders.iter().any(|f| f.state != FolderState::Ok)
                                    && paint::sharp_button(ui, "Resolve unavailable folder...")
                                        .clicked()
                                {
                                    let _ = core_tx.send(UiToWorkerCommand::VerifyFiles {
                                        scope: SyncScope::Selected(vec![artist.band_id]),
                                    });
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Add Folder...").clicked() {
                                    state.link_folder_modal = Some(
                                        FxLinkFolderModalState::new_empty(Some(artist.band_id)),
                                    );
                                    ui.close();
                                }
                                if paint::sharp_button(ui, "Match Folder").clicked() {
                                    let _ = core_tx.send(UiToWorkerCommand::MatchArtistFolder {
                                        band_id: artist.band_id,
                                        auto_apply_confident: true,
                                    });
                                    ui.close();
                                }
                                ui.menu_button("Change folder...", |ui| {
                                    for folder in &artist_folders {
                                        let label = crate::state::folder_display_label(
                                            folder,
                                            &artist_folders,
                                        );
                                        if paint::sharp_button(
                                            ui,
                                            format!("Folder moved: change {label}..."),
                                        )
                                        .clicked()
                                        {
                                            state.link_folder_modal =
                                                Some(FxLinkFolderModalState::for_repoint(
                                                    artist.band_id,
                                                    folder.id,
                                                    folder.path.display().to_string(),
                                                ));
                                            ui.close();
                                        }
                                    }
                                });
                                ui.menu_button("Unlink folder...", |ui| {
                                    for folder in &artist_folders {
                                        let label = crate::state::folder_display_label(
                                            folder,
                                            &artist_folders,
                                        );
                                        if paint::sharp_button(ui, format!("Unlink {label}..."))
                                            .clicked()
                                        {
                                            let _ = core_tx.send(
                                                UiToWorkerCommand::UnlinkArtistFolder {
                                                    folder_id: folder.id,
                                                },
                                            );
                                            ui.close();
                                        }
                                    }
                                });
                                if paint::sharp_button(ui, "Verify Files").clicked() {
                                    let _ = core_tx.send(UiToWorkerCommand::VerifyFiles {
                                        scope: SyncScope::Selected(vec![artist.band_id]),
                                    });
                                    ui.close();
                                }
                            }
                            if paint::sharp_button(ui, "Rename Artist...").clicked() {
                                state.rename_artist_modal = Some((
                                    artist.band_id,
                                    artist.name.clone(),
                                    artist.display_name().to_string(),
                                ));
                                ui.close();
                            }
                            if paint::sharp_button(ui, "Open in Browser").clicked() {
                                let _ = open::that_detached(&artist.url);
                                ui.close();
                            }
                            if paint::sharp_button(ui, "Remove Artist...").clicked() {
                                state.confirm_remove_artist =
                                    Some((artist.band_id, artist.display_name().to_string()));
                                ui.close();
                            }
                        });
                    }
                }
            }
        });

    ui.separator();

    // 4. Footer: Sync controls
    ui.horizontal(|ui| {
        ui.label(RichText::new("Mode:").color(th.text_dim));
        egui::ComboBox::from_id_salt("fx_sync_mode")
            .selected_text(state.sync_mode.label())
            .width(135.0)
            .show_ui(ui, |ui| {
                for mode in SyncMode::ALL {
                    ui.selectable_value(&mut state.sync_mode, mode, mode.label());
                }
            });
    });

    ui.horizontal(|ui| {
        let has_sel = state.query.base.band_id.is_some();
        if ui
            .add_enabled(
                has_sel,
                egui::Button::new("Sync Selected").corner_radius(CornerRadius::ZERO),
            )
            .clicked()
        {
            pending_actions.push(FxAction::SyncSelected(state.sync_mode));
        }
        if paint::sharp_button(ui, "Sync All").clicked() {
            pending_actions.push(FxAction::SyncAll(state.sync_mode));
        }
        if !state.active_syncs.is_empty() && paint::sharp_button(ui, "Stop").clicked() {
            let _ = core_tx.send(UiToWorkerCommand::CancelSync);
        }
    });

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}

fn row_flip_and_alpha(
    state: &mut FxState,
    key: crate::state::SidebarRowKey,
    row_idx: usize,
) -> (Vec2, f32) {
    let target_rel = Vec2::new(0.0, row_idx as f32 * 28.0);
    let offset = if state.sidebar_flip_active.contains(&key) {
        let anim_rel = state
            .motion
            .vec2(("sb_flip", key), target_rel, SpringCfg::SMOOTH);
        let d = anim_rel - target_rel;
        if d.length_sq() < 0.16 {
            state.sidebar_flip_active.remove(&key);
            Vec2::ZERO
        } else {
            d
        }
    } else {
        Vec2::ZERO
    };
    let alpha = if state.sidebar_enter_active.contains(&key) {
        let a = state
            .motion
            .val(("sb_enter", key), 1.0, SpringCfg::SMOOTH)
            .clamp(0.0, 1.0);
        if (1.0 - a).abs() < 0.02 {
            state.sidebar_enter_active.remove(&key);
            1.0
        } else {
            a
        }
    } else {
        1.0
    };
    (offset, alpha)
}

fn paint_sidebar_ghosts(
    ui: &Ui,
    state: &mut FxState,
    content_origin_y: f32,
    content_x: f32,
    content_w: f32,
    now_s: f64,
) {
    state
        .sidebar_ghosts
        .retain(|g| now_s >= g.created_s && now_s - g.created_s < 0.16);
    if state.sidebar_ghosts.is_empty() || state.motion.is_instant() {
        return;
    }
    ui.ctx().request_repaint();
    let th = state.active_theme;
    let clip = ui.clip_rect();
    let painter = ui.painter();
    for g in &state.sidebar_ghosts {
        let t = ((now_s - g.created_s) / 0.16).clamp(0.0, 1.0) as f32;
        let alpha = (1.0 - t) * 0.65;
        let shift = if g.is_header {
            Vec2::new(-12.0 * t, 0.0)
        } else {
            Vec2::new(0.0, -8.0 * t)
        };
        let r = Rect::from_min_size(
            Pos2::new(content_x, content_origin_y + g.y_rel) + shift,
            Vec2::new(content_w, 26.0),
        );
        if !clip.intersects(r) {
            continue;
        }
        let bg = if g.is_header { th.bg0 } else { th.bg1 };
        paint::rect(
            painter,
            r,
            bg.gamma_multiply(alpha),
            Stroke::new(1.0_f32, th.border.gamma_multiply(alpha)),
        );
        if let Some(bid) = g.band_id {
            let mos_rect = Rect::from_min_size(
                Pos2::new(r.min.x + 6.0, r.center().y - 8.0),
                Vec2::splat(16.0),
            );
            paint::geometric_placeholder(
                painter,
                mos_rect,
                bid,
                th.bg0.gamma_multiply(alpha),
                th.accent.gamma_multiply(alpha),
                th.accent2.gamma_multiply(alpha),
            );
        }
        let text_x = if g.band_id.is_some() {
            r.min.x + 28.0
        } else {
            r.min.x + 12.0
        };
        painter.text(
            Pos2::new(text_x, r.center().y),
            egui::Align2::LEFT_CENTER,
            &g.label,
            FontId::proportional(12.0),
            th.text.gamma_multiply(alpha),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_artist_row(
    ui: &mut Ui,
    state: &mut FxState,
    band_id: i64,
    name: &str,
    stats: ArtistStats,
    selected: bool,
    sync_frac: Option<f32>,
    now_s: f64,
    draw_offset: Vec2,
    row_alpha: f32,
) -> egui::Response {
    let th = state.active_theme;
    let desired = Vec2::new(ui.available_width(), 26.0);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    let empty_folders = Vec::new();
    let artist_folders = if band_id > 0 {
        state
            .folders
            .get(&band_id)
            .map(|v| v.as_slice())
            .unwrap_or(&empty_folders)
    } else {
        &empty_folders
    };

    let draw_rect = rect.translate(draw_offset);
    if ui.is_rect_visible(rect) || ui.is_rect_visible(draw_rect) {
        let target_bg = if selected || resp.hovered() {
            th.bg2
        } else {
            th.bg1
        };
        let bg = state
            .motion
            .color(("art_bg", band_id), target_bg, SpringCfg::SNAP)
            .gamma_multiply(row_alpha);
        let sel_frac = state.motion.val(
            ("art_sel", band_id),
            if selected { 1.0 } else { 0.0 },
            SpringCfg::SNAP,
        );

        let painter = ui.painter();
        paint::rect(
            painter,
            draw_rect,
            bg,
            Stroke::new(1.0_f32, th.border.gamma_multiply(row_alpha)),
        );

        if sel_frac > 0.01 {
            let half_h = (draw_rect.height() * 0.5) * sel_frac;
            let cy = draw_rect.center().y;
            let bar = Rect::from_min_max(
                Pos2::new(draw_rect.min.x, cy - half_h),
                Pos2::new(draw_rect.min.x + 3.0, cy + half_h),
            );
            paint::rect_filled(painter, bar, th.accent.gamma_multiply(row_alpha));
        }

        // 2x2 miniature mosaic square on the left
        let mos_rect = Rect::from_min_size(
            Pos2::new(draw_rect.min.x + 6.0, draw_rect.center().y - 8.0),
            Vec2::splat(16.0),
        );
        paint::geometric_placeholder(
            painter,
            mos_rect,
            band_id,
            th.bg0.gamma_multiply(row_alpha),
            th.accent.gamma_multiply(row_alpha),
            th.accent2.gamma_multiply(row_alpha),
        );

        if let Some(frac) = sync_frac {
            let prog_h = 2.0;
            let fill_w = (draw_rect.width() * frac).min(draw_rect.width());
            let fill_r = Rect::from_min_max(
                Pos2::new(draw_rect.min.x, draw_rect.max.y - prog_h),
                Pos2::new(draw_rect.min.x + fill_w, draw_rect.max.y),
            );
            paint::rect_filled(painter, fill_r, th.accent.gamma_multiply(row_alpha));
            let sweep_x = draw_rect.min.x + ((now_s as f32 * 1.4).rem_euclid(1.0) * fill_w);
            let sweep_r = Rect::from_min_max(
                Pos2::new(sweep_x, draw_rect.max.y - prog_h),
                Pos2::new((sweep_x + 18.0).min(draw_rect.max.x), draw_rect.max.y),
            );
            paint::rect_filled(painter, sweep_r, th.accent2.gamma_multiply(row_alpha));
            ui.ctx().request_repaint();
        }

        let el_dl_tween = state
            .motion
            .val(
                ("art_el_dl", band_id),
                stats.eligible_downloaded as f32,
                SpringCfg::SMOOTH,
            )
            .round() as u32;
        let non_el_dl_tween = state
            .motion
            .val(
                ("art_non_el_dl", band_id),
                stats.non_eligible_downloaded as f32,
                SpringCfg::SMOOTH,
            )
            .round() as u32;
        let el_tween = state
            .motion
            .val(
                ("art_el", band_id),
                stats.eligible as f32,
                SpringCfg::SMOOTH,
            )
            .round() as u32;
        let tot_tween = state
            .motion
            .val(("art_tot", band_id), stats.total as f32, SpringCfg::SMOOTH)
            .round() as u32;

        let badge_str = if non_el_dl_tween > 0 {
            format!("{el_dl_tween} (+{non_el_dl_tween})/{el_tween}/{tot_tween}")
        } else {
            format!("{el_dl_tween}/{el_tween}/{tot_tween}")
        };

        let inner = Rect::from_min_max(
            Pos2::new(mos_rect.max.x + 6.0, draw_rect.min.y + 2.0),
            Pos2::new(draw_rect.max.x - 6.0, draw_rect.max.y - 2.0),
        );

        let badge_color = th.text_dim.gamma_multiply(row_alpha);
        let badge_galley =
            ui.painter()
                .layout_no_wrap(badge_str, FontId::monospace(11.0), badge_color);
        let badge_w = badge_galley.size().x;
        let badge_pos = Pos2::new(
            inner.max.x - badge_w,
            inner.center().y - badge_galley.size().y * 0.5,
        );
        ui.painter().galley(badge_pos, badge_galley, badge_color);

        let name_rect = Rect::from_min_max(
            inner.min,
            Pos2::new((inner.max.x - badge_w - 6.0).max(inner.min.x), inner.max.y),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(name_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        if let Some(worst) = crate::state::worst_folder_state(artist_folders) {
            let dot_color = match worst {
                FolderState::Ok => th.accent,
                FolderState::Missing => th.err,
                FolderState::Unreadable => th.warn,
            }
            .gamma_multiply(row_alpha);
            let (glyph_rect, glyph_resp) =
                child.allocate_exact_size(Vec2::new(14.0, 12.0), Sense::hover());
            let tip = if artist_folders.len() == 1 {
                let folder = &artist_folders[0];
                match folder.state {
                    FolderState::Ok => {
                        if stats.archived > 0 && stats.downloaded == stats.archived {
                            format!(
                                "Linked (all archived): {}\nStatus: Reachable\n{} release(s) archived",
                                folder.path.display(),
                                stats.archived
                            )
                        } else {
                            format!("Linked: {}\nStatus: Reachable", folder.path.display())
                        }
                    }
                    FolderState::Missing => {
                        format!(
                            "Linked: {}\nStatus: Missing\nRight-click to resolve",
                            folder.path.display()
                        )
                    }
                    FolderState::Unreadable => {
                        format!("Linked: {}\nStatus: Unreadable", folder.path.display())
                    }
                }
            } else {
                let mut lines = vec![format!("Linked folders ({}):", artist_folders.len())];
                for f in artist_folders {
                    let st_str = match f.state {
                        FolderState::Ok => "Reachable",
                        FolderState::Missing => "Missing",
                        FolderState::Unreadable => "Unreadable",
                    };
                    lines.push(format!("- {}: {}", f.path.display(), st_str));
                }
                if worst != FolderState::Ok {
                    lines.push("Right-click to resolve".to_string());
                }
                lines.join("\n")
            };
            let icon_kind = if worst == FolderState::Ok
                && stats.archived > 0
                && stats.downloaded == stats.archived
            {
                paint::icons::IconKind::Archive
            } else {
                paint::icons::IconKind::Folder
            };
            glyph_resp.on_hover_text(tip);
            let p = child.painter();
            paint::icons::paint_icon(
                p,
                glyph_rect,
                icon_kind,
                th.text_dim.gamma_multiply(row_alpha),
            );
            let dot = Rect::from_min_max(
                Pos2::new(glyph_rect.max.x - 4.0, glyph_rect.max.y - 4.0),
                glyph_rect.max,
            );
            paint::rect_filled(p, dot, dot_color);
            child.add_space(3.0);
        } else if stats.archived > 0 {
            let (glyph_rect, glyph_resp) =
                child.allocate_exact_size(Vec2::new(14.0, 12.0), Sense::hover());
            let tip = format!(
                "Archived: {} release(s) archived (files removed, kept as done)",
                stats.archived
            );
            glyph_resp.on_hover_text(tip);
            let p = child.painter();
            paint::icons::paint_icon(
                p,
                glyph_rect,
                paint::icons::IconKind::Archive,
                th.text_dim.gamma_multiply(row_alpha),
            );
            child.add_space(3.0);
        }
        child.add(
            egui::Label::new(RichText::new(name).color(th.text.gamma_multiply(row_alpha)))
                .truncate(),
        );
    }

    let folder_tooltip = if artist_folders.is_empty() {
        String::new()
    } else if artist_folders.len() == 1 {
        format!(
            "\nLinked folder: {}\nFolder status: {:?}",
            artist_folders[0].path.display(),
            artist_folders[0].state
        )
    } else {
        let mut s = format!("\nLinked folders ({}):", artist_folders.len());
        for f in artist_folders {
            s.push_str(&format!("\n- {}: {:?}", f.path.display(), f.state));
        }
        s
    };

    let dl_desc = if stats.non_eligible_downloaded > 0 {
        format!(
            "{} ({} eligible + {} paid/extra)",
            stats.downloaded, stats.eligible_downloaded, stats.non_eligible_downloaded
        )
    } else {
        stats.downloaded.to_string()
    };

    resp.on_hover_text(format!(
        "Downloaded: {}\nEligible (Free + NYP + Email): {}\nTotal releases on page: {}\nEmail-gated: {}\nPending metadata: {}\nYour downloads (30d): {}/{}{}",
        dl_desc,
        stats.eligible,
        stats.total,
        stats.email_required,
        stats.unknown,
        stats.quota_used_30d,
        state.settings.quota_hard,
        folder_tooltip,
    ))
}

fn sum_fx_member_stats(state: &FxState, member_ids: &[i64]) -> (u32, u32) {
    let mut dl = 0u32;
    let mut elig = 0u32;
    for &bid in member_ids {
        if let Some(a) = state.artists.iter().find(|a| a.band_id == bid) {
            dl += a.stats.eligible_downloaded;
            elig += a.stats.eligible;
        }
    }
    (dl, elig)
}

#[allow(clippy::too_many_arguments)]
fn draw_fx_group_header_row(
    ui: &mut egui::Ui,
    state: &mut FxState,
    key: crate::state::SidebarRowKey,
    name: &str,
    count: usize,
    collapsed: bool,
    dl: u32,
    elig: u32,
    draw_offset: Vec2,
    row_alpha: f32,
) -> egui::Response {
    let th = state.active_theme;
    let desired = Vec2::new(ui.available_width(), 26.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let draw_rect = rect.translate(draw_offset);
    if ui.is_rect_visible(rect) || ui.is_rect_visible(draw_rect) {
        let target_bg = if resp.hovered() { th.bg2 } else { th.bg0 };
        let bg = state
            .motion
            .color(("grp_bg", key), target_bg, SpringCfg::SNAP)
            .gamma_multiply(row_alpha);
        let open_t = state.motion.val(
            ("grp_open", key),
            if collapsed { 0.0 } else { 1.0 },
            SpringCfg::SNAP,
        );
        let cnt_tween = state
            .motion
            .val(("grp_cnt", key), count as f32, SpringCfg::SMOOTH)
            .round() as usize;
        let dl_tween = state
            .motion
            .val(("grp_dl", key), dl as f32, SpringCfg::SMOOTH)
            .round() as u32;
        let elig_tween = state
            .motion
            .val(("grp_elig", key), elig as f32, SpringCfg::SMOOTH)
            .round() as u32;

        paint::rect(
            ui.painter(),
            draw_rect,
            bg,
            egui::Stroke::new(1.0_f32, th.border.gamma_multiply(row_alpha)),
        );
        let inner = draw_rect.shrink2(Vec2::new(8.0, 2.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let (chev_rect, _) = child.allocate_exact_size(Vec2::new(10.0, 12.0), Sense::hover());
        let angle = open_t * std::f32::consts::FRAC_PI_2;
        let (sin_a, cos_a) = angle.sin_cos();
        let center = chev_rect.center();
        let rot = |p: Vec2| {
            Pos2::new(
                center.x + p.x * cos_a - p.y * sin_a,
                center.y + p.x * sin_a + p.y * cos_a,
            )
        };
        let p0 = rot(Vec2::new(-2.0, -4.0));
        let p1 = rot(Vec2::new(2.5, 0.0));
        let p2 = rot(Vec2::new(-2.0, 4.0));
        let chev_stroke = Stroke::new(1.5_f32, th.text_dim.gamma_multiply(row_alpha));
        child.painter().line_segment([p0, p1], chev_stroke);
        child.painter().line_segment([p1, p2], chev_stroke);

        child.add_space(4.0);
        child.add(
            egui::Label::new(
                RichText::new(name)
                    .strong()
                    .color(th.text.gamma_multiply(row_alpha)),
            )
            .truncate(),
        );
        child.add_space(4.0);
        child.label(
            RichText::new(format!("({cnt_tween})"))
                .size(11.0)
                .color(th.text_dim.gamma_multiply(row_alpha)),
        );
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{dl_tween}/{elig_tween}"))
                    .monospace()
                    .size(11.0)
                    .color(th.text_dim.gamma_multiply(row_alpha)),
            );
        });
    }
    resp
}

fn show_fx_grouping_bar(ui: &mut egui::Ui, state: &FxState, pending_actions: &mut Vec<FxAction>) {
    let th = &state.active_theme;
    let current_label = match state.active_grouping {
        bcvault_core::groups::ActiveGrouping::None => "No Grouping".to_string(),
        bcvault_core::groups::ActiveGrouping::Builtin(kind) => format!("Auto: {}", kind.label()),
        bcvault_core::groups::ActiveGrouping::CustomSet(set_id) => state
            .group_sets
            .iter()
            .find(|s| s.id == set_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Custom Set".to_string()),
    };

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("fx_grouping_switcher")
            .selected_text(current_label)
            .width((ui.available_width() - 96.0).max(80.0))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        state.active_grouping == bcvault_core::groups::ActiveGrouping::None,
                        "No Grouping",
                    )
                    .clicked()
                {
                    pending_actions.push(FxAction::SetActiveGrouping(
                        bcvault_core::groups::ActiveGrouping::None,
                    ));
                }
                ui.separator();
                ui.label(
                    RichText::new("Built-in (Automatic)")
                        .size(11.0)
                        .color(th.text_dim),
                );
                for kind in bcvault_core::groups::BuiltinGrouping::ALL {
                    let target = bcvault_core::groups::ActiveGrouping::Builtin(kind);
                    if ui
                        .selectable_label(state.active_grouping == target, kind.label())
                        .clicked()
                    {
                        pending_actions.push(FxAction::SetActiveGrouping(target));
                    }
                }
                ui.separator();
                ui.label(
                    RichText::new("Custom Group Sets")
                        .size(11.0)
                        .color(th.text_dim),
                );
                for set in &state.group_sets {
                    let target = bcvault_core::groups::ActiveGrouping::CustomSet(set.id);
                    if ui
                        .selectable_label(state.active_grouping == target, &set.name)
                        .clicked()
                    {
                        pending_actions.push(FxAction::SetActiveGrouping(target));
                    }
                }
                if paint::sharp_button(ui, "+ New Grouping Set...").clicked() {
                    pending_actions.push(FxAction::NewGroupSet);
                    ui.close();
                }
            });

        if paint::sharp_button(ui, "+Grp")
            .on_hover_text("Create a new artist group")
            .clicked()
        {
            if state.selected_artists.is_empty() {
                pending_actions.push(FxAction::NewGroup);
            } else {
                pending_actions.push(FxAction::NewGroupFromSelection(
                    state.selected_artists.iter().copied().collect(),
                ));
            }
        }

        if let bcvault_core::groups::ActiveGrouping::CustomSet(set_id) = state.active_grouping
            && let Some(set) = state.group_sets.iter().find(|s| s.id == set_id)
        {
            ui.menu_button("Set", |ui| {
                if paint::sharp_button(ui, "Rename Set...").clicked() {
                    pending_actions.push(FxAction::RenameGroupSet(set.id));
                    ui.close();
                }
                if paint::sharp_button(ui, "New Set...").clicked() {
                    pending_actions.push(FxAction::NewGroupSet);
                    ui.close();
                }
                ui.separator();
                if paint::sharp_button(ui, "Delete Set...").clicked() {
                    pending_actions.push(FxAction::DeleteGroupSet(set.id));
                    ui.close();
                }
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn show_fx_group_context_menu(
    ui: &mut egui::Ui,
    state: &FxState,
    core_tx: &UnboundedSender<UiToWorkerCommand>,
    pending_actions: &mut Vec<FxAction>,
    group_id: i64,
    _group_name: &str,
    member_ids: &[i64],
) {
    let is_custom = matches!(
        state.active_grouping,
        bcvault_core::groups::ActiveGrouping::CustomSet(_)
    ) && group_id > 0;
    if is_custom {
        if paint::sharp_button(ui, "Edit group & members...").clicked() {
            pending_actions.push(FxAction::EditGroup(group_id));
            ui.close();
        }
        ui.separator();
    }
    if paint::sharp_button(ui, "Sync group").clicked() {
        pending_actions.push(FxAction::SyncGroup(group_id));
        ui.close();
    }
    if paint::sharp_button(ui, "Download remaining for group").clicked() {
        pending_actions.push(FxAction::DownloadRemainingGroup(group_id));
        ui.close();
    }
    if paint::sharp_button(ui, "Select artists in group").clicked() {
        pending_actions.push(FxAction::SelectArtists(member_ids.to_vec()));
        ui.close();
    }
    if paint::sharp_button(ui, "View releases of group").clicked() {
        pending_actions.push(FxAction::ViewGroup(group_id));
        ui.close();
    }
    if let bcvault_core::groups::ActiveGrouping::CustomSet(set_id) = state.active_grouping
        && group_id > 0
    {
        ui.separator();
        if let Some(pos) = state.groups.iter().position(|g| g.id == group_id) {
            if pos > 0 && paint::sharp_button(ui, "Move up").clicked() {
                let mut ids: Vec<i64> = state.groups.iter().map(|g| g.id).collect();
                ids.swap(pos, pos - 1);
                let _ = core_tx.send(UiToWorkerCommand::ReorderArtistGroups { set_id, ids });
                ui.close();
            }
            if pos + 1 < state.groups.len() && paint::sharp_button(ui, "Move down").clicked() {
                let mut ids: Vec<i64> = state.groups.iter().map(|g| g.id).collect();
                ids.swap(pos, pos + 1);
                let _ = core_tx.send(UiToWorkerCommand::ReorderArtistGroups { set_id, ids });
                ui.close();
            }
        }
        if paint::sharp_button(ui, "Delete group...").clicked() {
            pending_actions.push(FxAction::DeleteGroup(group_id));
            ui.close();
        }
    }
}

fn show_fx_move_to_group_submenu(
    ui: &mut egui::Ui,
    state: &FxState,
    pending_actions: &mut Vec<FxAction>,
    band_id: i64,
) {
    let target_ids: Vec<i64> =
        if state.selected_artists.contains(&band_id) && state.selected_artists.len() > 1 {
            state.selected_artists.iter().copied().collect()
        } else {
            vec![band_id]
        };
    let label = if target_ids.len() > 1 {
        format!("Move {} artists to group", target_ids.len())
    } else {
        "Move to group".to_string()
    };
    ui.menu_button(label, |ui| {
        if matches!(
            state.active_grouping,
            bcvault_core::groups::ActiveGrouping::CustomSet(_)
        ) {
            for g in &state.groups {
                if paint::sharp_button(ui, &g.name).clicked() {
                    pending_actions.push(FxAction::MoveArtistsToGroup(
                        target_ids.clone(),
                        bcvault_core::groups::PinTarget::Group(g.id),
                    ));
                    ui.close();
                }
            }
            let any_in_group = target_ids.iter().any(|id| state.pins.contains_key(id));
            if any_in_group {
                if !state.groups.is_empty() {
                    ui.separator();
                }
                if paint::sharp_button(ui, "Ungrouped").clicked() {
                    pending_actions.push(FxAction::MoveArtistsToGroup(
                        target_ids.clone(),
                        bcvault_core::groups::PinTarget::Ungrouped,
                    ));
                    ui.close();
                }
            }
            if !state.groups.is_empty() || any_in_group {
                ui.separator();
            }
        }
        if paint::sharp_button(
            ui,
            format!("New group from selection ({})...", target_ids.len()),
        )
        .clicked()
        {
            pending_actions.push(FxAction::NewGroupFromSelection(target_ids.clone()));
            ui.close();
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::theme::{FxTheme, apply_theme};
    use bcvault_core::types::ArtistView;

    #[test]
    fn sidebar_does_not_overflow_or_leave_gap_before_central_panel() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, &FxTheme::paper_for_tests());
        let mut state = FxState::new();
        state.artists = vec![
            ArtistView {
                band_id: 1,
                slug: "areyoudoneyet".to_string(),
                name: "areyoudoneyet".to_string(),
                custom_name: None,
                url: "https://areyoudoneyet.bandcamp.com".to_string(),
                last_synced_at: None,
                stats: ArtistStats {
                    downloaded: 17,
                    eligible_downloaded: 17,
                    non_eligible_downloaded: 0,
                    eligible: 17,
                    total: 17,
                    email_required: 1,
                    unknown: 0,
                    quota_used_30d: 0,
                    archived: 0,
                    missing_files: 0,
                },
            },
            ArtistView {
                band_id: 2,
                slug: "deafballerina".to_string(),
                name: "Deaf Ballerina".to_string(),
                custom_name: None,
                url: "https://deafballerina.bandcamp.com".to_string(),
                last_synced_at: None,
                stats: ArtistStats {
                    downloaded: 11,
                    eligible_downloaded: 11,
                    non_eligible_downloaded: 0,
                    eligible: 11,
                    total: 11,
                    email_required: 0,
                    unknown: 0,
                    quota_used_30d: 0,
                    archived: 0,
                    missing_files: 0,
                },
            },
        ];

        let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        for frame_idx in 0..5 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 860.0))),
                ..Default::default()
            };
            let mut sb_rect = Rect::NOTHING;
            let mut sb_clip = Rect::NOTHING;
            let mut sb_inner_min = Rect::NOTHING;
            let mut central_rect = Rect::NOTHING;
            let _ = ctx.run(raw, |ctx| {
                state.motion.begin_frame_dt(1.0 / 60.0);
                let sb = egui::SidePanel::left("fx_sidebar_expanded")
                    .default_width(280.0)
                    .min_width(240.0)
                    .max_width(500.0)
                    .resizable(true)
                    .show(ctx, |ui| {
                        sb_clip = ui.clip_rect();
                        ui.set_max_width(ui.available_width());
                        show(ui, &mut state, &core_tx, &fx_tx, frame_idx as f64 * 0.016);
                        sb_inner_min = ui.min_rect();
                    });
                sb_rect = sb.response.rect;
                let cp = egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("central");
                });
                central_rect = cp.response.rect;
                state.motion.end_frame(ctx);
            });

            assert!(
                sb_inner_min.max.x <= sb_clip.max.x,
                "sidebar inner content max.x ({}) exceeded clip_rect max.x ({}) on frame {frame_idx}",
                sb_inner_min.max.x,
                sb_clip.max.x
            );
            assert!(
                (sb_rect.max.x - sb_clip.max.x).abs() <= 1.0,
                "sidebar response rect max.x ({}) != clip_rect max.x ({}) on frame {frame_idx}",
                sb_rect.max.x,
                sb_clip.max.x
            );
            assert!(
                (sb_rect.width() - 280.0).abs() <= 1.0,
                "sidebar width drifted to {} on frame {frame_idx}",
                sb_rect.width()
            );
            assert!(
                (central_rect.min.x - sb_rect.max.x).abs() <= 1.0,
                "gap between sidebar ({}) and central panel ({}) on frame {frame_idx}",
                sb_rect.max.x,
                central_rect.min.x
            );
        }
    }

    #[test]
    fn sidebar_grouping_switch_and_move_triggers_flip_and_settles() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, &FxTheme::paper_for_tests());
        let mut state = FxState::new();
        state.artists = vec![
            ArtistView {
                band_id: 1,
                slug: "alpha".to_string(),
                name: "Alpha".to_string(),
                custom_name: None,
                url: "https://alpha.bandcamp.com".to_string(),
                last_synced_at: None,
                stats: ArtistStats::default(),
            },
            ArtistView {
                band_id: 2,
                slug: "beta".to_string(),
                name: "Beta".to_string(),
                custom_name: None,
                url: "https://beta.bandcamp.com".to_string(),
                last_synced_at: None,
                stats: ArtistStats::default(),
            },
        ];

        let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        let run_frame = |state: &mut FxState, now_s: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 860.0))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                state.motion.begin_frame_dt(1.0 / 60.0);
                egui::SidePanel::left("fx_sidebar_anim_test")
                    .default_width(280.0)
                    .show(ctx, |ui| {
                        show(ui, state, &core_tx, &fx_tx, now_s);
                    });
                state.motion.end_frame(ctx);
            });
        };

        // Frame 0: initial population (no grouping)
        run_frame(&mut state, 0.0);
        assert!(state.sidebar_anim_initialized);

        // Switch to FirstLetter built-in grouping -> seeds FLIP & entrance springs
        state.active_grouping = bcvault_core::groups::ActiveGrouping::Builtin(
            bcvault_core::groups::BuiltinGrouping::FirstLetter,
        );
        state.sidebar_dirty = true;
        run_frame(&mut state, 0.016);
        assert!(
            !state.sidebar_flip_active.is_empty(),
            "expected FLIP springs after switching grouping"
        );
        assert!(
            !state.sidebar_enter_active.is_empty(),
            "expected entrance springs for new group headers"
        );
        assert!(
            state.motion.active_springs() > 0,
            "expected active springs during sidebar transition"
        );

        // Advance enough frames for all springs and ghosts to settle cleanly
        for i in 2..120 {
            run_frame(&mut state, i as f64 * 0.016);
        }
        assert!(
            state.sidebar_flip_active.is_empty(),
            "FLIP active set should clear once settled"
        );
        assert!(
            state.sidebar_enter_active.is_empty(),
            "entrance active set should clear once settled"
        );
        assert!(
            state.sidebar_ghosts.is_empty(),
            "ghosts should expire after 0.16s"
        );
        assert_eq!(
            state.motion.active_springs(),
            0,
            "motion should have 0 active springs at rest"
        );
    }
}
