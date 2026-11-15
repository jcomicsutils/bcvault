use egui::{
    CornerRadius, Stroke, Style, ThemePreference,
    epaint::Shadow,
    style::{HandleShape, ScrollStyle},
};

pub mod pal {
    use egui::Color32;
    pub const BG0: Color32 = Color32::from_rgb(0x0A, 0x0A, 0x0B); // central panel, text inputs
    pub const BG1: Color32 = Color32::from_rgb(0x12, 0x13, 0x15); // side/top/bottom panels, stripes
    pub const BG2: Color32 = Color32::from_rgb(0x1B, 0x1D, 0x20); // hover, selected row
    pub const BORDER: Color32 = Color32::from_rgb(0x3A, 0x3D, 0x42);
    pub const BORDER_HI: Color32 = Color32::from_rgb(0x8A, 0x8F, 0x98);
    pub const TEXT: Color32 = Color32::from_rgb(0xEC, 0xEC, 0xEC);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9A, 0xA0, 0xA6);
    pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0xD1, 0xB2);
    pub const OK: Color32 = Color32::from_rgb(0x3D, 0xDC, 0x84);
    pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xB0, 0x20);
    pub const ERR: Color32 = Color32::from_rgb(0xFF, 0x4D, 0x4F);
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(ThemePreference::Dark);
    ctx.all_styles_mut(sharpen); // both themes, so a stray theme switch cannot reintroduce rounding
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (key, bytes) in bcvault_core::text::discover_fallback_fonts() {
        fonts.font_data.insert(
            key.to_string(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            prop.push(key.to_string());
        }
        if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            mono.push(key.to_string());
        }
    }
    ctx.set_fonts(fonts);
}

fn sharpen(style: &mut Style) {
    let v = &mut style.visuals;
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO; // combo popups, context menus, tooltips
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_stroke = Stroke::new(1.0_f32, pal::BORDER_HI);
    v.window_fill = pal::BG1;
    v.panel_fill = pal::BG0;
    v.extreme_bg_color = pal::BG0;
    v.faint_bg_color = pal::BG1;
    v.override_text_color = Some(pal::TEXT);
    v.handle_shape = HandleShape::Rect { aspect_ratio: 0.5 };
    v.selection.bg_fill = pal::ACCENT.gamma_multiply(0.25);
    v.selection.stroke = Stroke::new(1.0_f32, pal::ACCENT);
    let w = &mut v.widgets;
    for ws in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        ws.corner_radius = CornerRadius::ZERO;
        ws.expansion = 0.0; // no grow-on-hover
        ws.bg_stroke = Stroke::new(1.0_f32, pal::BORDER);
    }
    w.hovered.bg_stroke = Stroke::new(1.0_f32, pal::BORDER_HI);
    w.active.bg_stroke = Stroke::new(1.0_f32, pal::ACCENT);
    style.spacing.scroll = ScrollStyle::solid(); // solid bars; handle radius follows widget visuals
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Theme;

    #[test]
    fn theme_is_zero_radius() {
        let ctx = egui::Context::default();
        apply(&ctx);
        for theme in [Theme::Dark, Theme::Light] {
            let style = ctx.style_of(theme);
            let v = &style.visuals;
            assert_eq!(v.window_corner_radius, CornerRadius::ZERO);
            assert_eq!(v.menu_corner_radius, CornerRadius::ZERO);
            assert_eq!(v.widgets.noninteractive.corner_radius, CornerRadius::ZERO);
            assert_eq!(v.widgets.inactive.corner_radius, CornerRadius::ZERO);
            assert_eq!(v.widgets.hovered.corner_radius, CornerRadius::ZERO);
            assert_eq!(v.widgets.active.corner_radius, CornerRadius::ZERO);
            assert_eq!(v.widgets.open.corner_radius, CornerRadius::ZERO);
        }
    }
}
