//! Data of a 3D project and the path maths (surface Z gap, tool compensation).

use super::step::{self, Geometry, V3};
use crate::common::machine::Machine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Job3 {
    pub dispense_feed: f64,
    pub travel_feed: f64,
    /// Default distance between the tool tip and the chosen surface.
    pub z_gap: f64,
    /// Diameter of the tool. Paths are moved sideways by the radius, to the
    /// side each step chooses. 0 turns the offset off.
    pub tool_diameter: f64,
    pub on_delay_ms: u32,
    pub off_delay_ms: u32,
    /// Longest piece of a path when following a surface.
    pub sample_mm: f64,
}

impl Default for Job3 {
    fn default() -> Self {
        Self {
            dispense_feed: 300.0,
            travel_feed: 3000.0,
            z_gap: 1.0,
            tool_diameter: 0.0,
            on_delay_ms: 200,
            off_delay_ms: 100,
            sample_mm: 1.0,
        }
    }
}

impl Job3 {
    /// The job settings with a step's overrides applied.
    pub fn with(&self, o: &Over3) -> Job3 {
        Job3 {
            dispense_feed: o.feed.unwrap_or(self.dispense_feed),
            travel_feed: o.travel_feed.unwrap_or(self.travel_feed),
            z_gap: o.z_gap.unwrap_or(self.z_gap),
            tool_diameter: o.tool_diameter.unwrap_or(self.tool_diameter),
            on_delay_ms: o.on_delay_ms.unwrap_or(self.on_delay_ms),
            off_delay_ms: o.off_delay_ms.unwrap_or(self.off_delay_ms),
            sample_mm: self.sample_mm,
        }
    }
}

/// Per-step values that replace the job settings. None keeps the job value.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Over3 {
    pub feed: Option<f64>,
    pub travel_feed: Option<f64>,
    pub z_gap: Option<f64>,
    pub tool_diameter: Option<f64>,
    pub on_delay_ms: Option<u32>,
    pub off_delay_ms: Option<u32>,
}

impl Over3 {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Kind3 {
    /// Follow an edge of a part. The edge is picked after the step is added.
    Path,
    Home,
    Goto { x: f64, y: f64, z: f64 },
    Pause { ms: u32 },
}

/// Where a path starts or stops: a point, or where it comes closest to an edge
/// or reaches a surface.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Limit {
    Point(V3),
    Edge(usize, usize),
    /// Where the path first reaches the surface (part, face).
    Surface(usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Step3 {
    pub kind: Kind3,
    pub reverse: bool,
    /// Edge (part, edge) to follow.
    #[serde(default)]
    pub path: Option<(usize, usize)>,
    /// Surface (part, face) the Z gap is measured from. None follows the edge.
    #[serde(default)]
    pub surface: Option<(usize, usize)>,
    /// Optional start and stop, along the travel direction.
    #[serde(default)]
    pub start: Option<Limit>,
    #[serde(default)]
    pub stop: Option<Limit>,
    /// Distance from the start / stop element, along the path. None is the
    /// tool offset (half the tool diameter). Inward shortens the path from
    /// that end; the flag turns it outward, which lengthens it.
    #[serde(default)]
    pub start_offset: Option<f64>,
    #[serde(default)]
    pub start_flip: bool,
    #[serde(default)]
    pub stop_offset: Option<f64>,
    #[serde(default)]
    pub stop_flip: bool,
    /// Distance of the path from the edge, away from the surface (the middle
    /// of the part when there is none). None is the tool offset. The flag
    /// turns it toward the surface.
    #[serde(default)]
    pub offset: Option<f64>,
    #[serde(default)]
    pub offset_flip: bool,
    /// Lead in: the way from the end of the previous step to the start of this
    /// path, following `lead_surfaces` where it passes over them.
    #[serde(default)]
    pub lead_in: bool,
    #[serde(default)]
    pub lead_surfaces: Vec<(usize, usize)>,
    /// How far the lead in keeps from those surfaces. None is the tool offset
    /// (half the tool diameter).
    #[serde(default)]
    pub lead_offset: Option<f64>,
    /// Keep to the other side of those surfaces than the lead in is on.
    #[serde(default)]
    pub lead_flip: bool,
    #[serde(default)]
    pub over: Over3,
    /// Row expanded in the list. New steps start expanded; opened projects collapsed.
    #[serde(skip)]
    pub open: bool,
}

impl Step3 {
    pub fn new(kind: Kind3) -> Self {
        Self {
            kind,
            reverse: false,
            path: None,
            surface: None,
            start: None,
            stop: None,
            start_offset: None,
            start_flip: false,
            stop_offset: None,
            stop_flip: false,
            offset: None,
            offset_flip: false,
            lead_in: true,
            lead_surfaces: vec![],
            lead_offset: None,
            lead_flip: false,
            over: Over3::default(),
            open: true,
        }
    }

    pub fn is_path(&self) -> bool {
        self.kind == Kind3::Path
    }
}

/// Rotation matrix, row major.
pub type Rot = [f64; 9];

pub const NO_ROTATION: Rot = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

fn no_rotation() -> Rot {
    NO_ROTATION
}

fn rot_mul(a: &Rot, b: &Rot) -> Rot {
    let mut r = [0.0; 9];
    for i in 0..3 {
        for j in 0..3 {
            r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
        }
    }
    r
}

fn rot_apply(r: &Rot, v: V3) -> V3 {
    [
        r[0] * v[0] + r[1] * v[1] + r[2] * v[2],
        r[3] * v[0] + r[4] * v[1] + r[5] * v[2],
        r[6] * v[0] + r[7] * v[1] + r[8] * v[2],
    ]
}

/// Rotation by `angle` radians about a unit axis.
pub fn axis_rotation(axis: V3, angle: f64) -> Rot {
    let (x, y, z) = (axis[0], axis[1], axis[2]);
    let (c, s) = (angle.cos(), angle.sin());
    let t = 1.0 - c;
    [
        t * x * x + c,
        t * x * y - s * z,
        t * x * z + s * y,
        t * x * y + s * z,
        t * y * y + c,
        t * y * z - s * x,
        t * x * z - s * y,
        t * y * z + s * x,
        t * z * z + c,
    ]
}

/// Rotation that turns unit vector `a` onto unit vector `b`.
fn rotation_between(a: V3, b: V3) -> Rot {
    let axis = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let s = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    let c = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if s < 1e-9 {
        if c > 0.0 {
            return NO_ROTATION;
        }
        // Opposite: half a turn about any axis normal to `a`.
        let n = if a[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let p = [a[1] * n[2] - a[2] * n[1], a[2] * n[0] - a[0] * n[2], a[0] * n[1] - a[1] * n[0]];
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        return axis_rotation([p[0] / l, p[1] / l, p[2] / l], std::f64::consts::PI);
    }
    axis_rotation([axis[0] / s, axis[1] / s, axis[2] / s], s.atan2(c))
}

/// What a mate brings a face to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum MateTo {
    Wall(Wall),
    /// (part, face)
    Face(usize, usize),
}

/// A face of a part mated to a workspace surface or a face of another part.
/// Applied when made and again when `flip` changes; it does not follow later
/// moves.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mate {
    pub body: usize,
    pub face: usize,
    pub to: MateTo,
    /// Face turned the same way as the target instead of against it.
    pub flip: bool,
}

/// A surface of the workspace a part face can be aligned with.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Wall {
    Bed,
    Top,
    XMin,
    XMax,
    YMin,
    YMax,
}

impl Wall {
    pub const ALL: [Wall; 6] = [Wall::Bed, Wall::XMin, Wall::XMax, Wall::YMin, Wall::YMax, Wall::Top];

    pub fn label(self) -> &'static str {
        match self {
            Wall::Bed => "Bed (Z min)",
            Wall::Top => "Top (Z max)",
            Wall::XMin => "Left (X min)",
            Wall::XMax => "Right (X max)",
            Wall::YMin => "Front (Y min)",
            Wall::YMax => "Back (Y max)",
        }
    }

    /// Normal into the workspace and a point of the wall; `vol` is the
    /// workable volume (G-code coordinates).
    pub fn plane(self, vol: &[(f64, f64); 3]) -> (V3, V3) {
        let (k, out) = match self {
            Wall::Bed => (2, -1.0),
            Wall::Top => (2, 1.0),
            Wall::XMin => (0, -1.0),
            Wall::XMax => (0, 1.0),
            Wall::YMin => (1, -1.0),
            Wall::YMax => (1, 1.0),
        };
        let (mut n, mut p) = ([0.0; 3], [0.0; 3]);
        n[k] = -out;
        p[k] = if out < 0.0 { vol[k].0 } else { vol[k].1 };
        (n, p)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Body {
    pub name: String,
    pub source: String,
    /// Added to the rotated file coordinates to get bed coordinates.
    pub offset: V3,
    /// Orientation of the part on the bed.
    #[serde(default = "no_rotation")]
    pub rot: Rot,
    #[serde(skip)]
    pub geo: Option<Arc<Geometry>>,
}

impl Body {
    pub fn new(name: String, source: String, offset: V3, geo: Option<Arc<Geometry>>) -> Self {
        Self { name, source, offset, rot: NO_ROTATION, geo }
    }

    pub fn world(&self, p: V3) -> V3 {
        let r = rot_apply(&self.rot, p);
        [r[0] + self.offset[0], r[1] + self.offset[1], r[2] + self.offset[2]]
    }

    /// A direction (e.g. a normal) in bed coordinates.
    pub fn rotate_dir(&self, d: V3) -> V3 {
        rot_apply(&self.rot, d)
    }

    /// Is the point (bed coordinates) inside the solid? Counts how often a ray from the
    /// point crosses the surface: odd means inside. Needs a closed mesh.
    pub fn contains(&self, p: V3) -> bool {
        let Some(g) = &self.geo else { return false };
        let r = &self.rot;
        let t = [r[0], r[3], r[6], r[1], r[4], r[7], r[2], r[5], r[8]]; // inverse of a rotation
        let o = rot_apply(&t, [p[0] - self.offset[0], p[1] - self.offset[1], p[2] - self.offset[2]]);
        if (0..3).any(|k| o[k] < g.min[k] || o[k] > g.max[k]) {
            return false;
        }
        // Slightly tilted so the ray does not run along mesh edges.
        let d = [0.0123, 0.0071, 1.0];
        let (mut hits, sub, cross) = (0, |a: V3, b: V3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]], |a: V3, b: V3| {
            [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
        });
        let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for f in &g.faces {
            for tri in &f.tris {
                let (e1, e2) = (sub(tri[1], tri[0]), sub(tri[2], tri[0]));
                let pv = cross(d, e2);
                let det = dot(e1, pv);
                if det.abs() < 1e-12 {
                    continue;
                }
                let tv = sub(o, tri[0]);
                let u = dot(tv, pv) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let qv = cross(tv, e1);
                let v = dot(d, qv) / det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                if dot(e2, qv) / det > 0.0 {
                    hits += 1;
                }
            }
        }
        hits % 2 == 1
    }

    /// Bounding box on the bed.
    pub fn bounds(&self) -> (V3, V3) {
        let Some(g) = &self.geo else { return (self.offset, self.offset) };
        if self.rot == NO_ROTATION {
            return (self.world(g.min), self.world(g.max));
        }
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        let mut add = |p: &V3| {
            let w = self.world(*p);
            for k in 0..3 {
                lo[k] = lo[k].min(w[k]);
                hi[k] = hi[k].max(w[k]);
            }
        };
        for f in &g.faces {
            for t in &f.tris {
                t.iter().for_each(&mut add);
            }
        }
        for e in &g.edges {
            e.pts.iter().for_each(&mut add);
        }
        if lo[0] > hi[0] {
            return (self.offset, self.offset);
        }
        (lo, hi)
    }

    /// Lower corner of the body on the bed.
    pub fn min_corner(&self) -> V3 {
        self.bounds().0
    }

    /// Apply a rotation about the middle of the part. The part stays where it
    /// is in X and Y. It is put back on the bed (lowest point at Z 0), except
    /// for a turn about Z, which keeps its height.
    pub fn rotate(&mut self, r: &Rot, keep_height: bool) {
        let (lo, hi) = self.bounds();
        let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        self.rot = rot_mul(r, &self.rot);
        let (lo2, hi2) = self.bounds();
        let c2 = [(lo2[0] + hi2[0]) / 2.0, (lo2[1] + hi2[1]) / 2.0];
        self.offset[0] += c[0] - c2[0];
        self.offset[1] += c[1] - c2[1];
        self.offset[2] += if keep_height { lo[2] - lo2[2] } else { -lo2[2] };
    }

    pub fn reset_rotation(&mut self) {
        let inv = [
            self.rot[0], self.rot[3], self.rot[6], self.rot[1], self.rot[4], self.rot[7], self.rot[2], self.rot[5], self.rot[8],
        ];
        self.rotate(&inv, false);
    }

    /// Outward normal and centre (bed coordinates) of the flat face `face`.
    pub fn face_plane(&self, face: usize) -> Result<(V3, V3), String> {
        let g = self.geo.clone().ok_or("The part is not loaded.")?;
        let f = g.faces.get(face).ok_or("No such face.")?;
        let (mut sum, mut mag, mut centre) = ([0.0; 3], 0.0, [0.0; 3]);
        for t in &f.tris {
            let w = t.map(|p| self.world(p));
            let n = step::tri_normal(&w);
            let a = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            // Triangles of one face share a winding, so the sum is the face normal.
            for k in 0..3 {
                sum[k] += n[k];
                centre[k] += a * (w[0][k] + w[1][k] + w[2][k]) / 3.0;
            }
            mag += a;
        }
        let len = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
        if mag < 1e-9 || len < 0.9 * mag {
            return Err("That face is not flat.".into());
        }
        let mut n = [sum[0] / len, sum[1] / len, sum[2] / len];
        let p = [centre[0] / mag, centre[1] / mag, centre[2] / mag];
        // The normal must point away from the body: the middle of the body
        // lies on the other side of the face.
        let (lo, hi) = self.bounds();
        let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
        if (mid[0] - p[0]) * n[0] + (mid[1] - p[1]) * n[1] + (mid[2] - p[2]) * n[2] > 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        Ok((n, p))
    }

    /// Mate: turn and move the part so its flat face `face` lies on the plane
    /// through `tp` whose normal `tn` points into free space (coincident,
    /// facing the other way). The face may slide along the plane.
    pub fn mate_face(&mut self, face: usize, tn: V3, tp: V3, flip: bool) -> Result<(), String> {
        let (n, _) = self.face_plane(face)?;
        let want = if flip { tn } else { [-tn[0], -tn[1], -tn[2]] };
        self.rotate(&rotation_between(n, want), true);
        let (_, p) = self.face_plane(face)?;
        let d = (tp[0] - p[0]) * tn[0] + (tp[1] - p[1]) * tn[1] + (tp[2] - p[2]) * tn[2];
        for k in 0..3 {
            self.offset[k] += d * tn[k];
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project3 {
    pub machine: Machine,
    pub job: Job3,
    pub bodies: Vec<Body>,
    pub sequence: Vec<Step3>,
    #[serde(default)]
    pub mates: Vec<Mate>,
}

impl Project3 {
    pub fn new(machine: Machine) -> Self {
        Self { machine, job: Job3::default(), bodies: vec![], sequence: vec![], mates: vec![] }
    }

    /// Name of what a mate goes to.
    pub fn mate_target_name(&self, to: MateTo) -> String {
        match to {
            MateTo::Wall(w) => w.label().to_string(),
            MateTo::Face(b, f) => format!("{} face {}", self.bodies.get(b).map_or("?", |b| b.name.as_str()), f + 1),
        }
    }

    /// Move the part of mate `i` so the mate holds.
    pub fn apply_mate(&mut self, i: usize) -> Result<(), String> {
        let m = *self.mates.get(i).ok_or("No such mate.")?;
        let (n, p) = match m.to {
            MateTo::Wall(w) => w.plane(&self.machine.workable()),
            MateTo::Face(b, f) => self.bodies.get(b).ok_or("The part is gone.")?.face_plane(f)?,
        };
        self.bodies.get_mut(m.body).ok_or("The part is gone.")?.mate_face(m.face, n, p, m.flip)
    }

    pub fn remove_body(&mut self, idx: usize) {
        if idx >= self.bodies.len() {
            return;
        }
        self.bodies.remove(idx);
        self.mates.retain(|m| m.body != idx && !matches!(m.to, MateTo::Face(b, _) if b == idx));
        for m in &mut self.mates {
            if m.body > idx {
                m.body -= 1;
            }
            if let MateTo::Face(b, f) = m.to {
                if b > idx {
                    m.to = MateTo::Face(b - 1, f);
                }
            }
        }
        // Steps keep their place; what pointed at the removed part is cleared.
        let shift = |r: &mut Option<(usize, usize)>| {
            if let Some((b, f)) = *r {
                *r = if b == idx { None } else if b > idx { Some((b - 1, f)) } else { *r };
            }
        };
        for s in &mut self.sequence {
            shift(&mut s.path);
            shift(&mut s.surface);
            s.lead_surfaces.retain(|f| f.0 != idx);
            for f in &mut s.lead_surfaces {
                if f.0 > idx {
                    f.0 -= 1;
                }
            }
            for l in [&mut s.start, &mut s.stop] {
                *l = match *l {
                    Some(Limit::Edge(b, _) | Limit::Surface(b, _)) if b == idx => None,
                    Some(Limit::Edge(b, e)) if b > idx => Some(Limit::Edge(b - 1, e)),
                    Some(Limit::Surface(b, f)) if b > idx => Some(Limit::Surface(b - 1, f)),
                    other => other,
                };
            }
        }
    }

    /// Edge of a body in bed coordinates.
    pub fn edge_points(&self, body: usize, edge: usize) -> Vec<V3> {
        let Some(b) = self.bodies.get(body) else { return vec![] };
        let Some(g) = &b.geo else { return vec![] };
        g.edges.get(edge).map(|e| e.pts.iter().map(|p| b.world(*p)).collect()).unwrap_or_default()
    }

    /// Where a step leaves the tool: the end of a path, the point of a go to,
    /// home. None for a pause or a path without points.
    fn end_point(&self, step: &Step3) -> Option<V3> {
        match step.kind {
            Kind3::Path => self.step_points(step).last().copied(),
            Kind3::Home => Some(self.machine.home_position()),
            Kind3::Goto { x, y, z } => Some([x, y, z]),
            Kind3::Pause { .. } => None,
        }
    }

    /// The lead in of step `i`: from the end of the step before it to the start
    /// of its path, both included. It runs straight between the two, but is
    /// pushed away from the surfaces picked in the step so it stays at least the
    /// lead in offset from them, on the side it is on. That keeps it off walls
    /// as well as floors. When the straight way goes through one of those
    /// surfaces (a post, a wall), it goes around the whole piece instead, to the
    /// left of its travel direction, or the right when the direction is flipped.
    /// Empty when there is no lead in.
    pub fn lead_in_points(&self, i: usize) -> Vec<V3> {
        let Some(step) = self.sequence.get(i) else { return vec![] };
        if i == 0 || !step.is_path() || !step.lead_in {
            return vec![];
        }
        let (Some(from), Some(&to)) = (self.end_point(&self.sequence[i - 1]), self.step_points(step).first()) else {
            return vec![];
        };
        let job = self.job.with(&step.over);
        let faces: Vec<Vec<[V3; 3]>> = step
            .lead_surfaces
            .iter()
            .filter_map(|&(b, f)| {
                let body = self.bodies.get(b)?;
                let face = body.geo.as_ref()?.faces.get(f)?;
                Some(face.tris.iter().map(|t| t.map(|p| body.world(p))).collect())
            })
            .collect();
        let off = step.lead_offset.unwrap_or(job.tool_diameter.max(0.0) / 2.0);
        let len = (0..3).map(|k| (to[k] - from[k]).powi(2)).sum::<f64>().sqrt();
        // Pieces short enough to follow a wall closely.
        let mut piece = job.sample_mm.max(0.05);
        if !faces.is_empty() && off > 0.0 {
            piece = piece.min((off / 2.0).max(0.05));
        }
        let n = (len / piece).ceil().max(1.0) as usize;
        let mut out: Vec<V3> = (0..=n)
            .map(|k| if k == 0 { from } else if k == n { to } else { lerp(from, to, k as f64 / n as f64) })
            .collect();
        if off > 0.0 && bypass(&faces, &mut out, off, if step.lead_flip { -1.0 } else { 1.0 }) {
            return out;
        }
        for k in 1..n {
            if off > 0.0 {
                out[k] = keep_out(&faces, out[k], off, step.lead_flip);
            }
        }
        // Smooth the moved points a little and push them out again, so a wall
        // that is followed does not make the curve zig-zag.
        if !faces.is_empty() && off > 0.0 {
            for _ in 0..4 {
                let prev = out.clone();
                for k in 1..out.len() - 1 {
                    let mut p = [0.0; 3];
                    for a in 0..3 {
                        p[a] = 0.25 * prev[k - 1][a] + 0.5 * prev[k][a] + 0.25 * prev[k + 1][a];
                    }
                    // The side is settled by now: keep to wherever each point is.
                    out[k] = keep_out(&faces, p, off, false);
                }
            }
        }
        out
    }

    /// Where along `pts` a start or stop limit is: segment and position on it.
    fn limit_hit(&self, pts: &[V3], l: Limit) -> Option<(usize, f64)> {
        match l {
            Limit::Point(p) => nearest_on(pts, &[p]),
            Limit::Edge(b, e) => nearest_on(pts, &resample(&self.edge_points(b, e), 1.0)),
            Limit::Surface(b, f) => {
                let body = self.bodies.get(b)?;
                let face = body.geo.as_ref()?.faces.get(f)?;
                let tris: Vec<[V3; 3]> = face.tris.iter().map(|t| t.map(|p| body.world(p))).collect();
                nearest_on_surface(pts, &tris)
            }
        }
    }

    /// The path of a step in bed coordinates, in travel order: the edge cut
    /// to its start and stop, Z from the surface plus the Z gap, moved
    /// sideways by the path offset and the tool radius.
    pub fn step_points(&self, step: &Step3) -> Vec<V3> {
        let Some((body, edge)) = step.path else { return vec![] };
        let job = self.job.with(&step.over);
        let mut pts = self.edge_points(body, edge);
        let hit = |pts: &[V3], l: Option<Limit>| l.and_then(|l| self.limit_hit(pts, l));
        let pos = |h: Option<(usize, f64)>| h.map(|(i, t)| i as f64 + t);
        // With both a start and a stop the path runs from the start toward the
        // stop, whatever the direction of the edge. Otherwise Reverse decides.
        let reverse = match (pos(hit(&pts, step.start)), pos(hit(&pts, step.stop))) {
            (Some(a), Some(b)) => a > b,
            _ => step.reverse,
        };
        if reverse {
            pts.reverse();
        }
        // Positions of the start and the stop along the path (in mm from its
        // first point), moved by their offsets.
        // Everything sideways defaults to the tool offset.
        let r = job.tool_diameter.max(0.0) / 2.0;
        let cum = cumulative(&pts);
        let total = cum.last().copied().unwrap_or(0.0);
        let along = |l: Option<Limit>, off: f64, flip: bool, inward: f64| {
            let (i, t) = l.and_then(|l| self.limit_hit(&pts, l))?;
            let s = cum[i] + t * (cum[i + 1] - cum[i]) + inward * off * if flip { -1.0 } else { 1.0 };
            Some(s.clamp(0.0, total))
        };
        let s0 = along(step.start, step.start_offset.unwrap_or(r), step.start_flip, 1.0).unwrap_or(0.0);
        let s1 = along(step.stop, step.stop_offset.unwrap_or(r), step.stop_flip, -1.0).unwrap_or(total);
        if s0 > 0.0 || s1 < total {
            pts = cut_range(&pts, &cum, s0, s1);
        }
        pts.dedup_by(|a, b| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs() < 1e-9);
        if pts.len() < 2 {
            return vec![];
        }
        // The face in bed coordinates.
        let tris: Option<Vec<[V3; 3]>> = step.surface.and_then(|(sb, sf)| {
            let b = self.bodies.get(sb)?;
            let face = b.geo.as_ref()?.faces.get(sf)?;
            Some(face.tris.iter().map(|t| t.map(|p| b.world(p))).collect())
        });
        if let Some(tris) = &tris {
            // Z follows the surface, measured on the edge itself.
            pts = resample(&pts, job.sample_mm.max(0.05));
            for p in &mut pts {
                p[2] = surface_at(tris, [p[0], p[1]]).map_or(p[2], |s| s.0);
            }
        }
        for p in &mut pts {
            p[2] += job.z_gap;
        }
        // "Away" is away from the middle of the surface, or from the middle of
        // the part when there is no surface.
        let centre = match &tris {
            Some(t) if !t.is_empty() => {
                let n = t.len() as f64 * 3.0;
                let (mut x, mut y) = (0.0, 0.0);
                for tri in t {
                    for v in tri {
                        x += v[0];
                        y += v[1];
                    }
                }
                [x / n, y / n]
            }
            _ => {
                let (lo, hi) = self.bodies.get(body).map_or(([0.0; 3], [0.0; 3]), |b| b.bounds());
                [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
            }
        };
        let off = step.offset.unwrap_or(r);
        if off > 0.0 {
            offset_polyline(&mut pts, off, step.offset_flip, centre);
        }
        pts
    }
}

/// Move a polyline sideways (in X and Y) by `dist`, away from `centre`, or
/// toward it when `flip` is set. Corners are mitred.
pub fn offset_polyline(pts: &mut [V3], dist: f64, flip: bool, centre: [f64; 2]) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let closed = {
        let (a, b) = (pts[0], pts[n - 1]);
        n > 2 && (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    };
    // Unit left normal of each segment.
    let seg: Vec<[f64; 2]> = (0..n - 1)
        .map(|i| {
            let (dx, dy) = (pts[i + 1][0] - pts[i][0], pts[i + 1][1] - pts[i][1]);
            let l = (dx * dx + dy * dy).sqrt();
            if l < 1e-12 { [0.0, 0.0] } else { [-dy / l, dx / l] }
        })
        .collect();
    // Test on the longest segment, away from corners.
    let k = (0..n - 1)
        .max_by(|&a, &b| {
            let la = (pts[a + 1][0] - pts[a][0]).hypot(pts[a + 1][1] - pts[a][1]);
            let lb = (pts[b + 1][0] - pts[b][0]).hypot(pts[b + 1][1] - pts[b][1]);
            la.total_cmp(&lb)
        })
        .unwrap_or(0);
    let mid = [(pts[k][0] + pts[k + 1][0]) / 2.0, (pts[k][1] + pts[k + 1][1]) / 2.0];
    let away = (mid[0] - centre[0]) * seg[k][0] + (mid[1] - centre[1]) * seg[k][1];
    let sign = if (away >= 0.0) != flip { 1.0 } else { -1.0 };
    let orig: Vec<V3> = pts.to_vec();
    for i in 0..n {
        let prev = if i > 0 { Some(seg[i - 1]) } else if closed { Some(seg[n - 2]) } else { None };
        let next = if i + 1 < n { Some(seg[i]) } else if closed { Some(seg[0]) } else { None };
        let nrm = match (prev, next) {
            (Some(a), Some(b)) => {
                let m = [a[0] + b[0], a[1] + b[1]];
                let l = (m[0] * m[0] + m[1] * m[1]).sqrt();
                if l < 1e-9 {
                    a
                } else {
                    // Mitre: longer than the offset at a corner, capped for sharp ones.
                    let u = [m[0] / l, m[1] / l];
                    let cos = (u[0] * a[0] + u[1] * a[1]).max(0.5);
                    [u[0] / cos, u[1] / cos]
                }
            }
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => [0.0, 0.0],
        };
        pts[i][0] = orig[i][0] + sign * dist * nrm[0];
        pts[i][1] = orig[i][1] + sign * dist * nrm[1];
    }
}

/// The point of the triangle closest to `p`.
fn closest_on_tri(p: V3, t: &[V3; 3]) -> V3 {
    let sub = |a: V3, b: V3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let at = |o: V3, u: V3, k: f64| [o[0] + u[0] * k, o[1] + u[1] * k, o[2] + u[2] * k];
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return at(a, ab, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return at(a, ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return at(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let den = 1.0 / (va + vb + vc);
    at(at(a, ab, vb * den), ac, vc * den)
}

fn point_tri_dist2(p: V3, t: &[V3; 3]) -> f64 {
    let q = closest_on_tri(p, t);
    (0..3).map(|k| (p[k] - q[k]).powi(2)).sum()
}

/// Does the segment a-b pass through the triangle?
fn segment_hits(a: V3, b: V3, t: &[V3; 3]) -> bool {
    let sub = |a: V3, b: V3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: V3, b: V3| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dir = sub(b, a);
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let h = cross(dir, e2);
    let det = dot(e1, h);
    if det.abs() < 1e-14 {
        return false;
    }
    let f = 1.0 / det;
    let s = sub(a, t[0]);
    let u = f * dot(s, h);
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = cross(s, e1);
    let v = f * dot(dir, q);
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    (1e-9..=1.0 - 1e-9).contains(&(f * dot(e2, q)))
}

/// The triangles joined (sharing a corner) to any of the `start` triangles.
fn joined_to(tris: &[[V3; 3]], start: &[usize]) -> Vec<usize> {
    let key = |v: V3| ((v[0] * 1e4).round() as i64, (v[1] * 1e4).round() as i64, (v[2] * 1e4).round() as i64);
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            p[r] = p[p[r]];
            r = p[r];
        }
        r
    }
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    let mut seen: std::collections::HashMap<(i64, i64, i64), usize> = std::collections::HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for v in t {
            if let Some(&j) = seen.get(&key(*v)) {
                let (a, b) = (find(&mut parent, j), find(&mut parent, i));
                parent[a] = b;
            } else {
                seen.insert(key(*v), i);
            }
        }
    }
    let roots: std::collections::HashSet<usize> = start.iter().map(|&i| find(&mut parent, i)).collect();
    (0..tris.len()).filter(|&i| roots.contains(&find(&mut parent, i))).collect()
}

/// Outline of the points in X and Y, counter-clockwise. Two points for a line.
fn convex_hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let chain = |pts: &mut dyn Iterator<Item = [f64; 2]>| {
        let mut h: Vec<[f64; 2]> = vec![];
        for q in pts {
            while h.len() >= 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
        h
    };
    let mut lower = chain(&mut p.iter().copied());
    lower.extend(chain(&mut p.iter().rev().copied()));
    lower
}

/// Distance from a point to the outline and what it encloses (0 inside).
fn dist_to_hull(h: &[[f64; 2]], q: [f64; 2]) -> f64 {
    let seg = |a: [f64; 2], b: [f64; 2]| {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if l2 < 1e-18 { 0.0 } else { (((q[0] - a[0]) * ab[0] + (q[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0) };
        (a[0] + ab[0] * t - q[0]).hypot(a[1] + ab[1] * t - q[1])
    };
    match h.len() {
        0 => f64::MAX,
        1 => (h[0][0] - q[0]).hypot(h[0][1] - q[1]),
        2 => seg(h[0], h[1]),
        n => {
            let side = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
            if (0..n).all(|i| side(h[i], h[(i + 1) % n]) >= 0.0) {
                0.0
            } else {
                (0..n).map(|i| seg(h[i], h[(i + 1) % n])).fold(f64::MAX, f64::min)
            }
        }
    }
}

/// Bend a straight lead in around what is in its way. When it passes through
/// one of the triangles, the whole piece those belong to (all triangles joined
/// to them) is an obstacle, taken by its outline seen from above and its
/// height. Each point at that height is moved sideways (left of the travel
/// direction in X and Y, or right when `side` is -1) just far enough to be
/// `off` from the outline. The shifts then fade away along the lead at 45
/// degrees, so it makes a smooth detour. The two ends stay. Returns false, and
/// changes nothing, when the lead in does not pass through anything.
fn bypass(faces: &[Vec<[V3; 3]>], pts: &mut [V3], off: f64, side: f64) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let tris: Vec<[V3; 3]> = faces.iter().flatten().copied().collect();
    let hit: Vec<usize> = (0..tris.len()).filter(|&i| segment_hits(pts[0], pts[n - 1], &tris[i])).collect();
    if hit.is_empty() {
        return false;
    }
    let (mut xy, mut zlo, mut zhi) = (vec![], f64::MAX, f64::MIN);
    for i in joined_to(&tris, &hit) {
        for v in &tris[i] {
            xy.push([v[0], v[1]]);
            zlo = zlo.min(v[2]);
            zhi = zhi.max(v[2]);
        }
    }
    let hull = convex_hull(xy);
    let (dx, dy) = (pts[n - 1][0] - pts[0][0], pts[n - 1][1] - pts[0][1]);
    let l = dx.hypot(dy);
    if l < 1e-9 {
        return false;
    }
    let lat = [-dy / l * side, dx / l * side];
    let mut shift = vec![0.0_f64; n];
    for i in 1..n - 1 {
        if pts[i][2] < zlo - off || pts[i][2] > zhi + off {
            continue; // over or under it
        }
        let mut k = 0.0;
        while k <= 60.0 {
            if dist_to_hull(&hull, [pts[i][0] + lat[0] * k, pts[i][1] + lat[1] * k]) >= off - 1e-9 {
                shift[i] = k;
                break;
            }
            k += 0.02;
        }
    }
    let run = |a: V3, b: V3| (a[0] - b[0]).hypot(a[1] - b[1]);
    for i in 1..n {
        shift[i] = shift[i].max(shift[i - 1] - run(pts[i], pts[i - 1]));
    }
    for i in (0..n - 1).rev() {
        shift[i] = shift[i].max(shift[i + 1] - run(pts[i], pts[i + 1]));
    }
    for i in 1..n - 1 {
        pts[i][0] += lat[0] * shift[i];
        pts[i][1] += lat[1] * shift[i];
    }
    true
}

/// Move `p` away from the surfaces until it is `off` from all of them, along the
/// line from the closest surface point, so it stays on the side it is on. With
/// `flip` it goes to the other side of the surface instead.
/// shortcut: a point is only moved, a segment between two points is not checked,
/// and a path that crosses a surface stays on both sides of it.
fn keep_out(faces: &[Vec<[V3; 3]>], mut p: V3, off: f64, flip: bool) -> V3 {
    for _ in 0..3 {
        let mut best: Option<(f64, V3, V3)> = None; // distance squared, closest point, triangle normal
        for t in faces.iter().flatten() {
            let q = closest_on_tri(p, t);
            let d2: f64 = (0..3).map(|k| (p[k] - q[k]).powi(2)).sum();
            if best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, q, step::tri_normal(t)));
            }
        }
        let Some((d2, q, n)) = best else { return p };
        let dist = d2.sqrt();
        if dist >= off - 1e-9 {
            return p;
        }
        let dir = if dist > 1e-9 {
            [(p[0] - q[0]) / dist, (p[1] - q[1]) / dist, (p[2] - q[2]) / dist]
        } else {
            // On the surface itself: up from it.
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            let sgn = if n[2] < 0.0 { -1.0 } else { 1.0 };
            [n[0] / l * sgn, n[1] / l * sgn, n[2] / l * sgn]
        };
        let sgn = if flip { -1.0 } else { 1.0 };
        p = [q[0] + dir[0] * off * sgn, q[1] + dir[1] * off * sgn, q[2] + dir[2] * off * sgn];
        if flip {
            // Done: on the other side now, which is the far side of the band.
            return p;
        }
    }
    p
}

/// Where a polyline first comes closest to the triangles: segment and position
/// on it. Where it reaches the surface, that is the first point on it.
/// shortcut: samples every 0.5 mm and tests every triangle, so a very long path
/// against a very fine surface is slow; add a grid or bounds test if it shows.
fn nearest_on_surface(path: &[V3], tris: &[[V3; 3]]) -> Option<(usize, f64)> {
    let mut best: Option<(f64, usize, f64)> = None;
    for i in 0..path.len().saturating_sub(1) {
        let (a, b) = (path[i], path[i + 1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        let n = (len / 0.5).ceil().max(1.0) as usize;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let p = lerp(a, b, t);
            let d = tris.iter().map(|tr| point_tri_dist2(p, tr)).fold(f64::MAX, f64::min);
            if best.is_none_or(|x| d < x.0 - 1e-9) {
                best = Some((d, i, t));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// Where a polyline comes closest to any of `targets`: segment index and the
/// position along that segment (0..1).
fn nearest_on(path: &[V3], targets: &[V3]) -> Option<(usize, f64)> {
    let mut best: Option<(f64, usize, f64)> = None;
    for i in 0..path.len().saturating_sub(1) {
        let (a, b) = (path[i], path[i + 1]);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        for q in targets {
            let t = if l2 < 1e-18 { 0.0 } else { (((q[0] - a[0]) * ab[0] + (q[1] - a[1]) * ab[1] + (q[2] - a[2]) * ab[2]) / l2).clamp(0.0, 1.0) };
            let d2 = (0..3).map(|k| (a[k] + ab[k] * t - q[k]).powi(2)).sum::<f64>();
            if best.is_none_or(|x| d2 < x.0) {
                best = Some((d2, i, t));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

fn lerp(a: V3, b: V3, t: f64) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Length of the polyline up to each point.
fn cumulative(pts: &[V3]) -> Vec<f64> {
    let mut cum = vec![0.0];
    for w in pts.windows(2) {
        let l = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt();
        cum.push(cum.last().copied().unwrap_or(0.0) + l);
    }
    cum
}

/// The part of the polyline between two distances along it. Empty when the
/// end is not past the start.
fn cut_range(pts: &[V3], cum: &[f64], s0: f64, s1: f64) -> Vec<V3> {
    if s1 <= s0 || pts.len() < 2 {
        return vec![];
    }
    let at = |s: f64| {
        let i = cum.partition_point(|&c| c <= s).saturating_sub(1).min(pts.len() - 2);
        let l = cum[i + 1] - cum[i];
        (i, if l < 1e-12 { 0.0 } else { ((s - cum[i]) / l).clamp(0.0, 1.0) })
    };
    let ((i0, t0), (i1, t1)) = (at(s0), at(s1));
    let mut out = vec![lerp(pts[i0], pts[i0 + 1], t0)];
    out.extend_from_slice(&pts[i0 + 1..=i1]);
    out.push(lerp(pts[i1], pts[i1 + 1], t1));
    out
}

/// Split a polyline so no piece is longer than `max`.
pub fn resample(pts: &[V3], max: f64) -> Vec<V3> {
    let mut out: Vec<V3> = vec![];
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            out.push(*p);
            continue;
        }
        let q = pts[i - 1];
        let l = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
        let n = (l / max).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            out.push([q[0] + (p[0] - q[0]) * t, q[1] + (p[1] - q[1]) * t, q[2] + (p[2] - q[2]) * t]);
        }
    }
    out
}

/// Height and upward normal of a triangle mesh at a point (x, y). Uses the
/// triangle that contains the point, or the nearest one when the point lies a
/// little outside (e.g. on the border of the face).
pub fn surface_at(tris: &[[V3; 3]], xy: [f64; 2]) -> Option<(f64, V3)> {
    let mut best: Option<(f64, f64, V3)> = None; // distance outside, z, normal
    for t in tris {
        let n = step::tri_normal(t);
        if n[2].abs() < 1e-12 {
            continue; // vertical, no height
        }
        let (a, b, c) = ([t[0][0], t[0][1]], [t[1][0], t[1][1]], [t[2][0], t[2][1]]);
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-12 {
            continue;
        }
        let w0 = ((b[0] - xy[0]) * (c[1] - xy[1]) - (b[1] - xy[1]) * (c[0] - xy[0])) / area;
        let w1 = ((c[0] - xy[0]) * (a[1] - xy[1]) - (c[1] - xy[1]) * (a[0] - xy[0])) / area;
        let w2 = 1.0 - w0 - w1;
        let outside = (-w0).max(-w1).max(-w2).max(0.0) * area.abs().sqrt();
        if best.is_none_or(|b| outside < b.0) {
            let z = w0 * t[0][2] + w1 * t[1][2] + w2 * t[2][2];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let s = if n[2] < 0.0 { -1.0 } else { 1.0 } / l;
            best = Some((outside, z, [n[0] * s, n[1] * s, n[2] * s]));
            if outside == 0.0 {
                break;
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_height_and_normal() {
        // A 45 degree slope: z = x.
        let tris = vec![[[0.0, 0.0, 0.0], [10.0, 0.0, 10.0], [0.0, 10.0, 0.0]]];
        let (z, n) = surface_at(&tris, [2.0, 2.0]).unwrap();
        assert!((z - 2.0).abs() < 1e-9);
        assert!((n[2] - 0.5_f64.sqrt()).abs() < 1e-9 && n[0] < 0.0);
    }

    #[test]
    fn resample_splits_long_pieces() {
        let p = resample(&[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]], 2.5);
        assert_eq!(p.len(), 5);
        assert_eq!(p[4], [10.0, 0.0, 0.0]);
    }

    fn plate() -> Project3 {
        // A flat face at z = 10 and an edge along it, plus a tilted face.
        let flat = super::step::FaceMesh {
            tris: vec![
                [[0.0, 0.0, 10.0], [20.0, 0.0, 10.0], [20.0, 20.0, 10.0]],
                [[0.0, 0.0, 10.0], [20.0, 20.0, 10.0], [0.0, 20.0, 10.0]],
            ],
            nors: vec![],
        };
        let slope = super::step::FaceMesh { tris: vec![[[0.0, 0.0, 0.0], [10.0, 0.0, 10.0], [0.0, 10.0, 0.0]]], nors: vec![] };
        let geo = Geometry {
            faces: vec![flat, slope],
            edges: vec![super::step::EdgePoly { pts: vec![[2.0, 5.0, 10.0], [12.0, 5.0, 10.0]], faces: vec![0] }],
            min: [0.0; 3],
            max: [20.0, 20.0, 10.0],
            ..Default::default()
        };
        let mut p = Project3::new(Machine::new("T"));
        p.bodies.push(Body::new("b".into(), String::new(), [5.0, 0.0, 0.0], Some(Arc::new(geo))));
        p
    }

    /// A path step on the edge of the plate, with its flat face as surface.
    fn path_step() -> Step3 {
        let mut s = Step3::new(Kind3::Path);
        s.path = Some((0, 0));
        s.surface = Some((0, 0));
        s
    }

    #[test]
    fn z_gap_follows_the_surface() {
        let mut p = plate();
        let mut s = path_step();
        s.over.z_gap = Some(1.5);
        p.job.sample_mm = 5.0;
        let pts = p.step_points(&s);
        assert_eq!(pts.len(), 3);
        assert!(pts.iter().all(|q| (q[2] - 11.5).abs() < 1e-9), "{pts:?}");
        // The body is offset by 5 in X.
        assert!((pts[0][0] - 7.0).abs() < 1e-9);
        s.reverse = true;
        assert!((p.step_points(&s)[0][0] - 17.0).abs() < 1e-9);
        // A step without a path has no points.
        assert!(p.step_points(&Step3::new(Kind3::Path)).is_empty());
    }

    #[test]
    fn offsets_default_to_the_tool_offset_and_can_be_changed_and_flipped() {
        let mut p = plate();
        p.job.tool_diameter = 2.0;
        p.job.z_gap = 0.0;
        p.job.sample_mm = 50.0;
        // Edge from (7,5) to (17,5) on a face whose middle is at (15,10):
        // away from it is toward -Y. The path offset is the tool radius.
        let mut s = path_step();
        let pts = p.step_points(&s);
        assert_eq!(pts.len(), 2);
        assert!((pts[0][1] - 4.0).abs() < 1e-9 && (pts[0][0] - 7.0).abs() < 1e-9, "{pts:?}");
        assert!((pts[0][2] - 10.0).abs() < 1e-9, "Z stays on the surface");
        s.offset_flip = true;
        assert!((p.step_points(&s)[0][1] - 6.0).abs() < 1e-9);
        // Changed by hand, flipped or not.
        s.offset = Some(3.0);
        assert!((p.step_points(&s)[0][1] - 8.0).abs() < 1e-9);
        s.offset_flip = false;
        assert!((p.step_points(&s)[0][1] - 2.0).abs() < 1e-9);
        // No tool diameter and no offset set: the path stays on the edge.
        s.offset = None;
        p.job.tool_diameter = 0.0;
        assert!((p.step_points(&s)[0][1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn start_and_stop_offsets_default_to_the_tool_radius() {
        let mut p = plate();
        p.job.tool_diameter = 2.0; // radius 1
        p.job.sample_mm = 50.0;
        let mut s = path_step(); // x = 7 .. 17
        s.start = Some(Limit::Point([10.0, 5.0, 0.0]));
        s.stop = Some(Limit::Point([14.0, 5.0, 0.0]));
        let v = p.step_points(&s);
        assert!((v[0][0] - 11.0).abs() < 1e-9 && (v.last().unwrap()[0] - 13.0).abs() < 1e-9, "{v:?}");
        // Without a start or stop the whole edge is used.
        s.start = None;
        s.stop = None;
        let v = p.step_points(&s);
        assert!((v[0][0] - 7.0).abs() < 1e-9 && (v.last().unwrap()[0] - 17.0).abs() < 1e-9);
    }

    #[test]
    fn start_and_stop_cut_the_path() {
        let mut p = plate();
        p.job.sample_mm = 50.0;
        let mut s = path_step();
        s.start = Some(Limit::Point([10.0, 9.0, 0.0]));
        s.stop = Some(Limit::Point([14.0, 1.0, 0.0]));
        let pts = p.step_points(&s);
        assert_eq!(pts.len(), 2);
        assert!((pts[0][0] - 10.0).abs() < 1e-9 && (pts[1][0] - 14.0).abs() < 1e-9, "{pts:?}");
    }

    #[test]
    fn start_and_stop_offsets_move_the_ends_and_flip() {
        let mut p = plate();
        p.job.sample_mm = 50.0;
        let mut s = path_step(); // x = 7 .. 17
        s.start = Some(Limit::Point([10.0, 5.0, 0.0]));
        s.stop = Some(Limit::Point([14.0, 5.0, 0.0]));
        s.start_offset = Some(1.0);
        s.stop_offset = Some(2.0);
        let x = |s: &Step3| { let v = p.step_points(s); (v[0][0], v.last().unwrap()[0]) };
        // Inward: later start, earlier stop.
        let (a, b) = x(&s);
        assert!((a - 11.0).abs() < 1e-9 && (b - 12.0).abs() < 1e-9, "{a} {b}");
        // Each toggles on its own.
        s.start_flip = true;
        let (a, b) = x(&s);
        assert!((a - 9.0).abs() < 1e-9 && (b - 12.0).abs() < 1e-9, "{a} {b}");
        s.stop_flip = true;
        let (a, b) = x(&s);
        assert!((a - 9.0).abs() < 1e-9 && (b - 16.0).abs() < 1e-9, "{a} {b}");
        // Never past the ends of the edge.
        s.start_offset = Some(100.0);
        s.stop_offset = Some(100.0);
        let (a, b) = x(&s);
        assert!((a - 7.0).abs() < 1e-9 && (b - 17.0).abs() < 1e-9, "{a} {b}");
        // An inward offset larger than the path leaves nothing.
        s.start_flip = false;
        s.stop_flip = false;
        assert!(p.step_points(&s).is_empty());
    }

    #[test]
    fn start_and_stop_set_the_direction() {
        let mut p = plate();
        p.job.sample_mm = 50.0;
        let mut s = path_step(); // the edge runs from x = 7 to x = 17
        s.start = Some(Limit::Point([16.0, 5.0, 0.0]));
        s.stop = Some(Limit::Point([9.0, 5.0, 0.0]));
        let pts = p.step_points(&s);
        assert!((pts[0][0] - 16.0).abs() < 1e-9 && (pts[1][0] - 9.0).abs() < 1e-9, "{pts:?}");
        // Start and stop at the same place leave no path.
        s.stop = s.start;
        assert!(p.step_points(&s).is_empty());
    }

    #[test]
    fn a_surface_can_be_the_start_or_the_stop() {
        let mut p = plate();
        p.job.sample_mm = 50.0;
        // A wall across the path at x = 12.
        let q = |a: V3, b: V3, c: V3| [a, b, c];
        let wall = super::step::FaceMesh {
            tris: vec![
                q([12.0, 0.0, 0.0], [12.0, 20.0, 0.0], [12.0, 20.0, 20.0]),
                q([12.0, 0.0, 0.0], [12.0, 20.0, 20.0], [12.0, 0.0, 20.0]),
            ],
            nors: vec![],
        };
        let geo = Geometry { faces: vec![wall], min: [12.0, 0.0, 0.0], max: [12.0, 20.0, 20.0], ..Default::default() };
        p.bodies.push(Body::new("wall".into(), String::new(), [0.0; 3], Some(Arc::new(geo))));
        let mut s = path_step(); // x = 7 .. 17
        s.start = Some(Limit::Surface(1, 0));
        let pts = p.step_points(&s);
        assert!((pts[0][0] - 12.0).abs() < 1e-9 && (pts.last().unwrap()[0] - 17.0).abs() < 1e-9, "{pts:?}");
        s.start = None;
        s.stop = Some(Limit::Surface(1, 0));
        let pts = p.step_points(&s);
        assert!((pts[0][0] - 7.0).abs() < 1e-9 && (pts.last().unwrap()[0] - 12.0).abs() < 1e-9, "{pts:?}");
        // The distance to a triangle: on it, beside it, and past a corner.
        let t = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        assert!(point_tri_dist2([2.0, 2.0, 0.0], &t) < 1e-12);
        assert!((point_tri_dist2([2.0, 2.0, 3.0], &t) - 9.0).abs() < 1e-9);
        assert!((point_tri_dist2([-3.0, -4.0, 0.0], &t) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn removing_a_part_clears_what_pointed_at_it() {
        let mut p = plate();
        let mut s = path_step();
        s.start = Some(Limit::Edge(0, 0));
        s.stop = Some(Limit::Surface(0, 0));
        p.sequence.push(s);
        p.remove_body(0);
        let s = p.sequence[0].clone();
        assert!(s.path.is_none() && s.surface.is_none() && s.start.is_none() && s.stop.is_none());
    }

    #[test]
    fn corners_are_mitred() {
        // An L: right along +X then up +Y. Away from a point inside the turn
        // is to the right of the travel direction.
        let mut pts = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]];
        offset_polyline(&mut pts, 1.0, false, [5.0, 5.0]);
        assert!((pts[0][1] + 1.0).abs() < 1e-9);
        assert!((pts[1][0] - 11.0).abs() < 1e-9 && (pts[1][1] + 1.0).abs() < 1e-9, "{pts:?}");
        assert!((pts[2][0] - 11.0).abs() < 1e-9);
    }

    fn cube() -> Body {
        // 10 x 20 x 30 box, face 0 is x = 10 (outward +X). Faces share a winding
        // per face, but the sign is arbitrary, as in real files.
        let q = |a: V3, b: V3, c: V3, d: V3| super::step::FaceMesh { tris: vec![[a, b, c], [a, c, d]], nors: vec![] };
        let (x, y, z) = (10.0, 20.0, 30.0);
        let faces = vec![
            q([x, 0.0, 0.0], [x, y, 0.0], [x, y, z], [x, 0.0, z]),
            q([0.0, 0.0, 0.0], [0.0, 0.0, z], [0.0, y, z], [0.0, y, 0.0]),
            q([0.0, 0.0, 0.0], [x, 0.0, 0.0], [x, 0.0, z], [0.0, 0.0, z]),
            q([0.0, y, 0.0], [0.0, y, z], [x, y, z], [x, y, 0.0]),
            q([0.0, 0.0, 0.0], [0.0, y, 0.0], [x, y, 0.0], [x, 0.0, 0.0]),
            q([0.0, 0.0, z], [x, 0.0, z], [x, y, z], [0.0, y, z]),
        ];
        let geo = Geometry { faces, min: [0.0; 3], max: [x, y, z], ..Default::default() };
        Body::new("cube".into(), String::new(), [100.0, 50.0, 0.0], Some(Arc::new(geo)))
    }

    #[test]
    fn lay_a_side_face_on_the_bed() {
        let mut b = cube();
        let (n, p) = Wall::Bed.plane(&[(0.0, 300.0), (0.0, 300.0), (0.0, 100.0)]);
        b.mate_face(0, n, p, false).unwrap(); // the x = 10 face
        let (lo, hi) = b.bounds();
        assert!(lo[2].abs() < 1e-9, "{lo:?}");
        // The cube now stands 10 high on its 20 x 30 face.
        assert!((hi[2] - 10.0).abs() < 1e-9 && ((hi[0] - lo[0]) * (hi[1] - lo[1]) - 600.0).abs() < 1e-6, "{lo:?} {hi:?}");
        // Position in X and Y is kept: the middle stays at (105, 60).
        assert!(((lo[0] + hi[0]) / 2.0 - 105.0).abs() < 1e-9 && ((lo[1] + hi[1]) / 2.0 - 60.0).abs() < 1e-9);
        // Laying the opposite face (x = 0, now on top) puts it down instead.
        b.reset_rotation();
        let (lo, hi) = b.bounds();
        assert!((hi[0] - lo[0] - 10.0).abs() < 1e-9 && lo[2].abs() < 1e-9);
    }

    #[test]
    fn align_a_face_with_a_wall() {
        let vol = [(0.0, 300.0), (0.0, 200.0), (0.0, 100.0)];
        let mut b = cube(); // 10 x 20 x 30
        let (n, p) = Wall::XMax.plane(&vol);
        b.mate_face(0, n, p, false).unwrap(); // the x = 10 face
        let (lo, hi) = b.bounds();
        assert!((hi[0] - 300.0).abs() < 1e-9 && (hi[0] - lo[0] - 10.0).abs() < 1e-9 && lo[2].abs() < 1e-9, "{lo:?} {hi:?}");
        let (n, p) = Wall::Top.plane(&vol);
        b.mate_face(0, n, p, false).unwrap();
        assert!((b.bounds().1[2] - 100.0).abs() < 1e-9);
        // Mate to the face of another part: the x = 0 face of a cube at x = 200
        // meets that cube's x = 10 face (at x = 210) from the outside.
        let mut a = cube();
        a.offset = [200.0, 0.0, 0.0];
        let (n, p) = a.face_plane(0).unwrap();
        let mut c = cube();
        c.mate_face(2, n, p, false).unwrap(); // some other face first to prove rotation
        let (cn, cp) = c.face_plane(2).unwrap();
        assert!((cn[0] + 1.0).abs() < 1e-9 && (cp[0] - 210.0).abs() < 1e-9, "{cn:?} {cp:?}");
    }

    #[test]
    fn quarter_turn_about_z_keeps_height_and_centre() {
        let mut b = cube();
        b.offset[2] = 5.0;
        b.rotate(&axis_rotation([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2), true);
        let (lo, hi) = b.bounds();
        assert!((lo[2] - 5.0).abs() < 1e-9);
        assert!((hi[0] - lo[0] - 20.0).abs() < 1e-9 && (hi[1] - lo[1] - 10.0).abs() < 1e-9);
        assert!(((lo[0] + hi[0]) / 2.0 - 105.0).abs() < 1e-9);
    }

    /// A square post, 4 x 4 around (10, 10), z 0..10, made of its four sides.
    fn post() -> Vec<Vec<[V3; 3]>> {
        let q = |a: V3, b: V3| vec![[a, b, [b[0], b[1], 10.0]], [a, [b[0], b[1], 10.0], [a[0], a[1], 10.0]]];
        let sides = [
            q([8.0, 8.0, 0.0], [12.0, 8.0, 0.0]),
            q([12.0, 8.0, 0.0], [12.0, 12.0, 0.0]),
            q([12.0, 12.0, 0.0], [8.0, 12.0, 0.0]),
            q([8.0, 12.0, 0.0], [8.0, 8.0, 0.0]),
        ];
        vec![sides.into_iter().flatten().collect()]
    }

    #[test]
    fn a_lead_in_through_a_post_goes_around_it_on_the_chosen_side() {
        let f = post();
        let in_post = |p: &V3| p[0] > 8.0 && p[0] < 12.0 && p[1] > 8.0 && p[1] < 12.0 && p[2] < 10.0;
        // Straight through the middle, 41 points from x = 0 to 20.
        let line: Vec<V3> = (0..=40).map(|k| [k as f64 * 0.5, 10.0, 5.0]).collect();
        for side in [1.0, -1.0] {
            let mut pts = line.clone();
            assert!(bypass(&f, &mut pts, 0.4, side));
            assert_eq!(pts[0], line[0]);
            assert_eq!(pts[40], line[40]);
            assert!(pts.iter().all(|p| !in_post(p)), "{pts:?}");
            // Left of +X is +Y: round the far side of the post, 2 + 0.4 away.
            let far = pts.iter().map(|p| (p[1] - 10.0) * side).fold(f64::MIN, f64::max);
            assert!(far >= 2.4 - 1e-6, "{far}");
            // Never on the other side.
            assert!(pts.iter().all(|p| (p[1] - 10.0) * side >= -1e-9));
            // Smooth: it moves sideways no faster than it goes along.
            assert!(pts.windows(2).all(|w| (w[1][1] - w[0][1]).abs() <= 0.5 + 1e-6), "{pts:?}");
        }
        // Above the post, or not through anything: nothing changes.
        let high: Vec<V3> = line.iter().map(|p| [p[0], p[1], 15.0]).collect();
        let mut pts = high.clone();
        assert!(!bypass(&f, &mut pts, 0.4, 1.0));
        assert_eq!(pts, high);
        let beside: Vec<V3> = line.iter().map(|p| [p[0], 20.0, 5.0]).collect();
        let mut pts = beside.clone();
        assert!(!bypass(&f, &mut pts, 0.4, 1.0));
        // A single wall, not a closed solid, is gone round the same way.
        let wall = vec![vec![[[10.0, 6.0, 0.0], [10.0, 14.0, 0.0], [10.0, 14.0, 10.0]], [[10.0, 6.0, 0.0], [10.0, 14.0, 10.0], [10.0, 6.0, 10.0]]]];
        let mut pts = line.clone();
        assert!(bypass(&wall, &mut pts, 0.4, 1.0));
        assert!(pts.iter().all(|p| (p[0] - 10.0).abs() > 1e-9 || (p[1] - 10.0).abs() >= 4.4 - 1e-6), "{pts:?}");
    }

    #[test]
    fn keep_out_pushes_a_point_away_on_the_side_it_is_on() {
        // A wall in the plane y = 0, x 0..20, z 0..20.
        let wall = vec![[[0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 0.0, 20.0]], [[0.0, 0.0, 0.0], [20.0, 0.0, 20.0], [0.0, 0.0, 20.0]]];
        let f = vec![wall];
        assert_eq!(keep_out(&f, [5.0, -0.5, 5.0], 2.0, false), [5.0, -2.0, 5.0]);
        assert_eq!(keep_out(&f, [5.0, 0.5, 5.0], 2.0, false), [5.0, 2.0, 5.0]);
        assert_eq!(keep_out(&f, [5.0, -3.0, 5.0], 2.0, false), [5.0, -3.0, 5.0]);
        // Past the end of the wall it is the end that matters.
        assert_eq!(keep_out(&f, [21.0, 0.0, 5.0], 2.0, false), [22.0, 0.0, 5.0]);
        // Flipped: to the other side of the wall.
        assert_eq!(keep_out(&f, [5.0, -0.5, 5.0], 2.0, true), [5.0, 2.0, 5.0]);
        assert_eq!(keep_out(&f, [5.0, -3.0, 5.0], 2.0, true), [5.0, -3.0, 5.0]);
    }

    #[test]
    fn lead_in_runs_from_the_previous_end_and_follows_picked_surfaces() {
        let mut p = plate();
        p.job.sample_mm = 5.0;
        p.job.z_gap = 0.0;
        // Step 0: a go to above the plate; step 1: the path at x 7 .. 17, y 5, z 10.
        p.sequence.push(Step3::new(Kind3::Goto { x: 27.0, y: 5.0, z: 30.0 }));
        p.sequence.push(path_step());
        let lead = p.lead_in_points(1);
        // From the go to point to the start of the path: 28.3 mm in 5 mm pieces.
        assert_eq!(lead.len(), 7, "{lead:?}");
        assert_eq!(lead[0], [27.0, 5.0, 30.0]);
        assert!((lead[6][0] - 7.0).abs() < 1e-9 && (lead[6][2] - 10.0).abs() < 1e-9);
        // With no surface the height is a straight run.
        assert!((lead[3][2] - (lead[0][2] + lead[6][2]) / 2.0).abs() < 1e-9);
        // Following the flat face (z = 10, x 5..25) at 2 mm: never closer than that.
        p.sequence[1].lead_surfaces = vec![(0, 0)];
        p.sequence[1].lead_offset = Some(2.0);
        let lead = p.lead_in_points(1);
        let inner = &lead[1..lead.len() - 1];
        assert!(inner.iter().all(|q| q[2] >= 12.0 - 1e-9), "{lead:?}");
        assert!(inner.iter().any(|q| (q[2] - 12.0).abs() < 1e-9), "some are pushed up to the offset");
        // Flipped, the points that were in the way go to the far side, below it.
        p.sequence[1].lead_flip = true;
        let flipped = p.lead_in_points(1);
        assert!(flipped[1..flipped.len() - 1].iter().any(|q| q[2] < 10.0), "{flipped:?}");
        p.sequence[1].lead_flip = false;
        // The two ends stay where the steps put them.
        assert_eq!(lead[0], [27.0, 5.0, 30.0]);
        assert!((lead.last().unwrap()[2] - 10.0).abs() < 1e-9);
        // The first step, a step that is not a path, or the switch off: no lead in.
        assert!(p.lead_in_points(0).is_empty());
        p.sequence[1].lead_in = false;
        assert!(p.lead_in_points(1).is_empty());
    }

    #[test]
    fn opened_steps_are_collapsed_and_new_ones_expanded() {
        let s = Step3::new(Kind3::Path);
        assert!(s.open);
        let back: Step3 = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!back.open);
    }
}

#[cfg(test)]
mod contains_tests {
    use super::*;
    use crate::p3d::step::{FaceMesh, Geometry};

    #[test]
    fn point_inside_a_cube_only() {
        let v = |x: f64, y: f64, z: f64| [x * 10.0, y * 10.0, z * 10.0];
        let quads = [
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.)],
            [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
            [v(0., 1., 0.), v(1., 1., 0.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(0., 1., 0.), v(0., 1., 1.), v(0., 0., 1.)],
            [v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.), v(1., 0., 1.)],
        ];
        let faces = quads.iter().map(|q| FaceMesh { tris: vec![[q[0], q[1], q[2]], [q[0], q[2], q[3]]], nors: vec![] }).collect();
        let g = Geometry { notes: vec![], faces, edges: vec![], min: [0.0; 3], max: [10.0; 3] };
        let b = Body::new("cube".into(), String::new(), [100.0, 0.0, 0.0], Some(Arc::new(g)));
        assert!(b.contains([105.0, 5.0, 5.0]));
        assert!(!b.contains([95.0, 5.0, 5.0]));
        assert!(!b.contains([105.0, 5.0, 15.0]));
    }
}
