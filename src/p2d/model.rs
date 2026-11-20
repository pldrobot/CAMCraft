use crate::common::machine::*;
use serde::{Deserialize, Serialize};

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
