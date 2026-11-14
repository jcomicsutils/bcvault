use super::TabCtx;
use crate::{
    actions::FxAction, motion::EffectsLevel, paint, services::messages::FxCommand, theme::ThemeId,
};
use egui::RichText;

pub fn show(ui: &mut egui::Ui, ctx: &mut TabCtx<'_>) {
    let th = ctx.theme;
    ui.label(
        RichText::new("VISUALS & MOTION")
            .monospace()
            .strong()
            .color(th.accent),
    );
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Theme:");
        for (idx, tid) in ThemeId::ALL.into_iter().enumerate() {
            let active = ctx.prefs.theme == tid;
            let resp = ui.selectable_label(active, tid.label());
            if idx == 0 && ctx.focus_first {
                resp.request_focus();
            }
            if resp.clicked() {
                ctx.pending_actions.push(FxAction::SetTheme(tid));
            }
        }
    });

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
