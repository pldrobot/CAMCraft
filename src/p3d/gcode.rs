//! G-code for a 3D project. Own copy of the writer, the 2D one stays untouched.

use super::model::{Kind3, Project3};
use super::step::V3;
use crate::common::toolpath::Move;
use crate::common::machine::ControlType;

pub struct Result3 {
    pub text: String,
    pub warnings: Vec<String>,
    pub moves: Vec<Move>,
}

fn num(v: f64, dec: usize) -> String {
    let mut s = format!("{:.*}", dec, v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

pub fn generate(p: &Project3) -> Result3 {
    let m = &p.machine;
    let st = &m.gcode;
    let d = st.decimals;
    let job = &p.job;
    let ee = &m.effector;
    let travel = m.travel();
    let mut out: Vec<String> = vec![];
    let mut warnings: Vec<String> = vec![];
    let mut moves: Vec<Move> = vec![];
    let home = m.home_position();
    let mut cur = [home[0], home[1], home[2]];
    let go = |moves: &mut Vec<Move>, cur: &mut [f64; 3], to: [f64; 3], work: bool| {
        if (0..3).any(|i| (cur[i] - to[i]).abs() > 1e-9) {
            moves.push(Move { from: *cur, to, work });
        }
        *cur = to;
    };
    let dwell = |ms: u32| st.dwell.replace("{ms}", &ms.to_string()).replace("{s}", &num(ms as f64 / 1000.0, 3));
    let comment = |out: &mut Vec<String>, t: String| {
        if st.comments {
            out.push(format!("; {t}"));
        }
    };

    if p.sequence.is_empty() {
        warnings.push("No steps in the sequence.".into());
    }
    if ee.control == ControlType::Pwm && ee.pwm_max <= ee.pwm_min {
        warnings.push("PWM max must be larger than PWM min.".into());
    }

    comment(&mut out, format!("Machine: {}  Tool: {}  (3D project)", m.name, ee.kind.label()));
    for l in m.pre_script.lines().filter(|l| !l.trim().is_empty()) {
        out.push(l.trim().to_string());
    }
    out.push(ee.off_line());

    for (i, step) in p.sequence.iter().enumerate() {
        // Job settings of this step: the job values with its overrides.
        let job_step = job.with(&step.over);
        let job = &job_step;
        let rapid_f = if st.feed_every_move { format!(" F{}", num(job.travel_feed, 0)) } else { String::new() };
        match step.kind {
            Kind3::Pause { ms } => {
                comment(&mut out, format!("Step {}: pause {} ms", i + 1, ms));
                out.push(ee.off_line());
                if ms > 0 {
                    out.push(dwell(ms));
                }
            }
            Kind3::Home | Kind3::Goto { .. } => {
                let (x, y, z) = match step.kind {
                    Kind3::Goto { x, y, z } => (x, y, z),
                    _ => (home[0], home[1], home[2]),
                };
                comment(&mut out, format!("Step {}: go to X{} Y{} Z{}", i + 1, num(x, d), num(y, d), num(z, d)));
                out.push(ee.off_line());
                // Straight there: there is no safe Z to lift to.
                out.push(format!("{} X{} Y{} Z{}{}", st.rapid_cmd, num(x, d), num(y, d), num(z, d), rapid_f));
                go(&mut moves, &mut cur, [x, y, z], false);
            }
            Kind3::Path => {
                let pts = p.step_points(step);
                let Some((body, edge)) = step.path.filter(|_| pts.len() >= 2) else {
                    warnings.push(if step.path.is_some() && (step.start.is_some() || step.stop.is_some()) {
                        format!("Step {}: nothing is left between the start and the stop.", i + 1)
                    } else {
                        format!("Step {} has no path.", i + 1)
                    });
                    continue;
                };
                let name = p.bodies.get(body).map(|b| b.name.as_str()).unwrap_or("?");
                comment(&mut out, format!("Step {} ({}, edge {})", i + 1, name, edge + 1));
                if step.surface.is_none() {
                    warnings.push(format!("Step {} has no surface, Z follows the edge.", i + 1));
                }
                if pts.iter().any(|q| !m.in_workable([q[0], q[1]])) {
                    warnings.push(format!("Step {} goes outside the workable area.", i + 1));
                }
                if pts.iter().any(|q| q[2] < -1e-6) {
                    warnings.push(format!("Step {} goes below the bed (Z < 0).", i + 1));
                }
                if pts.iter().any(|q| q[2] > travel[2] + 1e-6) {
                    warnings.push(format!("Step {} goes above the Z travel.", i + 1));
                }
                let feed = job.dispense_feed;
                let first = pts[0];
                // Lead in: straight from where the previous step left the tool,
                // tool off.
                let lead = p.lead_in_points(i);
                let at_lead = lead.len() >= 2 && (0..3).map(|k| (cur[k] - lead[0][k]).powi(2)).sum::<f64>().sqrt() < 0.01;
                if at_lead {
                    comment(&mut out, "Lead in".into());
                    if step.lead_surfaces.is_empty() {
                        warnings.push(format!("Step {}: the lead in is a straight line, pick surfaces to follow.", i + 1));
                    }
                    for q in lead.iter().skip(1) {
                        out.push(format!("{} X{} Y{} Z{} F{}", st.linear_cmd, num(q[0], d), num(q[1], d), num(q[2], d), num(feed, 0)));
                        go(&mut moves, &mut cur, *q, false);
                    }
                } else {
                    // No lead in and no safe Z: straight to the start.
                    warnings.push(format!("Step {} moves straight to its start, with no lead in to follow.", i + 1));
                    out.push(format!("{} X{} Y{} Z{}{}", st.rapid_cmd, num(first[0], d), num(first[1], d), num(first[2], d), rapid_f));
                    go(&mut moves, &mut cur, first, false);
                }
                out.push(ee.on_line());
                if job.on_delay_ms > 0 {
                    out.push(dwell(job.on_delay_ms));
                }
                // With arcs on, runs of points on a circle become G2/G3.
                let segs = if st.arcs { fit_arcs(&pts, ARC_TOL) } else { (1..pts.len()).map(|k| (k, None)).collect() };
                let mut from = 0;
                for (k, arc) in segs {
                    let q = pts[k];
                    let f = if st.feed_every_move || from == 0 { format!(" F{}", num(feed, 0)) } else { String::new() };
                    match arc {
                        Some((c, cw)) => {
                            let a = pts[from];
                            out.push(format!(
                                "{} X{} Y{} Z{} I{} J{}{}",
                                if cw { &st.cw_cmd } else { &st.ccw_cmd },
                                num(q[0], d),
                                num(q[1], d),
                                num(q[2], d),
                                num(c[0] - a[0], d),
                                num(c[1] - a[1], d),
                                f
                            ));
                        }
                        None => out.push(format!("{} X{} Y{} Z{}{}", st.linear_cmd, num(q[0], d), num(q[1], d), num(q[2], d), f)),
                    }
                    // The preview follows the original points.
                    for r in &pts[from + 1..=k] {
                        go(&mut moves, &mut cur, *r, true);
                    }
                    from = k;
                }
                out.push(ee.off_line());
                if job.off_delay_ms > 0 {
                    out.push(dwell(job.off_delay_ms));
                }
            }
        }
    }

    comment(&mut out, "End".into());
    out.push(ee.off_line());
    for l in m.post_script.lines().filter(|l| !l.trim().is_empty()) {
        out.push(l.trim().to_string());
    }
    let mut text = out.join("\n");
    text.push('\n');
    Result3 { text, warnings, moves }
}

/// How far points may be from the arc, mm. Also how far Z may be from a straight
/// climb along it (a helix).
const ARC_TOL: f64 = 0.01;

/// Center and direction (true = clockwise) of the circle the points lie on in
/// X and Y, if they all do, turn one way, make less than a full turn and climb
/// evenly in Z.
fn arc_through(seg: &[V3], tol: f64) -> Option<([f64; 2], bool)> {
    let (a, b, c) = (seg[0], seg[seg.len() / 2], seg[seg.len() - 1]);
    // Circle through the first, middle and last point: equal radius at both ends.
    let (ax, ay, bx, by, cx, cy) = (a[0], a[1], b[0], b[1], c[0], c[1]);
    let den = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if den.abs() < 1e-9 {
        return None;
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / den;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / den;
    let r = (ax - ux).hypot(ay - uy);
    if !(0.2..=5000.0).contains(&r) {
        return None;
    }
    let ang = |p: V3| (p[1] - uy).atan2(p[0] - ux);
    let (mut prev, mut total, mut dir) = (ang(seg[0]), 0.0, 0.0_f64);
    let mut sweep = vec![0.0];
    for p in &seg[1..] {
        if ((p[0] - ux).hypot(p[1] - uy) - r).abs() > tol {
            return None;
        }
        let now = ang(*p);
        let mut step = now - prev;
        while step > std::f64::consts::PI {
            step -= 2.0 * std::f64::consts::PI;
        }
        while step <= -std::f64::consts::PI {
            step += 2.0 * std::f64::consts::PI;
        }
        if step.abs() < 1e-9 || (dir != 0.0 && step.signum() != dir) {
            return None;
        }
        dir = step.signum();
        total += step;
        sweep.push(total);
        prev = now;
    }
    if total.abs() > 2.0 * std::f64::consts::PI * 0.97 {
        return None;
    }
    let (z0, z1) = (seg[0][2], seg[seg.len() - 1][2]);
    for (p, s) in seg.iter().zip(&sweep) {
        if (p[2] - (z0 + (z1 - z0) * s / total)).abs() > tol {
            return None;
        }
    }
    Some(([ux, uy], dir < 0.0))
}

/// Cuts a path into straight pieces and arcs. Each entry is the index it ends
/// at and, for an arc, its center and direction. An arc covers at least 4 points.
fn fit_arcs(pts: &[V3], tol: f64) -> Vec<(usize, Option<([f64; 2], bool)>)> {
    let (n, mut out, mut i) = (pts.len(), vec![], 0);
    while i + 1 < n {
        let mut best = None;
        let mut j = i + 3;
        while j < n {
            match arc_through(&pts[i..=j], tol) {
                Some(a) => best = Some((j, a)),
                None => break,
            }
            j += 1;
        }
        match best {
            Some((j, a)) => {
                out.push((j, Some(a)));
                i = j;
            }
            None => {
                out.push((i + 1, None));
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_half_circle_becomes_one_arc_and_a_line_stays_lines() {
        // Counter-clockwise half circle of radius 10 about (50, 50), 21 points.
        let ccw: Vec<V3> = (0..=20)
            .map(|k| {
                let a = std::f64::consts::PI * k as f64 / 20.0;
                [50.0 + 10.0 * a.cos(), 50.0 + 10.0 * a.sin(), 3.0]
            })
            .collect();
        let segs = fit_arcs(&ccw, ARC_TOL);
        assert_eq!(segs.len(), 1);
        let (to, arc) = &segs[0];
        let (c, cw) = arc.unwrap();
        assert_eq!(*to, 20);
        assert!((c[0] - 50.0).abs() < 1e-6 && (c[1] - 50.0).abs() < 1e-6 && !cw);
        // The other way round is clockwise.
        let mut rev = ccw.clone();
        rev.reverse();
        assert!(fit_arcs(&rev, ARC_TOL)[0].1.unwrap().1);
        // A climbing arc is a helix and is still one arc; a bump in Z is not.
        let mut helix = ccw.clone();
        for (k, p) in helix.iter_mut().enumerate() {
            p[2] = k as f64 * 0.1;
        }
        assert_eq!(fit_arcs(&helix, ARC_TOL).len(), 1);
        helix[10][2] += 0.5;
        assert!(fit_arcs(&helix, ARC_TOL).len() > 1);
        // Straight points never make an arc.
        let line: Vec<V3> = (0..10).map(|k| [k as f64, 0.0, 0.0]).collect();
        assert!(fit_arcs(&line, ARC_TOL).iter().all(|s| s.1.is_none()));
        assert_eq!(fit_arcs(&line, ARC_TOL).len(), 9);
    }
}
