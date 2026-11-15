use crate::{
    actions::{FxAction, FxSortKey},
    motion::SpringCfg,
    paint,
    services::messages::{FxCommand, Tier},
    state::{FxState, paint_cover_slot},
    views::wall::class_color,
};
use bcvault_core::{
    types::{Classification, SortDir},
    worker::messages::UiToWorkerCommand,
};
use chrono::{TimeZone, Utc};
use egui::{Align, Color32, FontId, Layout, Pos2, RichText, Sense, Stroke, Ui, Vec2};
use egui_extras::{Column, TableBuilder};
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
    let search_lower = state.query.base.search.trim().to_lowercase();

    let table = TableBuilder::new(ui)
        .striped(false)
        .resizable(true)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(28.0)) // select + status
        .column(Column::exact(30.0)) // 24px T64 cover
        .column(Column::initial(150.0).at_least(90.0)) // Artist
        .column(Column::remainder().at_least(180.0)) // Title
        .column(Column::exact(58.0)) // Type
        .column(Column::exact(86.0)) // Released
        .column(Column::exact(52.0)) // Tracks
        .column(Column::exact(84.0)) // Class
        .column(Column::exact(62.0)) // Format
        .column(Column::exact(156.0)); // Actions

    table
        .header(26.0, |mut header| {
            header.col(|ui| {
                let stats = state.selection_stats();
                let glyph = if stats.total_in_view > 0 && stats.selected == stats.total_in_view {
                    "☑"
                } else if stats.selected > 0 {
                    "⊟"
                } else {
                    "☐"
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(glyph).monospace().color(
                            if stats.selected > 0 {
                                th.accent
                            } else {
                                th.text_dim
                            },
                        ))
                        .frame(false),
                    )
                    .on_hover_text("Select all / Clear selection (Ctrl+A)")
                    .clicked()
                {
                    if stats.total_in_view > 0 && stats.selected == stats.total_in_view {
                        pending_actions.push(FxAction::ClearSelection);
                    } else {
                        pending_actions.push(FxAction::SelectAll);
                    }
                }
            });
            header.col(|_ui| {});
            sort_header(
                &mut header,
                state,
                "Artist",
                FxSortKey::Artist,
                &mut pending_actions,
            );
            sort_header(
                &mut header,
                state,
                "Title",
                FxSortKey::Title,
                &mut pending_actions,
            );
            sort_header(
                &mut header,
                state,
                "Type",
                FxSortKey::Type,
                &mut pending_actions,
            );
            sort_header(
                &mut header,
                state,
                "Released",
                FxSortKey::Released,
                &mut pending_actions,
            );
            sort_header(
                &mut header,
                state,
                "Trk",
                FxSortKey::Tracks,
                &mut pending_actions,
            );
            sort_header(
                &mut header,
                state,
                "Class",
                FxSortKey::Classification,
                &mut pending_actions,
            );
            sort_header(
                &mut header,
                state,
                "Fmt",
                FxSortKey::Format,
                &mut pending_actions,
            );
            header.col(|ui| {
                ui.label(RichText::new("Actions").strong().color(th.text_dim));
            });
        })
        .body(|body| {
            body.rows(28.0, state.rows.len(), |mut row_ui| {
                let row_idx = row_ui.index();
                let w = state.rows[row_idx].clone();
                let key = w.rel.key;
                let is_sel = state.selected_keys.contains(&key);

                row_ui.col(|ui| {
                    let mut checked = is_sel;
                    if ui.checkbox(&mut checked, "").changed() {
                        pending_actions.push(FxAction::ToggleSelect(key));
                    }
                });

                row_ui.col(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    paint_cover_slot(
                        ui.painter(),
                        state,
                        w.art_id,
                        key.id,
                        r,
                        Tier::T64,
                        1.0,
                        Vec2::ZERO,
                    );
                });

                row_ui.col(|ui| {
                    draw_highlighted_text(
                        ui,
                        &w.rel.artist_name,
                        &search_lower,
                        th.text,
                        th.accent,
                    );
                });

                row_ui.col(|ui| {
                    // Flash overlay if row was recently updated
                    if let Some(&until) = state.row_flashes.get(&key)
                        && until > now_s
                    {
                        let a = ((until - now_s) as f32 / 0.6).clamp(0.0, 1.0);
                        paint::rect_filled(
                            ui.painter(),
                            ui.max_rect(),
                            th.accent.gamma_multiply(a * 0.28),
                        );
                        ui.ctx().request_repaint();
                    }
                    let resp =
                        draw_highlighted_text(ui, &w.rel.title, &search_lower, th.text, th.accent);
                    if resp.clicked() {
                        pending_actions.push(FxAction::OpenDrawer(key));
                    }
                    resp.context_menu(|ui| {
                        if paint::sharp_button(ui, "Inspect Details").clicked() {
                            pending_actions.push(FxAction::OpenDrawer(key));
                            ui.close();
                        }
                        if paint::sharp_button(ui, "Open in Browser").clicked() {
                            pending_actions.push(FxAction::OpenInBrowser(key));
                            ui.close();
                        }
                        if paint::sharp_button(ui, "Relink to Folder...").clicked() {
                            pending_actions.push(FxAction::RelinkReleaseModal(key));
                            ui.close();
                        }
                        if w.rel.downloaded {
                            if paint::sharp_button(ui, "Unmark Downloaded").clicked() {
                                pending_actions.push(FxAction::SetReleaseDownloaded(key, false));
                                ui.close();
                            }
                        } else if paint::sharp_button(ui, "Mark Downloaded").clicked() {
                            pending_actions.push(FxAction::SetReleaseDownloaded(key, true));
                            ui.close();
                        }
                        if w.rel.classification == Classification::Unavailable {
                            if paint::sharp_button(ui, "Unmark Unavailable").clicked() {
                                pending_actions.push(FxAction::SetReleaseUnavailable(key, false));
                                ui.close();
                            }
                        } else if paint::sharp_button(ui, "Mark as Unavailable").clicked() {
                            pending_actions.push(FxAction::SetReleaseUnavailable(key, true));
                            ui.close();
                        }
                    });
                    if w.rel.downloaded {
                        if w.rel.local_status == Some(bcvault_core::types::LocalStatus::Archived) {
                            paint_table_badge(ui, "ARCHIVED", th.text_dim, th.border);
                        } else if w.rel.local_status
                            == Some(bcvault_core::types::LocalStatus::Missing)
                        {
                            paint_table_badge(ui, "MISSING", th.warn, th.warn);
                        } else if let Some(folders) = state.folders.get(&w.rel.band_id) {
                            let folder_missing = match &w.rel.local_path {
                                Some(p) => {
                                    let rel_key =
                                        bcvault_core::library::link::normalize_path_key(p);
                                    let owning = folders
                                        .iter()
                                        .filter(|f| {
                                            let folder_key =
                                                bcvault_core::library::link::normalize_path_key(
                                                    &f.path,
                                                );
                                            rel_key == folder_key
                                                || rel_key.starts_with(&format!("{folder_key}/"))
                                        })
                                        .max_by_key(|f| f.path.as_os_str().len());
                                    owning
                                        .map(|f| f.state != bcvault_core::types::FolderState::Ok)
                                        .unwrap_or(false)
                                }
                                None => folders
                                    .iter()
                                    .any(|f| f.state != bcvault_core::types::FolderState::Ok),
                            };
                            if folder_missing {
                                paint_table_badge(ui, "FOLDER MISSING", th.warn, th.warn);
                            }
                        }
                    }
                });

                row_ui.col(|ui| {
                    ui.label(
                        RichText::new(w.rel.key.kind.label())
                            .size(11.5)
                            .color(th.text_dim),
                    );
                });

                row_ui.col(|ui| {
                    let date_s = w
                        .rel
                        .published_at
                        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
                        .map(|dt| dt.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "-".to_string());
                    ui.label(
                        RichText::new(date_s)
                            .monospace()
                            .size(11.0)
                            .color(th.text_dim),
                    );
                });

                row_ui.col(|ui| {
                    let t_s = w
                        .rel
                        .track_count
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    ui.label(RichText::new(t_s).monospace().size(11.0).color(th.text_dim));
                });

                row_ui.col(|ui| {
                    let col = class_color(w.rel.classification, &th);
                    ui.label(
                        RichText::new(w.rel.classification.label())
                            .monospace()
                            .size(10.5)
                            .color(col),
                    );
                });

                row_ui.col(|ui| {
                    let fmt_s = w.rel.downloaded_format.map(|f| f.label()).unwrap_or("-");
                    let is_archived =
                        w.rel.local_status == Some(bcvault_core::types::LocalStatus::Archived);
                    let col = if is_archived {
                        th.text_dim
                    } else if w.rel.downloaded {
                        th.ok
                    } else {
                        th.text_dim
                    };
                    ui.label(RichText::new(fmt_s).monospace().size(11.0).color(col));
                });

                row_ui.col(|ui| {
                    if paint::sharp_button(ui, "Details").clicked() {
                        pending_actions.push(FxAction::OpenDrawer(key));
                    }
                    let is_archived =
                        w.rel.local_status == Some(bcvault_core::types::LocalStatus::Archived);
                    if w.rel.downloaded && !is_archived {
                        if w.rel.local_path.is_some() && paint::sharp_button(ui, "Folder").clicked()
                        {
                            pending_actions.push(FxAction::OpenFolder(key));
                        }
                        if w.rel.is_downloadable() && paint::sharp_button(ui, "Re-dl").clicked() {
                            pending_actions.push(FxAction::Redownload(key));
                        }
                    } else if w.rel.is_downloadable()
                        && paint::sharp_button(ui, "Download").clicked()
                    {
                        pending_actions.push(FxAction::DownloadKey(key));
                    } else if paint::sharp_button(ui, "Browser").clicked() {
                        pending_actions.push(FxAction::OpenInBrowser(key));
                    }
                });
            });
        });

    for act in pending_actions {
        state.dispatch(act, core_tx, fx_tx, now_s);
    }
}

fn sort_header(
    header: &mut egui_extras::TableRow<'_, '_>,
    state: &mut FxState,
    label: &str,
    key: FxSortKey,
    pending: &mut Vec<FxAction>,
) {
    let th = state.active_theme;
    let is_active = state.query.fx_sort == key;
    let dir = state.query.base.dir;
    header.col(|ui| {
        let resp = ui.add(
            egui::Button::new(RichText::new(label).strong().color(if is_active {
                th.accent
            } else {
                th.text
            }))
            .frame(false)
            .corner_radius(egui::CornerRadius::ZERO),
        );
        if is_active {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            let rot_target = if dir == SortDir::Asc { 0.0 } else { 1.0 };
            let t = state
                .motion
                .val(("sort_arrow", key as u8), rot_target, SpringCfg::SNAP);
            let top_y = r.min.y + t * r.height();
            let bot_y = r.max.y - t * r.height();
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(r.center().x, top_y),
                    Pos2::new(r.max.x, bot_y),
                    Pos2::new(r.min.x, bot_y),
                ],
                th.accent,
                Stroke::NONE,
            ));
        }
        if resp.clicked() {
            let next_dir = if is_active && dir == SortDir::Desc {
                SortDir::Asc
            } else {
                SortDir::Desc
            };
            pending.push(FxAction::SetSort(key, next_dir));
        }
    });
}

fn draw_highlighted_text(
    ui: &mut Ui,
    text: &str,
    needle_lower: &str,
    text_col: Color32,
    accent: Color32,
) -> egui::Response {
    if !needle_lower.is_empty()
        && let Some(pos) = text.to_lowercase().find(needle_lower)
    {
        let end = (pos + needle_lower.len()).min(text.len());
        let mut job = egui::text::LayoutJob::default();
        let font = FontId::proportional(12.5);
        if pos > 0 && text.is_char_boundary(pos) {
            job.append(
                &text[..pos],
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: text_col,
                    ..Default::default()
                },
            );
        }
        if text.is_char_boundary(pos) && text.is_char_boundary(end) {
            job.append(
                &text[pos..end],
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: text_col,
                    background: accent.gamma_multiply(0.32),
                    ..Default::default()
                },
            );
        }
        if end < text.len() && text.is_char_boundary(end) {
            job.append(
                &text[end..],
                0.0,
                egui::TextFormat {
                    font_id: font,
                    color: text_col,
                    ..Default::default()
                },
            );
        }
        return ui.add(egui::Label::new(job).truncate().sense(Sense::click()));
    }
    ui.add(
        egui::Label::new(RichText::new(text).size(12.5).color(text_col))
            .truncate()
            .sense(Sense::click()),
    )
}

fn paint_table_badge(ui: &mut Ui, text: &str, text_col: Color32, border_col: Color32) {
    let font = FontId::monospace(9.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, text_col);
    let size = galley.size() + Vec2::new(8.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    paint::rect(
        ui.painter(),
        rect,
        Color32::TRANSPARENT,
        Stroke::new(1.0_f32, border_col),
    );
    ui.painter()
        .galley(rect.min + Vec2::new(4.0, 2.0), galley, text_col);
}
