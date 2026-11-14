use super::TabCtx;
use bcvault_core::types::{AudioFormat, OutputLayout};
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    ui.label(
        RichText::new("DOWNLOADS")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    egui::Grid::new("fx_settings_downloads_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Output Directory:");
            ui.horizontal(|ui| {
                let mut p_str = ctx.draft.out_dir.to_string_lossy().into_owned();
                let resp = ui.add(egui::TextEdit::singleline(&mut p_str).desired_width(320.0));
                if ctx.focus_first {
                    resp.request_focus();
                }
                if resp.changed() {
                    ctx.draft.out_dir = std::path::PathBuf::from(p_str);
                }
            });
            ui.end_row();

            ui.label("Output Layout:");
            egui::ComboBox::from_id_salt("fx_set_layout")
                .selected_text(ctx.draft.layout.label())
                .show_ui(ui, |ui| {
                    for l in OutputLayout::ALL {
                        ui.selectable_value(&mut ctx.draft.layout, l, l.label());
                    }
                });
            ui.end_row();

            ui.label("Filename Pattern:");
            ui.vertical(|ui| {
                if ui
                    .add(
                        egui::TextEdit::singleline(ctx.filename_pattern_input)
                            .hint_text("{artist} - {album}")
                            .desired_width(320.0),
                    )
                    .changed()
                {
                    ctx.draft.filename_pattern = ctx.filename_pattern_input.clone();
                }
                ui.label(
                    RichText::new(
                        "Tags: {artist}, {page_artist}, {album}, {title}, {year}, {date}, {format}, {type}, {id}",
                    )
                    .size(10.5)
                    .color(th.text_dim),
                );
            });
            ui.end_row();

            ui.label("Preferred Format:");
            egui::ComboBox::from_id_salt("fx_set_fmt")
                .selected_text(ctx.draft.format.label())
                .show_ui(ui, |ui| {
                    for f in AudioFormat::ALL {
                        ui.selectable_value(&mut ctx.draft.format, f, f.label());
                    }
                });
            ui.end_row();

            ui.label("Batch Limit:");
            ui.add(egui::DragValue::new(&mut ctx.draft.batch_limit).range(1..=200));
            ui.end_row();
        });
}
