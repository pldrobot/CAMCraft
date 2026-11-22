//! Manual control of the (simulated) machine: jog settings, keyboard and gamepad input, and
//! the jog panel in the Run tab.

use crate::common::machine::Machine;
use crate::common::preview::Preview;
use eframe::egui::{self, Key, RichText};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Act {
    XPlus,
    XMinus,
    YPlus,
    YMinus,
    ZPlus,
    ZMinus,
    StepUp,
    StepDown,
    HomeAll,
    Stop,
    StartPause,
}

/// The six jog actions, in the order `Input` keeps their hold times.
const JOG: [Act; 6] = [Act::XPlus, Act::XMinus, Act::YPlus, Act::YMinus, Act::ZPlus, Act::ZMinus];
const DIRS: [[f64; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

impl Act {
    pub const ALL: [Act; 11] = [
        Act::XPlus,
        Act::XMinus,
        Act::YPlus,
        Act::YMinus,
        Act::ZPlus,
        Act::ZMinus,
        Act::StepUp,
        Act::StepDown,
        Act::HomeAll,
        Act::Stop,
        Act::StartPause,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Act::XPlus => "Jog X+",
            Act::XMinus => "Jog X-",
            Act::YPlus => "Jog Y+",
            Act::YMinus => "Jog Y-",
            Act::ZPlus => "Jog Z+",
            Act::ZMinus => "Jog Z-",
            Act::StepUp => "Step size up",
            Act::StepDown => "Step size down",
            Act::HomeAll => "Home all",
            Act::Stop => "Stop",
            Act::StartPause => "Start / Pause",
        }
    }

    fn default_key(self) -> Key {
        match self {
            Act::XPlus => Key::ArrowRight,
            Act::XMinus => Key::ArrowLeft,
            Act::YPlus => Key::ArrowUp,
            Act::YMinus => Key::ArrowDown,
            Act::ZPlus => Key::PageUp,
            Act::ZMinus => Key::PageDown,
            Act::StepUp => Key::CloseBracket,
            Act::StepDown => Key::OpenBracket,
            Act::HomeAll => Key::H,
            Act::Stop => Key::Escape,
            Act::StartPause => Key::Space,
        }
    }
}

/// How the gamepad jogs.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PadMode {
    /// Sticks jog continuously, faster the further they are pushed. Y / X change the speed.
    #[default]
    Stick,
    /// D-pad and bumpers jog by the step size (hold for continuous). Y / X change the step.
    Buttons,
}

/// Speed levels of the gamepad, as a share of the jog speed.
const PAD_SPEEDS: [f64; 4] = [0.1, 0.25, 0.5, 1.0];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Controls {
    pub pad_mode: PadMode,
    /// Action and the name of its key (`egui::Key::name`).
    pub keys: Vec<(Act, String)>,
    /// Jog step sizes in mm.
    pub steps: Vec<f64>,
    /// Jog step sizes of the Z pad in mm.
    pub steps_z: Vec<f64>,
    /// Jog speed in mm/min, for holding a key and for the stick at full tilt.
    pub feed: f64,
    /// A key held longer than this jogs continuously instead of one step.
    pub long_press_ms: u32,
    /// Stick travel ignored around the centre, 0..1.
    pub dead_zone: f32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            pad_mode: PadMode::default(),
            keys: Act::ALL.iter().map(|a| (*a, a.default_key().name().to_string())).collect(),
            steps: vec![1.0, 10.0, 50.0],
            steps_z: vec![0.1, 1.0, 10.0],
            feed: 1500.0,
            long_press_ms: 300,
            dead_zone: 0.2,
        }
    }
}

impl Controls {
    /// Step sizes of the pad of axis `axis` (0 and 1: X/Y, 2: Z).
    pub fn steps_of(&self, axis: usize) -> &[f64] {
        if axis == 2 { &self.steps_z } else { &self.steps }
    }

    pub fn key(&self, a: Act) -> Option<Key> {
        self.keys.iter().find(|(x, _)| *x == a).and_then(|(_, n)| Key::from_name(n))
    }

    pub fn set_key(&mut self, a: Act, k: Key) {
        match self.keys.iter_mut().find(|(x, _)| *x == a) {
            Some(e) => e.1 = k.name().to_string(),
            None => self.keys.push((a, k.name().to_string())),
        }
    }
}

/// One thing the user asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    /// One step of the selected size along this unit direction.
    Step([f64; 3]),
    /// Continuous jog: this many mm this frame, at this share of the jog speed.
    Run([f64; 3], f64),
    /// The gamepad speed level changed (share of the jog speed).
    PadSpeed(f64),
    HomeAll,
    StepUp,
    StepDown,
    Stop,
    StartPause,
}

/// Reads the keyboard and the first gamepad.
pub struct Input {
    pad: Option<gilrs::Gilrs>,
    hold: [f32; 6],
    hold_pad: [f32; 6],
    /// Gamepad speed level, index into `PAD_SPEEDS`.
    speed_idx: usize,
    announced: bool,
}

impl Default for Input {
    fn default() -> Self {
        // No gamepad backend is fine: the keyboard still works.
        Self { pad: gilrs::Gilrs::new().ok(), hold: [0.0; 6], hold_pad: [0.0; 6], speed_idx: PAD_SPEEDS.len() - 1, announced: false }
    }
}

impl Input {
    pub fn poll(&mut self, ctx: &egui::Context, ctl: &Controls) -> Vec<Cmd> {
        use gilrs::{Axis, Button, EventType};
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let mut cmds = vec![];
        let mut down = [false; 6];
        let mut down_pad = [false; 6];
        if !ctx.egui_wants_keyboard_input() {
            ctx.input(|i| {
                for (n, a) in JOG.iter().enumerate() {
                    down[n] |= ctl.key(*a).is_some_and(|k| i.key_down(k));
                }
                let edge = [
                    (Act::StepUp, Cmd::StepUp),
                    (Act::StepDown, Cmd::StepDown),
                    (Act::HomeAll, Cmd::HomeAll),
                    (Act::Stop, Cmd::Stop),
                    (Act::StartPause, Cmd::StartPause),
                ];
                for (a, c) in edge {
                    let Some(k) = ctl.key(a) else { continue };
                    if i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat: false, .. } if *key == k)) {
                        cmds.push(c);
                    }
                }
            });
        }
        let mut stick = [0.0f32; 3];
        let mut pad_on = false;
        if let Some(g) = &mut self.pad {
            while let Some(ev) = g.next_event() {
                if let EventType::ButtonPressed(b, _) = ev.event {
                    let sticks = ctl.pad_mode == PadMode::Stick;
                    let before = self.speed_idx;
                    match b {
                        Button::South => cmds.push(Cmd::StartPause),
                        Button::East => cmds.push(Cmd::Stop),
                        Button::Start => cmds.push(Cmd::HomeAll),
                        Button::North if sticks => self.speed_idx = (self.speed_idx + 1).min(PAD_SPEEDS.len() - 1),
                        Button::West if sticks => self.speed_idx = self.speed_idx.saturating_sub(1),
                        Button::North => cmds.push(Cmd::StepUp),
                        Button::West => cmds.push(Cmd::StepDown),
                        _ => {}
                    }
                    if self.speed_idx != before {
                        cmds.push(Cmd::PadSpeed(PAD_SPEEDS[self.speed_idx]));
                    }
                }
            }
            if let Some((_, p)) = g.gamepads().find(|(_, p)| p.is_connected()) {
                pad_on = true;
                if !self.announced {
                    self.announced = true;
                    cmds.push(Cmd::PadSpeed(PAD_SPEEDS[self.speed_idx]));
                }
                let btn = [
                    Button::DPadRight,
                    Button::DPadLeft,
                    Button::DPadUp,
                    Button::DPadDown,
                    Button::RightTrigger,
                    Button::LeftTrigger,
                ];
                // One way at a time: the sticks or the buttons.
                if ctl.pad_mode == PadMode::Buttons {
                    for (n, b) in btn.iter().enumerate() {
                        down_pad[n] = p.is_pressed(*b);
                    }
                } else {
                    stick = [p.value(Axis::LeftStickX), p.value(Axis::LeftStickY), p.value(Axis::RightStickY)];
                }
            }
        }
        let long = ctl.long_press_ms as f32 / 1000.0;
        let per_frame = ctl.feed / 60.0 * dt as f64;
        let speed = PAD_SPEEDS[self.speed_idx];
        let mut busy = held(&down, &mut self.hold, (dt, long, per_frame, 1.0), &mut cmds);
        busy |= held(&down_pad, &mut self.hold_pad, (dt, long, per_frame * speed, speed), &mut cmds);
        let dz = ctl.dead_zone.clamp(0.0, 0.9);
        let axis = stick.map(|v| {
            let m = ((v.abs() - dz) / (1.0 - dz)).clamp(0.0, 1.0);
            (m * m).copysign(v) as f64 // quadratic: fine control near the centre
        });
        if axis.iter().any(|v| *v != 0.0) {
            busy = true;
            cmds.push(Cmd::Run(axis.map(|v| v * per_frame * speed), speed));
        }
        if busy {
            ctx.request_repaint();
        } else if pad_on {
            // The pad cannot wake the window, so look again soon.
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
        cmds
    }
}

/// Jog commands for held directions: one step on the first frame, continuous after the hold
/// time. `(dt, hold time, mm per frame, speed share)`. Returns true while something is held.
fn held(down: &[bool; 6], hold: &mut [f32; 6], (dt, long, per_frame, speed): (f32, f32, f64, f64), cmds: &mut Vec<Cmd>) -> bool {
    let mut busy = false;
    for n in 0..6 {
        if !down[n] {
            hold[n] = 0.0;
            continue;
        }
        busy = true;
        if hold[n] == 0.0 {
            cmds.push(Cmd::Step(DIRS[n]));
            hold[n] = f32::EPSILON;
        } else {
            hold[n] += dt;
            if hold[n] > long {
                cmds.push(Cmd::Run(DIRS[n].map(|v| v * per_frame), speed));
            }
        }
    }
    busy
}

/// Do what was asked on the simulated machine. Ignored when the real machine is selected:
/// nothing can be connected yet.
pub fn apply(cmds: &[Cmd], pv: &mut Preview, m: &Machine, ctl: &Controls) {
    for c in cmds {
        if pv.machine_mode {
            apply_machine(*c, pv, m, ctl);
            continue;
        }
        match *c {
            Cmd::Step(d) => {
                let s = step_of(pv, ctl, axis_of(d), pv.step_idx);
                pv.jog_by(d.map(|v| v * s), m);
            }
            Cmd::Run(d, _) => pv.jog_by(d, m),
            Cmd::PadSpeed(s) => pv.pad_speed = Some(s),
            Cmd::HomeAll => pv.jog_home([true; 3], m),
            Cmd::StepUp => pv.step_idx = (pv.step_idx + 1).min(ctl.steps.len()),
            Cmd::StepDown => pv.step_idx = pv.step_idx.saturating_sub(1),
            Cmd::Stop => pv.stop(),
            Cmd::StartPause => pv.start_pause(),
        }
    }
}

/// The axis a unit direction points along.
fn axis_of(d: [f64; 3]) -> usize {
    (0..3).find(|&i| d[i] != 0.0).unwrap_or(0)
}

/// The same requests for the real machine. Start only asks the question; the Run tab starts
/// the job once the user confirms.
fn apply_machine(c: Cmd, pv: &mut Preview, m: &Machine, ctl: &Controls) {
    match c {
        Cmd::Step(d) => {
            let s = step_of(pv, ctl, axis_of(d), pv.step_idx);
            pv.live.jog(m.firmware, d.map(|v| v * s), ctl.feed);
        }
        Cmd::Run(d, mult) => pv.live.jog_run(m.firmware, d, ctl.feed * mult),
        Cmd::PadSpeed(s) => pv.pad_speed = Some(s),
        Cmd::HomeAll => pv.live.home(m.firmware, [true; 3]),
        Cmd::StepUp => pv.step_idx = (pv.step_idx + 1).min(ctl.steps.len()),
        Cmd::StepDown => pv.step_idx = pv.step_idx.saturating_sub(1),
        Cmd::Stop => pv.live.stop(vec![m.effector.off_line()]),
        Cmd::StartPause if pv.live.job => pv.live.pause_resume(),
        Cmd::StartPause if pv.live.can_jog() => pv.live.confirm = true,
        Cmd::StartPause => {}
    }
}

/// What a click on the pads hit.
#[derive(Clone, Copy, PartialEq)]
enum Hit {
    /// Axis 0..2, direction +1 / -1, step band (0 = smallest step).
    Move(usize, f64, usize),
    HomeXY,
    HomeZ,
}

/// Step for ring `band`, or the custom step when that is selected.
fn step_of(pv: &Preview, ctl: &Controls, axis: usize, band: usize) -> f64 {
    let s = ctl.steps_of(axis);
    if pv.step_idx >= ctl.steps.len() || s.is_empty() {
        if axis == 2 { pv.custom_z } else { pv.custom_xy }
    } else {
        s[band.min(s.len() - 1)]
    }
}

fn hit_text(h: Hit, step: f64) -> String {
    match h {
        Hit::Move(a, s, _) => format!("{}{} {} mm", ["X", "Y", "Z"][a], if s > 0.0 { "+" } else { "-" }, step),
        Hit::HomeXY => "Home X and Y".into(),
        Hit::HomeZ => "Home Z".into(),
    }
}

/// Band index from the distance past the centre: the rings get bigger steps going outwards.
fn band(d: f32, from: f32, to: f32, n: usize) -> usize {
    (((d - from) / (to - from) * n as f32).floor().max(0.0) as usize).min(n - 1)
}

fn arrow(p: &egui::Painter, c: egui::Pos2, dir: egui::Vec2, color: egui::Color32) {
    let (f, s) = (dir * 5.0, egui::vec2(-dir.y, dir.x) * 5.0);
    p.add(egui::Shape::convex_polygon(vec![c + f, c - f + s, c - f - s], color, egui::Stroke::NONE));
}

/// The rounded square of the Repetier pad: |x|^4 + |y|^4 = a^4. Point at angle `t` for size `a`.
fn squircle(a: f32, t: f32) -> egui::Vec2 {
    let (s, c) = t.sin_cos();
    let r = a / (c.powi(4) + s.powi(4)).powf(0.25);
    egui::vec2(r * c, r * s)
}

/// Repetier style pads: a rounded cross for X/Y and a pill for Z. Every direction has one ring
/// per step, the biggest step outermost. `steps` are the step sizes shown in the tooltip.
/// Returns what was clicked.
fn pads(ui: &mut egui::Ui, steps: &[f64], steps_z: &[f64], custom: Option<(f64, f64)>, enabled: bool) -> Option<Hit> {
    let n = steps.len().max(1);
    let nz = steps_z.len().max(1);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let v = ui.visuals().clone();
    let dim = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let (idle, hot, line) = (dim(v.widgets.inactive.bg_fill), v.widgets.hovered.bg_fill, dim(v.widgets.noninteractive.fg_stroke.color));
    let mark = dim(v.widgets.inactive.fg_stroke.color);
    let thin = egui::Stroke::new(1.0, line);
    let thick = egui::Stroke::new(2.5, line);
    let tip = |h: Hit| {
        match h {
            Hit::Move(a, _, b) => hit_text(h, custom.map_or(if a == 2 { steps_z[b] } else { steps[b] }, |c| if a == 2 { c.1 } else { c.0 })),
            _ => hit_text(h, 0.0),
        }
    };
    let mut clicked = None;
    ui.horizontal(|ui| {
        // Centered in the panel, with a gap between the two pads.
        let total = 156.0 + 26.0 + 60.0;
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        // ---- X/Y: rounded square, diagonals, one ring per step ----
        let size = 156.0;
        let (resp, p) = ui.allocate_painter(egui::vec2(size, size), sense);
        let (c, r) = (resp.rect.center(), size / 2.0 - 2.0);
        let r0 = r * 0.27;
        // Distance in the rounded-square metric, so the rings follow its shape.
        let metric = |d: egui::Vec2| (d.x.powi(4) + d.y.powi(4)).powf(0.25);
        let hit = resp.hover_pos().and_then(|m| {
            let d = m - c;
            if d.length() <= r0 {
                Some(Hit::HomeXY)
            } else if metric(d) > r {
                None
            } else {
                let b = band(metric(d), r0, r, n);
                Some(if d.x.abs() > d.y.abs() { Hit::Move(0, d.x.signum() as f64, b) } else { Hit::Move(1, -d.y.signum() as f64, b) })
            }
        });
        let hit = hit.filter(|_| enabled);
        // (axis, sign) of the four sectors, with how (u outwards, v sideways) maps to the screen.
        let sectors: [(usize, f64, fn(egui::Vec2) -> egui::Vec2); 4] = [
            (0, 1.0, |p| egui::vec2(p.x, p.y)),
            (0, -1.0, |p| egui::vec2(-p.x, p.y)),
            (1, 1.0, |p| egui::vec2(p.y, -p.x)),
            (1, -1.0, |p| egui::vec2(p.y, p.x)),
        ];
        const STEPS: usize = 14;
        let quarter = std::f32::consts::FRAC_PI_4;
        for (axis, sign, map) in sectors {
            for k in 0..n {
                // The first ring starts under the centre button, so no gap shows.
                let ra = if k == 0 { r0 * 0.7 } else { r0 + (r - r0) * k as f32 / n as f32 };
                let rb = r0 + (r - r0) * (k + 1) as f32 / n as f32;
                let col = if hit == Some(Hit::Move(axis, sign, k)) { hot } else { idle };
                let mut mesh = egui::Mesh::default();
                for i in 0..=STEPS {
                    let t = -quarter + 2.0 * quarter * i as f32 / STEPS as f32;
                    mesh.colored_vertex(c + map(squircle(ra, t)), col);
                    mesh.colored_vertex(c + map(squircle(rb, t)), col);
                }
                for i in 0..STEPS as u32 {
                    let b = i * 2;
                    mesh.add_triangle(b, b + 1, b + 2);
                    mesh.add_triangle(b + 1, b + 3, b + 2);
                }
                p.add(egui::Shape::mesh(mesh));
                if k > 0 {
                    let pts = (0..=STEPS).map(|i| c + map(squircle(ra, -quarter + 2.0 * quarter * i as f32 / STEPS as f32))).collect();
                    p.add(egui::Shape::line(pts, thin));
                }
            }
            let mid = r0 + (r - r0) * (n as f32 / 2.0) / n as f32;
            arrow(&p, c + map(egui::vec2(mid, 0.0)), map(egui::vec2(1.0, 0.0)), mark);
        }
        // The four diagonals between the sectors.
        let (d0, d1) = (squircle(r0, quarter).x, squircle(r, quarter).x);
        for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            p.line_segment([c + egui::vec2(sx * d0, sy * d0), c + egui::vec2(sx * d1, sy * d1)], thin);
        }
        let outline = (0..=4 * STEPS).map(|i| c + squircle(r, std::f32::consts::TAU * i as f32 / (4 * STEPS) as f32)).collect();
        p.add(egui::Shape::closed_line(outline, thick));
        p.circle(c, r0, if hit == Some(Hit::HomeXY) { hot } else { idle }, thick);
        p.text(c, egui::Align2::CENTER_CENTER, "X/Y", egui::FontId::proportional(13.0), mark);
        if resp.clicked() {
            clicked = hit;
        }
        if let Some(h) = hit {
            resp.on_hover_text_at_pointer(tip(h));
        }

        // ---- Z: a pill with the centre button between the up and down rings ----
        ui.add_space(26.0 - ui.spacing().item_spacing.x);
        let (w, mid_h) = (60.0, r0 * 2.0);
        let (resp, p) = ui.allocate_painter(egui::vec2(w, size), sense);
        let rect = resp.rect.shrink(2.0);
        let bh = (rect.height() - mid_h) / (2 * nz) as f32;
        let (top, bottom) = (rect.top() + bh * nz as f32, rect.bottom() - bh * nz as f32);
        let hit = resp.hover_pos().and_then(|m| {
            if !rect.contains(m) {
                None
            } else if m.y < top {
                Some(Hit::Move(2, 1.0, nz - 1 - band(m.y, rect.top(), top, nz)))
            } else if m.y > bottom {
                Some(Hit::Move(2, -1.0, band(m.y, bottom, rect.bottom(), nz)))
            } else {
                Some(Hit::HomeZ)
            }
        });
        let hit = hit.filter(|_| enabled);
        let pill = |p: &egui::Painter, fill: egui::Color32| p.rect_filled(rect, rect.width() / 2.0, fill);
        pill(&p, idle);
        // A highlighted cell is the pill painted again, cut to the cell, so the ends stay round.
        let cell_of = |y0: f32, y1: f32| egui::Rect::from_min_max(egui::pos2(rect.left(), y0), egui::pos2(rect.right(), y1));
        if let Some(h) = hit {
            let cell = match h {
                Hit::Move(_, s, b) if s > 0.0 => {
                    let i = nz - 1 - b;
                    cell_of(rect.top() + bh * i as f32, rect.top() + bh * (i + 1) as f32)
                }
                Hit::Move(_, _, b) => cell_of(bottom + bh * b as f32, bottom + bh * (b + 1) as f32),
                _ => cell_of(top, bottom),
            };
            pill(&p.with_clip_rect(cell), hot);
        }
        for i in 1..nz {
            for y in [rect.top() + bh * i as f32, bottom + bh * i as f32] {
                p.line_segment([egui::pos2(rect.left() + 3.0, y), egui::pos2(rect.right() - 3.0, y)], thin);
            }
        }
        for y in [top, bottom] {
            p.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], thick);
        }
        p.rect_stroke(rect, rect.width() / 2.0, thick, egui::StrokeKind::Inside);
        p.text(egui::pos2(rect.center().x, (top + bottom) / 2.0), egui::Align2::CENTER_CENTER, "Z", egui::FontId::proportional(13.0), mark);
        arrow(&p, egui::pos2(rect.center().x, rect.top() + bh * nz as f32 / 2.0), egui::vec2(0.0, -1.0), mark);
        arrow(&p, egui::pos2(rect.center().x, rect.bottom() - bh * nz as f32 / 2.0), egui::vec2(0.0, 1.0), mark);
        if resp.clicked() {
            clicked = hit;
        }
        if let Some(h) = hit {
            resp.on_hover_text_at_pointer(tip(h));
        }
    });
    clicked
}

/// Home the axes on the simulator or the real machine. GRBL can only home all axes at once.
fn home(pv: &mut Preview, m: &Machine, axes: [bool; 3]) {
    if !pv.machine_mode {
        pv.jog_home(axes, m);
    } else if m.firmware != crate::common::machine::Firmware::Grbl || axes == [true; 3] {
        pv.live.home(m.firmware, axes);
    }
}

/// Jog pads, step size for the keys and home buttons.
pub fn jog_panel(ui: &mut egui::Ui, pv: &mut Preview, m: &Machine, ctl: &Controls) {
    let enabled = if pv.machine_mode { pv.live.can_jog() } else { !pv.playing && pv.time <= 0.0 };
    ui.strong("Manual control");
    ui.add_enabled_ui(enabled, |ui| {
        let custom = (pv.step_idx >= ctl.steps.len()).then_some((pv.custom_xy, pv.custom_z));
        match pads(ui, &ctl.steps, &ctl.steps_z, custom, enabled) {
            Some(Hit::Move(a, s, b)) => {
                let mut d = [0.0; 3];
                d[a] = s * step_of(pv, ctl, a, b);
                if pv.machine_mode {
                    pv.live.jog(m.firmware, d, ctl.feed);
                } else {
                    pv.jog_by(d, m);
                }
            }
            Some(Hit::HomeXY) => home(pv, m, [true, true, false]),
            Some(Hit::HomeZ) => home(pv, m, [false, false, true]),
            None => {}
        }
        let list = |v: &[f64]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" / ");
        let text = if custom.is_some() {
            "Every ring moves the custom step.".to_string()
        } else {
            format!("Rings from the centre, X/Y: {} mm, Z: {} mm.", list(&ctl.steps), list(&ctl.steps_z))
        };
        ui.label(RichText::new(format!("{text} Click the middle to home.")).small().weak());
        // Which step the keys use, or one size for all rings and keys.
        pv.step_idx = pv.step_idx.min(ctl.steps.len());
        if let Some(s) = pv.pad_speed {
            ui.label(RichText::new(format!("Gamepad speed {:.0}%", s * 100.0)).small().weak());
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Keys step");
            for (i, s) in ctl.steps.iter().enumerate() {
                let z = ctl.steps_z.get(i).map(|z| format!(" / {z}")).unwrap_or_default();
                if ui.selectable_label(pv.step_idx == i, format!("{s}{z}")).on_hover_text("X/Y step / Z step").clicked() {
                    pv.step_idx = i;
                }
            }
            if ui.selectable_label(pv.step_idx == ctl.steps.len(), "Custom").clicked() {
                pv.step_idx = ctl.steps.len();
            }
            let mut edited = false;
            ui.label("XY");
            edited |= ui.add(egui::DragValue::new(&mut pv.custom_xy).speed(0.1).range(0.001..=1000.0).suffix(" mm")).changed();
            ui.label("Z");
            edited |= ui.add(egui::DragValue::new(&mut pv.custom_z).speed(0.1).range(0.001..=1000.0).suffix(" mm")).changed();
            if edited {
                pv.step_idx = ctl.steps.len();
            }
        });
        ui.horizontal(|ui| {
            // GRBL homes all axes at once ($H); a machine can only be asked per axis on Marlin.
            let single = !pv.machine_mode || m.firmware != crate::common::machine::Firmware::Grbl;
            ui.add_enabled_ui(single, |ui| {
                for (i, n) in ["Home X", "Home Y", "Home Z"].iter().enumerate() {
                    if ui.button(*n).clicked() {
                        home(pv, m, [i == 0, i == 1, i == 2]);
                    }
                }
            });
            if ui.button("Home all").clicked() {
                home(pv, m, [true; 3]);
            }
        });
    });
    let (p, _) = pv.tool_now();
    ui.monospace(format!("X {:8.2}  Y {:8.2}  Z {:8.2}", p[0], p[1], p[2]));
}
