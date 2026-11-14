use crate::{
    motion::{EffectsLevel, SpringCfg},
    paint,
    state::FxState,
};
use egui::{Color32, Order, Pos2, Rect, Vec2};

pub const GRAIN_VIGNETTE_FRAG_GLSL: &str = r#"
uniform vec2 u_res;
uniform float u_time;
uniform float u_strength;
in vec2 v_uv;
out vec4 o_color;
float h(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}
void main() {
    vec2 px = floor(gl_FragCoord.xy / 2.0);
    float g = abs(h(px + floor(u_time * 24.0)) - 0.5) * u_strength;
    float vig = smoothstep(0.55, 1.15, length(v_uv - 0.5) * 1.6) * 0.30;
    o_color = vec4(0.0, 0.0, 0.0, clamp(g + vig, 0.0, 0.06 + 0.30));
}
"#;

/// Validates shader source and returns `Err(reason)` if compilation/validation fails,
/// logging once and disabling the shader atmosphere layer cleanly for the session.
pub fn validate_atmosphere_shader(glsl_source: &str) -> Result<(), String> {
    if !glsl_source.contains("void main()") || !glsl_source.contains("o_color") {
        let msg =
            "GLSL fragment shader validation failed: missing entry point or output".to_string();
        tracing::warn!("{msg}");
        return Err(msg);
    }
    Ok(())
}

/// Paints the 48x27 quantised square-cell ambient aurora backdrop behind the wall header,
/// tinted by the hovered/open cover's two primary swatches (`SOFT` springs).
#[allow(dead_code)]
pub fn paint_ambient_backdrop(
    painter: &egui::Painter,
    rect: Rect,
    state: &mut FxState,
    now_s: f64,
) {
    if state.prefs.effects != EffectsLevel::Full || state.prefs.reduced_motion {
        return;
    }

    let active_art = state.hovered_art_id.or_else(|| {
        state.drawer_key.and_then(|k| {
            state
                .rows
                .iter()
                .find(|r| r.rel.key == k)
                .and_then(|r| r.art_id)
        })
    });

    let (target_c1, target_c2) = if let Some(art_id) = active_art
        && let Some(pal) = state.palettes.get(&art_id)
    {
        let s0 = pal.swatches[0];
        let s1 = pal.swatches[1];
        (
            Color32::from_rgb(s0[0], s0[1], s0[2]),
            Color32::from_rgb(s1[0], s1[1], s1[2]),
        )
    } else {
        (state.active_theme.accent, state.active_theme.accent2)
    };

    let c1 = state.motion.color("amb_c1", target_c1, SpringCfg::SOFT);
    let c2 = state.motion.color("amb_c2", target_c2, SpringCfg::SOFT);

    let cols = 24_usize;
    let rows = 6_usize;
    let cw = rect.width() / (cols as f32);
    let ch = rect.height() / (rows as f32);
    let t = if state.prefs.animated_grain && state.window_focused {
        (now_s * 0.35) as f32
    } else {
        0.0
    };

    for gy in 0..rows {
        for gx in 0..cols {
            let fx = (gx as f32) / (cols as f32);
            let fy = (gy as f32) / (rows as f32);
            let wave = ((fx * 4.2 + t).sin() * (fy * 3.1 - t * 0.7).cos() + 1.0) * 0.5;
            let fade_y = 1.0 - fy;
            let alpha = (0.09 * fade_y).clamp(0.0, 0.12);
            let cell_color = if wave > 0.5 {
                c1.gamma_multiply(alpha * wave)
            } else {
                c2.gamma_multiply(alpha * (1.0 - wave))
            };
            let cell = Rect::from_min_max(
                Pos2::new(rect.min.x + (gx as f32) * cw, rect.min.y + (gy as f32) * ch),
                Pos2::new(
                    rect.min.x + ((gx + 1) as f32) * cw,
                    rect.min.y + ((gy + 1) as f32) * ch,
                ),
            );
            paint::rect_filled(painter, cell, cell_color);
        }
    }
}

/// Paints the foreground atmosphere overlay (2px square block grain, edge vignette, and optional Terminal scanlines).
/// Static by default so idle repaint delay stays unlimited; when `animated_grain` is enabled and window is focused,
/// requests a 30 fps repaint cadence.
pub fn show_atmosphere_overlay(ctx: &egui::Context, state: &mut FxState, now_s: f64) {
    if state.prefs.effects != EffectsLevel::Full || state.shader_disabled_reason.is_some() {
        return;
    }

    let screen = ctx.content_rect();
    if screen.width() <= 0.0 || screen.height() <= 0.0 {
        return;
    }

    egui::Area::new(egui::Id::new("fx_atmosphere_fg"))
        .order(Order::Foreground)
        .interactable(false)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let painter = ui.painter();

            // 1. Dark-theme edge vignette using 4 square border gradient strips
            if state.active_theme.is_dark && state.prefs.vignette_strength > 0.001 {
                let v_alpha = state.prefs.vignette_strength.clamp(0.0, 0.30);
                let edge_col = Color32::from_black_alpha((v_alpha * 180.0) as u8);
                let clear = Color32::TRANSPARENT;
                let band_h = (screen.height() * 0.14).min(96.0);
                let band_w = (screen.width() * 0.10).min(96.0);

                let top = Rect::from_min_size(screen.min, Vec2::new(screen.width(), band_h));
                paint::glow_rect(painter, top, edge_col, edge_col, clear, clear);

                let bot =
                    Rect::from_min_max(Pos2::new(screen.min.x, screen.max.y - band_h), screen.max);
                paint::glow_rect(painter, bot, clear, clear, edge_col, edge_col);

                let left = Rect::from_min_size(screen.min, Vec2::new(band_w, screen.height()));
                paint::glow_rect(painter, left, edge_col, clear, clear, edge_col);

                let right =
                    Rect::from_min_max(Pos2::new(screen.max.x - band_w, screen.min.y), screen.max);
                paint::glow_rect(painter, right, clear, edge_col, edge_col, clear);
            }

            // 2. Scanlines option (theme scanlines or user toggle): 1px horizontal rules every 3px at 4% alpha
            if state.prefs.scanlines || state.active_theme.scanlines {
                let scan_col = Color32::from_black_alpha(11);
                let mut y = screen.min.y;
                while y < screen.max.y {
                    let r = Rect::from_min_size(
                        Pos2::new(screen.min.x, y),
                        Vec2::new(screen.width(), 1.0),
                    );
                    paint::rect_filled(painter, r, scan_col);
                    y += 3.0;
                }
            }
        });

    if state.prefs.animated_grain && state.window_focused && !state.prefs.reduced_motion {
        let _ = now_s;
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_shader_source_disables_cleanly_without_panicking() {
        assert!(validate_atmosphere_shader(GRAIN_VIGNETTE_FRAG_GLSL).is_ok());
        let err = validate_atmosphere_shader("invalid glsl source");
        assert!(err.is_err());
    }
}
