use crate::{
    types::{ArtistFolder, ArtistStats, FolderState, SyncMode, SyncScope},
    ui::{
        state::{LinkFolderModalState, RenameArtistModalState, UiState},
        theme::pal,
        widgets::{paint_archive_glyph, paint_folder_glyph, sharp_button, sharp_rect},
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(ui: &mut Ui, state: &mut UiState, cmd_tx: &UnboundedSender<UiToWorkerCommand>) {
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);

    // 1. Add Artist bar at top of sidebar
    ui.horizontal(|ui| {
        let te = egui::TextEdit::singleline(&mut state.add_artist_input)
            .hint_text("Add Artist (URL or Slug)")
            .desired_width(ui.available_width() - 52.0);
        let resp = ui.add_enabled(!state.add_artist_pending, te);
        let enter_pressed = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let add_clicked = ui
            .add_enabled(
                !state.add_artist_pending && !state.add_artist_input.trim().is_empty(),
                egui::Button::new("Add").corner_radius(CornerRadius::ZERO),
            )
            .clicked();

        if (enter_pressed || add_clicked) && !state.add_artist_input.trim().is_empty() {
            let input = state.add_artist_input.trim().to_string();
            state.add_artist_input.clear();
            state.add_artist_pending = true;
            let _ = cmd_tx.send(UiToWorkerCommand::AddArtist { input });
        }
    });

    // 2. Artist filter TextEdit
    ui.add(
        egui::TextEdit::singleline(&mut state.artist_filter)
            .hint_text("Filter artists...")
            .desired_width(f32::INFINITY),
    );

    ui.separator();

    // Reserve footer height (~72px) for Sync controls
    let list_height = (ui.available_height() - 78.0).max(80.0);

    let filter_lower = state.artist_filter.trim().to_lowercase();
    let filtered_indices: Vec<usize> = state
        .artists
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            filter_lower.is_empty()
                || a.display_name().to_lowercase().contains(&filter_lower)
                || a.name.to_lowercase().contains(&filter_lower)
                || a.slug.to_lowercase().contains(&filter_lower)
        })
        .map(|(i, _)| i)
        .collect();

    let total_stats = aggregate_all_stats(state);
    let total_rows = 1 + filtered_indices.len(); // row 0 is "All artists"

    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show_rows(ui, 26.0, total_rows, |ui, row_range| {
            for row_idx in row_range {
                if row_idx == 0 {
                    let is_selected = state.selected_artists.is_empty();
                    let resp = draw_artist_row(
                        ui,
                        "All artists",
                        total_stats,
                        is_selected,
                        None,
                        state.settings.quota_hard,
                        &[],
                    );
                    if resp.clicked() {
                        state.selected_artists.clear();
                        state.last_clicked_artist = None;
                        state.mark_query_immediate();
                    }
                } else {
                    let artist_idx = filtered_indices[row_idx - 1];
                    let artist = state.artists[artist_idx].clone();
                    let is_selected = state.selected_artists.contains(&artist.band_id);
                    let sync_prog = state.active_syncs.get(&artist.band_id).cloned();
                    let artist_folders = state
                        .folders
                        .get(&artist.band_id)
                        .cloned()
                        .unwrap_or_default();

                    let name_disp = bcvault_core::text::format_display_text(
                        artist.display_name(),
                        state.settings.mark_invisible_chars,
                    );
                    let resp = draw_artist_row(
                        ui,
                        name_disp.as_ref(),
                        artist.stats,
                        is_selected,
                        sync_prog.as_ref().map(|p| {
                            if p.total == 0 {
                                0.0_f32
                            } else {
                                (p.done as f32 / p.total as f32).clamp(0.0, 1.0)
                            }
                        }),
                        state.settings.quota_hard,
                        &artist_folders,
                    );

                    let modifiers = ui.input(|i| i.modifiers);
                    if resp.clicked() {
                        if modifiers.shift
                            && let Some(anchor_id) = state.last_clicked_artist
                        {
                            let pos_a = filtered_indices
                                .iter()
                                .position(|&idx| state.artists[idx].band_id == anchor_id);
                            let pos_b = Some(row_idx - 1);
                            if let (Some(a), Some(b)) = (pos_a, pos_b) {
                                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                                if !modifiers.ctrl && !modifiers.command {
                                    state.selected_artists.clear();
                                }
                                for &f_idx in &filtered_indices[lo..=hi] {
                                    state.selected_artists.insert(state.artists[f_idx].band_id);
                                }
                            }
                        } else if modifiers.ctrl || modifiers.command {
                            if !state.selected_artists.remove(&artist.band_id) {
                                state.selected_artists.insert(artist.band_id);
                            }
                            state.last_clicked_artist = Some(artist.band_id);
                        } else {
                            state.selected_artists.clear();
                            state.selected_artists.insert(artist.band_id);
                            state.last_clicked_artist = Some(artist.band_id);
                        }
                        state.mark_query_immediate();
                    }

                    resp.context_menu(|ui| {
                        if sharp_button(ui, "Sync Artist").clicked() {
                            let _ = cmd_tx.send(UiToWorkerCommand::Sync {
                                scope: SyncScope::Selected(vec![artist.band_id]),
                                mode: state.sync_mode,
                            });
                            ui.close();
                        }
                        if artist_folders.is_empty() {
                            if sharp_button(ui, "Link Folder...").clicked() {
                                state.link_folder_modal = Some(LinkFolderModalState::for_artist(
                                    artist.band_id,
                                    artist.display_name(),
                                    None,
                                ));
                                ui.close();
                            }
                        } else if artist_folders.len() == 1 {
                            let folder = &artist_folders[0];
                            if folder.state != FolderState::Ok
                                && sharp_button(ui, "Resolve unavailable folder...").clicked()
                            {
                                let _ = cmd_tx.send(UiToWorkerCommand::VerifyFiles {
                                    scope: SyncScope::Selected(vec![artist.band_id]),
                                });
                                ui.close();
                            }
                            if sharp_button(ui, "Add Folder...").clicked() {
                                state.link_folder_modal = Some(LinkFolderModalState::for_artist(
                                    artist.band_id,
                                    artist.display_name(),
                                    None,
                                ));
                                ui.close();
                            }
                            if sharp_button(ui, "Change Folder...").clicked() {
                                state.link_folder_modal = Some(LinkFolderModalState::for_repoint(
                                    artist.band_id,
                                    artist.display_name(),
                                    folder.id,
                                    &folder.path,
                                ));
                                ui.close();
                            }
                            if sharp_button(ui, "Match Folder").clicked() {
                                let _ = cmd_tx.send(UiToWorkerCommand::MatchArtistFolder {
                                    band_id: artist.band_id,
                                    auto_apply_confident: true,
                                });
                                ui.close();
                            }
                            if sharp_button(ui, "Unlink Folder").clicked() {
                                let _ = cmd_tx.send(UiToWorkerCommand::UnlinkArtistFolder {
                                    folder_id: folder.id,
                                });
                                ui.close();
                            }
                        } else {
                            if artist_folders.iter().any(|f| f.state != FolderState::Ok)
                                && sharp_button(ui, "Resolve unavailable folder...").clicked()
                            {
                                let _ = cmd_tx.send(UiToWorkerCommand::VerifyFiles {
                                    scope: SyncScope::Selected(vec![artist.band_id]),
                                });
                                ui.close();
                            }
                            if sharp_button(ui, "Add Folder...").clicked() {
                                state.link_folder_modal = Some(LinkFolderModalState::for_artist(
                                    artist.band_id,
                                    artist.display_name(),
                                    None,
                                ));
                                ui.close();
                            }
                            if sharp_button(ui, "Match Folder").clicked() {
                                let _ = cmd_tx.send(UiToWorkerCommand::MatchArtistFolder {
                                    band_id: artist.band_id,
                                    auto_apply_confident: true,
                                });
                                ui.close();
                            }
                            ui.menu_button("Change folder...", |ui| {
                                for folder in &artist_folders {
                                    let label = crate::ui::state::folder_display_label(
                                        folder,
                                        &artist_folders,
                                    );
                                    if sharp_button(ui, &format!("Folder moved: change {label}..."))
                                        .clicked()
                                    {
                                        state.link_folder_modal =
                                            Some(LinkFolderModalState::for_repoint(
                                                artist.band_id,
                                                artist.display_name(),
                                                folder.id,
                                                &folder.path,
                                            ));
                                        ui.close();
                                    }
                                }
                            });
                            ui.menu_button("Unlink folder...", |ui| {
                                for folder in &artist_folders {
                                    let label = crate::ui::state::folder_display_label(
                                        folder,
                                        &artist_folders,
                                    );
                                    if sharp_button(ui, &format!("Unlink {label}...")).clicked() {
                                        let _ =
                                            cmd_tx.send(UiToWorkerCommand::UnlinkArtistFolder {
                                                folder_id: folder.id,
                                            });
                                        ui.close();
                                    }
                                }
                            });
                        }
                        if sharp_button(ui, "Verify Files").clicked() {
                            let _ = cmd_tx.send(UiToWorkerCommand::VerifyFiles {
                                scope: SyncScope::Selected(vec![artist.band_id]),
                            });
                            ui.close();
                        }
                        if sharp_button(ui, "Rename Artist...").clicked() {
                            state.rename_artist_modal = Some(RenameArtistModalState {
                                band_id: artist.band_id,
                                original_name: artist.name.clone(),
                                name_input: artist.display_name().to_string(),
                            });
                            ui.close();
                        }
                        if sharp_button(ui, "Open in Browser").clicked() {
                            let _ = open::that_detached(&artist.url);
                            ui.close();
                        }
                        if sharp_button(ui, "Remove Artist...").clicked() {
                            state.confirm_remove_artist =
                                Some((artist.band_id, artist.display_name().to_string()));
                            ui.close();
                        }
                    });
                }
            }
        });

    ui.separator();

    // 3. Footer: Sync Mode + Sync Selected / Sync All / Cancel
    ui.horizontal(|ui| {
        ui.label(RichText::new("Mode:").color(pal::TEXT_DIM));
        egui::ComboBox::from_id_salt("sync_mode_combo")
            .selected_text(state.sync_mode.label())
            .width(150.0)
            .show_ui(ui, |ui| {
                for mode in SyncMode::ALL {
                    ui.selectable_value(&mut state.sync_mode, mode, mode.label());
                }
            });
    });

    ui.horizontal(|ui| {
        let has_selected = !state.selected_artists.is_empty();
        if ui
            .add_enabled(
                has_selected,
                egui::Button::new("Sync Selected").corner_radius(CornerRadius::ZERO),
            )
            .clicked()
        {
            let ids: Vec<i64> = state.selected_artists.iter().copied().collect();
            let _ = cmd_tx.send(UiToWorkerCommand::Sync {
                scope: SyncScope::Selected(ids),
                mode: state.sync_mode,
            });
        }

        if sharp_button(ui, "Sync All").clicked() {
            let _ = cmd_tx.send(UiToWorkerCommand::Sync {
                scope: SyncScope::All,
                mode: state.sync_mode,
            });
        }

        if sharp_button(ui, "Verify Files").clicked() {
            let scope = if state.selected_artists.is_empty() {
                SyncScope::All
            } else {
                SyncScope::Selected(state.selected_artists.iter().copied().collect())
            };
            let _ = cmd_tx.send(UiToWorkerCommand::VerifyFiles { scope });
        }

        if !state.active_syncs.is_empty() && sharp_button(ui, "Stop").clicked() {
            let _ = cmd_tx.send(UiToWorkerCommand::CancelSync);
        }
    });
}

fn aggregate_all_stats(state: &UiState) -> ArtistStats {
    let mut out = ArtistStats::default();
    for a in &state.artists {
        out.downloaded += a.stats.downloaded;
        out.eligible_downloaded += a.stats.eligible_downloaded;
        out.non_eligible_downloaded += a.stats.non_eligible_downloaded;
        out.eligible += a.stats.eligible;
        out.total += a.stats.total;
        out.email_required += a.stats.email_required;
        out.unknown += a.stats.unknown;
        out.quota_used_30d += a.stats.quota_used_30d;
    }
    out
}

fn draw_artist_row(
    ui: &mut Ui,
    name: &str,
    stats: ArtistStats,
    selected: bool,
    sync_frac: Option<f32>,
    quota_hard: u32,
    folders: &[ArtistFolder],
) -> egui::Response {
    let desired = Vec2::new(ui.available_width(), 24.0);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bg = if selected || resp.hovered() {
            pal::BG2
        } else {
            pal::BG1
        };
        sharp_rect(painter, rect, bg, Stroke::new(1.0_f32, pal::BORDER));

        if selected {
            let bar = Rect::from_min_max(rect.min, Pos2::new(rect.min.x + 2.0, rect.max.y));
            sharp_rect(painter, bar, pal::ACCENT, Stroke::NONE);
        }

        if let Some(frac) = sync_frac {
            let prog_h = 2.0;
            let track = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - prog_h), rect.max);
            sharp_rect(painter, track, pal::BG0, Stroke::NONE);
            let fill_w = (rect.width() * frac.max(0.08)).min(rect.width());
            let fill_rect = Rect::from_min_max(
                Pos2::new(rect.min.x, rect.max.y - prog_h),
                Pos2::new(rect.min.x + fill_w, rect.max.y),
            );
            sharp_rect(painter, fill_rect, pal::ACCENT, Stroke::NONE);
        }

        let badge_str = stats.format_sidebar_badge();
        let badge_tip = if stats.non_eligible_downloaded > 0 {
            format!(
                "Downloaded: {} eligible + {} paid/extra\nEligible (Free / NYP): {}\nTotal on page: {}",
                stats.eligible_downloaded,
                stats.non_eligible_downloaded,
                stats.eligible,
                stats.total
            )
        } else {
            format!(
                "Downloaded: {}\nEligible (Free / NYP): {}\nTotal on page: {}",
                stats.eligible_downloaded, stats.eligible, stats.total
            )
        };
        let inner = rect.shrink2(Vec2::new(8.0, 2.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
        );
        if let Some(worst) = crate::ui::state::worst_folder_state(folders) {
            let tip = if folders.len() == 1 {
                let folder = &folders[0];
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
                let mut lines = vec![format!("Linked folders ({}):", folders.len())];
                for f in folders {
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
            paint_folder_glyph(&mut child, worst).on_hover_text(tip);
            child.add_space(2.0);
        } else if stats.archived > 0 {
            let tip = format!(
                "Archived: {} release(s) archived (files removed, kept as done)",
                stats.archived
            );
            paint_archive_glyph(&mut child).on_hover_text(tip);
            child.add_space(2.0);
        }
        child.add(egui::Label::new(RichText::new(name).color(pal::TEXT)).truncate());
        child.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(badge_str)
                    .monospace()
                    .size(11.0)
                    .color(pal::TEXT_DIM),
            )
            .on_hover_text(badge_tip);
        });
    }

    let folder_tooltip = if folders.is_empty() {
        String::new()
    } else if folders.len() == 1 {
        format!(
            "\nLinked folder: {}\nFolder status: {:?}",
            folders[0].path.display(),
            folders[0].state
        )
    } else {
        let mut s = format!("\nLinked folders ({}):", folders.len());
        for f in folders {
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
        quota_hard,
        folder_tooltip
    ))
}
