pub mod builtin;
pub mod file;
pub mod registry;

use crate::motion::{Motion, SpringCfg, oklab::clamp_accent_contrast};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, Style,
    TextStyle, Visuals, epaint::Shadow,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxTheme {
    pub bg0: Color32,
    pub bg1: Color32,
    pub bg2: Color32,
    pub border: Color32,
    pub border_hi: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub is_dark: bool,
    pub scanlines: bool,
}

impl FxTheme {
    pub const FALLBACK: Self = Self {
        bg0: Color32::from_rgb(0x0A, 0x0A, 0x0B),
        bg1: Color32::from_rgb(0x12, 0x13, 0x15),
        bg2: Color32::from_rgb(0x1B, 0x1D, 0x20),
        border: Color32::from_rgb(0x3A, 0x3D, 0x42),
        border_hi: Color32::from_rgb(0x8A, 0x8F, 0x98),
        text: Color32::from_rgb(0xEC, 0xEC, 0xEC),
        text_dim: Color32::from_rgb(0x9A, 0xA0, 0xA6),
        accent: Color32::from_rgb(0x00, 0xD1, 0xB2),
        accent2: Color32::from_rgb(0x3D, 0xDC, 0x84),
        ok: Color32::from_rgb(0x3D, 0xDC, 0x84),
        warn: Color32::from_rgb(0xFF, 0xB3, 0x00),
        err: Color32::from_rgb(0xFF, 0x52, 0x52),
        is_dark: true,
        scanlines: false,
    };

    #[cfg(test)]
    pub fn paper_for_tests() -> Self {
        registry::ThemeRegistry::builtin_only().resolve_theme("paper")
    }

    pub fn with_adaptive_accent(mut self, swatch: Option<[u8; 3]>) -> Self {
        if let Some([r, g, b]) = swatch {
            let candidate = Color32::from_rgb(r, g, b);
            self.accent = clamp_accent_contrast(candidate, self.bg1, 4.5);
        }
        self
    }

    pub fn animate_through(&self, motion: &mut Motion) -> Self {
        let cfg = SpringCfg::SMOOTH;
        Self {
            bg0: motion.color("th_bg0", self.bg0, cfg),
            bg1: motion.color("th_bg1", self.bg1, cfg),
            bg2: motion.color("th_bg2", self.bg2, cfg),
            border: motion.color("th_border", self.border, cfg),
            border_hi: motion.color("th_border_hi", self.border_hi, cfg),
            text: motion.color("th_text", self.text, cfg),
            text_dim: motion.color("th_text_dim", self.text_dim, cfg),
            accent: motion.color("th_accent", self.accent, cfg),
            accent2: motion.color("th_accent2", self.accent2, cfg),
            ok: motion.color("th_ok", self.ok, cfg),
            warn: motion.color("th_warn", self.warn, cfg),
            err: motion.color("th_err", self.err, cfg),
            is_dark: self.is_dark,
            scanlines: self.scanlines,
        }
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    for (key, bytes) in bcvault_core::text::discover_fallback_fonts() {
        fonts
            .font_data
            .insert(key.to_string(), Arc::new(FontData::from_owned(bytes)));
        if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
            prop.push(key.to_string());
        }
        if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
            mono.push(key.to_string());
        }
    }

    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &egui::Context, theme: &FxTheme) {
    let mut s = Style {
        visuals: if theme.is_dark {
            Visuals::dark()
        } else {
            Visuals::light()
        },
        ..Default::default()
    };

    s.visuals.window_corner_radius = CornerRadius::ZERO;
    s.visuals.menu_corner_radius = CornerRadius::ZERO;
    s.visuals.panel_fill = theme.bg1;
    s.visuals.window_fill = theme.bg1;
    s.visuals.extreme_bg_color = theme.bg0;
    s.visuals.faint_bg_color = theme.bg2;
    s.visuals.code_bg_color = theme.bg2;
    s.visuals.window_shadow = Shadow::NONE;
    s.visuals.popup_shadow = Shadow::NONE;
    s.visuals.window_stroke = Stroke::new(1.0_f32, theme.border);
    s.visuals.selection.bg_fill = theme.accent.gamma_multiply(0.22);
    s.visuals.selection.stroke = Stroke::new(1.0_f32, theme.accent);
    s.visuals.override_text_color = Some(theme.text);

    for w in [
        &mut s.visuals.widgets.noninteractive,
        &mut s.visuals.widgets.inactive,
        &mut s.visuals.widgets.hovered,
        &mut s.visuals.widgets.active,
        &mut s.visuals.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
    }

    s.visuals.widgets.noninteractive.bg_fill = theme.bg1;
    s.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, theme.border);
    s.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, theme.text);

    s.visuals.widgets.inactive.bg_fill = theme.bg2;
    s.visuals.widgets.inactive.weak_bg_fill = theme.bg2;
    s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, theme.border);
    s.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, theme.text);

    s.visuals.widgets.hovered.bg_fill = theme.bg2;
    s.visuals.widgets.hovered.weak_bg_fill = theme.bg2;
    s.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, theme.border_hi);
    s.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, theme.text);

    s.visuals.widgets.active.bg_fill = theme.accent.gamma_multiply(0.25);
    s.visuals.widgets.active.weak_bg_fill = theme.accent.gamma_multiply(0.25);
    s.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, theme.accent);
    s.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, theme.text);

    s.visuals.widgets.open.bg_fill = theme.bg2;
    s.visuals.widgets.open.weak_bg_fill = theme.bg2;
    s.visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, theme.accent);
    s.visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, theme.text);

    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(10.0, 5.0);
    s.spacing.interact_size = egui::vec2(40.0, 25.0);
    s.spacing.window_margin = Margin::same(10);
    s.spacing.menu_margin = Margin::same(6);
    s.spacing.scroll.bar_width = 8.0;
    s.spacing.scroll.floating = false;

    s.text_styles.insert(
        TextStyle::Heading,
        FontId::new(16.0, FontFamily::Proportional),
    );
    s.text_styles
        .insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
    s.text_styles.insert(
        TextStyle::Button,
        FontId::new(13.0, FontFamily::Proportional),
    );
    s.text_styles.insert(
        TextStyle::Small,
        FontId::new(11.0, FontFamily::Proportional),
    );
    s.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(12.0, FontFamily::Monospace),
    );

    ctx.set_style(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::oklab::contrast_ratio;

    #[test]
    fn fx_theme_is_zero_radius() {
        let ctx = egui::Context::default();
        let reg = registry::ThemeRegistry::builtin_only();
        for entry in reg.entries() {
            apply_theme(&ctx, &entry.theme);
            let s = ctx.style();
            assert_eq!(s.visuals.window_corner_radius, CornerRadius::ZERO);
            assert_eq!(s.visuals.menu_corner_radius, CornerRadius::ZERO);
            let w = &s.visuals.widgets;
            assert_eq!(w.noninteractive.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.inactive.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.hovered.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.active.corner_radius, CornerRadius::ZERO);
            assert_eq!(w.open.corner_radius, CornerRadius::ZERO);
        }
    }

    #[test]
    fn every_theme_meets_contrast_ratio() {
        let reg = registry::ThemeRegistry::builtin_only();
        for entry in reg.entries() {
            let th = entry.theme;
            let id = &entry.id;
            for (bg_name, bg) in [("bg0", th.bg0), ("bg1", th.bg1), ("bg2", th.bg2)] {
                let ratio = contrast_ratio(th.text, bg);
                assert!(
                    ratio >= 4.5,
                    "theme {id} text on {bg_name} contrast ratio {ratio:.2} < 4.5"
                );
            }
            let acc_ratio = contrast_ratio(th.accent, th.bg1);
            assert!(
                acc_ratio >= 4.5,
                "theme {id} accent on bg1 contrast ratio {acc_ratio:.2} < 4.5"
            );
        }
    }
}
