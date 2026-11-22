//! 3D viewer of the 3D project: orbit camera, shaded solids with dark edges on
//! a light gradient (CAD style), bed and workable volume, picking.

use super::model::{Project3, Wall};
use crate::common::machine::{AxisOwner, Machine};
use super::raster::{self, Frame};
use super::step::V3;
use eframe::egui::{self, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, TextureHandle, TextureOptions, Ui};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const FOV: f64 = 35.0 * std::f64::consts::PI / 180.0;
const NEAR: f64 = 1.0;

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Camera projection for one frame, in egui points.
#[derive(Clone, Copy)]
struct Proj {
    eye: V3,
    f: V3,
    r: V3,
    u: V3,
    focal: f64,
    c: Pos2,
    /// Added to every point before projecting: the bed sliding in the Run view.
    shift: V3,
}

impl Proj {
    fn cam(&self, p: V3) -> V3 {
        let d = sub([p[0] + self.shift[0], p[1] + self.shift[1], p[2] + self.shift[2]], self.eye);
        [dot(d, self.r), dot(d, self.u), dot(d, self.f)]
    }

    fn dir_cam(&self, d: V3) -> V3 {
        [dot(d, self.r), dot(d, self.u), dot(d, self.f)]
    }

    /// Screen position and depth, None when behind the camera.
    fn screen(&self, p: V3) -> Option<(Pos2, f64)> {
        let c = self.cam(p);
        if c[2] < NEAR {
            return None;
        }
        Some((
            Pos2::new(self.c.x + (c[0] / c[2] * self.focal) as f32, self.c.y - (c[1] / c[2] * self.focal) as f32),
            c[2],
        ))
    }

    fn ray(&self, pos: Pos2) -> (V3, V3) {
        let nx = (pos.x - self.c.x) as f64 / self.focal;
        let ny = -(pos.y - self.c.y) as f64 / self.focal;
        let d = norm([
            self.f[0] + self.r[0] * nx + self.u[0] * ny,
            self.f[1] + self.r[1] * nx + self.u[1] * ny,
            self.f[2] + self.r[2] * nx + self.u[2] * ny,
        ]);
        (self.eye, d)
    }

    /// Point where the cursor ray meets the horizontal plane at height z.
    fn ground(&self, pos: Pos2, z: f64) -> Option<V3> {
        let (o, d) = self.ray(pos);
        if d[2].abs() < 1e-9 {
            return None;
        }
        let t = (z - o[2]) / d[2];
        if t < 0.0 {
            return None;
        }
        Some([o[0] + d[0] * t, o[1] + d[1] * t, z])
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickMode {
    /// Prepare: drag bodies on the bed.
    Move,
    /// Path selection: click edges.
    Edges,
    /// Path selection: click a surface.
    Surface,
    /// Prepare, mate: click the flat face to move.
    LayFace,
    /// Prepare, mate: click the face of another part, or a workspace surface,
    /// to mate the first face to.
    MateTarget,
    /// Path selection: click a spot on a surface.
    Point,
    /// A start or stop: click an edge or a surface, Ctrl+click a point on it.
    Any,
    /// Path selection with nothing to pick: clicks do nothing.
    Idle,
}

/// One sequence step as drawn in the view.
pub struct StepViz {
    pub pts: Vec<V3>,
    /// The lead in, drawn apart from the path.
    pub lead: Vec<V3>,
    pub is_path: bool,
}

pub struct Overlay<'a> {
    pub mode: PickMode,
    pub selected_body: Option<usize>,
    /// Faces drawn in a colour (surfaces used by steps).
    pub tints: &'a [((usize, usize), [u8; 3])],
    /// Sequence steps, drawn only when `show_steps`.
    pub steps: &'a [StepViz],
    pub show_steps: bool,
    pub sel_steps: &'a [usize],
    /// Run tab: the simulated toolpath and tool, drawn over the parts.
    pub run: Option<&'a crate::common::preview::Preview>,
}

#[derive(Default)]
pub struct ViewOut {
    /// Click in Prepare: the body under the cursor, None for empty space.
    pub body_click: Option<Option<usize>>,
    pub edge_click: Option<(usize, usize)>,
    pub face_click: Option<(usize, usize)>,
    /// MateTarget: a workspace surface, when no part face is under the cursor.
    pub wall_click: Option<Wall>,
    pub point_click: Option<V3>,
    /// Cursor on the bed plane.
    pub cursor: Option<[f64; 2]>,
}

pub struct View3 {
    yaw: f64,
    pitch: f64,
    dist: f64,
    target: V3,
    fit: bool,
    frame: Option<Frame>,
    tex: Option<TextureHandle>,
    key: u64,
    scale: f32,
    drag_body: Option<usize>,
    /// Bit per body: the tool is inside it (Run view), drawn red.
    red_mask: u64,
    last_ground: Option<V3>,
}

impl Default for View3 {
    fn default() -> Self {
        Self {
            yaw: -0.9,
            pitch: 0.55,
            dist: 500.0,
            target: [0.0; 3],
            fit: true,
            frame: None,
            tex: None,
            key: 0,
            scale: 1.0,
            drag_body: None,
            red_mask: 0,
            last_ground: None,
        }
    }
}

/// Run view of a machine whose tool is at `tool` (G-code coordinates). Returns how far the
/// bed (and the parts on it) is displaced from the G-code frame, and the machine box corner
/// in the displaced frame. An axis that moves the bed shifts the bed by the opposite of the
/// tool move; the box and the tool stay put on the screen.
fn run_box(m: &Machine, tool: V3) -> (V3, V3) {
    let (wb, bp, ep) = (m.workable_on_bed(), m.bed_position(), m.effector_position());
    let home = m.home_position();
    let w0 = [wb[0].0, wb[1].0, 0.0];
    let (mut shift, mut lo) = ([0.0; 3], [0.0; 3]);
    for i in 0..3 {
        match m.axes[i] {
            AxisOwner::Bed => {
                shift[i] = home[i] - tool[i];
                lo[i] = home[i] - ep[i];
            }
            AxisOwner::EndEffector => lo[i] = -(w0[i] + bp[i]),
        }
    }
    (shift, lo)
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let ab = b - a;
    let l2 = ab.length_sq();
    if l2 < 1e-9 {
        return ((p - a).length(), 0.0);
    }
    let t = (((p - a).x * ab.x + (p - a).y * ab.y) / l2).clamp(0.0, 1.0);
    ((p - (a + ab * t)).length(), t)
}

impl View3 {
    pub fn fit(&mut self) {
        self.fit = true;
    }

    pub fn iso(&mut self) {
        self.yaw = -0.9;
        self.pitch = 0.55;
    }

    pub fn top(&mut self) {
        self.yaw = -std::f64::consts::FRAC_PI_2;
        self.pitch = 1.55;
    }

    pub fn front(&mut self) {
        self.yaw = -std::f64::consts::FRAC_PI_2;
        self.pitch = 0.0;
    }

    fn proj(&self, rect: Rect) -> Proj {
        let eye = [
            self.target[0] + self.dist * self.pitch.cos() * self.yaw.cos(),
            self.target[1] + self.dist * self.pitch.cos() * self.yaw.sin(),
            self.target[2] + self.dist * self.pitch.sin(),
        ];
        let f = norm(sub(self.target, eye));
        let r = norm(cross(f, [0.0, 0.0, 1.0]));
        let u = cross(r, f);
        Proj { eye, f, r, u, focal: rect.height() as f64 / (2.0 * (FOV / 2.0).tan()), c: rect.center(), shift: [0.0; 3] }
    }

    fn fit_view(&mut self, project: &Project3, run: bool) {
        let m = &project.machine;
        if run {
            // The whole machine box, so the bed can be followed over its travel.
            let (_, lo) = run_box(m, m.home_position());
            let f = m.frame_size();
            self.target = [lo[0] + f[0] / 2.0, lo[1] + f[1] / 2.0, lo[2] + f[2] / 2.0];
            self.dist = (0.5 * (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt() / (FOV / 2.0).sin() * 0.95).max(50.0);
            return;
        }
        let bed = m.bed_in_gcode();
        let tz = m.travel()[2];
        let (w, h) = (bed[0].1 - bed[0].0, bed[1].1 - bed[1].0);
        self.target = [(bed[0].0 + bed[0].1) / 2.0, (bed[1].0 + bed[1].1) / 2.0, tz * 0.25];
        let radius = 0.5 * (w * w + h * h + tz * tz).sqrt();
        self.dist = (radius / (FOV / 2.0).sin() * 0.95).max(50.0);
    }

    /// The workspace surface the cursor ray leaves through (the inside of the
    /// green box: the bed from above, the far walls).
    fn wall_at(&self, pr: &Proj, pos: Pos2, vol: &[(f64, f64); 3]) -> Option<Wall> {
        let (o, d) = pr.ray(pos);
        let mut best: Option<(f64, Wall)> = None;
        for w in Wall::ALL {
            let (n, p) = w.plane(vol);
            let nd = dot(n, d);
            if nd.abs() < 1e-9 {
                continue;
            }
            let t = dot(n, sub(p, o)) / nd;
            if t <= 0.0 || best.is_some_and(|b| b.0 >= t) {
                continue;
            }
            let hit = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
            if (0..3).all(|k| n[k] != 0.0 || (hit[k] >= vol[k].0 - 1e-6 && hit[k] <= vol[k].1 + 1e-6)) {
                best = Some((t, w));
            }
        }
        best.map(|b| b.1)
    }

    fn face_at(&self, rect: Rect, pos: Pos2) -> Option<(usize, usize)> {
        let fr = self.frame.as_ref()?;
        let x = ((pos.x - rect.min.x) * self.scale) as i64;
        let y = ((pos.y - rect.min.y) * self.scale) as i64;
        if x < 0 || y < 0 || x as usize >= fr.w || y as usize >= fr.h {
            return None;
        }
        raster::unpack_id(fr.id[y as usize * fr.w + x as usize])
    }

    /// The point on the visible surface under the cursor, from the depth buffer.
    fn point_at(&self, pr: &Proj, rect: Rect, pos: Pos2) -> Option<V3> {
        let fr = self.frame.as_ref()?;
        let x = ((pos.x - rect.min.x) * self.scale) as i64;
        let y = ((pos.y - rect.min.y) * self.scale) as i64;
        if x < 0 || y < 0 || x as usize >= fr.w || y as usize >= fr.h {
            return None;
        }
        let iz = fr.inv_z[y as usize * fr.w + x as usize];
        if iz <= 0.0 {
            return None;
        }
        // The buffer holds depth along the view direction.
        let (o, d) = pr.ray(pos);
        let t = (1.0 / iz as f64) / dot(d, pr.f);
        Some([o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t])
    }

    /// Visible edge near the cursor.
    fn edge_at(&self, project: &Project3, pr: &Proj, rect: Rect, pos: Pos2) -> Option<(usize, usize)> {
        let fr = self.frame.as_ref()?;
        let mut best: Option<(f32, usize, usize)> = None;
        for (bi, body) in project.bodies.iter().enumerate() {
            let Some(g) = &body.geo else { continue };
            for (ei, e) in g.edges.iter().enumerate() {
                let sp: Vec<Option<(Pos2, f64)>> = e.pts.iter().map(|p| pr.screen(body.world(*p))).collect();
                for w in sp.windows(2) {
                    let (Some(a), Some(b)) = (w[0], w[1]) else { continue };
                    let (d, t) = seg_dist(pos, a.0, b.0);
                    if d > 8.0 || best.is_some_and(|x| d >= x.0) {
                        continue;
                    }
                    // Not hidden behind a face.
                    let q = a.0 + (b.0 - a.0) * t;
                    let iz = (1.0 / a.1) * (1.0 - t as f64) + (1.0 / b.1) * t as f64;
                    let (px, py) = (((q.x - rect.min.x) * self.scale) as i64, ((q.y - rect.min.y) * self.scale) as i64);
                    if (iz as f32) >= fr.nearest_z(px, py) * 0.99 {
                        best = Some((d, bi, ei));
                    }
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    pub fn show(&mut self, ui: &mut Ui, project: &mut Project3, ov: &Overlay) -> ViewOut {
        let mut out = ViewOut::default();
        let avail = ui.available_size();
        let (resp, painter) = ui.allocate_painter(avail, Sense::click_and_drag());
        let rect = resp.rect;
        if self.fit {
            self.fit_view(project, ov.run.is_some());
            self.fit = false;
        }

        // Zoom.
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.dist = (self.dist * (-scroll as f64 * 0.0015).exp()).clamp(20.0, 100_000.0);
            }
        }
        let mut pr = self.proj(rect);
        // Run view: bodies the tool tip is inside.
        self.red_mask = ov.run.map_or(0, |pv| {
            let tool = pv.tool_now().0;
            project.bodies.iter().take(64).enumerate().filter(|(_, b)| b.contains(tool)).fold(0, |m, (i, _)| m | 1u64 << i)
        });
        // Run view: where the bed is displaced to and where the machine box sits.
        let run_state = ov.run.map(|pv| {
            let (tool, _) = pv.tool_now();
            run_box(&project.machine, tool)
        });
        if let Some((shift, _)) = run_state {
            pr.shift = shift;
        }
        let hover = resp.hover_pos();
        let face_hover = hover.and_then(|p| self.face_at(rect, p));
        let ctrl = ui.input(|i| i.modifiers.command);
        let edge_hover = if matches!(ov.mode, PickMode::Edges | PickMode::Any) {
            hover.and_then(|p| self.edge_at(project, &pr, rect, p))
        } else {
            None
        };
        let vol = project.machine.workable();
        let wall_hover = if ov.mode == PickMode::MateTarget && face_hover.is_none() && vol[0].1 > 0.0 && vol[1].1 > 0.0 {
            hover.and_then(|p| self.wall_at(&pr, p, &vol))
        } else {
            None
        };
        out.cursor = hover.and_then(|p| pr.ground(p, 0.0)).map(|g| [g[0], g[1]]);

        // Drag: move a body in Prepare, otherwise orbit. Right drag pans.
        if resp.drag_started_by(PointerButton::Primary) && ov.mode == PickMode::Move {
            if let (Some((b, _)), Some(p)) = (face_hover, hover) {
                self.drag_body = Some(b);
                out.body_click = Some(Some(b));
                let z = project.bodies[b].min_corner()[2];
                self.last_ground = pr.ground(p, z);
            }
        }
        if resp.dragged_by(PointerButton::Primary) {
            if let Some(b) = self.drag_body {
                if let (Some(p), Some(last)) = (resp.interact_pointer_pos(), self.last_ground) {
                    let z = last[2];
                    if let Some(now) = pr.ground(p, z) {
                        project.bodies[b].offset[0] += now[0] - last[0];
                        project.bodies[b].offset[1] += now[1] - last[1];
                        self.last_ground = Some(now);
                    }
                }
            } else {
                let d = resp.drag_delta();
                self.yaw -= d.x as f64 * 0.008;
                self.pitch = (self.pitch + d.y as f64 * 0.008).clamp(-1.55, 1.55);
            }
        }
        if resp.dragged_by(PointerButton::Middle) {
            let d = resp.drag_delta();
            self.yaw -= d.x as f64 * 0.008;
            self.pitch = (self.pitch + d.y as f64 * 0.008).clamp(-1.55, 1.55);
        }
        if resp.dragged_by(PointerButton::Secondary) {
            let d = resp.drag_delta();
            let k = self.dist / pr.focal;
            for i in 0..3 {
                self.target[i] += (-pr.r[i] * d.x as f64 + pr.u[i] * d.y as f64) * k;
            }
        }
        if resp.drag_stopped() {
            self.drag_body = None;
            self.last_ground = None;
        }
        pr = self.proj(rect);
        if resp.clicked() {
            match ov.mode {
                PickMode::Move => out.body_click = Some(face_hover.map(|f| f.0)),
                PickMode::Edges => out.edge_click = edge_hover,
                PickMode::Surface | PickMode::LayFace => out.face_click = face_hover,
                PickMode::MateTarget => {
                    out.face_click = face_hover;
                    out.wall_click = wall_hover;
                }
                PickMode::Point => out.point_click = hover.and_then(|p| self.point_at(&pr, rect, p)),
                PickMode::Any => {
                    if edge_hover.is_some() {
                        out.edge_click = edge_hover;
                    } else if ctrl {
                        out.point_click = hover.and_then(|p| self.point_at(&pr, rect, p));
                    } else {
                        out.face_click = face_hover;
                    }
                }
                PickMode::Idle => {}
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(match (ov.mode, self.drag_body.is_some() || face_hover.is_some() || edge_hover.is_some() || wall_hover.is_some()) {
                (PickMode::Move, true) => egui::CursorIcon::Grab,
                (_, true) => egui::CursorIcon::PointingHand,
                _ => egui::CursorIcon::Default,
            });
        }

        // ---- background ----
        let mut bg = egui::Mesh::default();
        let (tl, tr, bl, br) = (
            Color32::from_rgb(0xfb, 0xfc, 0xfd),
            Color32::from_rgb(0xe6, 0xe9, 0xed),
            Color32::from_rgb(0xe0, 0xe4, 0xe9),
            Color32::from_rgb(0xbd, 0xc3, 0xcb),
        );
        for (p, c) in [(rect.left_top(), tl), (rect.right_top(), tr), (rect.right_bottom(), br), (rect.left_bottom(), bl)] {
            bg.colored_vertex(p, c);
        }
        bg.add_triangle(0, 1, 2);
        bg.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(bg));

        // ---- bed, grid, workable floor ----
        let m = &project.machine;
        let bed = m.bed_in_gcode();
        let wk = m.workable();
        let (x0, x1, y0, y1) = (bed[0].0, bed[0].1, bed[1].0, bed[1].1);
        let quad = |pr: &Proj, a: [f64; 2], b: [f64; 2], z: f64| -> Option<Vec<Pos2>> {
            [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]]
                .iter()
                .map(|p| pr.screen([p[0], p[1], z]).map(|s| s.0))
                .collect()
        };
        if let Some(q) = quad(&pr, [x0, y0], [x1, y1], 0.0) {
            painter.add(Shape::convex_polygon(q.clone(), Color32::from_rgba_unmultiplied(0x9c, 0xa6, 0xb2, 120), Stroke::new(1.5, Color32::from_rgb(0x6b, 0x77, 0x84))));
        }
        let step = if (x1 - x0).max(y1 - y0) > 400.0 { 50.0 } else { 10.0 };
        let grid = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x6b, 0x77, 0x84, 70));
        let mut x = (x0 / step).ceil() * step;
        while x <= x1 {
            if let (Some(a), Some(b)) = (pr.screen([x, y0, 0.0]), pr.screen([x, y1, 0.0])) {
                painter.line_segment([a.0, b.0], grid);
            }
            x += step;
        }
        let mut y = (y0 / step).ceil() * step;
        while y <= y1 {
            if let (Some(a), Some(b)) = (pr.screen([x0, y, 0.0]), pr.screen([x1, y, 0.0])) {
                painter.line_segment([a.0, b.0], grid);
            }
            y += step;
        }
        let wk_ok = wk[0].1 > wk[0].0 && wk[1].1 > wk[1].0;
        if wk_ok {
            if let Some(q) = quad(&pr, [wk[0].0, wk[1].0], [wk[0].1, wk[1].1], 0.0) {
                painter.add(Shape::convex_polygon(q, Color32::from_rgba_unmultiplied(0x3f, 0xc1, 0x6b, 46), Stroke::NONE));
            }
        }

        // ---- shaded solids ----
        let ppp = ui.ctx().pixels_per_point();
        let mut scale = (ppp * 1.25).min(2.0);
        while (rect.width() * scale) * (rect.height() * scale) > 1.8e6 && scale > 0.3 {
            scale *= 0.9;
        }
        let (wpx, hpx) = (((rect.width() * scale) as usize).max(1), ((rect.height() * scale) as usize).max(1));
        let any_face = ov.mode == PickMode::Any && edge_hover.is_none() && !ctrl;
        let hover_tint = if matches!(ov.mode, PickMode::Surface | PickMode::LayFace | PickMode::MateTarget) || any_face { face_hover } else { None };

        let mut h = DefaultHasher::new();
        (wpx, hpx).hash(&mut h);
        for v in [self.yaw, self.pitch, self.dist, self.target[0], self.target[1], self.target[2], pr.shift[0], pr.shift[1], pr.shift[2]] {
            v.to_bits().hash(&mut h);
        }
        for b in &project.bodies {
            b.offset.map(f64::to_bits).hash(&mut h);
            b.rot.map(f64::to_bits).hash(&mut h);
            b.geo.as_ref().map(|g| std::sync::Arc::as_ptr(g) as usize).hash(&mut h);
        }
        (ov.selected_body, hover_tint, ov.tints, self.red_mask).hash(&mut h);
        let key = h.finish();
        if key != self.key || self.frame.is_none() {
            self.render(project, ov, &pr, rect, wpx, hpx, scale, hover_tint);
            self.key = key;
            self.scale = scale;
            if let Some(fr) = &self.frame {
                let img = egui::ColorImage::new([fr.w, fr.h], fr.color.clone());
                match &mut self.tex {
                    Some(t) => t.set(img, TextureOptions::LINEAR),
                    None => self.tex = Some(ui.ctx().load_texture("p3d_view", img, TextureOptions::LINEAR)),
                }
            }
        }
        if let Some(t) = &self.tex {
            painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }

        // ---- overlays ----
        if wk_ok {
            let (a, b) = ([wk[0].0, wk[1].0], [wk[0].1, wk[1].1]);
            let top = wk[2].1;
            let s = Stroke::new(1.3, Color32::from_rgb(0x1f, 0x9a, 0x4a));
            let corners = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]];
            for i in 0..4 {
                let (c, n) = (corners[i], corners[(i + 1) % 4]);
                for (z0, z1) in [(0.0, 0.0), (top, top)] {
                    if let (Some(p), Some(q)) = (pr.screen([c[0], c[1], z0]), pr.screen([n[0], n[1], z1])) {
                        painter.line_segment([p.0, q.0], s);
                    }
                }
                if let (Some(p), Some(q)) = (pr.screen([c[0], c[1], 0.0]), pr.screen([c[0], c[1], top])) {
                    painter.line_segment([p.0, q.0], s);
                }
            }
        }
        if let Some(w) = wall_hover {
            let (n, p) = w.plane(&vol);
            let k = (0..3).find(|&k| n[k] != 0.0).unwrap_or(2);
            let (i, j) = ((k + 1) % 3, (k + 2) % 3);
            let corner = |a: usize, b: usize| {
                let mut c = p;
                c[i] = if a == 0 { vol[i].0 } else { vol[i].1 };
                c[j] = if b == 0 { vol[j].0 } else { vol[j].1 };
                pr.screen(c).map(|s| s.0)
            };
            if let [Some(a), Some(b), Some(c), Some(d)] = [corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1)] {
                painter.add(Shape::convex_polygon(vec![a, b, c, d], Color32::from_rgba_unmultiplied(0x2f, 0x6f, 0xd0, 70), Stroke::new(1.5, Color32::from_rgb(0x2f, 0x6f, 0xd0))));
            }
        }
        // Origin axes.
        let len = ((x1 - x0).max(y1 - y0) * 0.08).max(10.0);
        if let Some(o) = pr.screen([0.0, 0.0, 0.0]) {
            for (d, col, label) in [
                ([len, 0.0, 0.0], Color32::from_rgb(0xd0, 0x3a, 0x3a), "X"),
                ([0.0, len, 0.0], Color32::from_rgb(0x2e, 0xa0, 0x4a), "Y"),
                ([0.0, 0.0, len], Color32::from_rgb(0x2f, 0x6f, 0xd0), "Z"),
            ] {
                if let Some(e) = pr.screen(d) {
                    painter.line_segment([o.0, e.0], Stroke::new(2.0, col));
                    painter.text(e.0, Align2::CENTER_BOTTOM, label, FontId::proportional(11.0), col);
                }
            }
        }
        painter.text(
            rect.left_top() + egui::vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            format!("{}  bed {} x {} x {} mm (grey)   workable volume (green)", m.name, m.bed_x, m.bed_y, m.bed_z),
            FontId::proportional(12.0),
            Color32::from_rgb(0x4a, 0x55, 0x61),
        );

        if ov.mode == PickMode::Point || (ov.mode == PickMode::Any && ctrl && edge_hover.is_none()) {
            if let Some(s) = hover.and_then(|p| self.point_at(&pr, rect, p)).and_then(|w| pr.screen(w)) {
                painter.circle_stroke(s.0, 6.0, Stroke::new(2.0, Color32::from_rgb(0xff, 0xb0, 0x00)));
            }
        }
        // Hovered edge.
        if let Some((b, e)) = edge_hover {
            let pts: Vec<Pos2> = project.edge_points(b, e).iter().filter_map(|p| pr.screen(*p)).map(|s| s.0).collect();
            painter.add(Shape::line(pts, Stroke::new(3.5, Color32::from_rgb(0xff, 0xb0, 0x00))));
        }
        // Sequence.
        if ov.show_steps {
            let green = Color32::from_rgb(0x1f, 0xa8, 0x40);
            let yellow = Color32::from_rgb(0xf0, 0x9a, 0x00);
            let mut prev: Option<Pos2> = None;
            for (n, viz) in ov.steps.iter().enumerate() {
                let sp: Vec<Pos2> = viz.pts.iter().filter_map(|p| pr.screen(*p)).map(|s| s.0).collect();
                if sp.is_empty() {
                    continue;
                }
                let picked = ov.sel_steps.contains(&n);
                let col = if picked { yellow } else { green };
                if let Some(pe) = prev {
                    painter.extend(Shape::dashed_line(&[pe, sp[0]], Stroke::new(1.0, Color32::from_rgb(0x55, 0x60, 0x6c)), 5.0, 4.0));
                }
                prev = sp.last().copied();
                if viz.lead.len() >= 2 {
                    let lp: Vec<Pos2> = viz.lead.iter().filter_map(|p| pr.screen(*p)).map(|s| s.0).collect();
                    painter.add(Shape::line(lp, Stroke::new(2.0, Color32::from_rgb(0x3b, 0x82, 0xd6))));
                }
                if viz.is_path {
                    painter.add(Shape::line(sp.clone(), Stroke::new(if picked { 3.5 } else { 2.5 }, col)));
                    painter.circle_filled(sp[0], 9.0, col);
                } else {
                    painter.rect_filled(Rect::from_center_size(sp[0], egui::vec2(18.0, 18.0)), 2.0, col);
                }
                painter.text(sp[0], Align2::CENTER_CENTER, (n + 1).to_string(), FontId::proportional(11.0), Color32::BLACK);
            }
        }
        // Run: the machine box and the rails of the axes that move the tool.
        if let Some((shift, lo)) = run_state {
            let f = project.machine.frame_size();
            let hi = [lo[0] + f[0], lo[1] + f[1], lo[2] + f[2]];
            let screen = |p: V3| pr.screen([p[0] - shift[0], p[1] - shift[1], p[2] - shift[2]]).map(|s| s.0);
            let line = |a: V3, b: V3, st: Stroke| {
                if let (Some(a), Some(b)) = (screen(a), screen(b)) {
                    painter.line_segment([a, b], st);
                }
            };
            let env = Stroke::new(1.2, Color32::from_rgb(0x6b, 0x77, 0x84));
            for z in [lo[2], hi[2]] {
                line([lo[0], lo[1], z], [hi[0], lo[1], z], env);
                line([hi[0], lo[1], z], [hi[0], hi[1], z], env);
                line([hi[0], hi[1], z], [lo[0], hi[1], z], env);
                line([lo[0], hi[1], z], [lo[0], lo[1], z], env);
            }
            for (x, y) in [(lo[0], lo[1]), (hi[0], lo[1]), (hi[0], hi[1]), (lo[0], hi[1])] {
                line([x, y, lo[2]], [x, y, hi[2]], env);
            }
            // The tool never moves on the screen when the bed carries the axis, so it is
            // placed from the same displaced coordinates the path uses.
            let t = ov.run.map(|pv| pv.tool_now().0).unwrap_or([0.0; 3]);
            let t = [t[0] + shift[0], t[1] + shift[1], t[2] + shift[2]];
            let rail = Stroke::new(4.0, Color32::from_rgb(0x55, 0x60, 0x6c));
            let ax = &project.machine.axes;
            if ax[0] == AxisOwner::EndEffector {
                line([lo[0], t[1], hi[2]], [hi[0], t[1], hi[2]], rail);
            }
            if ax[1] == AxisOwner::EndEffector {
                line([t[0], lo[1], hi[2]], [t[0], hi[1], hi[2]], rail);
            }
            if ax[2] == AxisOwner::EndEffector {
                line([t[0], t[1], t[2] + 26.0], [t[0], t[1], hi[2]], rail);
            }
        }
        // Run: toolpath (done solid, rest faint) and the tool.
        if let Some(pv) = ov.run {
            let accent = crate::common::preview::effector_color(project.machine.effector.kind);
            let faint = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90);
            let travel = Color32::from_rgba_unmultiplied(90, 100, 115, 120);
            let lerp = |a: V3, b: V3, f: f64| [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f);
            for (a, b, work, done) in pv.segments() {
                if !work && !pv.show_travel {
                    continue;
                }
                let (c_done, c_rest) = if work { (accent, faint) } else { (travel, travel) };
                let w = if work { 2.5 } else { 1.0 };
                let mid = lerp(a, b, done);
                let seg = |p: V3, q: V3, c: Color32, w: f32| {
                    if let (Some(s), Some(e)) = (pr.screen(p), pr.screen(q)) {
                        painter.line_segment([s.0, e.0], Stroke::new(w, c));
                    }
                };
                if done > 0.0 {
                    seg(a, mid, c_done, w);
                }
                if done < 1.0 && (pv.show_full || !work) {
                    seg(mid, b, c_rest, if work { 1.0 } else { w });
                }
            }
            let (tool, working) = pv.tool_now();
            if let (Some(t), Some(top)) = (pr.screen(tool), pr.screen([tool[0], tool[1], tool[2] + 25.0])) {
                painter.line_segment([t.0, top.0], Stroke::new(3.0, Color32::from_rgb(0x55, 0x60, 0x6c)));
                painter.circle_filled(t.0, if working { 5.0 } else { 3.5 }, if working { accent } else { Color32::from_rgb(0x55, 0x60, 0x6c) });
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn render(&mut self, project: &Project3, ov: &Overlay, pr: &Proj, rect: Rect, w: usize, h: usize, scale: f32, hover: Option<(usize, usize)>) {
        let mut fr = Frame::new(w, h);
        let l1 = norm([0.35, 0.55, -0.75]);
        let l2 = norm([-0.6, -0.2, -0.77]);
        let to_px = |s: Pos2| [(s.x - rect.min.x) * scale, (s.y - rect.min.y) * scale];
        for (bi, body) in project.bodies.iter().enumerate() {
            let Some(g) = &body.geo else { continue };
            let selected = ov.selected_body == Some(bi);
            for (fi, face) in g.faces.iter().enumerate() {
                let base: [f32; 3] = if hover == Some((bi, fi)) {
                    [255.0, 214.0, 70.0]
                } else if bi < 64 && self.red_mask >> bi & 1 == 1 {
                    [214.0, 58.0, 52.0]
                } else if let Some((_, c)) = ov.tints.iter().find(|t| t.0 == (bi, fi)) {
                    [c[0] as f32, c[1] as f32, c[2] as f32]
                } else if selected {
                    [150.0, 170.0, 196.0]
                } else {
                    [150.0, 156.0, 164.0]
                };
                let id = raster::pack_id(bi, fi);
                'tri: for (t, nr) in face.tris.iter().zip(&face.nors) {
                    let mut pp = [[0.0_f32; 2]; 3];
                    let mut iz = [0.0_f32; 3];
                    let mut col = [[0.0_f32; 3]; 3];
                    for k in 0..3 {
                        let Some((s, depth)) = pr.screen(body.world(t[k])) else { continue 'tri };
                        pp[k] = to_px(s);
                        iz[k] = (1.0 / depth) as f32;
                        let n = pr.dir_cam(body.rotate_dir(nr[k]));
                        let lam = (0.28 + 0.60 * dot(n, l1).abs() + 0.22 * dot(n, l2).abs()).min(1.1) as f32;
                        col[k] = [base[0] * lam, base[1] * lam, base[2] * lam];
                    }
                    fr.triangle(pp, iz, col, id);
                }
            }
        }
        // Edges on top, hidden where a face covers them.
        let edge_col = Color32::from_rgb(0x2a, 0x2f, 0x37);
        for body in &project.bodies {
            let Some(g) = &body.geo else { continue };
            for e in &g.edges {
                let sp: Vec<Option<(Pos2, f64)>> = e.pts.iter().map(|p| pr.screen(body.world(*p))).collect();
                for win in sp.windows(2) {
                    if let (Some(a), Some(b)) = (win[0], win[1]) {
                        fr.line(to_px(a.0), to_px(b.0), (1.0 / a.1) as f32, (1.0 / b.1) as f32, edge_col);
                    }
                }
            }
        }
        self.frame = Some(fr);
    }
}
