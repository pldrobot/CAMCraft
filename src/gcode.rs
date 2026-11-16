use crate::model::{ArcSpan, ControlType, GcodeStyle, Project, Pt};

/// One straight piece of the toolpath, for the preview.
#[derive(Clone, Copy, Debug)]
pub struct Move {
    pub from: [f64; 3],
    pub to: [f64; 3],
    /// True while the end effector is on.
    pub work: bool,
}

pub struct GcodeResult {
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

fn dwell(st: &GcodeStyle, ms: u32) -> String {
    st
        .dwell
        .replace("{ms}", &ms.to_string())
        .replace("{s}", &num(ms as f64 / 1000.0, 3))
}

fn dist(a: Pt, b: Pt) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// An arc is only written as G2/G3 when the radius seen from the real start
/// and end point agrees, otherwise controllers such as GRBL reject it.
fn arc_ok(a: &ArcSpan, from: Pt, to: Pt) -> bool {
    let (r0, r1) = (dist(from, a.center), dist(to, a.center));
    r0 > 1e-6 && (r0 - r1).abs() < 0.003
}

pub fn generate(project: &Project) -> GcodeResult {
    let st = &project.machine.gcode;
    let d = st.decimals;
    let job = &project.job;
    let m = &project.machine;
    let ee = &m.effector;
    let travel = m.travel();
    let mut out: Vec<String> = vec![];
    let mut warnings: Vec<String> = vec![];
    let mut moves: Vec<Move> = vec![];
    let home = m.home_position();
    let mut cur = [home[0], home[1], if job.use_z { home[2] } else { 0.0 }];
    let push_move = |moves: &mut Vec<Move>, cur: &mut [f64; 3], to: [f64; 3], work: bool| {
        if (0..3).any(|i| (cur[i] - to[i]).abs() > 1e-9) {
            moves.push(Move { from: *cur, to, work });
        }
        *cur = to;
    };

    if project.sequence.is_empty() {
        warnings.push("No paths in the sequence.".into());
    }
    if job.use_z && job.dispense_z > travel[2] {
        warnings.push("Work Z is above the Z travel of the machine.".into());
    }
    if job.use_z && job.safe_z > travel[2] {
        warnings.push("Safe Z is above the Z travel of the machine.".into());
    }
    if job.use_z && (job.dispense_z < 0.0 || job.safe_z < 0.0) {
        warnings.push("Negative Z is below the machine zero.".into());
    }
    if ee.control == ControlType::Pwm && ee.pwm_max <= ee.pwm_min {
        warnings.push("PWM max must be larger than PWM min.".into());
    }

    let comment = |out: &mut Vec<String>, t: String| {
        if st.comments {
            out.push(format!("; {t}"));
        }
    };

    comment(
        &mut out,
        format!("Machine: {}  Tool: {}", m.name, ee.kind.label()),
    );
    for l in m.pre_script.lines().filter(|l| !l.trim().is_empty()) {
        out.push(l.trim().to_string());
    }
    // The end effector must be off before anything moves.
    out.push(ee.off_line());
    if job.use_z {
        out.push(format!("{} Z{}", st.rapid_cmd, num(job.safe_z, d)));
    }

    let rapid_f = if st.rapid_with_feed {
        format!(" F{}", num(job.travel_feed, 0))
    } else {
        String::new()
    };

    for (i, item) in project.sequence.iter().enumerate() {
        if item.part >= project.parts.len()
            || item.path >= project.parts[item.part].paths.len()
        {
            continue;
        }
        let (pts, arcs) = project.world_path(item);
        if pts.len() < 2 {
            continue;
        }
        let part = &project.parts[item.part];
        comment(
            &mut out,
            format!("Path {} ({}, #{})", i + 1, part.name, item.path + 1),
        );
        if pts.iter().any(|p| !m.in_workable(*p)) {
            warnings.push(format!(
                "Path {} ({}) goes outside the workable area.",
                i + 1,
                part.name
            ));
        }
        let z_work = if job.use_z { job.dispense_z } else { cur[2] };
        let z_safe = if job.use_z { job.safe_z } else { cur[2] };
        { let c = cur; push_move(&mut moves, &mut cur, [c[0], c[1], z_safe], false); }
        push_move(&mut moves, &mut cur, [pts[0][0], pts[0][1], z_safe], false);

        out.push(format!(
            "{} X{} Y{}{}",
            st.rapid_cmd,
            num(pts[0][0], d),
            num(pts[0][1], d),
            rapid_f
        ));
        if job.use_z {
            out.push(format!(
                "{} Z{} F{}",
                st.linear_cmd,
                num(job.dispense_z, d),
                num(job.dispense_feed, 0)
            ));
        }
        push_move(&mut moves, &mut cur, [pts[0][0], pts[0][1], z_work], false);
        out.push(ee.on_line());
        if job.on_delay_ms > 0 {
            out.push(dwell(st, job.on_delay_ms));
        }
        let mut k = 1;
        while k < pts.len() {
            let f = if k == 1 && !job.use_z {
                format!(" F{}", num(job.dispense_feed, 0))
            } else {
                String::new()
            };
            let arc = if st.arcs {
                arcs.iter().find(|a| a.start == k - 1 && arc_ok(a, pts[a.start], pts[a.end]))
            } else {
                None
            };
            if let Some(a) = arc {
                let from = pts[a.start];
                let to = pts[a.end];
                out.push(format!(
                    "{} X{} Y{} I{} J{}{}",
                    if a.cw { &st.cw_cmd } else { &st.ccw_cmd },
                    num(to[0], d),
                    num(to[1], d),
                    num(a.center[0] - from[0], d),
                    num(a.center[1] - from[1], d),
                    f
                ));
                for q in &pts[a.start + 1..=a.end] {
                    push_move(&mut moves, &mut cur, [q[0], q[1], z_work], true);
                }
                k = a.end + 1;
            } else {
                let p = pts[k];
                out.push(format!(
                    "{} X{} Y{}{}",
                    st.linear_cmd,
                    num(p[0], d),
                    num(p[1], d),
                    f
                ));
                push_move(&mut moves, &mut cur, [p[0], p[1], z_work], true);
                k += 1;
            }
        }
        out.push(ee.off_line());
        if job.off_delay_ms > 0 {
            out.push(dwell(st, job.off_delay_ms));
        }
        if job.use_z {
            out.push(format!("{} Z{}{}", st.rapid_cmd, num(job.safe_z, d), rapid_f));
            { let c = cur; push_move(&mut moves, &mut cur, [c[0], c[1], z_safe], false); }
        }
    }

    comment(&mut out, "End".into());
    out.push(ee.off_line());
    for l in m.post_script.lines().filter(|l| !l.trim().is_empty()) {
        out.push(l.trim().to_string());
    }

    let mut text = out.join("\n");
    text.push('\n');
    GcodeResult { text, warnings, moves }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn sample() -> Project {
        let mut machine = Machine::new("Test");
        machine.bed_x = 125.0;
        machine.bed_y = 250.0;
        machine.travel_x = 125.0;
        machine.travel_y = 250.0;
        machine.travel_z = 20.0;
        let mut p = Project::new(machine);
        p.parts.push(Part {
            name: "sq".into(),
            paths: vec![PathData {
                pts: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                closed: true,
                arcs: vec![],
            }],
            offset: [20.0, 30.0],
        });
        p.sequence.push(SeqItem { part: 0, path: 0, reverse: false });
        p
    }

    #[test]
    fn grbl_output() {
        let r = generate(&sample());
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.text.contains("G0 X20 Y30\n"));
        assert!(r.text.contains("M8\nG4 P0.2\n"));
        assert!(r.text.contains("G1 X20 Y30\n")); // closed path returns to start
        assert!(r.text.trim_end().ends_with("M9\nM30"));
    }

    #[test]
    fn rapid_feed_and_ms_dwell() {
        let mut p = sample();
        p.machine.gcode.rapid_with_feed = true;
        p.machine.gcode.dwell = "G4 P{ms}".into();
        let r = generate(&p);
        assert!(r.text.contains("M8\nG4 P200\n"));
        assert!(r.text.contains("G0 X20 Y30 F3000\n"));
    }

    #[test]
    fn pwm_effector_uses_min_and_max() {
        let mut p = sample();
        p.machine.effector.control = ControlType::Pwm;
        p.machine.effector.pwm_cmd = "M3 S{v}".into();
        p.machine.effector.pwm_min = 0.0;
        p.machine.effector.pwm_max = 800.0;
        let r = generate(&p);
        assert!(r.text.contains("M3 S800\n"), "{}", r.text);
        assert!(r.text.contains("M3 S0\n"));
        assert!(!r.text.contains("M8"));
    }

    #[test]
    fn moves_are_recorded_and_work_only_while_on() {
        let r = generate(&sample());
        assert_eq!(r.moves.iter().filter(|m| m.work).count(), 4);
        assert!(r.moves.iter().any(|m| !m.work));
    }

    #[test]
    fn reverse_and_outside_bed() {
        let mut p = sample();
        p.sequence[0].reverse = true;
        p.parts[0].offset = [120.0, 30.0];
        let r = generate(&p);
        assert!(r.text.contains("G0 X120 Y30\n"));
        assert!(r.warnings.iter().any(|w| w.contains("outside the workable area")));
    }

    fn circle_project() -> Project {
        let mut p = sample();
        // Circle radius 10 about (10, 10), as two half arcs, 8 points.
        let pts: Vec<[f64; 2]> = (0..8)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 8.0;
                [10.0 + 10.0 * a.cos(), 10.0 + 10.0 * a.sin()]
            })
            .collect();
        p.parts[0].paths[0] = PathData {
            pts,
            closed: true,
            arcs: vec![
                ArcSpan { start: 0, end: 4, center: [10.0, 10.0], cw: false },
                ArcSpan { start: 4, end: 8, center: [10.0, 10.0], cw: false },
            ],
        };
        p.parts[0].offset = [0.0, 0.0];
        p
    }

    #[test]
    fn arcs_off_gives_lines_only() {
        let r = generate(&circle_project());
        assert!(!r.text.contains("G3"));
        assert_eq!(r.text.matches("G1 X").count(), 8);
    }

    #[test]
    fn arcs_on_gives_g3_and_reverse_gives_g2() {
        let mut p = circle_project();
        p.machine.gcode.arcs = true;
        let r = generate(&p);
        assert!(r.text.contains("G3 X0 Y10 I-10 J0\n"), "{}", r.text);
        assert_eq!(r.text.matches("G3 X").count(), 2);
        assert_eq!(r.text.matches("G1 X").count(), 0);
        p.sequence[0].reverse = true;
        let r = generate(&p);
        assert_eq!(r.text.matches("G2 X").count(), 2, "{}", r.text);
        assert!(r.text.contains("G2 X0 Y10 I-10 J0\n"), "{}", r.text);
    }

    #[test]
    fn bad_arc_falls_back_to_lines() {
        let mut p = circle_project();
        p.machine.gcode.arcs = true;
        p.parts[0].paths[0].arcs[0].center = [11.0, 10.0]; // wrong radius at the end point
        let r = generate(&p);
        assert_eq!(r.text.matches("G3 X").count(), 1);
    }

    #[test]
    fn number_format() {
        assert_eq!(num(-0.0001, 3), "0");
        assert_eq!(num(1.5, 3), "1.5");
        assert_eq!(num(10.0, 3), "10");
    }
}

#[cfg(test)]
mod workable_tests {
    use crate::model::*;

    /// X and Y move the bed, Z moves the end effector: the example machine.
    fn example() -> Machine {
        let mut m = Machine::new("t");
        m.bed_x = 110.0;
        m.bed_y = 110.0;
        m.bed_z = 5.0;
        m.travel_x = 80.0;
        m.travel_y = 175.0;
        m.travel_z = 35.0;
        m.axes = [AxisOwner::Bed, AxisOwner::Bed, AxisOwner::EndEffector];
        m.effector_pos = [95.0, 175.0, -1.0];
        m
    }

    #[test]
    fn size_is_travel_plus_bed_only_on_bed_axes() {
        // X and Y move the bed: bed + travel. Z moves the end effector: travel.
        assert_eq!(example().frame_size(), [190.0, 285.0, 35.0]);
        let mut m = example();
        m.axes[0] = AxisOwner::EndEffector;
        assert_eq!(m.frame_size()[0], 80.0);
    }

    #[test]
    fn example_workable_on_the_bed() {
        let w = example().workable_on_bed();
        assert_eq!(w[0], (15.0, 95.0));
        assert_eq!(w[1], (0.0, 110.0));
        assert_eq!(w[2], (0.0, 35.0));
    }

    #[test]
    fn work_zero_is_the_lowest_reachable_point() {
        let m = example();
        assert_eq!(m.workable()[0], (0.0, 80.0));
        assert_eq!(m.workable()[1], (0.0, 110.0));
        // The bed starts 15 mm before work zero in X.
        assert_eq!(m.bed_in_gcode()[0], (-15.0, 95.0));
        assert_eq!(m.bed_in_gcode()[1], (0.0, 110.0));
        assert_eq!(m.gcode_to_bed([0.0, 0.0, 3.0]), [15.0, 0.0, 3.0]);
        assert!(m.in_workable([0.0, 0.0]));
        assert!(m.in_workable([80.0, 110.0]));
        assert!(!m.in_workable([81.0, 10.0]));
        assert!(!m.in_workable([-1.0, 10.0]));
    }

    #[test]
    fn workable_never_exceeds_bed_or_travel() {
        let mut m = example();
        for e in [0.0, 20.0, 80.0, 95.0, 150.0, 190.0, 500.0] {
            m.effector_pos[0] = e;
            let w = m.workable_on_bed()[0];
            assert!(w.0 >= 0.0 && w.1 <= 110.0 && w.1 - w.0 <= 80.0 + 1e-9, "{e} {w:?}");
            assert!(w.1 >= w.0, "{e} {w:?}");
        }
        m.axes[0] = AxisOwner::EndEffector;
        for bp in [0.0, 10.0, 80.0, 500.0] {
            m.bed_pos[0] = bp;
            let w = m.workable_on_bed()[0];
            assert!(w.0 >= 0.0 && w.1 <= 110.0 && w.1 - w.0 <= 80.0 + 1e-9, "{bp} {w:?}");
            assert!(w.1 >= w.0, "{bp} {w:?}");
        }
    }

    #[test]
    fn tool_axis_workable_follows_bed_position() {
        let mut m = Machine::new("t");
        m.bed_x = 100.0;
        m.travel_x = 150.0;
        m.axes[0] = AxisOwner::EndEffector;
        m.bed_pos[0] = 20.0;
        // Tool 0..150, bed at 20: the whole bed is reachable.
        assert_eq!(m.workable_on_bed()[0], (0.0, 100.0));
        m.travel_x = 60.0;
        m.bed_pos[0] = 20.0;
        // Tool 0..60 reaches bed points -20 .. 40, so 0 .. 40.
        assert_eq!(m.workable_on_bed()[0], (0.0, 40.0));
        assert_eq!(m.workable()[0], (0.0, 40.0));
    }
}

#[cfg(test)]
mod home_tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn home_defaults_and_first_move_starts_at_home() {
        let mut m = Machine::new("t");
        m.travel_z = 35.0;
        assert_eq!(m.home_position(), [0.0, 0.0, 35.0]);
        m.home = [Some(5.0), None, Some(20.0)];
        assert_eq!(m.home_position(), [5.0, 0.0, 20.0]);

        let mut p = Project::new(m);
        p.parts.push(Part {
            name: "sq".into(),
            paths: vec![PathData { pts: vec![[0.0, 0.0], [10.0, 0.0]], closed: false, arcs: vec![] }],
            offset: [20.0, 30.0],
        });
        p.sequence.push(SeqItem { part: 0, path: 0, reverse: false });
        let r = generate(&p);
        assert_eq!(r.moves[0].from, [5.0, 0.0, 20.0]);
    }
}
