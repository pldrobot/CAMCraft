use crate::model::{Project, Pt, SeqItem};
use crate::theme::*;
use eframe::egui;
use egui::{Align2, Color32, CornerRadius, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Move,
    Sequence,
}

pub struct Canvas {
    pub center: Pt,
    pub scale: f32, // pixels per mm
    pub fit: bool,
    pub tool: Tool,
    pub selected_part: Option<usize>,
    pub cursor_world: Option<Pt>,
    drag_part: Option<usize>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            scale: 2.0,
            fit: true,
            tool: Tool::Move,
            selected_part: None,
            cursor_world: None,
            drag_part: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Xf {
    c: Pt,
    s: f32,
    rc: Pos2,
}

impl Xf {
    fn w2s(&self, p: Pt) -> Pos2 {
        Pos2::new(
            self.rc.x + (p[0] - self.c[0]) as f32 * self.s,
            self.rc.y - (p[1] - self.c[1]) as f32 * self.s,
        )
    }
    fn s2w(&self, p: Pos2) -> Pt {
        [
            self.c[0] + ((p.x - self.rc.x) / self.s) as f64,
            self.c[1] - ((p.y - self.rc.y) / self.s) as f64,
        ]
    }
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    if l2 < 1e-9 {
        return (p - a).length();
    }
    let t = (((p - a).x * ab.x + (p - a).y * ab.y) / l2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn path_screen_points(project: &Project, xf: &Xf, part: usize, path: usize) -> Vec<Pos2> {
    let item = SeqItem { part, path, reverse: false };
    project.world_points(&item).into_iter().map(|p| xf.w2s(p)).collect()
}

fn nearest_path(project: &Project, xf: &Xf, pos: Pos2, max_px: f32) -> Option<(usize, usize)> {
    let mut best: Option<(f32, usize, usize)> = None;
    for (pi, part) in project.parts.iter().enumerate() {
        for ci in 0..part.paths.len() {
            let pts = path_screen_points(project, xf, pi, ci);
            for w in pts.windows(2) {
                let d = seg_dist(pos, w[0], w[1]);
                if d <= max_px && best.is_none_or(|b| d < b.0) {
                    best = Some((d, pi, ci));
                }
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

fn arrow(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32) {
    let d = b - a;
    if d.length() < 24.0 {
        return;
    }
    let dir = d.normalized();
    let tip = a + d * 0.5;
    let n = egui::vec2(-dir.y, dir.x);
    let back = tip - dir * 8.0;
    painter.add(Shape::convex_polygon(
        vec![tip, back + n * 4.0, back - n * 4.0],
        color,
        Stroke::NONE,
    ));
}

impl Canvas {
    pub fn zoom_fit(&mut self) {
        self.fit = true;
    }

    pub fn show(&mut self, ui: &mut Ui, project: &mut Project) {
        let avail = ui.available_size();
        let (resp, painter) = ui.allocate_painter(avail, Sense::click_and_drag());
        let rect = resp.rect;
        let bg = project.machine.bed_in_gcode();
        let (bx, by) = (project.machine.bed_x, project.machine.bed_y);
        let (x0, y0, x1, y1) = (bg[0].0, bg[1].0, bg[0].1, bg[1].1);
        let wk = project.machine.workable();

        if self.fit {
            // Fit the bed and the origin.
            let (fx0, fy0) = (x0.min(0.0), y0.min(0.0));
            let (fw, fh) = (x1 - fx0, y1 - fy0);
            let sx = rect.width() as f64 / (fw * 1.15).max(1.0);
            let sy = rect.height() as f64 / (fh * 1.15).max(1.0);
            self.scale = sx.min(sy).max(0.05) as f32;
            self.center = [fx0 + fw / 2.0, fy0 + fh / 2.0];
            self.fit = false;
        }

        let mut xf = Xf { c: self.center, s: self.scale, rc: rect.center() };

        // Zoom about the cursor.
        if let Some(ptr) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let before = xf.s2w(ptr);
                self.scale = (self.scale * (scroll * 0.0015).exp()).clamp(0.05, 400.0);
                xf.s = self.scale;
                let after = xf.s2w(ptr);
                self.center[0] += before[0] - after[0];
                self.center[1] += before[1] - after[1];
                xf.c = self.center;
            }
        }
        // Pan with middle or right button.
        if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) {
            let d = resp.drag_delta();
            self.center[0] -= (d.x / self.scale) as f64;
            self.center[1] += (d.y / self.scale) as f64;
            xf.c = self.center;
        }

        let hover_pos = resp.hover_pos();
        self.cursor_world = hover_pos.map(|p| xf.s2w(p));

        // Tool interaction.
        let hit = hover_pos.and_then(|p| nearest_path(project, &xf, p, 8.0));
        match self.tool {
            Tool::Move => {
                if resp.drag_started_by(PointerButton::Primary) {
                    self.drag_part = hit.map(|h| h.0);
                    if let Some(p) = self.drag_part {
                        self.selected_part = Some(p);
                    }
                }
                if resp.dragged_by(PointerButton::Primary) {
                    if let Some(pi) = self.drag_part {
                        let d = resp.drag_delta();
                        let part = &mut project.parts[pi];
                        part.offset[0] += (d.x / self.scale) as f64;
                        part.offset[1] -= (d.y / self.scale) as f64;
                    }
                }
                if resp.drag_stopped() {
                    self.drag_part = None;
                }
                if resp.clicked() {
                    self.selected_part = hit.map(|h| h.0);
                }
            }
            Tool::Sequence => {
                if resp.clicked() {
                    if let Some((pi, ci)) = hit {
                        match project.seq_index(pi, ci) {
                            Some(idx) => {
                                project.sequence.remove(idx);
                            }
                            None => project.sequence.push(SeqItem { part: pi, path: ci, reverse: false }),
                        }
                    }
                }
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(match (self.tool, self.drag_part.is_some() || hit.is_some()) {
                (Tool::Move, true) => egui::CursorIcon::Grab,
                (Tool::Sequence, true) => egui::CursorIcon::PointingHand,
                _ => egui::CursorIcon::Crosshair,
            });
        }

        // ---- drawing ----
        painter.rect_filled(rect, CornerRadius::ZERO, CANVAS_BG);

        let bed = Rect::from_two_pos(xf.w2s([x0, y0]), xf.w2s([x1, y1]));
        painter.rect_filled(bed, CornerRadius::ZERO, BED_FILL);
        let work = Rect::from_two_pos(xf.w2s([wk[0].0, wk[1].0]), xf.w2s([wk[0].1, wk[1].1]));
        let work_ok = wk[0].1 > wk[0].0 && wk[1].1 > wk[1].0;
        if work_ok {
            painter.rect_filled(work, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0x3f, 0xc1, 0x6b, 22));
        }

        // Grid inside the bed.
        let steps = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0];
        let step = steps
            .iter()
            .copied()
            .find(|s| s * self.scale as f64 >= 14.0)
            .unwrap_or(200.0);
        let major = step * 5.0;
        let mut x = (x0 / step).ceil() * step;
        while x <= x1 + 1e-9 {
            let is_major = (x / major - (x / major).round()).abs() < 1e-6;
            let a = xf.w2s([x, y0]);
            let b = xf.w2s([x, y1]);
            painter.line_segment([a, b], Stroke::new(1.0, if is_major { GRID_MAJOR } else { GRID_MINOR }));
            x += step;
        }
        let mut y = (y0 / step).ceil() * step;
        while y <= y1 + 1e-9 {
            let is_major = (y / major - (y / major).round()).abs() < 1e-6;
            let a = xf.w2s([x0, y]);
            let b = xf.w2s([x1, y]);
            painter.line_segment([a, b], Stroke::new(1.0, if is_major { GRID_MAJOR } else { GRID_MINOR }));
            y += step;
        }
        painter.rect_stroke(bed, CornerRadius::ZERO, Stroke::new(1.5, Color32::from_rgb(0x7a, 0x86, 0x92)), StrokeKind::Middle);
        if work_ok {
            painter.rect_stroke(work, CornerRadius::ZERO, Stroke::new(1.5, BED_EDGE), StrokeKind::Middle);
        }
        painter.text(
            bed.left_top() + egui::vec2(0.0, -4.0),
            Align2::LEFT_BOTTOM,
            format!(
                "{}  bed {} x {} mm (grey)   workable {:.0} x {:.0} mm (green)",
                project.machine.name,
                bx,
                by,
                (wk[0].1 - wk[0].0).max(0.0),
                (wk[1].1 - wk[1].0).max(0.0)
            ),
            FontId::proportional(12.0),
            BED_EDGE,
        );

        // Origin marker (machine 0,0 at bottom-left).
        let o = xf.w2s([0.0, 0.0]);
        painter.line_segment([o, o + egui::vec2(28.0, 0.0)], Stroke::new(2.0, Color32::from_rgb(0xe0, 0x4a, 0x4a)));
        painter.line_segment([o, o + egui::vec2(0.0, -28.0)], Stroke::new(2.0, Color32::from_rgb(0x4a, 0xe0, 0x6a)));
        painter.text(o + egui::vec2(32.0, 0.0), Align2::LEFT_CENTER, "X", FontId::proportional(11.0), Color32::from_rgb(0xe0, 0x4a, 0x4a));
        painter.text(o + egui::vec2(0.0, -32.0), Align2::CENTER_BOTTOM, "Y", FontId::proportional(11.0), Color32::from_rgb(0x4a, 0xe0, 0x6a));

        // Paths.
        for (pi, part) in project.parts.iter().enumerate() {
            for ci in 0..part.paths.len() {
                let pts = path_screen_points(project, &xf, pi, ci);
                let outside = project
                    .world_points(&SeqItem { part: pi, path: ci, reverse: false })
                    .iter()
                    .any(|p| !project.machine.in_workable(*p));
                let in_seq = project.seq_index(pi, ci).is_some();
                let hovered = hit == Some((pi, ci)) && self.tool == Tool::Sequence;
                let color = if hovered {
                    PATH_HOVER
                } else if outside {
                    PATH_OUTSIDE
                } else if in_seq {
                    PATH_SEQ
                } else if self.selected_part == Some(pi) {
                    PATH_PART_SELECTED
                } else {
                    PATH_NORMAL
                };
                let w = if in_seq || hovered { 2.2 } else { 1.4 };
                painter.add(Shape::line(pts, Stroke::new(w, color)));
            }
        }

        // Sequence: travel lines, direction arrows and numbers.
        let mut prev_end: Option<Pos2> = None;
        for (n, item) in project.sequence.iter().enumerate() {
            if item.part >= project.parts.len() || item.path >= project.parts[item.part].paths.len() {
                continue;
            }
            let pts: Vec<Pos2> = project.world_points(item).into_iter().map(|p| xf.w2s(p)).collect();
            if pts.len() < 2 {
                continue;
            }
            if let Some(pe) = prev_end {
                painter.extend(Shape::dashed_line(&[pe, pts[0]], Stroke::new(1.0, TRAVEL), 5.0, 4.0));
            }
            prev_end = pts.last().copied();
            arrow(&painter, pts[0], pts[1], PATH_SEQ);
            painter.circle_filled(pts[0], 9.0, PATH_SEQ);
            painter.text(pts[0], Align2::CENTER_CENTER, (n + 1).to_string(), FontId::proportional(11.0), Color32::BLACK);
        }

        // Crosshair.
        if let Some(p) = hover_pos {
            let c = Color32::from_white_alpha(38);
            painter.line_segment([Pos2::new(rect.left(), p.y), Pos2::new(rect.right(), p.y)], Stroke::new(1.0, c));
            painter.line_segment([Pos2::new(p.x, rect.top()), Pos2::new(p.x, rect.bottom())], Stroke::new(1.0, c));
        }
    }
}
