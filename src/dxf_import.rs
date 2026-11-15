use crate::model::{ArcSpan, PathData, Pt};
use dxf::entities::EntityType;
use dxf::Drawing;
use std::collections::{BTreeMap, HashSet};
use std::f64::consts::PI;

const JOIN_TOL: f64 = 0.01; // mm
const CHORD_TOL: f64 = 0.02; // mm, arc flattening error

pub struct ImportResult {
    pub paths: Vec<PathData>,
    pub skipped: Vec<String>,
}

pub fn load_drawing(path: &std::path::Path) -> Result<Drawing, String> {
    Drawing::load_file(path).map_err(|e| format!("{e}"))
}

/// Layer names with their entity counts, sorted by name. Built from the
/// entities, so layers missing from the layer table are still listed.
pub fn layers(drawing: &Drawing) -> Vec<(String, usize)> {
    let mut map: BTreeMap<String, usize> = BTreeMap::new();
    for e in drawing.entities() {
        *map.entry(e.common.layer.clone()).or_insert(0) += 1;
    }
    map.into_iter().collect()
}

fn dist(a: Pt, b: Pt) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn arc_points(c: Pt, r: f64, a0: f64, sweep: f64) -> Vec<Pt> {
    let step = if r > CHORD_TOL {
        2.0 * (1.0 - CHORD_TOL / r).clamp(-1.0, 1.0).acos()
    } else {
        PI / 4.0
    }
    .max(0.02);
    let n = ((sweep.abs() / step).ceil() as usize).max(2);
    (0..=n)
        .map(|i| {
            let a = a0 + sweep * i as f64 / n as f64;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

/// Points of a bulge segment from p0 to p1, excluding p0, including p1,
/// plus the arc center when the segment is curved.
fn bulge_points(p0: Pt, p1: Pt, bulge: f64) -> (Vec<Pt>, Option<Pt>) {
    if bulge.abs() < 1e-9 {
        return (vec![p1], None);
    }
    let theta = 4.0 * bulge.atan();
    let d = dist(p0, p1);
    if d < 1e-12 {
        return (vec![p1], None);
    }
    let r = d / (2.0 * (theta / 2.0).sin().abs());
    let mid = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
    let dir = [(p1[0] - p0[0]) / d, (p1[1] - p0[1]) / d];
    let left = [-dir[1], dir[0]];
    let off = (d / 2.0) / (theta / 2.0).tan();
    let c = [mid[0] + left[0] * off, mid[1] + left[1] * off];
    let a0 = (p0[1] - c[1]).atan2(p0[0] - c[0]);
    let mut pts = arc_points(c, r, a0, theta);
    pts.remove(0);
    if let Some(last) = pts.last_mut() {
        *last = p1;
    }
    (pts, Some(c))
}

fn poly_points(verts: &[(Pt, f64)], closed: bool) -> (Vec<Pt>, Vec<ArcSpan>) {
    let mut pts: Vec<Pt> = vec![];
    let mut arcs: Vec<ArcSpan> = vec![];
    if verts.is_empty() {
        return (pts, arcs);
    }
    pts.push(verts[0].0);
    let count = if closed { verts.len() } else { verts.len() - 1 };
    for i in 0..count {
        let (p0, b) = verts[i];
        let p1 = verts[(i + 1) % verts.len()].0;
        let start = pts.len() - 1;
        let (seg, center) = bulge_points(p0, p1, b);
        pts.extend(seg);
        if let Some(center) = center {
            arcs.push(ArcSpan { start, end: pts.len() - 1, center, cw: b < 0.0 });
        }
    }
    // Popping the duplicate last point turns an arc ending there into one
    // that ends at index pts.len(), which means "back to the first point".
    if closed && pts.len() > 1 && dist(pts[0], *pts.last().unwrap()) < JOIN_TOL {
        pts.pop();
    }
    (pts, arcs)
}

/// Rational B-spline evaluation (de Boor). Weights are applied when given.
fn eval_spline(deg: usize, knots: &[f64], ctrl: &[Pt], weights: &[f64]) -> Vec<Pt> {
    let n = ctrl.len();
    if n < 2 || deg == 0 || knots.len() < n + deg + 1 {
        return ctrl.to_vec();
    }
    let w = |i: usize| weights.get(i).copied().unwrap_or(1.0);
    let t0 = knots[deg];
    let t1 = knots[n];
    let samples = (n * 24).max(32);
    let mut out = vec![];
    for s in 0..=samples {
        let t = if s == samples {
            t1
        } else {
            t0 + (t1 - t0) * s as f64 / samples as f64
        };
        // find span
        let mut k = deg;
        while k + 1 < n && t >= knots[k + 1] {
            k += 1;
        }
        let mut d: Vec<[f64; 3]> = (0..=deg)
            .map(|j| {
                let i = j + k - deg;
                [ctrl[i][0] * w(i), ctrl[i][1] * w(i), w(i)]
            })
            .collect();
        for r in 1..=deg {
            for j in (r..=deg).rev() {
                let i = j + k - deg;
                let den = knots[i + deg + 1 - r] - knots[i];
                let a = if den.abs() < 1e-14 { 0.0 } else { (t - knots[i]) / den };
                for c in 0..3 {
                    d[j][c] = (1.0 - a) * d[j - 1][c] + a * d[j][c];
                }
            }
        }
        let v = d[deg];
        if v[2].abs() > 1e-14 {
            out.push([v[0] / v[2], v[1] / v[2]]);
        }
    }
    out
}

struct Seg {
    pts: Vec<Pt>,
    closed: bool,
    arcs: Vec<ArcSpan>,
}

pub fn import(drawing: &Drawing) -> ImportResult {
    import_layers(drawing, None)
}

/// Import entities, only from the given layers when a filter is set.
pub fn import_layers(drawing: &Drawing, only: Option<&HashSet<String>>) -> ImportResult {
    let mut segs: Vec<Seg> = vec![];
    let mut skipped: Vec<String> = vec![];
    let mut note = |s: &str| {
        if !skipped.iter().any(|x| x == s) {
            skipped.push(s.to_string());
        }
    };

    for e in drawing.entities() {
        if only.is_some_and(|set| !set.contains(&e.common.layer)) {
            continue;
        }
        match &e.specific {
            EntityType::Line(l) => segs.push(Seg {
                pts: vec![[l.p1.x, l.p1.y], [l.p2.x, l.p2.y]],
                closed: false,
                arcs: vec![],
            }),
            EntityType::LwPolyline(p) => {
                let verts: Vec<(Pt, f64)> =
                    p.vertices.iter().map(|v| ([v.x, v.y], v.bulge)).collect();
                let closed = p.flags & 1 != 0;
                let (pts, arcs) = poly_points(&verts, closed);
                segs.push(Seg { pts, closed, arcs });
            }
            EntityType::Polyline(p) => {
                let verts: Vec<(Pt, f64)> = p
                    .vertices()
                    .map(|v| ([v.location.x, v.location.y], v.bulge))
                    .collect();
                let closed = p.flags & 1 != 0;
                let (pts, arcs) = poly_points(&verts, closed);
                segs.push(Seg { pts, closed, arcs });
            }
            EntityType::Arc(a) => {
                let s = a.start_angle.to_radians();
                let mut en = a.end_angle.to_radians();
                while en <= s {
                    en += 2.0 * PI;
                }
                let pts = arc_points([a.center.x, a.center.y], a.radius, s, en - s);
                let arcs = vec![ArcSpan {
                    start: 0,
                    end: pts.len() - 1,
                    center: [a.center.x, a.center.y],
                    cw: false,
                }];
                segs.push(Seg { pts, closed: false, arcs });
            }
            EntityType::Circle(c) => {
                let mut pts = arc_points([c.center.x, c.center.y], c.radius, 0.0, 2.0 * PI);
                pts.pop();
                // Two half circles: a full circle with equal start and end
                // point is rejected by some controllers.
                let n = pts.len();
                let center = [c.center.x, c.center.y];
                let arcs = if n >= 4 {
                    vec![
                        ArcSpan { start: 0, end: n / 2, center, cw: false },
                        ArcSpan { start: n / 2, end: n, center, cw: false },
                    ]
                } else {
                    vec![]
                };
                segs.push(Seg { pts, closed: true, arcs });
            }
            EntityType::Spline(s) => {
                let ctrl: Vec<Pt> = s.control_points.iter().map(|p| [p.x, p.y]).collect();
                let pts = if s.fit_points.len() >= 2 {
                    s.fit_points.iter().map(|p| [p.x, p.y]).collect()
                } else {
                    eval_spline(
                        s.degree_of_curve.max(1) as usize,
                        &s.knot_values,
                        &ctrl,
                        &s.weight_values,
                    )
                };
                segs.push(Seg { pts, closed: false, arcs: vec![] });
            }
            other => {
                let name = format!("{:?}", other);
                let kind = name.split(['(', ' ', '{']).next().unwrap_or("entity");
                note(&format!("Skipped {kind} entities"));
            }
        }
    }

    segs.retain(|s| s.pts.len() >= 2);
    ImportResult { paths: chain(segs), skipped }
}

fn rev_arcs(arcs: &[ArcSpan], n: usize) -> Vec<ArcSpan> {
    let mut v: Vec<ArcSpan> = arcs.iter().map(|a| a.reversed(n)).collect();
    v.sort_by_key(|a| a.start);
    v
}

fn shifted(arcs: &[ArcSpan], by: usize) -> Vec<ArcSpan> {
    arcs.iter()
        .map(|a| ArcSpan { start: a.start + by, end: a.end + by, ..*a })
        .collect()
}

/// Join open segments whose end points touch into longer paths.
fn chain(segs: Vec<Seg>) -> Vec<PathData> {
    let mut out: Vec<PathData> = vec![];
    let mut open: Vec<Option<(Vec<Pt>, Vec<ArcSpan>)>> = vec![];
    for s in segs {
        if s.closed {
            out.push(PathData { pts: s.pts, closed: true, arcs: s.arcs });
        } else {
            open.push(Some((s.pts, s.arcs)));
        }
    }

    for i in 0..open.len() {
        let Some((mut cur, mut cur_arcs)) = open[i].take() else { continue };
        loop {
            let mut extended = false;
            for j in 0..open.len() {
                let Some((other, _)) = open[j].as_ref() else { continue };
                let end = *cur.last().unwrap();
                let start = cur[0];
                let (o0, o1) = (other[0], *other.last().unwrap());
                if dist(end, o0) < JOIN_TOL {
                    let (o, oa) = open[j].take().unwrap();
                    cur_arcs.extend(shifted(&oa, cur.len() - 1));
                    cur.extend(o.into_iter().skip(1));
                } else if dist(end, o1) < JOIN_TOL {
                    let (o, oa) = open[j].take().unwrap();
                    cur_arcs.extend(shifted(&rev_arcs(&oa, o.len()), cur.len() - 1));
                    cur.extend(o.into_iter().rev().skip(1));
                } else if dist(start, o1) < JOIN_TOL {
                    let (mut o, oa) = open[j].take().unwrap();
                    let mut arcs = oa;
                    arcs.extend(shifted(&cur_arcs, o.len() - 1));
                    o.extend(cur.into_iter().skip(1));
                    cur = o;
                    cur_arcs = arcs;
                } else if dist(start, o0) < JOIN_TOL {
                    let (mut o, oa) = open[j].take().unwrap();
                    let mut arcs = rev_arcs(&oa, o.len());
                    arcs.extend(shifted(&cur_arcs, o.len() - 1));
                    o.reverse();
                    o.extend(cur.into_iter().skip(1));
                    cur = o;
                    cur_arcs = arcs;
                } else {
                    continue;
                }
                extended = true;
                break;
            }
            if !extended {
                break;
            }
        }
        let closed = cur.len() > 2 && dist(cur[0], *cur.last().unwrap()) < JOIN_TOL;
        if closed {
            // An arc ending on the dropped last point now ends at cur.len(),
            // the "back to the first point" index.
            cur.pop();
        }
        out.push(PathData { pts: cur, closed, arcs: cur_arcs });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dxf::entities::{Entity, Line};
    use dxf::Point;

    fn line(a: (f64, f64), b: (f64, f64)) -> Entity {
        Entity::new(EntityType::Line(Line::new(
            Point::new(a.0, a.1, 0.0),
            Point::new(b.0, b.1, 0.0),
        )))
    }

    #[test]
    fn four_lines_become_one_closed_path() {
        let mut d = Drawing::new();
        // Deliberately shuffled and with one reversed line.
        d.add_entity(line((0.0, 0.0), (10.0, 0.0)));
        d.add_entity(line((10.0, 10.0), (0.0, 10.0)));
        d.add_entity(line((10.0, 10.0), (10.0, 0.0)));
        d.add_entity(line((0.0, 10.0), (0.0, 0.0)));
        let r = import(&d);
        assert_eq!(r.paths.len(), 1);
        assert!(r.paths[0].closed);
        assert_eq!(r.paths[0].pts.len(), 4);
    }

    #[test]
    fn layer_filter() {
        let mut d = Drawing::new();
        let mut a = line((0.0, 0.0), (10.0, 0.0));
        a.common.layer = "CUT".into();
        let mut b = line((0.0, 5.0), (10.0, 5.0));
        b.common.layer = "GLUE".into();
        d.add_entity(a);
        d.add_entity(b);
        assert_eq!(layers(&d), vec![("CUT".to_string(), 1), ("GLUE".to_string(), 1)]);
        let only: HashSet<String> = ["GLUE".to_string()].into();
        let r = import_layers(&d, Some(&only));
        assert_eq!(r.paths.len(), 1);
        assert_eq!(r.paths[0].pts[0], [0.0, 5.0]);
    }

    #[test]
    fn circle_is_closed_and_round() {
        let mut d = Drawing::new();
        d.add_entity(Entity::new(EntityType::Circle(dxf::entities::Circle::new(
            Point::new(5.0, 5.0, 0.0),
            10.0,
        ))));
        let r = import(&d);
        assert_eq!(r.paths.len(), 1);
        assert!(r.paths[0].closed);
        for p in &r.paths[0].pts {
            assert!((dist(*p, [5.0, 5.0]) - 10.0).abs() < 1e-6);
        }
    }

    #[test]
    fn bulge_semicircle() {
        // bulge 1 = half circle, CCW from (0,0) to (10,0): bulges below the chord.
        let (pts, center) = bulge_points([0.0, 0.0], [10.0, 0.0], 1.0);
        assert!(dist(center.unwrap(), [5.0, 0.0]) < 1e-9);
        let mid = pts[pts.len() / 2];
        assert!((dist(mid, [5.0, 0.0]) - 5.0).abs() < 0.05, "{mid:?}");
        assert!(mid[1] < 0.0);
        assert_eq!(*pts.last().unwrap(), [10.0, 0.0]);
    }

    #[test]
    fn arcs_survive_chaining_and_reversal() {
        // Quarter circle radius 10 about the origin, then a line to the start.
        let mut d = Drawing::new();
        d.add_entity(Entity::new(EntityType::Arc(dxf::entities::Arc::new(
            Point::new(0.0, 0.0, 0.0),
            10.0,
            0.0,
            90.0,
        ))));
        d.add_entity(line((0.0, 10.0), (0.0, 0.0)));
        d.add_entity(line((0.0, 0.0), (10.0, 0.0)));
        let r = import(&d);
        assert_eq!(r.paths.len(), 1);
        let p = &r.paths[0];
        assert!(p.closed);
        assert_eq!(p.arcs.len(), 1);
        let a = p.arcs[0];
        assert!(!a.cw);
        assert!(dist(p.pts[a.start], [10.0, 0.0]) < 1e-9 || dist(p.pts[a.start], [0.0, 10.0]) < 1e-9);
        // Every flattened point of the span lies on the circle.
        for k in a.start..=a.end {
            assert!((dist(p.pts[k], a.center) - 10.0).abs() < 1e-6);
        }
    }

    #[test]
    fn circle_has_two_half_arcs() {
        let mut d = Drawing::new();
        d.add_entity(Entity::new(EntityType::Circle(dxf::entities::Circle::new(
            Point::new(5.0, 5.0, 0.0),
            10.0,
        ))));
        let p = &import(&d).paths[0];
        assert_eq!(p.arcs.len(), 2);
        assert_eq!(p.arcs[1].end, p.pts.len());
    }

    #[test]
    fn spline_endpoints() {
        // Degree 2 Bezier-like spline with clamped knots.
        let ctrl = [[0.0, 0.0], [5.0, 10.0], [10.0, 0.0]];
        let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let pts = eval_spline(2, &knots, &ctrl, &[]);
        assert!(dist(pts[0], [0.0, 0.0]) < 1e-9);
        assert!(dist(*pts.last().unwrap(), [10.0, 0.0]) < 1e-9);
        // Midpoint of this quadratic Bezier is (5, 5).
        let m = pts[pts.len() / 2];
        assert!(dist(m, [5.0, 5.0]) < 0.05, "{m:?}");
    }
}
