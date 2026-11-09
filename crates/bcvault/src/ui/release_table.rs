use crate::{
    types::{Classification, DownloadOpts, FolderState, LocalStatus, SortDir, SortKey},
    ui::{
        state::{RedeemModalState, UiState},
        theme::pal,
        widgets::{
            badge, class_badge, paint_sort_arrow, paint_status_glyph, sharp_small_button,
            update_badge,
        },
    },
    worker::messages::UiToWorkerCommand,
};
use egui::{Align, CornerRadius, Layout, Pos2, RichText, Sense, Ui, Vec2};
use egui_extras::{Column, TableBuilder};
use tokio::sync::mpsc::UnboundedSender;

pub fn show(
    ui: &mut Ui,
    state: &mut UiState,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
    rt_handle: &tokio::runtime::Handle,
) {
    if let Some((key, path)) = state
        .relink_picked_folder
        .lock()
        .ok()
        .and_then(|mut lock| lock.take())
    {
        let _ = cmd_tx.send(UiToWorkerCommand::RelinkRelease { key, path });
    }

    let eligible_visible_keys: Vec<_> = state
        .rows
        .iter()
        .filter(|cr| !cr.row.is_preorder && cr.row.classification.auto_downloadable())
        .map(|cr| cr.row.key)
        .collect();

    let all_eligible_selected = !eligible_visible_keys.is_empty()
        && eligible_visible_keys
            .iter()
            .all(|k| state.selected.contains(k));

    let mut header_check = all_eligible_selected;
    let mut header_check_clicked = false;
    let mut sort_clicked: Option<SortKey> = None;

    let total_rows = state.rows.len();

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(28.0)) // checkbox
        .column(Column::exact(28.0)) // status glyph
        .column(Column::initial(160.0).at_least(80.0).clip(true)) // artist
        .column(Column::initial(300.0).at_least(200.0).clip(true)) // title
        .column(Column::exact(60.0)) // type
        .column(Column::exact(92.0)) // released
        .column(Column::exact(56.0)) // tracks
        .column(Column::exact(110.0)) // classification
        .column(Column::exact(96.0)) // downloaded format
        .column(Column::remainder().at_least(230.0)) // actions
        .header(24.0, |mut h| {
            h.col(|ui| {
                if ui.checkbox(&mut header_check, "").clicked() {
                    header_check_clicked = true;
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "St", SortKey::Status, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Status);
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "Artist", SortKey::Artist, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Artist);
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "Title", SortKey::Title, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Title);
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "Type", SortKey::Type, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Type);
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "Released", SortKey::Released, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Released);
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "Tracks", SortKey::Tracks, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Tracks);
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "Class", SortKey::Classification, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Classification);
                }
            });
            h.col(|ui| {
                if sortable_header(ui, "Format", SortKey::Format, state.sort, state.dir) {
                    sort_clicked = Some(SortKey::Format);
                }
            });
            h.col(|ui| {
                ui.label(RichText::new("Actions").strong().color(pal::TEXT_DIM));
            });
        })
        .body(|body| {
            body.rows(24.0, total_rows, |mut row| {
                let row_idx = row.index();
                let cached = state.rows[row_idx].clone();
                let r = &cached.row;
                let key = r.key;
                let is_selected = state.selected.contains(&key);
                let in_queue = state.active_item_jobs.contains(&key);

                row.set_selected(is_selected);

                // Column 0: Checkbox
                let mut checked = is_selected;
                let mut check_changed = false;
                row.col(|ui| {
                    let can_select = !r.is_preorder && r.classification.auto_downloadable();
                    if ui
                        .add_enabled(can_select, egui::Checkbox::without_text(&mut checked))
                        .clicked()
                    {
                        check_changed = true;
                    }
                });

                // Column 1: Status glyph
                row.col(|ui| {
                    let glyph_resp = paint_status_glyph(
                        ui,
                        r.downloaded,
                        r.local_status,
                        r.has_remote_update,
                        in_queue,
                    );
                    let tip = if in_queue {
                        "Queued or downloading".to_string()
                    } else {
                        let mut s = match r.local_status {
                            Some(LocalStatus::Present) => "Downloaded (files present)".to_string(),
                            Some(LocalStatus::Archived) => {
                                "Archived (files removed, kept as done)".to_string()
                            }
                            Some(LocalStatus::Missing) => {
                                "Missing (files not found on disk)".to_string()
                            }
                            Some(LocalStatus::Unverified) => "Downloaded (unverified)".to_string(),
                            None if r.downloaded => "Downloaded".to_string(),
                            _ => "Not downloaded".to_string(),
                        };
                        if r.has_remote_update {
                            s.push_str(" · remote update available");
                        }
                        if let Some(path) = &r.local_path {
                            s.push_str(&format!("\nLocal path: {}", path.display()));
                        }
                        s
                    };
                    glyph_resp.on_hover_text(tip);
                });

                // Column 2: Artist
                row.col(|ui| {
                    ui.add(egui::Label::new(cached.artist_arc.as_ref()).truncate());
                });

                // Column 3: Title (+ optional thumbnail + badges)
                row.col(|ui| {
                    if state.settings.show_thumbnails {
                        state.thumbs.draw_thumb(ui, key);
                    }
                    ui.add(egui::Label::new(&r.title).truncate());
                    if r.downloaded {
                        if r.local_status == Some(LocalStatus::Archived) {
                            badge(ui, "ARCHIVED", pal::TEXT_DIM, pal::BORDER_HI);
                        } else if r.local_status == Some(LocalStatus::Missing) {
                            badge(ui, "MISSING", pal::WARN, pal::WARN);
                        } else if let Some(folder) = state.folders.get(&r.band_id)
                            && folder.state != FolderState::Ok
                        {
                            let is_under_folder = match &r.local_path {
                                Some(p) => {
                                    let rel_key =
                                        bcvault_core::library::link::normalize_path_key(p);
                                    let folder_key =
                                        bcvault_core::library::link::normalize_path_key(
                                            &folder.path,
                                        );
                                    rel_key == folder_key
                                        || rel_key.starts_with(&format!("{folder_key}/"))
                                }
                                None => true,
                            };
                            if is_under_folder {
                                badge(ui, "FOLDER MISSING", pal::WARN, pal::WARN);
                            }
                        }
                    }
                    if r.has_remote_update
                        && let Some(reason) = r.remote_update_reason
                    {
                        update_badge(ui, reason);
                    }
                });

                // Column 4: Type
                row.col(|ui| {
                    ui.label(RichText::new(r.key.kind.label()).color(pal::TEXT_DIM));
                });

                // Column 5: Released
                row.col(|ui| {
                    ui.label(
                        RichText::new(cached.date_str.as_ref())
                            .monospace()
                            .size(11.5)
                            .color(pal::TEXT_DIM),
                    );
                });

                // Column 6: Tracks
                row.col(|ui| {
                    ui.label(
                        RichText::new(cached.tracks_str.as_ref())
                            .monospace()
                            .color(pal::TEXT_DIM),
                    );
                });

                // Column 7: Classification
                row.col(|ui| {
                    class_badge(ui, r.classification);
                });

                // Column 8: Downloaded format
                row.col(|ui| {
                    if let Some(fmt) = r.downloaded_format {
                        ui.label(RichText::new(fmt.label()).monospace().color(pal::OK));
                    } else {
                        ui.label(RichText::new("-").color(pal::TEXT_DIM));
                    }
                });

                // Column 9: Actions
                row.col(|ui| {
                    draw_row_actions(ui, state, r, cmd_tx);
                });

                let row_resp = row.response();
                row_resp.context_menu(|ui| {
                    if r.downloaded {
                        if sharp_small_button(ui, "Mark as Archived").clicked() {
                            let _ =
                                cmd_tx.send(UiToWorkerCommand::MarkArchived { keys: vec![key] });
                            ui.close();
                        }
                        if sharp_small_button(ui, "Mark as Not Downloaded").clicked() {
                            let _ = cmd_tx
                                .send(UiToWorkerCommand::UnmarkDownloaded { keys: vec![key] });
                            ui.close();
                        }
                        if sharp_small_button(ui, "Relink to Folder...").clicked() {
                            let slot = state.relink_picked_folder.clone();
                            rt_handle.spawn(async move {
                                if let Some(folder) =
                                    rfd::AsyncFileDialog::new().pick_folder().await
                                    && let Ok(mut lock) = slot.lock()
                                {
                                    *lock = Some((key, folder.path().to_path_buf()));
                                }
                            });
                            ui.close();
                        }
                        if let Some(ref path) = r.local_path {
                            let is_missing = r.local_status == Some(LocalStatus::Missing);
                            let btn = ui.add_enabled(
                                !is_missing,
                                egui::Button::new(RichText::new("Open Folder").size(11.5))
                                    .corner_radius(CornerRadius::ZERO),
                            );
                            if btn.clicked() {
                                let target_dir = if path.is_dir() {
                                    path.as_path()
                                } else {
                                    path.parent().unwrap_or(path.as_path())
                                };
                                let _ = open::that_detached(target_dir);
                                ui.close();
                            }
                        }
                        if r.classification.auto_downloadable()
                            && !r.is_preorder
                            && sharp_small_button(ui, "Re-download").clicked()
                        {
                            let opts = DownloadOpts {
                                format: state.settings.format,
                                out_dir: state.settings.out_dir.clone(),
                                batch_limit: state.settings.batch_limit,
                            };
                            let _ = cmd_tx.send(UiToWorkerCommand::EnqueueDownloads {
                                items: vec![r.key],
                                opts,
                                ack_soft_quota: false,
                            });
                            ui.close();
                        }
                    } else {
                        if sharp_small_button(ui, "Mark Downloaded").clicked() {
                            let _ = cmd_tx.send(UiToWorkerCommand::SetReleaseDownloaded {
                                key,
                                downloaded: true,
                            });
                            ui.close();
                        }
                        if sharp_small_button(ui, "Relink to Folder...").clicked() {
                            let slot = state.relink_picked_folder.clone();
                            rt_handle.spawn(async move {
                                if let Some(folder) =
                                    rfd::AsyncFileDialog::new().pick_folder().await
                                    && let Ok(mut lock) = slot.lock()
                                {
                                    *lock = Some((key, folder.path().to_path_buf()));
                                }
                            });
                            ui.close();
                        }
                    }

                    if state.selected.len() > 1 && state.selected.contains(&key) {
                        ui.separator();
                        let sel_count = state.selected.len();
                        if sharp_small_button(
                            ui,
                            &format!("Mark Selected as Archived ({sel_count})"),
                        )
                        .clicked()
                        {
                            let keys = state.selected.iter().copied().collect();
                            let _ = cmd_tx.send(UiToWorkerCommand::MarkArchived { keys });
                            ui.close();
                        }
                        if sharp_small_button(
                            ui,
                            &format!("Mark Selected as Not Downloaded ({sel_count})"),
                        )
                        .clicked()
                        {
                            let keys = state.selected.iter().copied().collect();
                            let _ = cmd_tx.send(UiToWorkerCommand::UnmarkDownloaded { keys });
                            ui.close();
                        }
                    }

                    ui.separator();
                    if sharp_small_button(ui, "Open in Browser").clicked() {
                        let _ = open::that_detached(&r.url);
                        ui.close();
                    }
                });
                let modifiers = row_resp.ctx.input(|i| i.modifiers);

                if check_changed || row_resp.clicked() {
                    if modifiers.shift
                        && let Some(anchor) = state.last_clicked_row
                    {
                        let (lo, hi) = if anchor <= row_idx {
                            (anchor, row_idx)
                        } else {
                            (row_idx, anchor)
                        };
                        for idx in lo..=hi {
                            if let Some(cr) = state.rows.get(idx)
                                && !cr.row.is_preorder
                                && cr.row.classification.auto_downloadable()
                            {
                                state.selected.insert(cr.row.key);
                            }
                        }
                    } else if !r.is_preorder && r.classification.auto_downloadable() {
                        if is_selected {
                            state.selected.remove(&key);
                        } else {
                            state.selected.insert(key);
                        }
                        state.last_clicked_row = Some(row_idx);
                    }
                }
            });
        });

    if header_check_clicked {
        if all_eligible_selected {
            for k in &eligible_visible_keys {
                state.selected.remove(k);
            }
        } else {
            for &k in &eligible_visible_keys {
                state.selected.insert(k);
            }
        }
    }

    if let Some(clicked_key) = sort_clicked {
        if state.sort == clicked_key {
            state.dir = state.dir.toggle();
        } else {
            state.sort = clicked_key;
            state.dir = match clicked_key {
                SortKey::Released | SortKey::Tracks | SortKey::Status => SortDir::Desc,
                _ => SortDir::Asc,
            };
        }
        state.mark_query_immediate();
    }
}

fn sortable_header(
    ui: &mut Ui,
    label: &str,
    key: SortKey,
    active_sort: SortKey,
    dir: SortDir,
) -> bool {
    let is_active = key == active_sort;
    let text_color = if is_active {
        pal::ACCENT
    } else {
        pal::TEXT_DIM
    };
    let resp = ui.add(
        egui::Button::new(RichText::new(label).strong().color(text_color))
            .frame(false)
            .corner_radius(CornerRadius::ZERO),
    );
    if is_active {
        let (arrow_rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
        paint_sort_arrow(
            ui.painter(),
            Pos2::new(arrow_rect.center().x, arrow_rect.center().y),
            dir,
            pal::ACCENT,
        );
    }
    resp.clicked()
}

fn draw_row_actions(
    ui: &mut Ui,
    state: &mut UiState,
    r: &crate::types::ReleaseRow,
    cmd_tx: &UnboundedSender<UiToWorkerCommand>,
) {
    let opts = DownloadOpts {
        format: state.settings.format,
        out_dir: state.settings.out_dir.clone(),
        batch_limit: state.settings.batch_limit,
    };

    if r.downloaded {
        if r.has_remote_update
            && r.classification.auto_downloadable()
            && !r.is_preorder
            && sharp_small_button(ui, "Update").clicked()
        {
            let _ = cmd_tx.send(UiToWorkerCommand::EnqueueDownloads {
                items: vec![r.key],
                opts: opts.clone(),
                ack_soft_quota: false,
            });
        }

        if let Some(ref path) = r.local_path {
            let is_missing = r.local_status == Some(LocalStatus::Missing);
            let btn = ui.add_enabled(
                !is_missing,
                egui::Button::new(RichText::new("Open Folder").size(11.5))
                    .corner_radius(CornerRadius::ZERO),
            );
            if btn.clicked() {
                let target_dir = if path.is_dir() {
                    path.as_path()
                } else {
                    path.parent().unwrap_or(path.as_path())
                };
                let _ = open::that_detached(target_dir);
            }
        }

        if r.classification == Classification::EmailRequired {
            if sharp_small_button(ui, "Open in Browser").clicked() {
                let _ = open::that_detached(&r.url);
            }
            if sharp_small_button(ui, "Redeem URL...").clicked() {
                state.redeem_modal = Some(RedeemModalState {
                    target: r.key,
                    title: format!("{} - {}", r.artist_name, r.title),
                    url_input: String::new(),
                    error: None,
                });
            }
            if sharp_small_button(ui, "Unmark").clicked() {
                let _ = cmd_tx.send(UiToWorkerCommand::SetReleaseDownloaded {
                    key: r.key,
                    downloaded: false,
                });
            }
        }

        if r.classification.auto_downloadable()
            && !r.is_preorder
            && sharp_small_button(ui, "Re-download").clicked()
        {
            let _ = cmd_tx.send(UiToWorkerCommand::EnqueueDownloads {
                items: vec![r.key],
                opts,
                ack_soft_quota: false,
            });
        }
        return;
    }

    match r.classification {
        Classification::Free | Classification::Nyp => {
            let btn = egui::Button::new(RichText::new("Download").size(11.5))
                .corner_radius(CornerRadius::ZERO);
            let resp = ui.add_enabled(!r.is_preorder, btn);
            let resp = if r.is_preorder {
                resp.on_disabled_hover_text("Pre-order release cannot be downloaded yet")
            } else {
                resp
            };
            if resp.clicked() {
                let _ = cmd_tx.send(UiToWorkerCommand::EnqueueDownloads {
                    items: vec![r.key],
                    opts,
                    ack_soft_quota: false,
                });
            }
        }
        Classification::EmailRequired => {
            if sharp_small_button(ui, "Open in Browser").clicked() {
                let _ = open::that_detached(&r.url);
            }
            if sharp_small_button(ui, "Redeem URL...").clicked() {
                state.redeem_modal = Some(RedeemModalState {
                    target: r.key,
                    title: format!("{} - {}", r.artist_name, r.title),
                    url_input: String::new(),
                    error: None,
                });
            }
            if sharp_small_button(ui, "Mark Downloaded").clicked() {
                let _ = cmd_tx.send(UiToWorkerCommand::SetReleaseDownloaded {
                    key: r.key,
                    downloaded: true,
                });
            }
        }
        Classification::Paid | Classification::Unknown => {
            if sharp_small_button(ui, "Open").clicked() {
                let _ = open::that_detached(&r.url);
            }
        }
    }
}
