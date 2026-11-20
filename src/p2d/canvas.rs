use super::model::{PathData, Project, SeqItem, StepKind};
use crate::common::machine::Pt;
use crate::common::theme::*;
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
    /// Snap parts to points of other parts and to the bed while moving.
    pub snap: bool,
    /// Selected steps of the sequence, highlighted in Path Selection.
    pub sel_steps: Vec<usize>,
    drag_part: Option<usize>,
    /// Part offset without snapping, so the part does not stick to a snap point.
    drag_raw: Pt,
    snap_hint: Option<SnapHint>,
}

/// What the part is snapped to, drawn while dragging.
#[derive(Clone, Copy)]
struct SnapHint {
    x: Option<f64>,
    y: Option<f64>,
    point: Option<Pt>,
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
            snap: true,
            sel_steps: vec![],
            drag_part: None,
            drag_raw: [0.0, 0.0],
            snap_hint: None,
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
    let item = SeqItem::path(part, path);
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

const SNAP_PX: f64 = 12.0;

/// Snap points of a drawing in its own coordinates: bounding box corners,
/// edge midpoints and centre, plus path end points and path centres.
fn key_points(paths: &[PathData]) -> Vec<Pt> {
    let mut out = Vec::new();
    let (mut ax, mut ay, mut bx, mut by) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in paths {
        if p.pts.is_empty() {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for q in &p.pts {
            x0 = x0.min(q[0]);
            y0 = y0.min(q[1]);
            x1 = x1.max(q[0]);
            y1 = y1.max(q[1]);
        }
        ax = ax.min(x0);
        ay = ay.min(y0);
        bx = bx.max(x1);
        by = by.max(y1);
        out.push(p.pts[0]);
        if let Some(l) = p.pts.last() {
            out.push(*l);
        }
        out.push([(x0 + x1) / 2.0, (y0 + y1) / 2.0]);
    }
    if ax <= bx {
        let (mx, my) = ((ax + bx) / 2.0, (ay + by) / 2.0);
        for x in [ax, mx, bx] {
            for y in [ay, my, by] {
                out.push([x, y]);
            }
        }
    }
    out
}

/// Find the offset adjustment that snaps the moving part to a target.
/// `moving` and `targets` are bed coordinates. Returns the adjustment and
/// what it snapped to.
fn find_snap(moving: &[Pt], targets: &[Pt], tol: f64) -> Option<([f64; 2], SnapHint)> {
    let mut best_pt: Option<(f64, Pt, Pt)> = None; // distance, delta, target
    let mut best_x: Option<(f64, f64, f64)> = None; // |dx|, dx, target x
    let mut best_y: Option<(f64, f64, f64)> = None;
    for m in moving {
        for t in targets {
            let (dx, dy) = (t[0] - m[0], t[1] - m[1]);
            let (ax, ay) = (dx.abs(), dy.abs());
            if ax <= tol && ay <= tol {
                let d = (dx * dx + dy * dy).sqrt();
                if best_pt.is_none_or(|b| d < b.0) {
                    best_pt = Some((d, [dx, dy], *t));
                }
            }
            if ax <= tol && best_x.is_none_or(|b| ax < b.0) {
                best_x = Some((ax, dx, t[0]));
            }
            if ay <= tol && best_y.is_none_or(|b| ay < b.0) {
                best_y = Some((ay, dy, t[1]));
            }
        }
    }
    if let Some((_, d, t)) = best_pt {
        return Some((d, SnapHint { x: None, y: None, point: Some(t) }));
    }
    if best_x.is_none() && best_y.is_none() {
        return None;
    }
    Some((
        [best_x.map_or(0.0, |b| b.1), best_y.map_or(0.0, |b| b.1)],
        SnapHint { x: best_x.map(|b| b.2), y: best_y.map(|b| b.2), point: None },
    ))
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
                        self.drag_raw = project.parts[p].offset;
                    }
                }
                if resp.dragged_by(PointerButton::Primary) {
                    if let Some(pi) = self.drag_part {
                        let d = resp.drag_delta();
                        self.drag_raw[0] += (d.x / self.scale) as f64;
                        self.drag_raw[1] -= (d.y / self.scale) as f64;
                        let mut off = self.drag_raw;
                        self.snap_hint = None;
                        if self.snap && !ui.input(|i| i.modifiers.alt) {
                            let local = key_points(&project.parts[pi].paths);
                            let moving: Vec<Pt> =
                                local.iter().map(|p| [p[0] + off[0], p[1] + off[1]]).collect();
                            let mut targets: Vec<Pt> = vec![[0.0, 0.0]];
                            for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
                                targets.push([x, y]);
                            }
                            if wk[0].1 > wk[0].0 && wk[1].1 > wk[1].0 {
                                for x in [wk[0].0, (wk[0].0 + wk[0].1) / 2.0, wk[0].1] {
                                    for y in [wk[1].0, (wk[1].0 + wk[1].1) / 2.0, wk[1].1] {
                                        targets.push([x, y]);
                                    }
                                }
                            }
                            for (oi, other) in project.parts.iter().enumerate() {
                                if oi == pi {
                                    continue;
                                }
                                for p in key_points(&other.paths) {
                                    targets.push([p[0] + other.offset[0], p[1] + other.offset[1]]);
                                }
                            }
                            if let Some((adj, hint)) = find_snap(&moving, &targets, SNAP_PX / self.scale as f64) {
                                off[0] += adj[0];
                                off[1] += adj[1];
                                self.snap_hint = Some(hint);
                            }
                        }
                        project.parts[pi].offset = off;
                    }
                }
                if resp.drag_stopped() {
                    self.drag_part = None;
                    self.snap_hint = None;
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
                            None => project.sequence.push(SeqItem::path(pi, ci)),
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
                    .world_points(&SeqItem::path(pi, ci))
                    .iter()
                    .any(|p| !project.machine.in_workable(*p));
                let seq_view = self.tool == Tool::Sequence;
                let in_seq = seq_view && project.seq_index(pi, ci).is_some();
                let hovered = hit == Some((pi, ci)) && self.tool == Tool::Sequence;
                let color = if hovered {
                    PATH_HOVER
                } else if outside {
                    PATH_OUTSIDE
                } else if in_seq {
                    PATH_SEQ
                } else if !seq_view && self.selected_part == Some(pi) {
                    PATH_PART_SELECTED
                } else {
                    PATH_NORMAL
                };
                let w = if in_seq || hovered { 2.2 } else { 1.4 };
                painter.add(Shape::line(pts, Stroke::new(w, color)));
            }
        }

        // Snap indicators.
        if let Some(h) = self.snap_hint {
            let c = Color32::from_rgb(0xff, 0xdc, 0x3c);
            if let Some(x) = h.x {
                let (a, b) = (xf.w2s([x, y0]), xf.w2s([x, y1]));
                painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, c), 6.0, 4.0));
            }
            if let Some(y) = h.y {
                let (a, b) = (xf.w2s([x0, y]), xf.w2s([x1, y]));
                painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, c), 6.0, 4.0));
            }
            if let Some(p) = h.point {
                let sp = xf.w2s(p);
                painter.rect_stroke(
                    Rect::from_center_size(sp, egui::vec2(12.0, 12.0)),
                    CornerRadius::ZERO,
                    Stroke::new(2.0, c),
                    StrokeKind::Middle,
                );
            }
        }

        // Sequence: travel lines, direction arrows and numbers. Only shown
        // while selecting paths; Prepare shows the raw drawing.
        let mut prev_end: Option<Pos2> = None;
        let seq_items = if self.tool == Tool::Sequence { project.sequence.as_slice() } else { &[] };
        for (n, item) in seq_items.iter().enumerate() {
            let picked = self.sel_steps.contains(&n);
            let badge = if picked { PATH_HOVER } else { PATH_SEQ };
            let pts: Vec<Pos2> = match item.kind {
                StepKind::Path => {
                    if item.part >= project.parts.len() || item.path >= project.parts[item.part].paths.len() {
                        continue;
                    }
                    project.world_points(item).into_iter().map(|p| xf.w2s(p)).collect()
                }
                StepKind::Home => {
                    let h = project.machine.home_position();
                    vec![xf.w2s([h[0], h[1]])]
                }
                StepKind::Goto { x, y, .. } => vec![xf.w2s([x, y])],
                StepKind::Pause { .. } => continue,
            };
            if pts.is_empty() || (item.is_path() && pts.len() < 2) {
                continue;
            }
            if let Some(pe) = prev_end {
                painter.extend(Shape::dashed_line(&[pe, pts[0]], Stroke::new(1.0, TRAVEL), 5.0, 4.0));
            }
            prev_end = pts.last().copied();
            if item.is_path() {
                if picked {
                    painter.add(Shape::line(pts.clone(), Stroke::new(3.0, PATH_HOVER)));
                }
                arrow(&painter, pts[0], pts[1], badge);
                painter.circle_filled(pts[0], 9.0, badge);
            } else {
                // Home and go-to points are drawn as squares.
                painter.rect_filled(Rect::from_center_size(pts[0], egui::vec2(18.0, 18.0)), CornerRadius::same(2), badge);
            }
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
