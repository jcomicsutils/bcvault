use crate::{
    types::{Classification, FolderState, SortDir, Ts, UpdateReason},
    ui::theme::pal,
};
use egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};

pub fn sharp_rect(painter: &Painter, rect: Rect, fill: Color32, stroke: Stroke) {
    painter.rect(rect, CornerRadius::ZERO, fill, stroke, StrokeKind::Inside);
}

pub fn sharp_button(ui: &mut Ui, label: &str) -> Response {
    ui.add(egui::Button::new(label).corner_radius(CornerRadius::ZERO))
}

pub fn sharp_small_button(ui: &mut Ui, label: &str) -> Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(11.5)).corner_radius(CornerRadius::ZERO),
    )
}

pub fn badge(ui: &mut Ui, text: &str, fg: Color32, border_color: Color32) {
    let font_id = egui::FontId::monospace(10.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, fg);
    let pad = Vec2::new(5.0, 2.0);
    let desired = galley.size() + pad * 2.0;
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    if ui.is_rect_visible(rect) {
        sharp_rect(
            ui.painter(),
            rect,
            border_color.gamma_multiply(0.15),
            Stroke::new(1.0_f32, border_color),
        );
        ui.painter().galley(rect.min + pad, galley, fg);
    }
}

pub fn class_badge(ui: &mut Ui, class: Classification) {
    let (fg, border) = match class {
        Classification::Free => (pal::OK, pal::OK),
        Classification::Nyp => (pal::ACCENT, pal::ACCENT),
        Classification::EmailRequired => (pal::WARN, pal::WARN),
        Classification::Paid => (pal::TEXT_DIM, pal::BORDER),
        Classification::Unknown => (pal::TEXT_DIM, pal::BORDER),
    };
    badge(ui, class.label(), fg, border);
}

pub fn update_badge(ui: &mut Ui, reason: UpdateReason) {
    let (fg, border) = match reason {
        UpdateReason::TracksAdded => (pal::WARN, pal::WARN),
        UpdateReason::Modified => (pal::TEXT_DIM, pal::BORDER_HI),
    };
    badge(ui, reason.badge(), fg, border);
}

/// Painter-drawn status glyph (squares, crosses, triangles - never relies on font coverage).
pub fn paint_status_glyph(
    ui: &mut Ui,
    downloaded: bool,
    has_remote_update: bool,
    in_queue: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return resp;
    }

    let c = rect.center();
    let painter = ui.painter();

    if in_queue {
        // Active/queued diamond
        let pts = vec![
            Pos2::new(c.x, c.y - 5.0),
            Pos2::new(c.x + 5.0, c.y),
            Pos2::new(c.x, c.y + 5.0),
            Pos2::new(c.x - 5.0, c.y),
        ];
        painter.add(egui::Shape::convex_polygon(
            pts,
            pal::ACCENT,
            Stroke::new(1.0_f32, pal::ACCENT),
        ));
    } else if downloaded && has_remote_update {
        // Warning square with inner dot
        let box_rect = Rect::from_center_size(c, Vec2::splat(10.0));
        sharp_rect(
            painter,
            box_rect,
            pal::WARN.gamma_multiply(0.25),
            Stroke::new(1.5_f32, pal::WARN),
        );
        let inner = Rect::from_center_size(c, Vec2::splat(4.0));
        sharp_rect(painter, inner, pal::WARN, Stroke::NONE);
    } else if downloaded {
        // Solid green square
        let box_rect = Rect::from_center_size(c, Vec2::splat(9.0));
        sharp_rect(painter, box_rect, pal::OK, Stroke::new(1.0_f32, pal::OK));
    } else {
        // Empty dim square outline
        let box_rect = Rect::from_center_size(c, Vec2::splat(9.0));
        sharp_rect(
            painter,
            box_rect,
            Color32::TRANSPARENT,
            Stroke::new(1.0_f32, pal::BORDER),
        );
    }

    resp
}

/// Square status dot for folder links (green for Ok, red for Missing, yellow for Unreadable).
pub fn paint_folder_status_dot(ui: &mut Ui, state: FolderState) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let color = match state {
            FolderState::Ok => pal::OK,
            FolderState::Missing => pal::ERR,
            FolderState::Unreadable => pal::WARN,
        };
        let dot_rect = Rect::from_center_size(rect.center(), Vec2::splat(6.0));
        sharp_rect(ui.painter(), dot_rect, color, Stroke::NONE);
    }
    resp
}

/// Painter-drawn folder glyph with status dot for linked artists in the sidebar.
pub fn paint_folder_glyph(ui: &mut Ui, state: FolderState) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(16.0, 14.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let min = rect.min;
        // Tab: top-left
        let tab = Rect::from_min_max(
            Pos2::new(min.x, min.y + 1.0),
            Pos2::new(min.x + 5.0, min.y + 3.0),
        );
        sharp_rect(painter, tab, pal::TEXT_DIM, Stroke::NONE);
        // Folder body: rect
        let body = Rect::from_min_max(
            Pos2::new(min.x, min.y + 3.0),
            Pos2::new(min.x + 13.0, min.y + 12.0),
        );
        sharp_rect(painter, body, pal::BG2, Stroke::new(1.0_f32, pal::TEXT_DIM));
        // Status dot on right/inside
        let dot_color = match state {
            FolderState::Ok => pal::OK,
            FolderState::Missing => pal::ERR,
            FolderState::Unreadable => pal::WARN,
        };
        let dot_rect = Rect::from_min_max(
            Pos2::new(min.x + 8.0, min.y + 7.0),
            Pos2::new(min.x + 12.0, min.y + 11.0),
        );
        sharp_rect(painter, dot_rect, dot_color, Stroke::NONE);
    }
    resp
}

/// Painter-drawn sort direction triangle.
pub fn paint_sort_arrow(painter: &Painter, center: Pos2, dir: SortDir, color: Color32) {
    let pts = match dir {
        SortDir::Asc => vec![
            Pos2::new(center.x, center.y - 3.5),
            Pos2::new(center.x + 4.0, center.y + 3.5),
            Pos2::new(center.x - 4.0, center.y + 3.5),
        ],
        SortDir::Desc => vec![
            Pos2::new(center.x - 4.0, center.y - 3.5),
            Pos2::new(center.x + 4.0, center.y - 3.5),
            Pos2::new(center.x, center.y + 3.5),
        ],
    };
    painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_speed(bps: f64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const KB: f64 = 1024.0;
    if bps >= MB {
        format!("{:.2} MB/s", bps / MB)
    } else if bps >= KB {
        format!("{:.1} KB/s", bps / KB)
    } else {
        format!("{:.0} B/s", bps)
    }
}

pub fn format_date(ts: Option<Ts>) -> String {
    let Some(secs) = ts else {
        return "-".to_string();
    };
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => "-".to_string(),
    }
}
