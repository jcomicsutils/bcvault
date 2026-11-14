use super::TabCtx;
use crate::{
    actions::FxAction,
    motion::EffectsLevel,
    paint,
    services::messages::FxCommand,
    theme::{
        FxTheme,
        registry::{IssueSeverity, ThemeEntry},
    },
};
use egui::{Align2, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    ui.label(
        RichText::new("VISUALS & MOTION")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    ui.label(RichText::new("Built-in").small().color(th.text_dim));
    let mut focus_first = ctx.focus_first;
    let builtins: Vec<&ThemeEntry> = ctx.themes.builtin_entries().collect();
    ui.horizontal_wrapped(|ui| {
        for entry in builtins {
            let active = ctx.prefs.theme == entry.id;
            let resp = theme_swatch_tile(ui, entry, active, th);
            if focus_first {
                resp.request_focus();
                focus_first = false;
            }
            if resp.clicked() {
                ctx.pending_actions
                    .push(FxAction::SetTheme(entry.id.clone()));
            }
        }
    });

    let user_themes: Vec<&ThemeEntry> = ctx.themes.user_entries().collect();
    ui.add_space(4.0);
    ui.label(RichText::new("Yours").small().color(th.text_dim));
    if user_themes.is_empty() {
        ui.label(
            RichText::new("No user themes yet — drop .toml files in the themes folder.")
                .small()
                .color(th.text_dim),
        );
    } else {
        ui.horizontal_wrapped(|ui| {
            for entry in user_themes {
                let active = ctx.prefs.theme == entry.id;
                let resp = theme_swatch_tile(ui, entry, active, th);
                if resp.clicked() {
                    ctx.pending_actions
                        .push(FxAction::SetTheme(entry.id.clone()));
                }
            }
        });
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if paint::sharp_button(ui, "Open themes folder").clicked() {
            ctx.pending_actions.push(FxAction::OpenThemesFolder);
        }
        if paint::sharp_button(ui, "Reload").clicked() {
            ctx.pending_actions.push(FxAction::ReloadThemes);
        }
    });

    let issues = ctx.themes.issues();
    if !issues.is_empty() {
        ui.add_space(4.0);
        egui::CollapsingHeader::new(
            RichText::new(format!("{} theme file(s) had problems", issues.len()))
                .small()
                .color(th.warn),
        )
        .default_open(false)
        .show(ui, |ui| {
            for issue in issues {
                let file_name = issue
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("themes");
                let col = match issue.severity {
                    IssueSeverity::Error => th.err,
                    IssueSeverity::Warning => th.warn,
                };
                ui.label(
                    RichText::new(format!("{file_name}: {}", issue.message))
                        .small()
                        .color(col),
                );
            }
        });
    }

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Effects Level:");
        for lvl in EffectsLevel::ALL {
            let active = ctx.prefs.effects == lvl;
            if ui.selectable_label(active, lvl.label()).clicked() {
                ctx.pending_actions.push(FxAction::SetEffects(lvl));
            }
        }
    });

    ui.horizontal(|ui| {
        let mut ad = ctx.prefs.adaptive_accent;
        if ui
            .checkbox(&mut ad, "Adaptive accent from hovered cover")
            .changed()
        {
            ctx.pending_actions.push(FxAction::SetAdaptiveAccent(ad));
        }
        let mut rm = ctx.prefs.reduced_motion;
        if ui.checkbox(&mut rm, "Reduced motion").changed() {
            ctx.pending_actions.push(FxAction::SetReducedMotion(rm));
        }
    });

    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut ctx.prefs.animated_grain, "Animated grain (30 fps)")
            .changed()
        {
            let _ = ctx.fx_tx.send(FxCommand::SetPref {
                key: "animated_grain".to_string(),
                value: if ctx.prefs.animated_grain { "1" } else { "0" }.to_string(),
            });
        }
        if ui
            .checkbox(&mut ctx.prefs.scanlines, "CRT Scanlines")
            .changed()
        {
            let _ = ctx.fx_tx.send(FxCommand::SetPref {
                key: "scanlines".to_string(),
                value: if ctx.prefs.scanlines { "1" } else { "0" }.to_string(),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Texture VRAM Budget (MB):");
        if ui
            .add(egui::DragValue::new(&mut ctx.prefs.tex_budget_mb).range(64..=1024))
            .changed()
        {
            ctx.tex_cache.set_budget_mb(ctx.prefs.tex_budget_mb);
            let _ = ctx.fx_tx.send(FxCommand::SetPref {
                key: "tex_budget_mb".to_string(),
                value: ctx.prefs.tex_budget_mb.to_string(),
            });
        }
        let cache_mb = (ctx.cover_cache_bytes as f64) / (1024.0 * 1024.0);
        ui.label(
            RichText::new(format!("Disk cache: {cache_mb:.1} MB"))
                .monospace()
                .color(th.text_dim),
        );
        if paint::sharp_button(ui, "Purge Covers").clicked() {
            ctx.pending_actions.push(FxAction::PurgeCovers);
        }
    });
}

fn theme_swatch_tile(
    ui: &mut egui::Ui,
    entry: &ThemeEntry,
    active: bool,
    th: FxTheme,
) -> egui::Response {
    let font_id = FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(
        entry.name.clone(),
        font_id,
        if active { th.text } else { th.text_dim },
    );
    let swatch_sz = 10.0;
    let swatch_gap = 2.0;
    let swatches_w = swatch_sz * 3.0 + swatch_gap * 2.0;
    let pad_x = 8.0;
    let pad_y = 5.0;
    let w = (pad_x * 2.0 + swatches_w + 6.0 + galley.size().x).max(90.0);
    let h = galley.size().y.max(swatch_sz) + pad_y * 2.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg = if active {
            th.accent.gamma_multiply(0.16)
        } else if resp.hovered() {
            th.bg2
        } else {
            th.bg0
        };
        let border_col = if active {
            th.accent
        } else if resp.hovered() {
            th.border_hi
        } else {
            th.border
        };
        paint::rect_filled(p, rect, bg);
        paint::rect_outline(p, rect, Stroke::new(1.0_f32, border_col));

        let sy = rect.center().y - swatch_sz * 0.5;
        let mut sx = rect.min.x + pad_x;
        for col in [entry.theme.bg1, entry.theme.accent, entry.theme.accent2] {
            let sr = Rect::from_min_size(Pos2::new(sx, sy), Vec2::splat(swatch_sz));
            paint::rect_filled(p, sr, col);
            paint::rect_outline(p, sr, Stroke::new(1.0_f32, th.border));
            sx += swatch_sz + swatch_gap;
        }

        let text_pos = Pos2::new(sx + 4.0, rect.center().y);
        p.galley(
            Align2::LEFT_CENTER.pos_in_rect(&Rect::from_min_max(
                Pos2::new(text_pos.x, rect.min.y),
                rect.max,
            )),
            galley,
            th.text,
        );
    }

    let mut tip = String::new();
    if !entry.description.is_empty() {
        tip.push_str(&entry.description);
    }
    if !entry.author.is_empty() {
        if !tip.is_empty() {
            tip.push('\n');
        }
        tip.push_str(&format!("by {}", entry.author));
    }
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}
