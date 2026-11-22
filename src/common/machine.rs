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

/// Controller firmware. Decides dwell units and the serial defaults.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Firmware {
    #[default]
    Grbl,
    Marlin,
    LinuxCnc,
}

impl Firmware {
    pub const ALL: [Firmware; 3] = [Firmware::Grbl, Firmware::Marlin, Firmware::LinuxCnc];

    pub fn label(self) -> &'static str {
        match self {
            Firmware::Grbl => "GRBL",
            Firmware::Marlin => "Marlin",
            Firmware::LinuxCnc => "LinuxCNC",
        }
    }

    pub fn default_baud(self) -> u32 {
        match self {
            Firmware::Marlin => 250_000,
            _ => 115_200,
        }
    }

    pub fn default_dwell(self) -> &'static str {
        match self {
            Firmware::Marlin => "G4 P{ms}",
            _ => "G4 P{s}",
        }
    }
}

fn default_baud() -> u32 {
    115_200
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
            EffectorKind::Glue => "Dispenser",
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
    /// Lowest and highest position of each axis. Travel is the difference. When both are
    /// 0 (old files) the legacy lengths below, or the bed size, are used.
    #[serde(default)]
    pub travel_min: [f64; 3],
    #[serde(default)]
    pub travel_max: [f64; 3],
    /// Old files stored only the travel length. Read, never written.
    #[serde(default, skip_serializing)]
    pub travel_x: f64,
    #[serde(default, skip_serializing)]
    pub travel_y: f64,
    #[serde(default, skip_serializing)]
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
    /// Old libraries have no firmware: they load as GRBL.
    #[serde(default)]
    pub firmware: Firmware,
    /// Serial speed used when connecting.
    #[serde(default = "default_baud")]
    pub baud: u32,
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
            travel_min: [0.0; 3],
            travel_max: [150.0, 100.0, 50.0],
            travel_x: 0.0,
            travel_y: 0.0,
            travel_z: 0.0,
            axes: [AxisOwner::EndEffector; 3],
            effector: EndEffector::default(),
            bed_pos: auto3(),
            effector_pos: auto3(),
            home: [None; 3],
            pre_script: default_pre_script(),
            post_script: default_post_script(),
            gcode: GcodeStyle::default(),
            firmware: Firmware::default(),
            baud: default_baud(),
            job: JobSettings::default(),
        }
    }

    /// Switch firmware and apply its defaults (the user can edit them after).
    pub fn set_firmware(&mut self, f: Firmware) {
        self.firmware = f;
        self.baud = f.default_baud();
        self.gcode.dwell = f.default_dwell().into();
    }

    pub fn bed(&self) -> [f64; 3] {
        [self.bed_x, self.bed_y, self.bed_z]
    }

    /// Lowest and highest position of axis `i`.
    pub fn travel_range(&self, i: usize) -> (f64, f64) {
        if self.travel_max[i] > self.travel_min[i] {
            return (self.travel_min[i], self.travel_max[i]);
        }
        let legacy = [self.travel_x, self.travel_y, self.travel_z][i];
        (0.0, if legacy > 0.0 { legacy } else { self.bed()[i] })
    }

    /// Axis travel (highest minus lowest position).
    pub fn travel(&self) -> [f64; 3] {
        [0, 1, 2].map(|i| {
            let (lo, hi) = self.travel_range(i);
            hi - lo
        })
    }

    /// Everything the tool can reach in G-code coordinates, bed or not: the full travel of
    /// each axis. X and Y are placed relative to the bed through the workable area.
    pub fn tool_range(&self) -> [(f64, f64); 3] {
        let (t, w, bp, ep) = (self.travel(), self.workable_on_bed(), self.bed_position(), self.effector_position());
        [0, 1, 2].map(|i| {
            if i == 2 {
                // A tool that rides on Z reaches the box top only above the bed thickness.
                let top = if self.axes[2] == AxisOwner::EndEffector { t[2] - bp[2] } else { t[2] };
                return (0.0, top.max(0.0));
            }
            let lo_bed = match self.axes[i] {
                AxisOwner::Bed => ep[i] - t[i],
                AxisOwner::EndEffector => -bp[i],
            };
            let lo = lo_bed - w[i].0;
            (lo, lo + t[i])
        })
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

#[cfg(test)]
mod firmware_tests {
    use super::*;

    #[test]
    fn old_machine_loads_as_grbl() {
        let mut v = serde_json::to_value(Machine::new("m")).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("firmware");
        o.remove("baud");
        let m: Machine = serde_json::from_value(v).unwrap();
        assert_eq!(m.firmware, Firmware::Grbl);
        assert_eq!(m.baud, 115_200);
    }

    #[test]
    fn firmware_applies_defaults() {
        let mut m = Machine::new("m");
        m.set_firmware(Firmware::Marlin);
        assert_eq!((m.baud, m.gcode.dwell.as_str()), (250_000, "G4 P{ms}"));
        m.set_firmware(Firmware::Grbl);
        assert_eq!((m.baud, m.gcode.dwell.as_str()), (115_200, "G4 P{s}"));
    }
}
