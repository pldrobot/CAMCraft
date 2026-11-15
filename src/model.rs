use serde::{Deserialize, Serialize};

pub type Pt = [f64; 2];

/// Per-job values. A flavor carries defaults, each project keeps its own copy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct JobSettings {
    pub dispense_feed: f64,
    pub travel_feed: f64,
    pub use_z: bool,
    pub safe_z: f64,
    pub dispense_z: f64,
    /// Wait after valve on, before moving (glue pressure build-up).
    pub pre_flow_ms: u32,
    /// Wait after valve off, before lifting (pressure release).
    pub post_flow_ms: u32,
}

impl Default for JobSettings {
    fn default() -> Self {
        Self {
            dispense_feed: 300.0,
            travel_feed: 3000.0,
            use_z: true,
            safe_z: 10.0,
            dispense_z: 2.0,
            pre_flow_ms: 200,
            post_flow_ms: 100,
        }
    }
}

/// A G-code dialect. Everything is plain text so it can be edited in the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Flavor {
    pub name: String,
    /// Lines written at the start (units, absolute mode, homing...).
    pub header: String,
    /// Lines written at the end.
    pub footer: String,
    pub valve_on: String,
    pub valve_off: String,
    /// Dwell template. `{s}` is seconds, `{ms}` is milliseconds.
    pub dwell: String,
    pub rapid_cmd: String,
    pub linear_cmd: String,
    /// Add an F word to rapid moves (Marlin needs it, GRBL ignores it).
    pub rapid_with_feed: bool,
    pub decimals: usize,
    pub comments: bool,
    /// Write G2/G3 for arcs instead of many short G1 lines.
    #[serde(default)]
    pub arcs: bool,
    #[serde(default = "default_cw")]
    pub cw_cmd: String,
    #[serde(default = "default_ccw")]
    pub ccw_cmd: String,
    pub defaults: JobSettings,
}

fn default_cw() -> String {
    "G2".into()
}

fn default_ccw() -> String {
    "G3".into()
}

impl Flavor {
    pub fn grbl() -> Self {
        Self {
            name: "GRBL".into(),
            header: "G21\nG90\nG94".into(),
            footer: "M30".into(),
            valve_on: "M8".into(),
            valve_off: "M9".into(),
            dwell: "G4 P{s}".into(),
            rapid_cmd: "G0".into(),
            linear_cmd: "G1".into(),
            rapid_with_feed: false,
            decimals: 3,
            comments: true,
            arcs: false,
            cw_cmd: default_cw(),
            ccw_cmd: default_ccw(),
            defaults: JobSettings::default(),
        }
    }

    pub fn marlin() -> Self {
        Self {
            name: "Marlin".into(),
            header: "G21\nG90\nG92 E0".into(),
            footer: "M84".into(),
            valve_on: "M106 S255".into(),
            valve_off: "M107".into(),
            dwell: "G4 P{ms}".into(),
            rapid_cmd: "G0".into(),
            linear_cmd: "G1".into(),
            rapid_with_feed: true,
            decimals: 3,
            comments: true,
            arcs: false,
            cw_cmd: default_cw(),
            ccw_cmd: default_ccw(),
            defaults: JobSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    pub size_x: f64,
    pub size_y: f64,
    pub size_z: f64,
    pub flavor: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Library {
    pub machines: Vec<Machine>,
    pub flavors: Vec<Flavor>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            machines: vec![],
            flavors: vec![Flavor::grbl(), Flavor::marlin()],
        }
    }
}

impl Library {
    pub fn flavor(&self, name: &str) -> Option<&Flavor> {
        self.flavors.iter().find(|f| f.name == name)
    }

    fn path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("CAMCraft").join("library.json"))
    }

    pub fn load() -> Self {
        let Some(p) = Self::path() else {
            return Self::default();
        };
        let mut lib: Library = std::fs::read_to_string(p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if lib.flavors.is_empty() {
            lib.flavors = Library::default().flavors;
        }
        lib
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub paths: Vec<PathData>,
    /// Position of the DXF origin on the bed, in mm.
    pub offset: Pt,
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SeqItem {
    pub part: usize,
    pub path: usize,
    pub reverse: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub machine: Machine,
    pub flavor: String,
    pub job: JobSettings,
    pub parts: Vec<Part>,
    pub sequence: Vec<SeqItem>,
}

impl Project {
    pub fn new(machine: Machine, flavor: &Flavor) -> Self {
        Self {
            flavor: flavor.name.clone(),
            job: flavor.defaults.clone(),
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
        self.sequence.retain(|s| s.part != idx);
        for s in &mut self.sequence {
            if s.part > idx {
                s.part -= 1;
            }
        }
    }

    pub fn seq_index(&self, part: usize, path: usize) -> Option<usize> {
        self.sequence
            .iter()
            .position(|s| s.part == part && s.path == path)
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
