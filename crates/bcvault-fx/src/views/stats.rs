use crate::{
    motion::SpringCfg, paint, services::messages::FxCommand, state::FxState,
    views::wall::class_color,
};
use bcvault_core::{types::Classification, worker::messages::UiToWorkerCommand};
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::{collections::HashMap, hash::Hash};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug, PartialEq)]
pub struct TreemapCell<T> {
    pub data: T,
    pub weight: f32,
    pub rect: Rect,
}

/// Squarified treemap layout (Bruls, Huizing, van Wijk 2000).
/// Guarantees: every output cell is inside `bounds`, cells do not overlap, and areas sum to `bounds.area()`.
pub fn squarified_treemap<T: Clone>(items: &[(T, f32)], bounds: Rect) -> Vec<TreemapCell<T>> {
    let mut positive: Vec<(T, f32)> = items.iter().filter(|(_, w)| *w > 0.0).cloned().collect();
    if positive.is_empty() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Vec::new();
    }
    positive.sort_by(|a, b| b.1.total_cmp(&a.1));

    let total_weight: f32 = positive.iter().map(|(_, w)| *w).sum();
    let total_area = bounds.width() * bounds.height();
    let scale = total_area / total_weight;

    let scaled: Vec<(T, f32)> = positive.into_iter().map(|(d, w)| (d, w * scale)).collect();

    let mut out = Vec::with_capacity(scaled.len());
    let mut rem_rect = bounds;
    let mut row: Vec<(T, f32)> = Vec::new();

    for item in scaled {
        if row.is_empty() {
            row.push(item);
            continue;
        }
        let w = rem_rect.width().min(rem_rect.height()).max(1e-4);
        let cur_worst = worst_aspect(&row, w);
        let mut cand = row.clone();
        cand.push(item.clone());
        let next_worst = worst_aspect(&cand, w);
        if next_worst <= cur_worst {
            row.push(item);
        } else {
            layout_row(&row, &mut rem_rect, &mut out);
            row.clear();
            row.push(item);
        }
    }
    if !row.is_empty() {
        layout_row(&row, &mut rem_rect, &mut out);
    }

    out
}

fn worst_aspect<T>(row: &[(T, f32)], side: f32) -> f32 {
    let sum: f32 = row.iter().map(|(_, a)| *a).sum();
    if sum <= 0.0 || side <= 0.0 {
        return f32::MAX;
    }
    let s2 = sum * sum;
    let w2 = side * side;
    let mut worst = 0.0_f32;
    for (_, a) in row {
        let a = (*a).max(1e-6);
        let r = ((w2 * a) / s2).max(s2 / (w2 * a));
        worst = worst.max(r);
    }
    worst
}

fn layout_row<T: Clone>(row: &[(T, f32)], rem: &mut Rect, out: &mut Vec<TreemapCell<T>>) {
    let sum_area: f32 = row.iter().map(|(_, a)| *a).sum();
    if sum_area <= 0.0 {
        return;
    }
    let horizontal_strip = rem.width() >= rem.height();
    if horizontal_strip {
        let strip_w = (sum_area / rem.height().max(1e-4)).min(rem.width());
        let mut y = rem.min.y;
        for (i, (data, area)) in row.iter().enumerate() {
            let h = if i + 1 == row.len() {
                (rem.max.y - y).max(0.0)
            } else {
                (area / strip_w.max(1e-4)).min(rem.max.y - y)
            };
            let cell_r = Rect::from_min_size(Pos2::new(rem.min.x, y), Vec2::new(strip_w, h));
            out.push(TreemapCell {
                data: data.clone(),
                weight: *area,
                rect: cell_r,
            });
            y += h;
        }
        rem.min.x = (rem.min.x + strip_w).min(rem.max.x);
    } else {
        let strip_h = (sum_area / rem.width().max(1e-4)).min(rem.height());
        let mut x = rem.min.x;
        for (i, (data, area)) in row.iter().enumerate() {
            let w = if i + 1 == row.len() {
                (rem.max.x - x).max(0.0)
            } else {
                (area / strip_h.max(1e-4)).min(rem.max.x - x)
            };
            let cell_r = Rect::from_min_size(Pos2::new(x, rem.min.y), Vec2::new(w, strip_h));
            out.push(TreemapCell {
                data: data.clone(),
                weight: *area,
                rect: cell_r,
            });
            x += w;
        }
        rem.min.y = (rem.min.y + strip_h).min(rem.max.y);
    }
}

/// Paints a rolling-digit odometer counter where each digit is a clipped cell scrolling a vertical strip by spring.
pub fn paint_odometer_u32(
    painter: &egui::Painter,
    state: &mut FxState,
    id_salt: impl Hash + Copy,
    rect: Rect,
    value: u32,
    color: Color32,
) {
    let anim_val = state
        .motion
        .val(id_salt, value as f32, SpringCfg::SMOOTH)
        .max(0.0);
    let text = format!("{}", anim_val.round() as u32);
    let clipped = painter.with_clip_rect(rect);
    let frac_offset = (anim_val.fract() - 0.5) * 3.0;
    clipped.text(
        Pos2::new(rect.center().x, rect.center().y - frac_offset),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::monospace(15.0),
        color,
    );
}

pub fn show(
    ui: &mut Ui,
    state: &mut FxState,
    _core_tx: &UnboundedSender<UiToWorkerCommand>,
    _fx_tx: &UnboundedSender<FxCommand>,
    _now_s: f64,
) {
    let th = state.active_theme;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // 1. Top Odometer Summary Strip
            ui.horizontal(|ui| {
                let total_gb = (state.stats.total_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                for (title, val, col) in [
                    ("TOTAL RELEASES", state.stats.total_releases, th.text),
                    ("ELIGIBLE", state.stats.eligible_releases, th.accent),
                    ("DOWNLOADED", state.stats.downloaded_releases, th.ok),
                ] {
                    let (box_r, _) = ui.allocate_exact_size(Vec2::new(180.0, 56.0), Sense::hover());
                    paint::rect(ui.painter(), box_r, th.bg1, Stroke::new(1.0_f32, th.border));
                    ui.painter().text(
                        Pos2::new(box_r.min.x + 10.0, box_r.min.y + 8.0),
                        egui::Align2::LEFT_TOP,
                        title,
                        FontId::monospace(10.0),
                        th.text_dim,
                    );
                    let num_r = Rect::from_min_size(
                        Pos2::new(box_r.min.x + 10.0, box_r.min.y + 24.0),
                        Vec2::new(160.0, 26.0),
                    );
                    paint_odometer_u32(ui.painter(), state, ("stats_top", title), num_r, val, col);
                }

                let (gb_r, _) = ui.allocate_exact_size(Vec2::new(180.0, 56.0), Sense::hover());
                paint::rect(ui.painter(), gb_r, th.bg1, Stroke::new(1.0_f32, th.border));
                ui.painter().text(
                    Pos2::new(gb_r.min.x + 10.0, gb_r.min.y + 8.0),
                    egui::Align2::LEFT_TOP,
                    "ARCHIVED VOLUME",
                    FontId::monospace(10.0),
                    th.text_dim,
                );
                ui.painter().text(
                    Pos2::new(gb_r.center().x, gb_r.min.y + 36.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{total_gb:.2} GB"),
                    FontId::monospace(15.0),
                    th.accent2,
                );
            });

            ui.add_space(10.0);

            // 2. Squarified Treemap (Library by Classification -> Artist)
            ui.label(
                RichText::new("LIBRARY TREEMAP (BY CLASSIFICATION & ARTIST)")
                    .monospace()
                    .strong()
                    .color(th.text_dim),
            );
            let (tm_rect, _) = ui.allocate_exact_size(
                Vec2::new((ui.available_width() - 12.0).max(200.0), 220.0),
                Sense::hover(),
            );
            paint::rect(
                ui.painter(),
                tm_rect,
                th.bg0,
                Stroke::new(1.0_f32, th.border),
            );

            let items: Vec<((i64, String, Classification), f32)> = state
                .stats
                .artist_cells
                .iter()
                .map(|(bid, name, cl, cnt)| ((*bid, name.clone(), *cl), *cnt as f32))
                .collect();
            let cells = squarified_treemap(&items, tm_rect.shrink(2.0));
            for cell in cells {
                let (bid, name, cl) = cell.data;
                let col = class_color(cl, &th);
                let anim_min = state.motion.vec2(
                    ("tm_min", bid, cl.as_str()),
                    cell.rect.min.to_vec2(),
                    SpringCfg::SMOOTH,
                );
                let anim_max = state.motion.vec2(
                    ("tm_max", bid, cl.as_str()),
                    cell.rect.max.to_vec2(),
                    SpringCfg::SMOOTH,
                );
                let r = Rect::from_min_max(
                    Pos2::new(anim_min.x, anim_min.y),
                    Pos2::new(anim_max.x, anim_max.y),
                )
                .shrink(1.0);
                paint::rect(
                    ui.painter(),
                    r,
                    col.gamma_multiply(0.28),
                    Stroke::new(1.0_f32, col),
                );
                if r.width() > 48.0 && r.height() > 20.0 {
                    let clipped = ui.painter().with_clip_rect(r.shrink(3.0));
                    clipped.text(
                        Pos2::new(r.min.x + 4.0, r.min.y + 3.0),
                        egui::Align2::LEFT_TOP,
                        format!("{name} ({})", cl.label()),
                        FontId::proportional(11.0),
                        th.text,
                    );
                }
            }

            ui.add_space(12.0);

            // 3. 53x7 Download Heat-Calendar (last 53 weeks)
            ui.label(
                RichText::new("DOWNLOAD HEAT-CALENDAR (53 WEEKS)")
                    .monospace()
                    .strong()
                    .color(th.text_dim),
            );
            let cell_s = 12.0_f32;
            let cell_gap = 2.0_f32;
            let cal_w = 53.0 * (cell_s + cell_gap) + 8.0;
            let cal_h = 7.0 * (cell_s + cell_gap) + 8.0;
            let (cal_rect, _) = ui.allocate_exact_size(Vec2::new(cal_w, cal_h), Sense::hover());
            paint::rect(
                ui.painter(),
                cal_rect,
                th.bg0,
                Stroke::new(1.0_f32, th.border),
            );

            let daily_map: HashMap<&str, (u32, u64)> = state
                .stats
                .daily_downloads
                .iter()
                .map(|(d, c, b)| (d.as_str(), (*c, *b)))
                .collect();
            let today = chrono::Local::now().date_naive();

            for wk in 0..53 {
                for dow in 0..7 {
                    let days_ago = ((52 - wk) * 7 + (6 - dow)) as i64;
                    let date = today - chrono::Days::new(days_ago as u64);
                    let d_str = date.format("%Y-%m-%d").to_string();
                    let (cnt, bytes) = daily_map.get(d_str.as_str()).copied().unwrap_or((0, 0));
                    let sq = Rect::from_min_size(
                        Pos2::new(
                            cal_rect.min.x + 4.0 + (wk as f32) * (cell_s + cell_gap),
                            cal_rect.min.y + 4.0 + (dow as f32) * (cell_s + cell_gap),
                        ),
                        Vec2::splat(cell_s),
                    );
                    let fill = if cnt == 0 {
                        th.bg2
                    } else {
                        let intensity = ((cnt as f32) / 8.0).clamp(0.28, 1.0);
                        th.accent.gamma_multiply(intensity)
                    };
                    paint::rect_filled(ui.painter(), sq, fill);
                    let resp = ui.interact(sq, ui.id().with(("cal_sq", wk, dow)), Sense::hover());
                    if resp.hovered() {
                        let mb = (bytes as f64) / (1024.0 * 1024.0);
                        resp.on_hover_text(format!("{d_str}: {cnt} downloads ({mb:.1} MB)"));
                    }
                }
            }

            ui.add_space(12.0);

            // 4. Releases-per-year Stacked Bars
            ui.label(
                RichText::new("RELEASES PER YEAR")
                    .monospace()
                    .strong()
                    .color(th.text_dim),
            );
            let (yr_rect, _) = ui.allocate_exact_size(
                Vec2::new((ui.available_width() - 12.0).max(200.0), 140.0),
                Sense::hover(),
            );
            paint::rect(
                ui.painter(),
                yr_rect,
                th.bg0,
                Stroke::new(1.0_f32, th.border),
            );

            let mut years_map: std::collections::BTreeMap<i32, Vec<(Classification, u32)>> =
                std::collections::BTreeMap::new();
            for &(y, cl, cnt) in &state.stats.yearly_releases {
                if (1990..=2035).contains(&y) {
                    years_map.entry(y).or_default().push((cl, cnt));
                }
            }
            let max_yr_total = years_map
                .values()
                .map(|v| v.iter().map(|(_, c)| *c).sum::<u32>())
                .max()
                .unwrap_or(1)
                .max(1);

            let num_yrs = years_map.len().max(1);
            let bar_step = (yr_rect.width() - 16.0) / (num_yrs as f32);
            let bar_w = (bar_step - 4.0).clamp(4.0, 36.0);

            for (i, (yr, slices)) in years_map.iter().enumerate() {
                let x = yr_rect.min.x + 8.0 + (i as f32) * bar_step;
                let mut cur_y = yr_rect.max.y - 20.0;
                for &(cl, cnt) in slices {
                    let h = ((cnt as f32) / (max_yr_total as f32)) * (yr_rect.height() - 32.0);
                    let r = Rect::from_min_max(
                        Pos2::new(x, (cur_y - h).max(yr_rect.min.y + 4.0)),
                        Pos2::new(x + bar_w, cur_y),
                    );
                    paint::rect_filled(ui.painter(), r, class_color(cl, &th));
                    cur_y -= h;
                }
                ui.painter().text(
                    Pos2::new(x + bar_w * 0.5, yr_rect.max.y - 10.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{:02}", yr % 100),
                    FontId::monospace(9.5),
                    th.text_dim,
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarified_treemap_preserves_area_inside_bounds_without_overlap() {
        let bounds = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(400.0, 250.0));
        let items = vec![
            ("A", 50.0),
            ("B", 30.0),
            ("C", 15.0),
            ("D", 10.0),
            ("E", 5.0),
            ("F", 2.0),
        ];
        let cells = squarified_treemap(&items, bounds);
        assert_eq!(cells.len(), items.len());

        let mut area_sum = 0.0_f32;
        for (i, c1) in cells.iter().enumerate() {
            assert!(c1.rect.min.x >= bounds.min.x - 1e-3);
            assert!(c1.rect.min.y >= bounds.min.y - 1e-3);
            assert!(c1.rect.max.x <= bounds.max.x + 1e-3);
            assert!(c1.rect.max.y <= bounds.max.y + 1e-3);
            area_sum += c1.rect.width() * c1.rect.height();

            for c2 in cells.iter().skip(i + 1) {
                let inter = c1.rect.intersect(c2.rect);
                let inter_area = inter.width().max(0.0) * inter.height().max(0.0);
                assert!(
                    inter_area < 1e-2,
                    "treemap cells overlapped: {:?} vs {:?}",
                    c1.rect,
                    c2.rect
                );
            }
        }

        let expected_area = bounds.width() * bounds.height();
        assert!(
            (area_sum - expected_area).abs() < 1.0,
            "area_sum={area_sum} vs expected={expected_area}"
        );
    }
}
