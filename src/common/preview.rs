//! 3D toolpath simulation (Preview tab). Software projection on the egui
//! painter, so it needs no extra graphics dependency.

use crate::common::toolpath::Move;
use crate::common::machine::{AxisOwner, EffectorKind, JobSettings, Machine};
use eframe::egui;
use egui::{Align2, Color32, FontId, PointerButton, Pos2, Sense, Shape, Stroke, Ui, Vec2};

type V3 = [f64; 3];

pub struct Preview {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan: Vec2,
    pub playing: bool,
    /// Run target: false = simulator, true = real machine.
    pub machine_mode: bool,
    /// The real machine on a serial port.
    pub live: crate::common::link::Live,
    /// Manual position of the simulated machine (G-code coordinates). None: at home.
    pub jog: Option<V3>,
    /// Selected jog step size, index into the controls' step list.
    pub step_idx: usize,
    /// Steps in mm used when the custom step is selected (`step_idx` past the list).
    pub custom_xy: f64,
    /// Gamepad speed level, when a gamepad is connected.
    pub pad_speed: Option<f64>,
    pub custom_z: f64,
    /// Index into `speeds`.
    pub speed_idx: usize,
    pub show_travel: bool,
    pub show_full: bool,
    /// Draw the project's parts (outlines on the bed) under the toolpath.
    pub show_parts: bool,
    /// Part outlines in bed coordinates, set when the G-code is generated.
    pub outlines: Vec<Vec<V3>>,
    /// Playback position in seconds of machine time.
    pub time: f64,
    moves: Vec<Move>,
    /// End time of every move.
    cum: Vec<f64>,
    pub path_len: f64,
}

pub const SPEEDS: [f64; 5] = [1.0, 5.0, 20.0, 100.0, 500.0];

impl Default for Preview {
    fn default() -> Self {
        Self {
            yaw: -0.6,
            pitch: 0.9,
            zoom: 1.0,
            pan: Vec2::ZERO,
            playing: false,
            machine_mode: false,
            live: Default::default(),
            jog: None,
            step_idx: 1,
            custom_xy: 5.0,
            pad_speed: None,
            custom_z: 1.0,
            speed_idx: 2,
            show_travel: true,
            show_full: true,
            show_parts: true,
            outlines: vec![],
            time: 0.0,
            moves: vec![],
            cum: vec![],
            path_len: 0.0,
        }
    }
}

fn len3(a: V3, b: V3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

pub(crate) fn effector_color(kind: EffectorKind) -> Color32 {
    match kind {
        EffectorKind::Glue => Color32::from_rgb(0xff, 0xb3, 0x2c),
        EffectorKind::Laser => Color32::from_rgb(0xff, 0x4d, 0x4d),
        EffectorKind::Motor => Color32::from_rgb(0x3c, 0xc8, 0xff),
        EffectorKind::Other => Color32::from_rgb(0x6e, 0xe0, 0x7a),
    }
}

impl Preview {
    pub fn has_path(&self) -> bool {
        !self.moves.is_empty()
    }

    pub fn total_time(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    pub fn work_moves(&self) -> usize {
        self.moves.iter().filter(|m| m.work).count()
    }

    pub fn clear(&mut self) {
        self.moves.clear();
        self.cum.clear();
        self.time = 0.0;
        self.playing = false;
        self.path_len = 0.0;
    }

    pub fn set_moves(&mut self, moves: Vec<Move>, job: &JobSettings) {
        let mut t = 0.0;
        let mut total_len = 0.0;
        self.cum = moves
            .iter()
            .map(|m| {
                let l = len3(m.from, m.to);
                total_len += l;
                let feed = if m.work { job.dispense_feed } else { job.travel_feed };
                t += l / feed.max(1.0) * 60.0;
                t
            })
            .collect();
        self.moves = moves;
        self.path_len = total_len;
        self.time = 0.0;
        self.playing = false;
    }

    /// Tool position and whether it is working, at the current time.
    fn state_at(&self, t: f64) -> (V3, bool) {
        if self.moves.is_empty() {
            return ([0.0; 3], false);
        }
        let i = self.cum.partition_point(|&c| c < t).min(self.moves.len() - 1);
        let start = if i == 0 { 0.0 } else { self.cum[i - 1] };
        let dur = (self.cum[i] - start).max(1e-12);
        let f = ((t - start) / dur).clamp(0.0, 1.0);
        let m = &self.moves[i];
        let p = [0, 1, 2].map(|k| m.from[k] + (m.to[k] - m.from[k]) * f);
        (p, m.work && t > 0.0 && t < self.total_time())
    }

    /// G-code line being run: shown from the first Start until Stop.
    pub fn current_line(&self) -> Option<usize> {
        if self.machine_mode {
            return self.live.acked.filter(|_| self.live.connected());
        }
        if self.moves.is_empty() || (!self.playing && self.time <= 0.0) {
            return None;
        }
        let i = self.cum.partition_point(|&c| c < self.time).min(self.moves.len() - 1);
        Some(self.moves[i].line)
    }

    pub fn view_top(&mut self) {
        self.yaw = 0.0;
        self.pitch = std::f32::consts::FRAC_PI_2 - 0.001;
        self.pan = Vec2::ZERO;
    }

    pub fn view_front(&mut self) {
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.pan = Vec2::ZERO;
    }

    pub fn view_iso(&mut self) {
        let d = Self::default();
        (self.yaw, self.pitch, self.zoom, self.pan) = (d.yaw, d.pitch, d.zoom, d.pan);
    }

    pub fn show(&mut self, ui: &mut Ui, machine: &Machine) {
        let avail = ui.available_size();
        let (resp, painter) = ui.allocate_painter(avail, Sense::click_and_drag());
        let rect = resp.rect;

        // Camera input.
        if resp.dragged_by(PointerButton::Primary) {
            let d = resp.drag_delta();
            self.yaw -= d.x * 0.008;
            self.pitch = (self.pitch + d.y * 0.008).clamp(0.0, std::f32::consts::FRAC_PI_2 - 0.001);
        }
        if resp.dragged_by(PointerButton::Secondary) || resp.dragged_by(PointerButton::Middle) {
            self.pan += resp.drag_delta();
        }
        if resp.hovered() {
            let s = ui.input(|i| i.smooth_scroll_delta.y);
            if s != 0.0 {
                self.zoom = (self.zoom * (s * 0.0015).exp()).clamp(0.2, 12.0);
            }
        }

        self.tick(ui.ctx());
        let (tool, working) = self.tool_now();

        self.paint(&painter, rect, machine, tool, working, false);
    }

    /// Advance the simulated run by the frame time while it is playing.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if !self.playing {
            return;
        }
        self.time += ctx.input(|i| i.stable_dt) as f64 * SPEEDS[self.speed_idx];
        let total = self.total_time();
        if self.time >= total {
            self.time = total;
            self.playing = false;
        }
        ctx.request_repaint();
    }

    /// Tool position (G-code coordinates) and whether it is working, now.
    pub fn tool_now(&self) -> (V3, bool) {
        if let (true, Some(p)) = (self.machine_mode, self.live.pos) {
            return (p, self.live.job);
        }
        match self.jog {
            Some(p) if !self.playing && self.time <= 0.0 => (p, false),
            _ => self.state_at(self.time),
        }
    }

    /// Move the idle simulated machine by `d` mm, staying inside the machine travel.
    pub fn jog_by(&mut self, d: V3, m: &Machine) {
        if self.playing || self.time > 0.0 || self.moves.is_empty() {
            return;
        }
        let cur = self.tool_now().0;
        let w = m.tool_range(); // the full travel, also beyond the bed
        self.jog = Some([0, 1, 2].map(|i| (cur[i] + d[i]).clamp(w[i].0, w[i].1)));
    }

    /// Send the chosen axes to the home position.
    pub fn jog_home(&mut self, axes: [bool; 3], m: &Machine) {
        if self.playing || self.time > 0.0 || self.moves.is_empty() {
            return;
        }
        let (cur, home) = (self.tool_now().0, m.home_position());
        self.jog = Some([0, 1, 2].map(|i| if axes[i] { home[i] } else { cur[i] }));
    }

    pub fn stop(&mut self) {
        self.time = 0.0;
        self.playing = false;
        self.jog = None;
    }

    /// Start, pause or resume the run. A run always starts from home.
    pub fn start_pause(&mut self) {
        if self.machine_mode || self.moves.is_empty() {
            return;
        }
        if self.playing {
            self.playing = false;
            return;
        }
        if self.time <= 0.0 || self.time >= self.total_time() {
            self.time = 0.0;
        }
        self.jog = None;
        self.playing = true;
    }

    /// Every move as (from, to, work, done) where done is 0..1 of the move already run.
    pub fn segments(&self) -> impl Iterator<Item = (V3, V3, bool, f64)> + '_ {
        self.moves.iter().enumerate().map(move |(i, m)| {
            let start = if i == 0 { 0.0 } else { self.cum[i - 1] };
            let done = ((self.time - start) / (self.cum[i] - start).max(1e-12)).clamp(0.0, 1.0);
            (m.from, m.to, m.work, done)
        })
    }

    /// Static isometric picture of the machine for the machine editor: wireframe
    /// box, bed and workable volume as transparent blocks, end effector at home.
    pub fn show_schematic(ui: &mut Ui, machine: &Machine, size: Vec2) {
        let (resp, painter) = ui.allocate_painter(size, Sense::hover());
        let v = Preview { yaw: -std::f32::consts::FRAC_PI_4, pitch: 0.615, ..Self::default() };
        v.paint(&painter, resp.rect, machine, machine.home_position(), false, true);
    }

    fn paint(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        machine: &Machine,
        tool: V3,
        working: bool,
        schematic: bool,
    ) {
        let bed = machine.bed();
        let kind = machine.effector.kind;
        let accent = effector_color(kind);

        // Everything lives inside the machine size box. Axes that move the
        // bed slide the bed under a tool that stays at its fixed position;
        // axes that move the end effector move the tool over a bed that stays
        // at its fixed position.
        let frame = machine.frame_size();
        let fo = [0.0f64; 3];
        let (bp, ep) = (machine.bed_position(), machine.effector_position());
        let wb = machine.workable_on_bed();
        let mut bo = [0.0f64; 3]; // where the bed corner (Z: the bed top) is
        let mut tool_vis = [0.0f64; 3];
        for i in 0..3 {
            // Bed coordinate the tool is over.
            let q = if i < 2 { tool[i] + wb[i].0 } else { tool[2] };
            match machine.axes[i] {
                AxisOwner::Bed => {
                    // The tool is fixed and the bed slides under it.
                    tool_vis[i] = ep[i];
                    bo[i] = ep[i] - q;
                }
                AxisOwner::EndEffector => {
                    // The bed is fixed and the tool slides over it.
                    bo[i] = bp[i];
                    tool_vis[i] = bp[i] + q;
                }
            }
        }
        let on_bed = |p: V3| [p[0] + bo[0], p[1] + bo[1], p[2] + bo[2]];
        // G-code point to the box: G-code to bed coordinates, then onto the bed.
        let on_path = |p: V3| {
            let q = machine.gcode_to_bed(p);
            [q[0] + bo[0], q[1] + bo[1], q[2] + bo[2]]
        };

        // Projection.
        let diag = (frame[0].powi(2) + frame[1].powi(2) + frame[2].powi(2)).sqrt().max(1.0);
        let center = [fo[0] + frame[0] / 2.0, fo[1] + frame[1] / 2.0, fo[2] + frame[2] / 2.0];
        let (sy, cy) = (self.yaw.sin() as f64, self.yaw.cos() as f64);
        let (sp, cp) = (self.pitch.sin() as f64, self.pitch.cos() as f64);
        let cam_d = diag * 2.5;
        let base = (rect.width().min(rect.height()) as f64 / diag) * 0.8 * self.zoom as f64;
        let rc = rect.center() + self.pan;
        let proj = |p: V3| -> Pos2 {
            let v = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
            let x1 = v[0] * cy - v[1] * sy;
            let y1 = v[0] * sy + v[1] * cy;
            let up = v[2] * cp + y1 * sp;
            let depth = y1 * cp - v[2] * sp;
            let k = base * cam_d / (cam_d + depth).max(1.0);
            Pos2::new(rc.x + (x1 * k) as f32, rc.y - (up * k) as f32)
        };

        painter.rect_filled(rect, 0.0, Color32::from_rgb(0x16, 0x1b, 0x21));
        let line = |a: V3, b: V3, stroke: Stroke| {
            painter.line_segment([proj(a), proj(b)], stroke);
        };

        // Machine size box.
        let env_c = Color32::from_rgba_unmultiplied(150, 170, 190, 120);
        let env = Stroke::new(1.0, env_c);
        let (x0, y0, z0) = (fo[0], fo[1], fo[2]);
        let (ex, ey, ez) = (fo[0] + frame[0], fo[1] + frame[1], fo[2] + frame[2]);
        let corners = |x: f64, y: f64, z: f64| [x, y, z];
        for &z in &[z0, ez] {
            line(corners(x0, y0, z), corners(ex, y0, z), env);
            line(corners(ex, 0.0, z), corners(ex, ey, z), env);
            line(corners(ex, ey, z), corners(x0, ey, z), env);
            line(corners(x0, ey, z), corners(x0, y0, z), env);
        }
        for &(x, y) in &[(x0, y0), (ex, y0), (ex, ey), (x0, ey)] {
            line(corners(x, y, z0), corners(x, y, ez), env);
        }

        // Rails for the axes that move the end effector.
        let rail = Stroke::new(3.0, Color32::from_rgb(0x55, 0x60, 0x6c));
        let top = ez;
        if machine.axes[0] == AxisOwner::EndEffector {
            line([x0, tool_vis[1], top], [ex, tool_vis[1], top], rail);
        }
        if machine.axes[1] == AxisOwner::EndEffector {
            line([tool_vis[0], y0, top], [tool_vis[0], ey, top], rail);
        }
        if machine.axes[2] == AxisOwner::EndEffector {
            line([tool_vis[0], tool_vis[1], tool_vis[2] + 26.0], [tool_vis[0], tool_vis[1], top], rail);
        }

        // Bed: top face, grid and thickness.
        let bz = bed[2].max(0.0);
        let bed_pts = [[0.0, 0.0], [bed[0], 0.0], [bed[0], bed[1]], [0.0, bed[1]]];
        let top_face: Vec<Pos2> = bed_pts.iter().map(|p| proj(on_bed([p[0], p[1], 0.0]))).collect();
        let top_fill = if schematic {
            Color32::from_rgba_unmultiplied(0x3f, 0xc1, 0x6b, 34)
        } else {
            Color32::from_rgb(0x22, 0x29, 0x31)
        };
        painter.add(Shape::convex_polygon(top_face, top_fill, Stroke::new(1.5, crate::common::theme::BED_EDGE)));
        if schematic {
            // Transparent bed block: the four sides.
            let side_fill = Color32::from_rgba_unmultiplied(0x3f, 0xc1, 0x6b, 50);
            for k in 0..4 {
                let (a, b) = (bed_pts[k], bed_pts[(k + 1) % 4]);
                let quad = vec![
                    proj(on_bed([a[0], a[1], 0.0])),
                    proj(on_bed([b[0], b[1], 0.0])),
                    proj(on_bed([b[0], b[1], -bz])),
                    proj(on_bed([a[0], a[1], -bz])),
                ];
                painter.add(Shape::convex_polygon(quad, side_fill, Stroke::NONE));
            }
        }
        let step = if bed[0].max(bed[1]) > 400.0 { 50.0 } else { 10.0 };
        let grid = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 14));
        let mut x = step;
        while x < bed[0] {
            line(on_bed([x, 0.0, 0.0]), on_bed([x, bed[1], 0.0]), grid);
            x += step;
        }
        let mut y = step;
        while y < bed[1] {
            line(on_bed([0.0, y, 0.0]), on_bed([bed[0], y, 0.0]), grid);
            y += step;
        }
        let side = Stroke::new(1.0, Color32::from_rgb(0x2c, 0x7a, 0x4a));
        for p in bed_pts {
            line(on_bed([p[0], p[1], 0.0]), on_bed([p[0], p[1], -bz]), side);
        }
        for i in 0..4 {
            let (a, b) = (bed_pts[i], bed_pts[(i + 1) % 4]);
            line(on_bed([a[0], a[1], -bz]), on_bed([b[0], b[1], -bz]), side);
        }

        // Workable area on the bed surface.
        let wk = machine.workable();
        if wk[0].1 > wk[0].0 && wk[1].1 > wk[1].0 {
            let c = [
                [wk[0].0, wk[1].0],
                [wk[0].1, wk[1].0],
                [wk[0].1, wk[1].1],
                [wk[0].0, wk[1].1],
            ];
            let st = Stroke::new(1.5, Color32::from_rgba_unmultiplied(0x3f, 0xc1, 0x6b, 200));
            for k in 0..4 {
                let (p, q) = (c[k], c[(k + 1) % 4]);
                line(on_path([p[0], p[1], 0.0]), on_path([q[0], q[1], 0.0]), st);
            }
        }

        // Workable volume as a transparent block on top of the bed.
        if schematic && wk[0].1 > wk[0].0 && wk[1].1 > wk[1].0 {
            let h = wk[2].1;
            let c = [
                [wk[0].0, wk[1].0],
                [wk[0].1, wk[1].0],
                [wk[0].1, wk[1].1],
                [wk[0].0, wk[1].1],
            ];
            let fill = Color32::from_rgba_unmultiplied(0x4f, 0xb6, 0xff, 30);
            let edge = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x4f, 0xb6, 0xff, 170));
            for k in 0..4 {
                let (p, q) = (c[k], c[(k + 1) % 4]);
                let quad = vec![
                    proj(on_path([p[0], p[1], 0.0])),
                    proj(on_path([q[0], q[1], 0.0])),
                    proj(on_path([q[0], q[1], h])),
                    proj(on_path([p[0], p[1], h])),
                ];
                painter.add(Shape::convex_polygon(quad, fill, edge));
            }
            let top: Vec<Pos2> = c.iter().map(|p| proj(on_path([p[0], p[1], h]))).collect();
            painter.add(Shape::convex_polygon(top, fill, edge));
        }

        // Origin axes.
        let o = on_path([0.0, 0.0, 0.0]);
        let len = (diag * 0.1).max(10.0);
        line(o, [o[0] + len, o[1], o[2]], Stroke::new(2.0, Color32::from_rgb(0xe0, 0x4a, 0x4a)));
        line(o, [o[0], o[1] + len, o[2]], Stroke::new(2.0, Color32::from_rgb(0x4a, 0xe0, 0x6a)));
        line(o, [o[0], o[1], o[2] + len], Stroke::new(2.0, Color32::from_rgb(0x4f, 0xb6, 0xff)));

        // Homing position of the end effector.
        {
            let hp = machine.home_position();
            let mut hv = [0.0f64; 3];
            for i in 0..3 {
                let q = if i < 2 { hp[i] + wb[i].0 } else { hp[2] };
                hv[i] = match machine.axes[i] {
                    AxisOwner::Bed => ep[i],
                    AxisOwner::EndEffector => bp[i] + q,
                };
            }
            let c = Color32::from_rgb(0xff, 0xdc, 0x3c);
            painter.circle_stroke(proj(hv), 6.0, Stroke::new(1.5, c));
            painter.text(
                proj(hv) + egui::vec2(9.0, -9.0),
                Align2::LEFT_BOTTOM,
                "Home",
                FontId::proportional(11.0),
                c,
            );
        }

        // Parts, so the toolpath can be followed over the real shapes.
        if self.show_parts {
            let st = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x9a, 0xb4, 0xcc, 150));
            for o in &self.outlines {
                painter.add(Shape::line(o.iter().map(|p| proj(on_bed(*p))).collect(), st));
            }
        }

        // Toolpath.
        let done_c = accent;
        let todo_c = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 50);
        let travel_c = Color32::from_rgba_unmultiplied(150, 160, 175, 70);
        let t_now = self.time;
        for (i, m) in self.moves.iter().enumerate() {
            if !m.work && !self.show_travel {
                continue;
            }
            let start = if i == 0 { 0.0 } else { self.cum[i - 1] };
            let end = self.cum[i];
            let (a, b) = (on_path(m.from), on_path(m.to));
            if t_now >= end {
                let st = if m.work { Stroke::new(2.0, done_c) } else { Stroke::new(1.0, travel_c) };
                line(a, b, st);
            } else if t_now > start {
                let f = (t_now - start) / (end - start).max(1e-12);
                let mid = [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f);
                let st = if m.work { Stroke::new(2.0, done_c) } else { Stroke::new(1.0, travel_c) };
                line(a, mid, st);
                if self.show_full && m.work {
                    line(mid, b, Stroke::new(1.0, todo_c));
                }
            } else if self.show_full && m.work {
                line(a, b, Stroke::new(1.0, todo_c));
            }
        }

        // End effector drawing (illustration only).
        let body = Color32::from_rgb(0xb8, 0xc2, 0xcc);
        let ring = |c: V3, r: f64| -> Vec<Pos2> {
            (0..16)
                .map(|k| {
                    let a = std::f64::consts::TAU * k as f64 / 16.0;
                    proj([c[0] + r * a.cos(), c[1] + r * a.sin(), c[2]])
                })
                .collect()
        };
        let cyl = |z0: f64, z1: f64, r0: f64, r1: f64, col: Color32| {
            let st = Stroke::new(1.2, col);
            let a = ring([tool_vis[0], tool_vis[1], tool_vis[2] + z0], r0);
            let b = ring([tool_vis[0], tool_vis[1], tool_vis[2] + z1], r1);
            for k in 0..16 {
                painter.line_segment([a[k], a[(k + 1) % 16]], st);
                painter.line_segment([b[k], b[(k + 1) % 16]], st);
            }
            for k in (0..16).step_by(4) {
                painter.line_segment([a[k], b[k]], st);
            }
        };
        let hot = if working { accent } else { body };
        match kind {
            EffectorKind::Glue => {
                cyl(0.0, 5.0, 0.4, 2.5, hot);
                cyl(5.0, 24.0, 3.5, 3.5, body);
                if working {
                    painter.circle_filled(proj(tool_vis), 3.0, accent);
                }
            }
            EffectorKind::Laser => {
                cyl(4.0, 24.0, 4.0, 4.0, body);
                let beam = Stroke::new(if working { 2.5 } else { 1.0 }, hot);
                painter.line_segment([proj([tool_vis[0], tool_vis[1], tool_vis[2] + 4.0]), proj(tool_vis)], beam);
                if working {
                    painter.circle_filled(proj(tool_vis), 4.0, accent);
                }
            }
            EffectorKind::Motor => {
                cyl(0.0, 7.0, 0.8, 0.8, hot);
                cyl(7.0, 12.0, 2.5, 5.0, body);
                cyl(12.0, 28.0, 5.0, 5.0, body);
            }
            EffectorKind::Other => {
                cyl(3.0, 3.0, 4.0, 4.0, body);
                cyl(0.0, 3.0, 0.5, 4.0, hot);
                painter.line_segment([proj([tool_vis[0], tool_vis[1], tool_vis[2] + 3.0]), proj([tool_vis[0], tool_vis[1], tool_vis[2] + 22.0])], Stroke::new(2.0, body));
            }
        }

        if schematic {
            let y = rect.bottom() - 8.0;
            let mut x = rect.left() + 10.0;
            for (label, col) in [
                ("Machine", Color32::from_rgb(0x96, 0xaa, 0xbe)),
                ("Bed", crate::common::theme::BED_EDGE),
                ("Workable", Color32::from_rgb(0x4f, 0xb6, 0xff)),
                ("Home", Color32::from_rgb(0xff, 0xdc, 0x3c)),
            ] {
                let r = painter.text(Pos2::new(x, y), Align2::LEFT_BOTTOM, label, FontId::proportional(11.0), col);
                x = r.right() + 12.0;
            }
            return;
        }
        painter.text(
            rect.left_top() + egui::vec2(10.0, 8.0),
            Align2::LEFT_TOP,
            format!(
                "{}   bed {} x {} mm   X {:.2}  Y {:.2}  Z {:.2}",
                machine.name, bed[0], bed[1], tool[0], tool[1], tool[2]
            ),
            FontId::monospace(12.0),
            Color32::from_rgb(0xb8, 0xc2, 0xcc),
        );
        painter.text(
            rect.left_bottom() + egui::vec2(10.0, -8.0),
            Align2::LEFT_BOTTOM,
            "Left drag: rotate   Right or middle drag: pan   Wheel: zoom",
            FontId::proportional(11.0),
            Color32::from_rgb(0x7a, 0x86, 0x92),
        );
    }
}

/// Left panel of the Run tab: pick the target, start or stop the run, jog by hand.
pub fn run_controls(
    ui: &mut egui::Ui,
    pv: &mut Preview,
    m: &Machine,
    ctl: &crate::common::controls::Controls,
    cmds: &[crate::common::controls::Cmd],
    gcode: &str,
) {
    pv.live.poll();
    crate::common::controls::apply(cmds, pv, m, ctl);
    ui.horizontal(|ui| {
        ui.radio_value(&mut pv.machine_mode, false, "Simulator");
        ui.radio_value(&mut pv.machine_mode, true, "Machine");
    });
    if pv.machine_mode {
        pv.playing = false;
        machine_panel(ui, pv, m, gcode);
        ui.add_space(6.0);
        crate::common::controls::jog_panel(ui, pv, m, ctl);
        return;
    }
    let total = pv.total_time();
    ui.horizontal(|ui| {
        let label = if pv.playing { "Pause" } else if pv.time > 0.0 && pv.time < total { "Resume" } else { "Start" };
        if ui.button(label).clicked() {
            pv.start_pause();
        }
        if ui.button("Stop").clicked() {
            pv.stop();
        }
        egui::ComboBox::from_id_salt("run_speed")
            .width(60.0)
            .selected_text(format!("{}x", SPEEDS[pv.speed_idx]))
            .show_ui(ui, |ui| {
                for (i, s) in SPEEDS.iter().enumerate() {
                    ui.selectable_value(&mut pv.speed_idx, i, format!("{s}x"));
                }
            });
    });
    ui.horizontal(|ui| {
        let status = if pv.playing { "Running" } else if pv.time >= total && total > 0.0 { "Done" } else if pv.time > 0.0 { "Paused" } else { "Idle" };
        ui.label("Status:");
        ui.strong(status);
        ui.monospace(format!("{} / {}", fmt_time(pv.time), fmt_time(total)));
    });
    ui.add_space(6.0);
    crate::common::controls::jog_panel(ui, pv, m, ctl);
}

#[cfg(test)]
mod line_tests {
    use super::*;

    #[test]
    fn current_line_follows_time() {
        let mv = |x: f64, line| Move { from: [x - 1.0, 0.0, 0.0], to: [x, 0.0, 0.0], work: true, line };
        let mut pv = Preview::default();
        pv.set_moves(vec![mv(1.0, 3), mv(2.0, 7)], &JobSettings::default());
        assert_eq!(pv.current_line(), None);
        pv.time = 0.001;
        assert_eq!(pv.current_line(), Some(3));
        pv.time = pv.total_time();
        assert_eq!(pv.current_line(), Some(7));
    }
}

#[cfg(test)]
mod jog_tests {
    use super::*;

    #[test]
    fn jog_clamps_homes_and_is_blocked_while_running() {
        let m = Machine::new("m");
        let mv = Move { from: m.home_position(), to: [10.0, 0.0, m.home_position()[2]], work: true, line: 0 };
        let mut pv = Preview::default();
        pv.set_moves(vec![mv], &JobSettings::default());
        pv.jog_by([-500.0, 5.0, 0.0], &m);
        assert_eq!(pv.tool_now().0[0], 0.0); // clamped to the workable area
        assert_eq!(pv.tool_now().0[1], 5.0);
        pv.jog_home([false, true, false], &m);
        assert_eq!(pv.tool_now().0[1], m.home_position()[1]);
        pv.start_pause();
        assert!(pv.jog.is_none() && pv.playing);
        pv.jog_by([1.0, 0.0, 0.0], &m);
        assert!(pv.jog.is_none());
    }
}

/// Port, connection, start / pause / stop and the console of the real machine.
fn machine_panel(ui: &mut egui::Ui, pv: &mut Preview, m: &Machine, gcode: &str) {
    use crate::common::machine::Firmware;
    let live = &mut pv.live;
    if !live.scanned {
        live.refresh_ports();
    }
    let usable = m.firmware != Firmware::LinuxCnc;
    if !usable {
        ui.label(egui::RichText::new("LinuxCNC has no serial G-code link. Save the G-code and run it from LinuxCNC.").small().weak());
    }
    ui.add_enabled_ui(usable && !live.connected(), |ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("live_port")
                .width(120.0)
                .selected_text(if live.port.is_empty() { "no port".to_string() } else { live.port.clone() })
                .show_ui(ui, |ui| {
                    for p in live.ports.clone() {
                        ui.selectable_value(&mut live.port, p.clone(), p);
                    }
                });
            if ui.button("Refresh").clicked() {
                live.refresh_ports();
            }
            let baud = if live.baud == 0 { m.baud } else { live.baud };
            egui::ComboBox::from_id_salt("live_baud").width(80.0).selected_text(baud.to_string()).show_ui(ui, |ui| {
                for b in [9600, 19200, 38400, 57600, 115_200, 230_400, 250_000, 460_800, 500_000, 1_000_000] {
                    ui.selectable_value(&mut live.baud, b, b.to_string());
                }
            });
        });
    });
    ui.horizontal(|ui| {
        if live.connected() {
            if ui.button("Disconnect").clicked() {
                live.disconnect();
            }
        } else if ui.add_enabled(usable && !live.port.is_empty(), egui::Button::new("Connect")).clicked() {
            let baud = if live.baud == 0 { m.baud } else { live.baud };
            live.connect(baud, m.firmware);
        }
        let state = if live.connected() { live.state.clone() } else { "Disconnected".into() };
        ui.label("Status:");
        ui.strong(if state.is_empty() { "Connected".into() } else { state });
    });
    if let Some(e) = &live.error {
        ui.colored_label(egui::Color32::from_rgb(0xff, 0x6a, 0x5a), e);
    }
    let alarm = live.state.eq_ignore_ascii_case("alarm");
    ui.horizontal(|ui| {
        let ready = live.can_jog() && !gcode.is_empty();
        if live.job {
            if ui.button(if live.paused { "Resume" } else { "Pause" }).clicked() {
                live.pause_resume();
            }
        } else if ui.add_enabled(ready, egui::Button::new("Start")).clicked() {
            live.confirm = true;
        }
        // Always there, also when nothing is running: it also cancels a jog.
        let stop = egui::Button::new(egui::RichText::new("STOP").strong()).fill(egui::Color32::from_rgb(0xb0, 0x2a, 0x20));
        if ui.add_enabled(live.connected(), stop).clicked() {
            live.stop(vec![m.effector.off_line()]);
        }
        if alarm && m.firmware == Firmware::Grbl && ui.button("Unlock").on_hover_text("Send $X to clear the alarm").clicked() {
            live.unlock();
        }
    });
    if live.job {
        let total = crate::common::link::job_lines(gcode).len();
        ui.label(format!("Sending: line {} of {}", live.acked.map_or(0, |n| n + 1), gcode.lines().count().max(total)));
    }
    if live.confirm {
        egui::Window::new("Start the job on the machine?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(format!("{} will move. Make sure the area is clear and the end effector is safe.", m.name));
                ui.horizontal(|ui| {
                    if ui.button("Start job").clicked() {
                        live.confirm = false;
                        live.start_job(gcode);
                    }
                    if ui.button("Cancel").clicked() {
                        live.confirm = false;
                    }
                });
            });
    }
    if live.connected() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Console of the real machine, fixed to the bottom of the left panel.
pub fn console_panel(ui: &mut egui::Ui, pv: &mut Preview) {
    let live = &mut pv.live;
    // A fixed height: a resizable panel grew to fill the whole left panel.
    egui::Panel::bottom("console_bottom").resizable(false).exact_size(200.0).show(ui, |ui| {
        // Built from the bottom up: the input line keeps its place, the log takes the rest.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut live.console).hint_text("G-code").desired_width(ui.available_width() - 50.0));
                let go = ui.button("Send").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if go && live.can_jog() && !live.console.trim().is_empty() {
                    let l = std::mem::take(&mut live.console);
                    live.send_manual(vec![l.trim().to_string()]);
                    r.request_focus();
                }
            });
            let h = (ui.available_height() - 24.0).max(20.0);
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).max_height(h).show(ui, |ui| {
                for l in &live.log {
                    ui.label(egui::RichText::new(l).monospace().small());
                }
            });
            ui.strong("Console");
        });
    });
}
