pub mod icons;

use egui::{
    Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2,
    epaint::Vertex,
};

pub fn rect(painter: &Painter, r: Rect, fill: Color32, stroke: Stroke) {
    painter.rect(r, CornerRadius::ZERO, fill, stroke, StrokeKind::Inside);
}

pub fn rect_filled(painter: &Painter, r: Rect, fill: Color32) {
    painter.rect_filled(r, CornerRadius::ZERO, fill);
}

pub fn rect_outline(painter: &Painter, r: Rect, stroke: Stroke) {
    painter.rect_stroke(r, CornerRadius::ZERO, stroke, StrokeKind::Inside);
}

/// Hard-edged offset rectangle shadow (the only shadow permitted in bcvault-fx).
pub fn hard_shadow(painter: &Painter, r: Rect, offset: Vec2, color: Color32) {
    let sr = r.translate(offset);
    painter.rect_filled(sr, CornerRadius::ZERO, color);
}

/// Vertex-coloured 4-corner gradient mesh with strictly square edges.
pub fn glow_rect(painter: &Painter, r: Rect, tl: Color32, tr: Color32, br: Color32, bl: Color32) {
    let mut mesh = Mesh::default();
    let idx = 0_u32;
    mesh.vertices.push(Vertex {
        pos: r.left_top(),
        uv: Pos2::ZERO,
        color: tl,
    });
    mesh.vertices.push(Vertex {
        pos: r.right_top(),
        uv: Pos2::ZERO,
        color: tr,
    });
    mesh.vertices.push(Vertex {
        pos: r.right_bottom(),
        uv: Pos2::ZERO,
        color: br,
    });
    mesh.vertices.push(Vertex {
        pos: r.left_bottom(),
        uv: Pos2::ZERO,
        color: bl,
    });
    mesh.indices
        .extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
    painter.add(Shape::mesh(mesh));
}

/// 45-degree diagonal hatch stripes clipped to `r`.
pub fn hatch(painter: &Painter, r: Rect, phase: f32, spacing: f32, stroke: Stroke) {
    let step = spacing.max(4.0);
    let clipped = painter.with_clip_rect(r);
    let h = r.height();
    let start_x = r.min.x - h + (phase.rem_euclid(1.0) * step);
    let mut x = start_x;
    while x < r.max.x + h {
        clipped.line_segment([Pos2::new(x, r.max.y), Pos2::new(x + h, r.min.y)], stroke);
        x += step;
    }
}

/// Rotated square diamond shape.
pub fn diamond(painter: &Painter, center: Pos2, half: f32, fill: Color32) {
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(center.x, center.y - half),
            Pos2::new(center.x + half, center.y),
            Pos2::new(center.x, center.y + half),
            Pos2::new(center.x - half, center.y),
        ],
        fill,
        Stroke::NONE,
    ));
}

/// 3x3 square chase loader indicator.
pub fn busy_squares(painter: &Painter, r: Rect, phase: f32, color: Color32) {
    let gap = 2.0_f32;
    let side = ((r.width().min(r.height()) - gap * 2.0) / 3.0).max(2.0);
    let order = [
        (0, 0),
        (1, 0),
        (2, 0),
        (2, 1),
        (2, 2),
        (1, 2),
        (0, 2),
        (0, 1),
    ];
    let active_idx = ((phase.rem_euclid(1.0) * 8.0) as usize) % 8;
    for (step_i, &(gx, gy)) in order.iter().enumerate() {
        let dist = (8 + active_idx - step_i) % 8;
        let alpha = match dist {
            0 => 1.0,
            1 => 0.65,
            2 => 0.35,
            _ => 0.14,
        };
        let cell = Rect::from_min_size(
            Pos2::new(
                r.min.x + (gx as f32) * (side + gap),
                r.min.y + (gy as f32) * (side + gap),
            ),
            Vec2::splat(side),
        );
        painter.rect_filled(cell, CornerRadius::ZERO, color.gamma_multiply(alpha));
    }
}

/// Draws a stroke-by-stroke animated check mark (`progress` in 0..=1).
pub fn drawn_check(painter: &Painter, r: Rect, progress: f32, stroke: Stroke) {
    let p = progress.clamp(0.0, 1.0);
    if p <= 0.0 {
        return;
    }
    let s = r.shrink(2.0);
    let p1 = Pos2::new(s.min.x, s.center().y);
    let p2 = Pos2::new(s.min.x + s.width() * 0.38, s.max.y);
    let p3 = Pos2::new(s.max.x, s.min.y);

    let seg1_t = (p / 0.4).clamp(0.0, 1.0);
    let mid1 = p1.lerp(p2, seg1_t);
    painter.line_segment([p1, mid1], stroke);

    if p > 0.4 {
        let seg2_t = ((p - 0.4) / 0.6).clamp(0.0, 1.0);
        let mid2 = p2.lerp(p3, seg2_t);
        painter.line_segment([p2, mid2], stroke);
    }
}

/// Paints an 8x8 RGB (`192` bytes) tiny mosaic as 64 solid square quads.
pub fn tiny8_mosaic(painter: &Painter, r: Rect, tiny8: &[u8; 192], alpha: f32) {
    let cw = r.width() / 8.0;
    let ch = r.height() / 8.0;
    for gy in 0..8 {
        for gx in 0..8 {
            let idx = (gy * 8 + gx) * 3;
            let c =
                Color32::from_rgb(tiny8[idx], tiny8[idx + 1], tiny8[idx + 2]).gamma_multiply(alpha);
            let cell = Rect::from_min_max(
                Pos2::new(r.min.x + (gx as f32) * cw, r.min.y + (gy as f32) * ch),
                Pos2::new(
                    r.min.x + ((gx + 1) as f32) * cw,
                    r.min.y + ((gy + 1) as f32) * ch,
                ),
            );
            painter.rect_filled(cell, CornerRadius::ZERO, c);
        }
    }
}

/// Deterministic square geometric tile placeholder when cover art or `tiny8` is not yet cached.
pub fn geometric_placeholder(
    painter: &Painter,
    r: Rect,
    seed: i64,
    bg: Color32,
    fg1: Color32,
    fg2: Color32,
) {
    painter.rect_filled(r, CornerRadius::ZERO, bg);
    let mut h = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let cw = r.width() / 4.0;
    let ch = r.height() / 4.0;
    for gy in 0..4 {
        for gx in 0..2 {
            h = h.rotate_left(7) ^ 0xBF58_476D_1CE4_E5B9;
            if (h & 3) != 0 {
                let color = if (h & 4) == 0 {
                    fg1.gamma_multiply(0.22)
                } else {
                    fg2.gamma_multiply(0.18)
                };
                for mx in [gx, 3 - gx] {
                    let cell = Rect::from_min_size(
                        Pos2::new(r.min.x + (mx as f32) * cw, r.min.y + (gy as f32) * ch),
                        Vec2::new(cw, ch),
                    );
                    painter.rect_filled(cell, CornerRadius::ZERO, color);
                }
            }
        }
    }
}

pub fn sharp_button(ui: &mut Ui, label: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Button::new(label).corner_radius(CornerRadius::ZERO))
}

pub fn sharp_small_button(ui: &mut Ui, label: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(
        egui::Button::new(label)
            .small()
            .corner_radius(CornerRadius::ZERO),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::icons::{IconKind, paint_icon};

    #[test]
    fn no_round_primitives_in_any_paint_helper() {
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput::default();
        let full_output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let painter = ui.painter();
                let r = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(64.0, 64.0));
                rect(
                    painter,
                    r,
                    Color32::BLACK,
                    Stroke::new(1.0_f32, Color32::WHITE),
                );
                rect_outline(painter, r, Stroke::new(1.0_f32, Color32::WHITE));
                hard_shadow(painter, r, Vec2::new(4.0, 4.0), Color32::BLACK);
                glow_rect(
                    painter,
                    r,
                    Color32::RED,
                    Color32::GREEN,
                    Color32::BLUE,
                    Color32::WHITE,
                );
                hatch(painter, r, 0.25, 8.0, Stroke::new(1.0_f32, Color32::WHITE));
                diamond(painter, r.center(), 6.0, Color32::WHITE);
                busy_squares(painter, r, 0.5, Color32::WHITE);
                drawn_check(painter, r, 1.0, Stroke::new(1.5_f32, Color32::WHITE));
                tiny8_mosaic(painter, r, &[128; 192], 1.0);
                geometric_placeholder(painter, r, 42, Color32::BLACK, Color32::RED, Color32::BLUE);
                for kind in [
                    IconKind::Search,
                    IconKind::Folder,
                    IconKind::Download,
                    IconKind::Check,
                    IconKind::Cross,
                    IconKind::ArrowLeft,
                    IconKind::ArrowRight,
                    IconKind::SortAsc,
                    IconKind::SortDesc,
                    IconKind::Wall,
                    IconKind::Table,
                    IconKind::Artists,
                    IconKind::Stats,
                    IconKind::Sidebar,
                ] {
                    paint_icon(painter, r, kind, Color32::WHITE);
                }
            });
        });

        fn inspect_shape(shape: &Shape) {
            match shape {
                Shape::Circle(_) | Shape::Ellipse(_) => {
                    panic!("Round primitive found in paint output: {shape:?}");
                }
                Shape::Rect(rs) => {
                    assert_eq!(rs.corner_radius, CornerRadius::ZERO);
                }
                Shape::Vec(v) => {
                    for s in v {
                        inspect_shape(s);
                    }
                }
                _ => {}
            }
        }

        for clipped in &full_output.shapes {
            inspect_shape(&clipped.shape);
        }
    }
}
