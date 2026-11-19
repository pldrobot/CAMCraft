use serde::{Deserialize, Serialize};

pub type Pt = [f64; 2];

/// Per-job values. A machine carries defaults, each project keeps its own copy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct JobSettings {
    pub dispense_feed: f64,
    pub travel_feed: f64,
    pub use_z: bool,
    pub safe_z: f64,
    pub dispense_z: f64,
    /// Wait after the end effector is switched on, before moving.
    #[serde(alias = "pre_flow_ms")]
    pub on_delay_ms: u32,
    /// Wait after the end effector is switched off, before lifting.
    #[serde(alias = "post_flow_ms")]
    pub off_delay_ms: u32,
}

impl Default for JobSettings {
    fn default() -> Self {
        Self {
            dispense_feed: 300.0,
            travel_feed: 3000.0,
            use_z: true,
            safe_z: 10.0,
            dispense_z: 2.0,
            on_delay_ms: 200,
            off_delay_ms: 100,
        }
    }
}

/// How G-code is written. Plain text so it can be edited in the UI.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GcodeStyle {
    /// Dwell template. `{s}` is seconds, `{ms}` is milliseconds.
    pub dwell: String,
    pub rapid_cmd: String,
    pub linear_cmd: String,
    /// F word on every G0-G3 line (rapids use the travel feed). Off: only
    /// where the feed changes.
    pub feed_every_move: bool,
    pub decimals: usize,
    pub comments: bool,
    /// Write G2/G3 for arcs instead of many short G1 lines.
    pub arcs: bool,
    pub cw_cmd: String,
    pub ccw_cmd: String,
}

impl Default for GcodeStyle {
    fn default() -> Self {
        Self {
            dwell: "G4 P{s}".into(),
            rapid_cmd: "G0".into(),
            linear_cmd: "G1".into(),
            feed_every_move: true,
            decimals: 3,
            comments: true,
            arcs: false,
            cw_cmd: "G2".into(),
            ccw_cmd: "G3".into(),
        }
    }
}

fn default_pre_script() -> String {
    "G21\nG90\nG94".into()
}

fn default_post_script() -> String {
    "M30".into()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum EffectorKind {
    Motor,
    Laser,
    #[default]
    Glue,
    /// Only used for the simulation drawing.
    Other,
}

impl EffectorKind {
    pub const ALL: [EffectorKind; 4] =
        [EffectorKind::Motor, EffectorKind::Laser, EffectorKind::Glue, EffectorKind::Other];

    pub fn label(self) -> &'static str {
        match self {
            EffectorKind::Motor => "Motor",
            EffectorKind::Laser => "Laser",
            EffectorKind::Glue => "Glue",
            EffectorKind::Other => "Other",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ControlType {
    /// Two commands: on and off.
    #[default]
    Binary,
    /// One command template with a value; off is the minimum, on is the maximum.
    Pwm,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EndEffector {
    pub kind: EffectorKind,
    pub control: ControlType,
    pub on_cmd: String,
    pub off_cmd: String,
    /// PWM command template. `{v}` is replaced by the value.
    pub pwm_cmd: String,
    /// Value written for "off".
    pub pwm_min: f64,
    /// Value written for "on".
    pub pwm_max: f64,
}

impl Default for EndEffector {
    fn default() -> Self {
        Self {
            kind: EffectorKind::Glue,
            control: ControlType::Binary,
            on_cmd: "M8".into(),
            off_cmd: "M9".into(),
            pwm_cmd: "M3 S{v}".into(),
            pwm_min: 0.0,
            pwm_max: 1000.0,
        }
    }
}

impl EndEffector {
    pub fn on_line(&self) -> String {
        match self.control {
            ControlType::Binary => self.on_cmd.clone(),
            ControlType::Pwm => self.pwm_line(self.pwm_max),
        }
    }

    pub fn off_line(&self) -> String {
        match self.control {
            ControlType::Binary => self.off_cmd.clone(),
            ControlType::Pwm => self.pwm_line(self.pwm_min),
        }
    }

    fn pwm_line(&self, v: f64) -> String {
        let mut t = format!("{:.3}", v);
        while t.ends_with('0') {
            t.pop();
        }
        if t.ends_with('.') {
            t.pop();
        }
        self.pwm_cmd.replace("{v}", &t)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AxisOwner {
    /// The axis moves the end effector.
    #[default]
    EndEffector,
    /// The axis moves the bed.
    Bed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    /// Usable bed size. Old files call these size_x/y/z.
    #[serde(alias = "size_x")]
    pub bed_x: f64,
    #[serde(alias = "size_y")]
    pub bed_y: f64,
    #[serde(alias = "size_z")]
    pub bed_z: f64,
    /// Travel of each axis. 0 means "same as the bed" (old files).
    #[serde(default)]
    pub travel_x: f64,
    #[serde(default)]
    pub travel_y: f64,
    #[serde(default)]
    pub travel_z: f64,
    /// Which part each axis (X, Y, Z) moves: the end effector or the bed.
    #[serde(default)]
    pub axes: [AxisOwner; 3],
    #[serde(default)]
    pub effector: EndEffector,
    /// Fixed bed position (see `bed_position`). Negative = automatic.
    #[serde(default = "auto3")]
    pub bed_pos: [f64; 3],
    /// Fixed end effector position (see `effector_position`). Negative = automatic.
    #[serde(default = "auto3")]
    pub effector_pos: [f64; 3],
    /// Homing position of the end effector in G-code coordinates. `None` is
    /// automatic: work zero on X and Y, the top of the travel on Z.
    #[serde(default)]
    pub home: [Option<f64>; 3],
    /// Lines written at the start of the G-code (units, absolute mode, homing...).
    #[serde(default = "default_pre_script")]
    pub pre_script: String,
    /// Lines written at the end of the G-code.
    #[serde(default = "default_post_script")]
    pub post_script: String,
    #[serde(default)]
    pub gcode: GcodeStyle,
    /// Speeds and other defaults for new projects.
    #[serde(default)]
    pub job: JobSettings,
}

fn auto3() -> [f64; 3] {
    [-1.0; 3]
}

impl Machine {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            bed_x: 150.0,
            bed_y: 100.0,
            bed_z: 20.0,
            travel_x: 150.0,
            travel_y: 100.0,
            travel_z: 50.0,
            axes: [AxisOwner::EndEffector; 3],
            effector: EndEffector::default(),
            bed_pos: auto3(),
            effector_pos: auto3(),
            home: [None; 3],
            pre_script: default_pre_script(),
            post_script: default_post_script(),
            gcode: GcodeStyle::default(),
            job: JobSettings::default(),
        }
    }

    pub fn bed(&self) -> [f64; 3] {
        [self.bed_x, self.bed_y, self.bed_z]
    }

    /// Axis travel, falling back to the bed size when not set.
    pub fn travel(&self) -> [f64; 3] {
        let t = [self.travel_x, self.travel_y, self.travel_z];
        let b = self.bed();
        [0, 1, 2].map(|i| if t[i] > 0.0 { t[i] } else { b[i] })
    }

    /// Where the end effector sits after homing, in G-code coordinates.
    pub fn home_position(&self) -> [f64; 3] {
        let t = self.travel();
        [self.home[0].unwrap_or(0.0), self.home[1].unwrap_or(0.0), self.home[2].unwrap_or(t[2])]
    }

    /// Size of the machine box. An axis that moves the bed needs the bed plus
    /// the travel; an axis that moves the end effector needs only the travel.
    pub fn frame_size(&self) -> [f64; 3] {
        let (t, b) = (self.travel(), self.bed());
        [0, 1, 2].map(|i| match self.axes[i] {
            AxisOwner::Bed => b[i] + t[i],
            AxisOwner::EndEffector => t[i],
        })
    }

    /// Valid range for the end effector position of a bed axis.
    pub fn effector_range(&self, i: usize) -> (f64, f64) {
        let (t, b) = (self.travel(), self.bed());
        (0.0, b[i] + t[i])
    }

    /// Valid range for the bed position of an end effector axis.
    pub fn bed_range(&self, i: usize) -> (f64, f64) {
        (0.0, self.travel()[i])
    }

    /// Fixed bed position, measured from the machine box corner. Only an axis
    /// that moves the end effector has one (a bed axis sweeps the bed instead,
    /// so 0 is returned). Z: the bed top sits at the bed thickness.
    pub fn bed_position(&self) -> [f64; 3] {
        let b = self.bed();
        [0, 1, 2].map(|i| match (self.axes[i], i) {
            (AxisOwner::Bed, _) => 0.0,
            (AxisOwner::EndEffector, 2) => b[2],
            (AxisOwner::EndEffector, _) => {
                let (lo, hi) = self.bed_range(i);
                let v = if self.bed_pos[i] >= 0.0 { self.bed_pos[i] } else { 0.0 };
                v.clamp(lo, hi)
            }
        })
    }

    /// Fixed end effector position, measured from the machine box corner. Only
    /// an axis that moves the bed has one (an end effector axis sweeps the tool
    /// from 0, so 0 is returned). Z: the tool is at the top of the box.
    pub fn effector_position(&self) -> [f64; 3] {
        let (t, b) = (self.travel(), self.bed());
        [0, 1, 2].map(|i| match (self.axes[i], i) {
            (AxisOwner::EndEffector, _) => 0.0,
            (AxisOwner::Bed, 2) => b[2] + t[2],
            (AxisOwner::Bed, _) => {
                let (lo, hi) = self.effector_range(i);
                let v = if self.effector_pos[i] >= 0.0 { self.effector_pos[i] } else { t[i] };
                v.clamp(lo, hi)
            }
        })
    }

    /// The part of the bed the end effector can reach, as (low, high) in bed
    /// coordinates (0 at the bed corner). Z is height above the bed top.
    /// Never larger than the bed or the travel.
    /// - Bed axis: the tool is fixed at e and the bed corner sweeps 0..travel,
    ///   so the tool reaches bed points e - travel .. e.
    /// - End effector axis: the tool sweeps 0..travel over a bed at bp, so it
    ///   reaches bed points -bp .. travel - bp.
    pub fn workable_on_bed(&self) -> [(f64, f64); 3] {
        let (t, b) = (self.travel(), self.bed());
        let (bp, ep) = (self.bed_position(), self.effector_position());
        [0, 1, 2].map(|i| {
            if i == 2 {
                return (0.0, t[2]);
            }
            match self.axes[i] {
                AxisOwner::Bed => ((ep[i] - t[i]).max(0.0), b[i].min(ep[i])),
                AxisOwner::EndEffector => ((-bp[i]).max(0.0), b[i].min(t[i] - bp[i])),
            }
        })
    }

    /// Workable range in G-code coordinates. Work zero is the lowest point the
    /// end effector can reach on the bed, so every range starts at 0.
    pub fn workable(&self) -> [(f64, f64); 3] {
        self.workable_on_bed().map(|(lo, hi)| (0.0, hi - lo))
    }

    /// The bed rectangle in G-code X/Y coordinates.
    pub fn bed_in_gcode(&self) -> [(f64, f64); 2] {
        let (w, b) = (self.workable_on_bed(), self.bed());
        [0, 1].map(|i| (-w[i].0, b[i] - w[i].0))
    }

    /// G-code X/Y/Z to bed coordinates.
    pub fn gcode_to_bed(&self, g: [f64; 3]) -> [f64; 3] {
        let w = self.workable_on_bed();
        [g[0] + w[0].0, g[1] + w[1].0, g[2]]
    }

    pub fn in_workable(&self, p: Pt) -> bool {
        let w = self.workable();
        let e = 1e-6;
        p[0] >= w[0].0 - e && p[0] <= w[0].1 + e && p[1] >= w[1].0 - e && p[1] <= w[1].1 + e
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Library {
    pub machines: Vec<Machine>,
}

impl Default for Library {
    fn default() -> Self {
        Self { machines: vec![] }
    }
}

impl Library {
    fn path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("CAMCraft").join("library.json"))
    }

    pub fn load() -> Self {
        let Some(p) = Self::path() else {
            return Self::default();
        };
        std::fs::read_to_string(p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = Self::path().ok_or("no config directory")?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(p, s).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathData {
    pub pts: Vec<Pt>,
    pub closed: bool,
    /// Stretches of `pts` that are really circular arcs (already flattened
    /// into `pts`, so drawing and line output keep working).
    #[serde(default)]
    pub arcs: Vec<ArcSpan>,
}

/// `pts[start..=end]` lie on a circle around `center`. For a closed path,
/// `end == pts.len()` means the arc ends at `pts[0]`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct ArcSpan {
    pub start: usize,
    pub end: usize,
    pub center: Pt,
    /// Clockwise when seen from +Z (G2).
    pub cw: bool,
}

impl ArcSpan {
    /// The same arc when the point list is reversed (`n` points).
    pub fn reversed(&self, n: usize) -> ArcSpan {
        ArcSpan {
            start: n - 1 - self.end,
            end: n - 1 - self.start,
            center: self.center,
            cw: !self.cw,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub paths: Vec<PathData>,
    /// Position of the DXF origin on the bed, in mm.
    pub offset: Pt,
    /// DXF file the part was imported from, used to change its layers.
    #[serde(default)]
    pub source: String,
    /// Layers imported from the source file.
    #[serde(default)]
    pub layers: Vec<String>,
}

impl Part {
    /// Lower-left corner of the drawing's bounding box, in bed coordinates.
    pub fn min_corner(&self) -> Pt {
        let (mut x, mut y) = (f64::MAX, f64::MAX);
        for p in self.paths.iter().flat_map(|p| p.pts.iter()) {
            x = x.min(p[0]);
            y = y.min(p[1]);
        }
        if x == f64::MAX {
            return self.offset;
        }
        [x + self.offset[0], y + self.offset[1]]
    }
}

/// Per-step values that replace the job settings. None keeps the job value.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct JobOverride {
    pub dispense_feed: Option<f64>,
    pub travel_feed: Option<f64>,
    pub safe_z: Option<f64>,
    pub dispense_z: Option<f64>,
    pub on_delay_ms: Option<u32>,
    pub off_delay_ms: Option<u32>,
}

impl JobOverride {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl JobSettings {
    /// The job settings with a step's overrides applied.
    pub fn with(&self, o: &JobOverride) -> JobSettings {
        JobSettings {
            dispense_feed: o.dispense_feed.unwrap_or(self.dispense_feed),
            travel_feed: o.travel_feed.unwrap_or(self.travel_feed),
            use_z: self.use_z,
            safe_z: o.safe_z.unwrap_or(self.safe_z),
            dispense_z: o.dispense_z.unwrap_or(self.dispense_z),
            on_delay_ms: o.on_delay_ms.unwrap_or(self.on_delay_ms),
            off_delay_ms: o.off_delay_ms.unwrap_or(self.off_delay_ms),
        }
    }
}

/// What a step of the sequence does.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum StepKind {
    /// Follow a path of a part.
    #[default]
    Path,
    /// Go to the machine home position.
    Home,
    /// Go to a bed coordinate. Z is left alone when None.
    Goto { x: f64, y: f64, z: Option<f64> },
    /// Wait.
    Pause { ms: u32 },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SeqItem {
    /// Part and path, used by path steps only.
    pub part: usize,
    pub path: usize,
    pub reverse: bool,
    #[serde(default)]
    pub kind: StepKind,
    /// Job setting overrides of this step.
    #[serde(default)]
    pub over: JobOverride,
}

impl SeqItem {
    pub fn path(part: usize, path: usize) -> Self {
        Self { part, path, reverse: false, kind: StepKind::Path, over: JobOverride::default() }
    }

    pub fn other(kind: StepKind) -> Self {
        Self { part: 0, path: 0, reverse: false, kind, over: JobOverride::default() }
    }

    pub fn is_path(&self) -> bool {
        self.kind == StepKind::Path
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub machine: Machine,
    pub job: JobSettings,
    pub parts: Vec<Part>,
    pub sequence: Vec<SeqItem>,
}

impl Project {
    pub fn new(machine: Machine) -> Self {
        Self {
            job: machine.job.clone(),
            machine,
            parts: vec![],
            sequence: vec![],
        }
    }

    pub fn remove_part(&mut self, idx: usize) {
        if idx >= self.parts.len() {
            return;
        }
        self.parts.remove(idx);
        self.sequence.retain(|s| !s.is_path() || s.part != idx);
        for s in &mut self.sequence {
            if s.is_path() && s.part > idx {
                s.part -= 1;
            }
        }
    }

    /// Drop the sequence entries of a part, e.g. after its paths changed.
    pub fn clear_part_sequence(&mut self, part: usize) {
        self.sequence.retain(|s| !s.is_path() || s.part != part);
    }

    pub fn seq_index(&self, part: usize, path: usize) -> Option<usize> {
        self.sequence
            .iter()
            .position(|s| s.is_path() && s.part == part && s.path == path)
    }

    /// Points of a path in bed coordinates, in travel order.
    pub fn world_points(&self, item: &SeqItem) -> Vec<Pt> {
        self.world_path(item).0
    }

    /// Points and arc spans of a path in bed coordinates, in travel order.
    /// Span indices refer to the returned points.
    pub fn world_path(&self, item: &SeqItem) -> (Vec<Pt>, Vec<ArcSpan>) {
        let part = &self.parts[item.part];
        let path = &part.paths[item.path];
        let (ox, oy) = (part.offset[0], part.offset[1]);
        let mut pts: Vec<Pt> = path.pts.iter().map(|p| [p[0] + ox, p[1] + oy]).collect();
        let mut arcs: Vec<ArcSpan> = path
            .arcs
            .iter()
            .map(|a| ArcSpan { center: [a.center[0] + ox, a.center[1] + oy], ..*a })
            .collect();
        if path.closed && !pts.is_empty() {
            pts.push(pts[0]);
        }
        // Drop spans that do not fit the point list (damaged project files).
        arcs.retain(|a| a.start < a.end && a.end < pts.len());
        if item.reverse {
            let n = pts.len();
            pts.reverse();
            arcs = arcs.iter().map(|a| a.reversed(n)).collect();
            arcs.sort_by_key(|a| a.start);
        }
        (pts, arcs)
    }
}
