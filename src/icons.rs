//! Small painted icon buttons, so they do not depend on font glyphs.

use crate::theme;
use eframe::egui;
use egui::{Pos2, Response, Sense, Shape, Stroke, Ui, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Reload,
    Layers,
    Remove,
    Plus,
    Trash,
    Settings,
    Reverse,
    SelEdge,
    SelFace,
    SelPoint,
    Offset,
}

pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> Response {
    icon_button_ex(ui, icon, tip, false, true)
}

/// `active` draws the icon in the highlight colour (e.g. a step with custom
/// settings). A disabled button is dimmed and does not click.
pub fn icon_button_ex(ui: &mut Ui, icon: Icon, tip: &str, active: bool, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), sense);
    let v = ui.style().interact(&resp);
    if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        ui.painter().rect_filled(rect, v.corner_radius, v.weak_bg_fill);
    }
    let color = if !enabled {
        v.fg_stroke.color.gamma_multiply(0.35)
    } else if active {
        theme::PATH_PART_SELECTED
    } else {
        v.fg_stroke.color
    };
    draw(ui.painter(), rect.center(), icon, color);
    resp.on_hover_text(tip)
}

/// A non-clickable icon in the given colour.
pub fn icon_label(ui: &mut Ui, icon: Icon, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
    draw(ui.painter(), rect.center(), icon, color);
}

fn draw(p: &egui::Painter, c: Pos2, icon: Icon, color: egui::Color32) {
    let stroke = Stroke::new(1.6, color);
    match icon {
        Icon::Reload => {
            // Open circle with an arrow head at the end.
            let r = 6.0;
            let n = 20;
            let (a0, a1) = (-0.5_f32, 4.6_f32);
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / n as f32;
                    c + Vec2::new(a.cos(), a.sin()) * r
                })
                .collect();
            let tip_pt = *pts.last().unwrap();
            p.add(Shape::line(pts, stroke));
            let dir = Vec2::new(-a1.sin(), a1.cos());
            let side = Vec2::new(-dir.y, dir.x);
            let base = tip_pt - dir * 1.0;
            p.add(Shape::convex_polygon(
                vec![tip_pt + dir * 4.0, base + side * 3.5, base - side * 3.5],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Layers => {
            // Three stacked diamonds.
            for dy in [-4.0_f32, 0.0, 4.0] {
                let m = c + Vec2::new(0.0, dy);
                let pts = vec![
                    m + Vec2::new(-7.0, 0.0),
                    m + Vec2::new(0.0, -3.5),
                    m + Vec2::new(7.0, 0.0),
                    m + Vec2::new(0.0, 3.5),
                ];
                if dy == -4.0 {
                    p.add(Shape::closed_line(pts, stroke));
                } else {
                    // Only the lower half of the lower diamonds shows.
                    p.add(Shape::line(vec![pts[0], pts[3], pts[2]], stroke));
                }
            }
        }
        Icon::Remove => {
            let d = 5.0;
            p.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
            p.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
        }
        Icon::Plus => {
            let d = 6.0;
            p.line_segment([c + Vec2::new(-d, 0.0), c + Vec2::new(d, 0.0)], stroke);
            p.line_segment([c + Vec2::new(0.0, -d), c + Vec2::new(0.0, d)], stroke);
        }
        Icon::Trash => {
            // Lid, handle and bin.
            p.line_segment([c + Vec2::new(-6.0, -4.5), c + Vec2::new(6.0, -4.5)], stroke);
            p.line_segment([c + Vec2::new(-2.0, -4.5), c + Vec2::new(-2.0, -7.0)], stroke);
            p.line_segment([c + Vec2::new(2.0, -4.5), c + Vec2::new(2.0, -7.0)], stroke);
            p.line_segment([c + Vec2::new(-2.0, -7.0), c + Vec2::new(2.0, -7.0)], stroke);
            p.add(Shape::line(
                vec![
                    c + Vec2::new(-4.5, -2.5),
                    c + Vec2::new(-3.5, 7.0),
                    c + Vec2::new(3.5, 7.0),
                    c + Vec2::new(4.5, -2.5),
                ],
                stroke,
            ));
        }
        Icon::Settings => {
            // Gear: ring with eight teeth.
            p.circle_stroke(c, 4.0, stroke);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let d = Vec2::new(a.cos(), a.sin());
                p.line_segment([c + d * 5.5, c + d * 8.0], Stroke::new(2.2, color));
            }
        }
        Icon::Reverse => {
            // Two opposite arrows.
            let (y1, y2) = (-3.5_f32, 3.5_f32);
            p.line_segment([c + Vec2::new(-6.0, y1), c + Vec2::new(6.0, y1)], stroke);
            p.line_segment([c + Vec2::new(6.0, y1), c + Vec2::new(3.0, y1 - 2.5)], stroke);
            p.line_segment([c + Vec2::new(6.0, y1), c + Vec2::new(3.0, y1 + 2.5)], stroke);
            p.line_segment([c + Vec2::new(-6.0, y2), c + Vec2::new(6.0, y2)], stroke);
            p.line_segment([c + Vec2::new(-6.0, y2), c + Vec2::new(-3.0, y2 - 2.5)], stroke);
            p.line_segment([c + Vec2::new(-6.0, y2), c + Vec2::new(-3.0, y2 + 2.5)], stroke);
        }
        Icon::SelEdge => {
            // Dashed diagonal with a dot at each end.
            for k in 0..3 {
                let a = c + Vec2::new(-6.0 + 4.5 * k as f32, 6.0 - 4.5 * k as f32);
                p.line_segment([a, a + Vec2::new(2.5, -2.5)], stroke);
            }
            p.circle_filled(c + Vec2::new(-7.0, 7.0), 1.8, color);
            p.circle_filled(c + Vec2::new(7.0, -7.0), 1.8, color);
        }
        Icon::SelFace => {
            let pts = vec![c + Vec2::new(-7.0, 5.0), c + Vec2::new(-3.0, -5.0), c + Vec2::new(7.0, -5.0), c + Vec2::new(3.0, 5.0)];
            p.add(Shape::convex_polygon(pts, color.gamma_multiply(0.25), stroke));
        }
        Icon::SelPoint => {
            p.line_segment([c + Vec2::new(-6.0, 0.0), c + Vec2::new(6.0, 0.0)], stroke);
            p.line_segment([c + Vec2::new(0.0, -6.0), c + Vec2::new(0.0, 6.0)], stroke);
            p.circle_filled(c, 2.2, color);
        }
        Icon::Offset => {
            // Two lines with a double arrow between them.
            p.line_segment([c + Vec2::new(-7.0, -6.0), c + Vec2::new(-7.0, 6.0)], stroke);
            p.line_segment([c + Vec2::new(5.0, -6.0), c + Vec2::new(5.0, 6.0)], stroke);
            p.line_segment([c + Vec2::new(-7.0, 0.0), c + Vec2::new(5.0, 0.0)], stroke);
            for (e, dx) in [(-7.0_f32, 3.0_f32), (5.0, -3.0)] {
                p.line_segment([c + Vec2::new(e, 0.0), c + Vec2::new(e + dx, -2.5)], stroke);
                p.line_segment([c + Vec2::new(e, 0.0), c + Vec2::new(e + dx, 2.5)], stroke);
            }
        }
    }
}
