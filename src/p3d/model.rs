//! Data of a 3D project and the path maths (surface Z gap, tool compensation).

use super::step::{self, Geometry, V3};
use crate::model::Machine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Job3 {
    pub dispense_feed: f64,
    pub travel_feed: f64,
    pub safe_z: f64,
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
            safe_z: 30.0,
            z_gap: 1.0,
            tool_diameter: 0.0,
            on_delay_ms: 200,
            off_delay_ms: 100,
            sample_mm: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Kind3 {
    /// Follow an edge of a body.
    Edge { body: usize, edge: usize },
    Home,
    Goto { x: f64, y: f64, z: f64 },
    Pause { ms: u32 },
}

/// Which side of the edge the tool centre runs on.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Side {
    /// Away from the chosen surface (the tool stays off the part).
    #[default]
    Outside,
    /// Onto the chosen surface.
    Inside,
    /// Left of the travel direction, seen from above.
    Left,
    Right,
    /// On the edge itself.
    None,
}

impl Side {
    pub const ALL: [Side; 5] = [Side::Outside, Side::Inside, Side::Left, Side::Right, Side::None];

    pub fn label(self) -> &'static str {
        match self {
            Side::Outside => "Outside the surface",
            Side::Inside => "Inside the surface",
            Side::Left => "Left of the path",
            Side::Right => "Right of the path",
            Side::None => "On the edge",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Step3 {
    pub kind: Kind3,
    pub reverse: bool,
    /// Surface (body, face) the Z gap is measured from. None follows the edge.
    #[serde(default)]
    pub surface: Option<(usize, usize)>,
    /// Z gap of this step; None uses the job value.
    #[serde(default)]
    pub gap: Option<f64>,
    #[serde(default)]
    pub feed: Option<f64>,
    /// Side of the edge for the tool diameter offset.
    #[serde(default)]
    pub side: Side,
}

impl Step3 {
    pub fn new(kind: Kind3) -> Self {
        Self { kind, reverse: false, surface: None, gap: None, feed: None, side: Side::Outside }
    }

    pub fn is_edge(&self) -> bool {
        matches!(self.kind, Kind3::Edge { .. })
    }

    pub fn customised(&self) -> bool {
        self.gap.is_some() || self.feed.is_some() || self.side != Side::Outside
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

    pub fn max_corner(&self) -> V3 {
        self.bounds().1
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

    /// Turn the part so the flat face `face` lies on the bed.
    pub fn lay_face_down(&mut self, face: usize) -> Result<(), String> {
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
        let r = rotation_between(n, [0.0, 0.0, -1.0]);
        self.rotate(&r, false);
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project3 {
    pub machine: Machine,
    pub job: Job3,
    pub bodies: Vec<Body>,
    pub sequence: Vec<Step3>,
}

impl Project3 {
    pub fn new(machine: Machine) -> Self {
        Self { machine, job: Job3::default(), bodies: vec![], sequence: vec![] }
    }

    pub fn remove_body(&mut self, idx: usize) {
        if idx >= self.bodies.len() {
            return;
        }
        self.bodies.remove(idx);
        self.sequence.retain(|s| match s.kind {
            Kind3::Edge { body, .. } => body != idx,
            _ => true,
        });
        for s in &mut self.sequence {
            if let Kind3::Edge { body, .. } = &mut s.kind {
                if *body > idx {
                    *body -= 1;
                }
            }
            if let Some(sf) = &mut s.surface {
                if sf.0 == idx {
                    s.surface = None;
                } else if sf.0 > idx {
                    sf.0 -= 1;
                }
            }
        }
    }

    pub fn edge_in_sequence(&self, body: usize, edge: usize) -> Option<usize> {
        self.sequence.iter().position(|s| s.kind == Kind3::Edge { body, edge })
    }

    /// Edge of a body in bed coordinates.
    pub fn edge_points(&self, body: usize, edge: usize) -> Vec<V3> {
        let Some(b) = self.bodies.get(body) else { return vec![] };
        let Some(g) = &b.geo else { return vec![] };
        g.edges.get(edge).map(|e| e.pts.iter().map(|p| b.world(*p)).collect()).unwrap_or_default()
    }

    /// The face next to an edge that faces up the most. The default surface
    /// of a new edge step.
    pub fn default_surface(&self, body: usize, edge: usize) -> Option<(usize, usize)> {
        let g = self.bodies.get(body)?.geo.as_ref()?;
        let e = g.edges.get(edge)?;
        e.faces
            .iter()
            .copied()
            .max_by(|a, b| g.face_normal(*a)[2].total_cmp(&g.face_normal(*b)[2]))
            .map(|f| (body, f))
    }

    pub fn new_edge_step(&self, body: usize, edge: usize) -> Step3 {
        let mut s = Step3::new(Kind3::Edge { body, edge });
        s.surface = self.default_surface(body, edge);
        s
    }

    /// Tool tip points of an edge step in bed coordinates, in travel order.
    /// Follows the surface with the Z gap and the tool compensation.
    pub fn step_points(&self, step: &Step3) -> Vec<V3> {
        let Kind3::Edge { body, edge } = step.kind else { return vec![] };
        let mut pts = self.edge_points(body, edge);
        let gap = step.gap.unwrap_or(self.job.z_gap);
        // The face in bed coordinates.
        let tris: Option<Vec<[V3; 3]>> = step.surface.and_then(|(sb, sf)| {
            let b = self.bodies.get(sb)?;
            let face = b.geo.as_ref()?.faces.get(sf)?;
            Some(face.tris.iter().map(|t| t.map(|p| b.world(p))).collect())
        });
        if let Some(tris) = &tris {
            // Z follows the surface, measured on the edge itself.
            pts = resample(&pts, self.job.sample_mm.max(0.05));
            for p in &mut pts {
                p[2] = surface_at(tris, [p[0], p[1]]).map_or(p[2], |s| s.0);
            }
        }
        for p in &mut pts {
            p[2] += gap;
        }
        // Tool diameter: move the path sideways by the radius.
        let r = self.job.tool_diameter.max(0.0) / 2.0;
        if r > 0.0 && step.side != Side::None {
            // Which way is "outside": away from the middle of the surface, or
            // from the middle of the part when there is no surface.
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
            offset_polyline(&mut pts, r, step.side, centre);
        }
        if step.reverse {
            pts.reverse();
        }
        pts
    }
}

/// Move a polyline sideways (in X and Y) by `dist`. `side` says which side;
/// Outside and Inside are decided against `centre`. Corners are mitred.
pub fn offset_polyline(pts: &mut [V3], dist: f64, side: Side, centre: [f64; 2]) {
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
    let sign = match side {
        Side::Left => 1.0,
        Side::Right => -1.0,
        Side::Outside | Side::Inside => {
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
            let out = if away >= 0.0 { 1.0 } else { -1.0 };
            if side == Side::Outside { out } else { -out }
        }
        Side::None => return,
    };
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

    #[test]
    fn z_gap_follows_the_surface() {
        let mut p = plate();
        let mut s = p.new_edge_step(0, 0);
        assert_eq!(s.surface, Some((0, 0)));
        s.gap = Some(1.5);
        p.job.sample_mm = 5.0;
        let pts = p.step_points(&s);
        assert_eq!(pts.len(), 3);
        assert!(pts.iter().all(|q| (q[2] - 11.5).abs() < 1e-9), "{pts:?}");
        // The body is offset by 5 in X.
        assert!((pts[0][0] - 7.0).abs() < 1e-9);
        s.reverse = true;
        assert!((p.step_points(&s)[0][0] - 17.0).abs() < 1e-9);
    }

    #[test]
    fn tool_diameter_offsets_the_path_sideways() {
        let mut p = plate();
        p.job.tool_diameter = 2.0;
        p.job.z_gap = 0.0;
        p.job.sample_mm = 50.0;
        // Edge from (7,5) to (17,5) on a face whose middle is at (15,10):
        // outside is toward -Y.
        let mut s = p.new_edge_step(0, 0);
        let pts = p.step_points(&s);
        assert!((pts[0][1] - 4.0).abs() < 1e-9 && (pts[0][0] - 7.0).abs() < 1e-9, "{pts:?}");
        assert!((pts[0][2] - 10.0).abs() < 1e-9, "Z stays on the surface");
        s.side = Side::Inside;
        assert!((p.step_points(&s)[0][1] - 6.0).abs() < 1e-9);
        s.side = Side::Left; // travelling +X, left is +Y
        assert!((p.step_points(&s)[0][1] - 6.0).abs() < 1e-9);
        s.side = Side::None;
        assert!((p.step_points(&s)[0][1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn corners_are_mitred() {
        // An L: right along +X then up +Y, offset to the right (outside the turn).
        let mut pts = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]];
        offset_polyline(&mut pts, 1.0, Side::Right, [0.0, 0.0]);
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
        b.lay_face_down(0).unwrap(); // the x = 10 face
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
    fn quarter_turn_about_z_keeps_height_and_centre() {
        let mut b = cube();
        b.offset[2] = 5.0;
        b.rotate(&axis_rotation([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2), true);
        let (lo, hi) = b.bounds();
        assert!((lo[2] - 5.0).abs() < 1e-9);
        assert!((hi[0] - lo[0] - 20.0).abs() < 1e-9 && (hi[1] - lo[1] - 10.0).abs() < 1e-9);
        assert!(((lo[0] + hi[0]) / 2.0 - 105.0).abs() < 1e-9);
    }
}
