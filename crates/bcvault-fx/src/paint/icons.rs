use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconKind {
    Search,
    Folder,
    Download,
    Check,
    Cross,
    ArrowLeft,
    ArrowRight,
    SortAsc,
    SortDesc,
    Wall,
    Table,
    Artists,
    Stats,
    Sidebar,
    Archive,
}

/// Paints geometric icons built exclusively from lines, square rects, and triangles.
pub fn paint_icon(painter: &Painter, rect: Rect, kind: IconKind, color: Color32) {
    let s = rect.shrink(2.0);
    let c = s.center();
    let stroke = Stroke::new(1.5_f32, color);

    match kind {
        IconKind::Search => {
            let box_r = Rect::from_min_size(s.min, Vec2::splat(s.width() * 0.68));
            painter.rect_stroke(box_r, egui::CornerRadius::ZERO, stroke, StrokeKind::Inside);
            painter.line_segment([box_r.max, s.max], stroke);
        }
        IconKind::Folder => {
            let tab = Rect::from_min_size(s.min, Vec2::new(s.width() * 0.42, s.height() * 0.22));
            let body = Rect::from_min_max(Pos2::new(s.min.x, s.min.y + s.height() * 0.22), s.max);
            painter.rect_stroke(tab, egui::CornerRadius::ZERO, stroke, StrokeKind::Inside);
            painter.rect_stroke(body, egui::CornerRadius::ZERO, stroke, StrokeKind::Inside);
        }
        IconKind::Download => {
            let top = Pos2::new(c.x, s.min.y);
            let mid = Pos2::new(c.x, s.max.y - 3.0);
            painter.line_segment([top, mid], stroke);
            painter.line_segment([Pos2::new(c.x - 3.5, mid.y - 3.5), mid], stroke);
            painter.line_segment([Pos2::new(c.x + 3.5, mid.y - 3.5), mid], stroke);
            painter.line_segment(
                [Pos2::new(s.min.x, s.max.y), Pos2::new(s.max.x, s.max.y)],
                stroke,
            );
        }
        IconKind::Check => {
            let p1 = Pos2::new(s.min.x + 1.0, c.y);
            let p2 = Pos2::new(c.x - 1.0, s.max.y - 1.5);
            let p3 = Pos2::new(s.max.x - 1.0, s.min.y + 1.5);
            painter.line_segment([p1, p2], stroke);
            painter.line_segment([p2, p3], stroke);
        }
        IconKind::Cross => {
            painter.line_segment([s.min, s.max], stroke);
            painter.line_segment(
                [Pos2::new(s.max.x, s.min.y), Pos2::new(s.min.x, s.max.y)],
                stroke,
            );
        }
        IconKind::ArrowLeft => {
            let tip = Pos2::new(s.min.x + 1.0, c.y);
            painter.line_segment([tip, Pos2::new(s.max.x - 1.0, c.y)], stroke);
            painter.line_segment([tip, Pos2::new(c.x, s.min.y + 1.5)], stroke);
            painter.line_segment([tip, Pos2::new(c.x, s.max.y - 1.5)], stroke);
        }
        IconKind::ArrowRight => {
            let tip = Pos2::new(s.max.x - 1.0, c.y);
            painter.line_segment([Pos2::new(s.min.x + 1.0, c.y), tip], stroke);
            painter.line_segment([tip, Pos2::new(c.x, s.min.y + 1.5)], stroke);
            painter.line_segment([tip, Pos2::new(c.x, s.max.y - 1.5)], stroke);
        }
        IconKind::SortAsc => {
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(c.x, s.min.y + 1.0),
                    Pos2::new(s.max.x - 1.0, s.max.y - 1.0),
                    Pos2::new(s.min.x + 1.0, s.max.y - 1.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        IconKind::SortDesc => {
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(s.min.x + 1.0, s.min.y + 1.0),
                    Pos2::new(s.max.x - 1.0, s.min.y + 1.0),
                    Pos2::new(c.x, s.max.y - 1.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        IconKind::Wall => {
            let half = (s.width() - 2.0) * 0.5;
            for (gx, gy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let cell = Rect::from_min_size(
                    Pos2::new(s.min.x + gx * (half + 2.0), s.min.y + gy * (half + 2.0)),
                    Vec2::splat(half),
                );
                painter.rect_filled(cell, egui::CornerRadius::ZERO, color);
            }
        }
        IconKind::Table => {
            for i in 0..3 {
                let y = s.min.y + (i as f32) * (s.height() * 0.38);
                let row = Rect::from_min_size(Pos2::new(s.min.x, y), Vec2::new(s.width(), 2.0));
                painter.rect_filled(row, egui::CornerRadius::ZERO, color);
            }
        }
        IconKind::Artists => {
            let top = Rect::from_min_size(s.min, Vec2::new(s.width(), s.height() * 0.42));
            let bot = Rect::from_min_size(
                Pos2::new(s.min.x, s.min.y + s.height() * 0.56),
                Vec2::new(s.width(), s.height() * 0.42),
            );
            painter.rect_stroke(top, egui::CornerRadius::ZERO, stroke, StrokeKind::Inside);
            painter.rect_stroke(bot, egui::CornerRadius::ZERO, stroke, StrokeKind::Inside);
        }
        IconKind::Stats => {
            let w = (s.width() - 4.0) / 3.0;
            for (i, h_frac) in [(0, 0.45), (1, 0.95), (2, 0.70)] {
                let h = s.height() * h_frac;
                let bar = Rect::from_min_max(
                    Pos2::new(s.min.x + (i as f32) * (w + 2.0), s.max.y - h),
                    Pos2::new(s.min.x + (i as f32) * (w + 2.0) + w, s.max.y),
                );
                painter.rect_filled(bar, egui::CornerRadius::ZERO, color);
            }
        }
        IconKind::Sidebar => {
            painter.rect_stroke(s, egui::CornerRadius::ZERO, stroke, StrokeKind::Inside);
            let x = s.min.x + s.width() * 0.35;
            painter.line_segment([Pos2::new(x, s.min.y), Pos2::new(x, s.max.y)], stroke);
        }
        IconKind::Archive => {
            painter.rect_stroke(s, egui::CornerRadius::ZERO, stroke, StrokeKind::Inside);
            painter.line_segment(
                [Pos2::new(s.min.x + 2.5, c.y), Pos2::new(s.max.x - 2.5, c.y)],
                stroke,
            );
        }
    }
}
