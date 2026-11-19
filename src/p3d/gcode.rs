//! G-code for a 3D project. Own copy of the writer, the 2D one stays untouched.

use super::model::{Kind3, Project3};
use crate::gcode::Move;
use crate::model::ControlType;

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
    if job.safe_z > travel[2] {
        warnings.push("Safe Z is above the Z travel of the machine.".into());
    }
    if ee.control == ControlType::Pwm && ee.pwm_max <= ee.pwm_min {
        warnings.push("PWM max must be larger than PWM min.".into());
    }
    let top = p.bodies.iter().map(|b| b.max_corner()[2]).fold(f64::MIN, f64::max);
    if !p.bodies.is_empty() && job.safe_z < top {
        warnings.push(format!("Safe Z ({:.1}) is below the top of the parts ({:.1}).", job.safe_z, top));
    }

    comment(&mut out, format!("Machine: {}  Tool: {}  (3D project)", m.name, ee.kind.label()));
    for l in m.pre_script.lines().filter(|l| !l.trim().is_empty()) {
        out.push(l.trim().to_string());
    }
    out.push(ee.off_line());
    out.push(format!("{} Z{}", st.rapid_cmd, num(job.safe_z, d)));
    cur[2] = job.safe_z;

    for (i, step) in p.sequence.iter().enumerate() {
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
                let zs = job.safe_z.max(cur[2]);
                out.push(format!("{} Z{}{}", st.rapid_cmd, num(zs, d), rapid_f));
                { let c = cur; go(&mut moves, &mut cur, [c[0], c[1], zs], false); }
                out.push(format!("{} X{} Y{}{}", st.rapid_cmd, num(x, d), num(y, d), rapid_f));
                { let c = cur; go(&mut moves, &mut cur, [x, y, c[2]], false); }
                out.push(format!("{} Z{}{}", st.rapid_cmd, num(z, d), rapid_f));
                go(&mut moves, &mut cur, [x, y, z], false);
            }
            Kind3::Edge { body, edge } => {
                let pts = p.step_points(step);
                if pts.len() < 2 {
                    warnings.push(format!("Step {} has no path.", i + 1));
                    continue;
                }
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
                let feed = step.feed.unwrap_or(job.dispense_feed);
                let first = pts[0];
                let zs = job.safe_z.max(cur[2]).max(first[2]);
                out.push(format!("{} Z{}{}", st.rapid_cmd, num(zs, d), rapid_f));
                { let c = cur; go(&mut moves, &mut cur, [c[0], c[1], zs], false); }
                out.push(format!("{} X{} Y{}{}", st.rapid_cmd, num(first[0], d), num(first[1], d), rapid_f));
                go(&mut moves, &mut cur, [first[0], first[1], zs], false);
                out.push(format!("{} Z{} F{}", st.linear_cmd, num(first[2], d), num(feed, 0)));
                go(&mut moves, &mut cur, first, false);
                out.push(ee.on_line());
                if job.on_delay_ms > 0 {
                    out.push(dwell(job.on_delay_ms));
                }
                for (k, q) in pts.iter().enumerate().skip(1) {
                    let f = if st.feed_every_move || k == 1 { format!(" F{}", num(feed, 0)) } else { String::new() };
                    out.push(format!("{} X{} Y{} Z{}{}", st.linear_cmd, num(q[0], d), num(q[1], d), num(q[2], d), f));
                    go(&mut moves, &mut cur, *q, true);
                }
                out.push(ee.off_line());
                if job.off_delay_ms > 0 {
                    out.push(dwell(job.off_delay_ms));
                }
                let zs = job.safe_z.max(cur[2]);
                out.push(format!("{} Z{}{}", st.rapid_cmd, num(zs, d), rapid_f));
                { let c = cur; go(&mut moves, &mut cur, [c[0], c[1], zs], false); }
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
