use crate::model::{ArcSpan, Flavor, Project, Pt};

pub struct GcodeResult {
    pub text: String,
    pub warnings: Vec<String>,
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

fn dwell(flavor: &Flavor, ms: u32) -> String {
    flavor
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

pub fn generate(project: &Project, flavor: &Flavor) -> GcodeResult {
    let d = flavor.decimals;
    let job = &project.job;
    let m = &project.machine;
    let mut out: Vec<String> = vec![];
    let mut warnings: Vec<String> = vec![];

    if project.sequence.is_empty() {
        warnings.push("No paths in the sequence.".into());
    }
    if job.use_z && job.dispense_z > m.size_z {
        warnings.push("Dispense Z is above the machine Z size.".into());
    }
    if job.use_z && job.safe_z > m.size_z {
        warnings.push("Safe Z is above the machine Z size.".into());
    }

    let comment = |out: &mut Vec<String>, t: String| {
        if flavor.comments {
            out.push(format!("; {t}"));
        }
    };

    comment(&mut out, format!("Machine: {}  Flavor: {}", m.name, flavor.name));
    for l in flavor.header.lines().filter(|l| !l.trim().is_empty()) {
        out.push(l.trim().to_string());
    }
    // Valve must be closed before anything moves.
    out.push(flavor.valve_off.clone());
    if job.use_z {
        out.push(format!("{} Z{}", flavor.rapid_cmd, num(job.safe_z, d)));
    }

    let rapid_f = if flavor.rapid_with_feed {
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
        if pts.iter().any(|p| {
            p[0] < -1e-6 || p[1] < -1e-6 || p[0] > m.size_x + 1e-6 || p[1] > m.size_y + 1e-6
        }) {
            warnings.push(format!(
                "Path {} ({}) goes outside the machine bed.",
                i + 1,
                part.name
            ));
        }

        out.push(format!(
            "{} X{} Y{}{}",
            flavor.rapid_cmd,
            num(pts[0][0], d),
            num(pts[0][1], d),
            rapid_f
        ));
        if job.use_z {
            out.push(format!(
                "{} Z{} F{}",
                flavor.linear_cmd,
                num(job.dispense_z, d),
                num(job.dispense_feed, 0)
            ));
        }
        out.push(flavor.valve_on.clone());
        if job.pre_flow_ms > 0 {
            out.push(dwell(flavor, job.pre_flow_ms));
        }
        let mut k = 1;
        while k < pts.len() {
            let f = if k == 1 && !job.use_z {
                format!(" F{}", num(job.dispense_feed, 0))
            } else {
                String::new()
            };
            let arc = if flavor.arcs {
                arcs.iter().find(|a| a.start == k - 1 && arc_ok(a, pts[a.start], pts[a.end]))
            } else {
                None
            };
            if let Some(a) = arc {
                let from = pts[a.start];
                let to = pts[a.end];
                out.push(format!(
                    "{} X{} Y{} I{} J{}{}",
                    if a.cw { &flavor.cw_cmd } else { &flavor.ccw_cmd },
                    num(to[0], d),
                    num(to[1], d),
                    num(a.center[0] - from[0], d),
                    num(a.center[1] - from[1], d),
                    f
                ));
                k = a.end + 1;
            } else {
                let p = pts[k];
                out.push(format!(
                    "{} X{} Y{}{}",
                    flavor.linear_cmd,
                    num(p[0], d),
                    num(p[1], d),
                    f
                ));
                k += 1;
            }
        }
        out.push(flavor.valve_off.clone());
        if job.post_flow_ms > 0 {
            out.push(dwell(flavor, job.post_flow_ms));
        }
        if job.use_z {
            out.push(format!("{} Z{}{}", flavor.rapid_cmd, num(job.safe_z, d), rapid_f));
        }
    }

    comment(&mut out, "End".into());
    out.push(flavor.valve_off.clone());
    for l in flavor.footer.lines().filter(|l| !l.trim().is_empty()) {
        out.push(l.trim().to_string());
    }

    let mut text = out.join("\n");
    text.push('\n');
    GcodeResult { text, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn sample(flavor: &Flavor) -> Project {
        let machine = Machine {
            name: "Test".into(),
            size_x: 125.0,
            size_y: 250.0,
            size_z: 20.0,
            flavor: flavor.name.clone(),
        };
        let mut p = Project::new(machine, flavor);
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
        let f = Flavor::grbl();
        let r = generate(&sample(&f), &f);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.text.contains("G0 X20 Y30\n"));
        assert!(r.text.contains("M8\nG4 P0.2\n"));
        assert!(r.text.contains("G1 X20 Y30\n")); // closed path returns to start
        assert!(r.text.trim_end().ends_with("M9\nM30"));
    }

    #[test]
    fn marlin_output() {
        let f = Flavor::marlin();
        let r = generate(&sample(&f), &f);
        assert!(r.text.contains("M106 S255\nG4 P200\n"));
        assert!(r.text.contains("G0 X20 Y30 F3000\n"));
    }

    #[test]
    fn reverse_and_outside_bed() {
        let f = Flavor::grbl();
        let mut p = sample(&f);
        p.sequence[0].reverse = true;
        p.parts[0].offset = [120.0, 30.0];
        let r = generate(&p, &f);
        assert!(r.text.contains("G0 X120 Y30\n"));
        assert_eq!(r.warnings.len(), 1);
    }

    fn circle_project(flavor: &Flavor) -> Project {
        let mut p = sample(flavor);
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
        let f = Flavor::grbl();
        let r = generate(&circle_project(&f), &f);
        assert!(!r.text.contains("G3"));
        assert_eq!(r.text.matches("G1 X").count(), 8);
    }

    #[test]
    fn arcs_on_gives_g3_and_reverse_gives_g2() {
        let mut f = Flavor::grbl();
        f.arcs = true;
        let mut p = circle_project(&f);
        let r = generate(&p, &f);
        assert!(r.text.contains("G3 X0 Y10 I-10 J0\n"), "{}", r.text);
        assert_eq!(r.text.matches("G3 X").count(), 2);
        assert_eq!(r.text.matches("G1 X").count(), 0);
        p.sequence[0].reverse = true;
        let r = generate(&p, &f);
        assert_eq!(r.text.matches("G2 X").count(), 2, "{}", r.text);
        assert!(r.text.contains("G2 X0 Y10 I-10 J0\n"), "{}", r.text);
    }

    #[test]
    fn bad_arc_falls_back_to_lines() {
        let mut f = Flavor::grbl();
        f.arcs = true;
        let mut p = circle_project(&f);
        p.parts[0].paths[0].arcs[0].center = [11.0, 10.0]; // wrong radius at the end point
        let r = generate(&p, &f);
        assert_eq!(r.text.matches("G3 X").count(), 1);
    }

    #[test]
    fn number_format() {
        assert_eq!(num(-0.0001, 3), "0");
        assert_eq!(num(1.5, 3), "1.5");
        assert_eq!(num(10.0, 3), "10");
    }
}
