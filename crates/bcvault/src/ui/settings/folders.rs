use super::TabCtx;
use crate::types::FolderState;
use crate::ui::theme::pal;
use crate::ui::widgets::{paint_folder_status_dot, sharp_small_button};
use crate::worker::messages::UiToWorkerCommand;
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    ui.label(RichText::new("Folder Naming").strong());
    ui.label(
        RichText::new(
            "Declare how your folders are named using placeholders: {title} (required), \
            {artist}, {year}, and {noise}. Punctuation/brackets are ignored during matching.",
        )
        .color(pal::TEXT_DIM)
        .size(11.5),
    );

    ui.add_space(4.0);
    ui.label(RichText::new("Templates:").size(12.0));

    let mut remove_tmpl = None;
    let can_remove_tmpl = ctx.draft.folder_style.templates.len() > 1;
    for (idx, tmpl) in ctx.draft.folder_style.templates.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(tmpl).desired_width(260.0));
            if idx == 0 && ctx.focus_first {
                edit.request_focus();
            }
            if can_remove_tmpl && sharp_small_button(ui, "✕").clicked() {
                remove_tmpl = Some(idx);
            }
        });
    }
    if let Some(idx) = remove_tmpl {
        ctx.draft.folder_style.templates.remove(idx);
    }

    ui.horizontal(|ui| {
        if ctx.draft.folder_style.templates.len() < 8
            && sharp_small_button(ui, "+ Add Template").clicked()
        {
            ctx.draft.folder_style.templates.push("{title}".to_string());
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Presets:").color(pal::TEXT_DIM).size(11.5));
        let presets = [
            "{title}",
            "{artist} - {title}",
            "{year} - {title}",
            "{title} ({year})",
            "{title} ({noise})",
            "{artist} - {year} - {title}",
        ];
        for p in presets {
            if sharp_small_button(ui, p).clicked()
                && !ctx.draft.folder_style.templates.iter().any(|t| t == p)
                && ctx.draft.folder_style.templates.len() < 8
            {
                ctx.draft.folder_style.templates.push(p.to_string());
            }
        }
    });

    ui.add_space(4.0);
    ui.label(RichText::new("Noise List ({noise} tokens):").size(12.0));
    ui.label(
        RichText::new("Noise decorations are only recognized through the {noise} placeholder.")
            .color(pal::TEXT_DIM)
            .size(11.5),
    );

    let mut remove_noise = None;
    for (idx, entry) in ctx.draft.folder_style.noise.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(entry).desired_width(180.0));
            if sharp_small_button(ui, "✕").clicked() {
                remove_noise = Some(idx);
            }
        });
    }
    if let Some(idx) = remove_noise {
        ctx.draft.folder_style.noise.remove(idx);
    }

    ui.horizontal(|ui| {
        if ctx.draft.folder_style.noise.len() < 32
            && sharp_small_button(ui, "+ Add Noise").clicked()
        {
            ctx.draft.folder_style.noise.push(String::new());
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Quick-add:").color(pal::TEXT_DIM).size(11.5));
        let quick_adds = [
            "FLAC", "MP3", "V0", "320", "WAV", "AAC", "OGG", "ALAC", "24bit",
        ];
        for q in quick_adds {
            if sharp_small_button(ui, q).clicked()
                && !ctx
                    .draft
                    .folder_style
                    .noise
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(q))
                && ctx.draft.folder_style.noise.len() < 32
            {
                ctx.draft.folder_style.noise.push(q.to_string());
            }
        }
    });

    if let Err(err) = ctx.draft.folder_style.validate() {
        ui.label(RichText::new(format!("⚠ {err}")).color(pal::ERR));
    }
    for hint in ctx.draft.folder_style.hints() {
        ui.label(RichText::new(format!("ℹ {hint}")).color(pal::WARN));
    }

    ui.add_space(8.0);
    ui.separator();
    ui.label(RichText::new("Linked Artist Folders").strong());

    if ctx.folders.is_empty() {
        ui.label(RichText::new("No artist folders linked.").color(pal::TEXT_DIM));
        return;
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new("Filter:").color(pal::TEXT_DIM));
        ui.add(
            egui::TextEdit::singleline(ctx.folder_filter)
                .hint_text("Filter by artist or path...")
                .desired_width(240.0),
        );
        if !ctx.folder_filter.is_empty() && sharp_small_button(ui, "X").clicked() {
            ctx.folder_filter.clear();
        }
    });

    let q = ctx.folder_filter.trim().to_lowercase();
    let mut linked: Vec<_> = ctx
        .folders
        .values()
        .flatten()
        .filter(|f| {
            if q.is_empty() {
                return true;
            }
            let artist_name = ctx
                .artists
                .iter()
                .find(|a| a.band_id == f.band_id)
                .map(|a| a.display_name().to_lowercase())
                .unwrap_or_default();
            let path_str = f.path.display().to_string().to_lowercase();
            artist_name.contains(&q) || path_str.contains(&q)
        })
        .cloned()
        .collect();

    linked.sort_by(|a, b| {
        let name_a = ctx
            .artists
            .iter()
            .find(|x| x.band_id == a.band_id)
            .map(|x| x.display_name())
            .unwrap_or("");
        let name_b = ctx
            .artists
            .iter()
            .find(|x| x.band_id == b.band_id)
            .map(|x| x.display_name())
            .unwrap_or("");
        name_a
            .to_lowercase()
            .cmp(&name_b.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });

    if linked.is_empty() {
        ui.label(RichText::new("No matching linked folders.").color(pal::TEXT_DIM));
        return;
    }

    ui.add_space(4.0);
    let row_h = 24.0;
    egui::ScrollArea::vertical()
        .id_salt("classic_linked_folders_virtual")
        .auto_shrink([false, false])
        .max_height(200.0)
        .show_rows(ui, row_h, linked.len(), |ui, range| {
            let row_w = ui.available_width();
            for folder in &linked[range] {
                ui.allocate_ui_with_layout(
                    egui::vec2(row_w, row_h),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let tip = match folder.state {
                            FolderState::Ok => "Folder OK and accessible",
                            FolderState::Missing => "Folder missing on disk",
                            FolderState::Unreadable => "Folder unreadable or inaccessible",
                        };
                        paint_folder_status_dot(ui, folder.state).on_hover_text(tip);

                        let artist_name = ctx
                            .artists
                            .iter()
                            .find(|a| a.band_id == folder.band_id)
                            .map(|a| a.display_name())
                            .unwrap_or("Unknown Artist");
                        ui.add_sized(
                            [150.0, row_h],
                            egui::Label::new(RichText::new(artist_name).strong()).truncate(),
                        )
                        .on_hover_text(artist_name);

                        let path_str = folder.path.display().to_string();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if sharp_small_button(ui, "Unlink").clicked() {
                                let _ = ctx.cmd_tx.send(UiToWorkerCommand::UnlinkArtistFolder {
                                    folder_id: folder.id,
                                });
                            }
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&path_str)
                                                .color(pal::TEXT_DIM)
                                                .monospace(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&path_str);
                                },
                            );
                        });
                    },
                );
            }
        });
}
