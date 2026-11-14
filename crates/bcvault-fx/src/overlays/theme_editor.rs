use crate::{
    actions::FxAction,
    paint,
    services::messages::{FxCommand, ToastKind},
    state::FxState,
    theme::{
        file::{COLOR_KEYS, ThemeFile, format_hex, parse_hex_color, slugify, unique_slug},
        registry::{ThemeEntry, ThemeSource},
    },
};
use egui::{CornerRadius, RichText, Stroke, Vec2};
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug)]
pub struct ThemeEditorState {
    pub is_new: bool,
    pub existing_path: Option<PathBuf>,
    pub file: ThemeFile,
    pub hex_inputs: [String; 12],
    pub hex_errors: [Option<String>; 12],
    pub save_error: Option<String>,
    pub confirm_delete: bool,
}

impl ThemeEditorState {
    pub fn from_entry(entry: &ThemeEntry, duplicate: bool, existing_ids: &[String]) -> Self {
        let mut file = entry.file.clone();
        let existing_path = if duplicate {
            let mut base_name = format!("{} Copy", entry.name);
            if base_name.chars().count() > 40 {
                base_name = base_name.chars().take(40).collect();
            }
            file.name = base_name;
            file.id = unique_slug(&slugify(&file.name), |id| {
                existing_ids.iter().any(|x| x == id)
            });
            file.author = String::new();
            None
        } else {
            match &entry.source {
                ThemeSource::User(p) => Some(p.clone()),
                ThemeSource::Builtin => None,
            }
        };

        let hex_inputs = std::array::from_fn(|i| {
            file.colors
                .get(COLOR_KEYS[i])
                .map(format_hex)
                .unwrap_or_else(|| "#000000".to_string())
        });

        Self {
            is_new: duplicate,
            existing_path,
            file,
            hex_inputs,
            hex_errors: std::array::from_fn(|_| None),
            save_error: None,
            confirm_delete: false,
        }
    }

    pub fn has_hex_errors(&self) -> bool {
        self.hex_errors.iter().any(Option::is_some)
    }
}

pub fn show(
    ctx: &egui::Context,
    state: &mut FxState,
    fx_tx: &UnboundedSender<FxCommand>,
    pending_actions: &mut Vec<FxAction>,
    now_s: f64,
) {
    let Some(mut editor) = state.theme_editor.take() else {
        return;
    };

    let th = state.active_theme;
    let title = if editor.is_new {
        "New Theme"
    } else {
        "Edit Theme"
    };
    let mut keep_open = true;
    let mut cancel_clicked = false;
    let mut save_clicked = false;
    let mut export_clicked = false;
    let mut copy_clicked = false;
    let mut delete_confirmed = false;

    let screen = ctx.available_rect();
    let max_size = Vec2::new(
        (screen.width() - 32.0).max(380.0),
        (screen.height() - 48.0).max(360.0),
    );

    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .default_width(540.0)
        .max_size(max_size)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(
            egui::Frame::window(&ctx.style())
                .fill(th.bg1)
                .stroke(Stroke::new(1.0_f32, th.border_hi))
                .corner_radius(CornerRadius::ZERO),
        )
        .open(&mut keep_open)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height((max_size.y - 90.0).max(220.0))
                .show(ui, |ui| {
                    egui::Grid::new("theme_editor_meta_grid")
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Name:");
                            let name_resp = ui.add(
                                egui::TextEdit::singleline(&mut editor.file.name)
                                    .desired_width(300.0),
                            );
                            if editor.is_new && name_resp.changed() {
                                let base = slugify(&editor.file.name);
                                editor.file.id =
                                    unique_slug(&base, |id| state.themes.contains_id(id));
                            }
                            ui.end_row();

                            ui.label("ID:");
                            ui.label(
                                RichText::new(&editor.file.id)
                                    .monospace()
                                    .color(th.text_dim),
                            );
                            ui.end_row();

                            ui.label("Description:");
                            ui.add(
                                egui::TextEdit::singleline(&mut editor.file.description)
                                    .desired_width(300.0),
                            );
                            ui.end_row();

                            ui.label("Author:");
                            ui.add(
                                egui::TextEdit::singleline(&mut editor.file.author)
                                    .desired_width(220.0),
                            );
                            ui.end_row();
                        });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut editor.file.dark, "Dark theme");
                        ui.checkbox(&mut editor.file.scanlines, "Scanlines overlay");
                    });

                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("COLORS")
                            .monospace()
                            .strong()
                            .color(th.accent),
                    );
                    ui.add_space(4.0);

                    egui::Grid::new("theme_editor_colors_grid")
                        .num_columns(4)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            for (idx, key) in COLOR_KEYS.iter().enumerate() {
                                ui.label(RichText::new(*key).monospace());
                                let mut color =
                                    editor.file.colors.get(key).unwrap_or(egui::Color32::BLACK);
                                if ui.color_edit_button_srgba(&mut color).changed() {
                                    let opaque =
                                        egui::Color32::from_rgb(color.r(), color.g(), color.b());
                                    editor.file.colors.set(key, opaque);
                                    editor.hex_inputs[idx] = format_hex(opaque);
                                    editor.hex_errors[idx] = None;
                                }
                                let hex_resp = ui.add(
                                    egui::TextEdit::singleline(&mut editor.hex_inputs[idx])
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(84.0),
                                );
                                if hex_resp.changed() {
                                    match parse_hex_color(&editor.hex_inputs[idx]) {
                                        Ok(parsed) => {
                                            editor.file.colors.set(key, parsed);
                                            editor.hex_errors[idx] = None;
                                        }
                                        Err(reason) => {
                                            editor.hex_errors[idx] = Some(reason);
                                        }
                                    }
                                }
                                if let Some(err) = &editor.hex_errors[idx] {
                                    ui.label(RichText::new(err).small().color(th.err));
                                } else {
                                    ui.label("");
                                }
                                ui.end_row();
                            }
                        });

                    let hints = editor.file.hints();
                    if !hints.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("CONTRAST & THEME HINTS")
                                .monospace()
                                .small()
                                .color(th.warn),
                        );
                        for hint in hints {
                            ui.label(
                                RichText::new(format!("• {}", hint.message))
                                    .small()
                                    .color(th.warn),
                            );
                        }
                    }

                    if let Some(err) = &editor.save_error {
                        ui.add_space(4.0);
                        ui.label(RichText::new(err).small().color(th.err));
                    }
                });

            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if paint::sharp_button(ui, "Save").clicked() {
                    save_clicked = true;
                }
                if paint::sharp_button(ui, "Cancel").clicked() {
                    cancel_clicked = true;
                }
                if paint::sharp_button(ui, "Export").clicked() {
                    export_clicked = true;
                }
                if paint::sharp_button(ui, "Copy as text").clicked() {
                    copy_clicked = true;
                }
                if !editor.is_new {
                    if editor.confirm_delete {
                        if ui
                            .button(RichText::new("Confirm delete").color(th.err))
                            .clicked()
                        {
                            delete_confirmed = true;
                        }
                        if paint::sharp_button(ui, "Keep").clicked() {
                            editor.confirm_delete = false;
                        }
                    } else if paint::sharp_button(ui, "Delete").clicked() {
                        editor.confirm_delete = true;
                    }
                }
            });
        });

    if !keep_open || cancel_clicked {
        state.theme_preview = None;
        return;
    }

    if copy_clicked {
        ctx.copy_text(editor.file.to_toml_string());
        state.push_toast(
            ToastKind::Info,
            format!("Copied '{}' TOML to clipboard", editor.file.name),
            now_s,
        );
    }

    if export_clicked {
        state.pending_theme_export_pick = Some(editor.file.clone());
    }

    if delete_confirmed && let Some(dir) = state.themes_dir.clone() {
        match state.themes.delete_user_theme(&dir, &editor.file.id) {
            Ok(()) => {
                let _ = fx_tx.send(FxCommand::SetThemeCommands(state.themes.palette_commands()));
                if state.prefs.theme == editor.file.id {
                    pending_actions.push(FxAction::SetTheme("carbon".to_string()));
                }
                state.theme_preview = None;
                state.push_toast(
                    ToastKind::Info,
                    format!("Deleted theme '{}'", editor.file.name),
                    now_s,
                );
                return;
            }
            Err(err) => {
                editor.save_error = Some(err.message);
                editor.confirm_delete = false;
            }
        }
    }

    if save_clicked {
        if editor.has_hex_errors() {
            editor.save_error = Some("Fix invalid hex colors before saving".to_string());
        } else if let Some(dir) = state.themes_dir.clone() {
            match state
                .themes
                .save_user_theme(&dir, &editor.file, editor.existing_path.as_deref())
            {
                Ok(_) => {
                    let saved_id = editor.file.id.clone();
                    let saved_name = editor.file.name.clone();
                    let _ =
                        fx_tx.send(FxCommand::SetThemeCommands(state.themes.palette_commands()));
                    pending_actions.push(FxAction::SetTheme(saved_id));
                    state.theme_preview = None;
                    state.push_toast(
                        ToastKind::Info,
                        format!("Saved theme '{saved_name}'"),
                        now_s,
                    );
                    return;
                }
                Err(err) => {
                    editor.save_error = Some(err.message);
                }
            }
        } else {
            editor.save_error = Some("Themes directory is unavailable".to_string());
        }
    }

    state.theme_preview = Some(editor.file.to_fx_theme());
    state.theme_editor = Some(editor);
}
