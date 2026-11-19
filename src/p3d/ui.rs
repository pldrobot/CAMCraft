//! Screen of the 3D project. Same layout as the 2D project: Prepare,
//! Path selection, Preview and G-code tabs.

use super::gcode;
use super::model::*;
use super::step::{self, ImportJob};
use super::view::{Overlay, PickMode, StepViz, View3};
use crate::icons;
use crate::model::{AxisOwner, ControlType, JobSettings, Library};
use crate::preview::{self, Preview};
use crate::theme;
use eframe::egui;
use egui::{Color32, RichText};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Prepare,
    PathSelection,
    Preview,
    Gcode,
}

pub struct P3App {
    pub project: Project3,
    pub path: Option<PathBuf>,
    view: View3,
    tab: Tab,
    mode: PickMode,
    selected_body: Option<usize>,
    sel_steps: Vec<usize>,
    seq_drag: Option<usize>,
    /// Prepare: the next click on a face lays it on the bed.
    lay_face: bool,
    /// Angle of the rotate buttons, degrees.
    rot_step: f64,
    gcode: String,
    warnings: Vec<String>,
    preview: Preview,
    /// Messages for the status line, taken by the app.
    pub msgs: Vec<String>,
    /// Cursor on the bed plane, for the status line.
    pub cursor: Option<[f64; 2]>,
    viz_key: u64,
    viz: Vec<StepViz>,
    jobs: Vec<Task>,
}

/// A STEP import in progress.
struct Task {
    job: ImportJob,
    /// True for a new part, false when reloading a part of an opened project.
    new_part: bool,
}

enum AddStep {
    Home,
    Goto,
    Pause,
    Clear,
}

impl P3App {
    pub fn new(machine: crate::model::Machine) -> Self {
        Self::from_project(Project3::new(machine))
    }

    fn from_project(project: Project3) -> Self {
        Self {
            project,
            path: None,
            view: View3::default(),
            tab: Tab::Prepare,
            mode: PickMode::Edges,
            selected_body: None,
            sel_steps: vec![],
            seq_drag: None,
            lay_face: false,
            rot_step: 90.0,
            gcode: String::new(),
            warnings: vec![],
            preview: Preview::default(),
            msgs: vec![],
            cursor: None,
            viz_key: u64::MAX,
            viz: vec![],
            jobs: vec![],
        }
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let project: Project3 = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut app = Self::from_project(project);
        // The meshes are not stored in the project, read them again.
        for b in &app.project.bodies {
            app.jobs.push(Task { job: ImportJob::spawn(PathBuf::from(&b.source)), new_part: false });
        }
        app.msgs.push(format!("Opened {}", path.display()));
        app.path = Some(path);
        Ok(app)
    }

    pub fn save(&mut self, as_new: bool) {
        let path = if as_new || self.path.is_none() {
            rfd::FileDialog::new()
                .add_filter("CAMCraft 3D project", &["camcraft3d"])
                .set_file_name("project.camcraft3d")
                .save_file()
        } else {
            self.path.clone()
        };
        let Some(path) = path else { return };
        let res = serde_json::to_string_pretty(&self.project)
            .map_err(|e| e.to_string())
            .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        match res {
            Ok(()) => {
                self.msgs.push(format!("Saved {}", path.display()));
                self.path = Some(path);
            }
            Err(e) => self.msgs.push(format!("Save failed: {e}")),
        }
    }

    fn insert_step_file(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("STEP", &["step", "stp"]).pick_file() else {
            return;
        };
        self.msgs.push(format!("Reading {} ...", path.display()));
        self.jobs.push(Task { job: ImportJob::spawn(path), new_part: true });
    }

    /// Collect finished imports.
    fn poll_jobs(&mut self) {
        let tasks = std::mem::take(&mut self.jobs);
        for mut t in tasks {
            match t.job.poll() {
                None => self.jobs.push(t),
                Some(Err(e)) => self.msgs.push(format!("{}: {e}", t.job.path.display())),
                Some(Ok(geo)) => {
                    self.msgs.extend(geo.notes.iter().cloned());
                    if t.new_part {
                        self.add_part(t.job.path.clone(), geo);
                    } else {
                        let src = t.job.path.to_string_lossy().to_string();
                        let geo = std::sync::Arc::new(geo);
                        for b in self.project.bodies.iter_mut().filter(|b| b.source == src && b.geo.is_none()) {
                            b.geo = Some(geo.clone());
                        }
                    }
                }
            }
        }
    }

    fn jobs_ui(&mut self, ui: &mut egui::Ui) {
        for t in &self.jobs {
            ui.horizontal(|ui| {
                ui.spinner();
                let name = t.job.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                ui.label(RichText::new(format!("{name}: {}", t.job.status)).small());
                if ui.small_button("Cancel").clicked() {
                    t.job.cancel();
                }
            });
        }
    }

    fn add_part(&mut self, path: PathBuf, geo: step::Geometry) {
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "part".into());
        let wk = self.project.machine.workable();
        // Lower-left corner 5 mm inside the workable area, resting on the bed.
        let offset = [wk[0].0 + 5.0 - geo.min[0], wk[1].0 + 5.0 - geo.min[1], -geo.min[2]];
        let size = [geo.max[0] - geo.min[0], geo.max[1] - geo.min[1], geo.max[2] - geo.min[2]];
        self.msgs.push(format!(
            "Inserted {name}: {} faces, {} edges, {:.1} x {:.1} x {:.1} mm (assumed mm)",
            geo.faces.len(),
            geo.edges.len(),
            size[0],
            size[1],
            size[2]
        ));
        if size[0] > wk[0].1 - wk[0].0 || size[1] > wk[1].1 - wk[1].0 {
            self.msgs.push("Warning: the part is larger than the workable area.".into());
        }
        self.project.bodies.push(Body::new(
            name,
            path.to_string_lossy().to_string(),
            offset,
            Some(std::sync::Arc::new(geo)),
        ));
        self.selected_body = Some(self.project.bodies.len() - 1);
        self.view.fit();
    }

    fn generate(&mut self, lib: &Library) {
        if let Some(m) = lib.machines.iter().find(|m| m.name == self.project.machine.name) {
            self.project.machine = m.clone();
        }
        let r = gcode::generate(&self.project);
        let job = JobSettings {
            dispense_feed: self.project.job.dispense_feed,
            travel_feed: self.project.job.travel_feed,
            ..Default::default()
        };
        let lines = r.text.lines().count();
        self.preview.set_moves(r.moves, &job);
        self.gcode = r.text;
        self.warnings = r.warnings;
        self.msgs.push(format!("G-code generated: {lines} lines, {} warning(s).", self.warnings.len()));
    }

    /// Rebuild the drawn tool paths when the sequence, job or bodies change.
    fn refresh_viz(&mut self) {
        let p = &self.project;
        let mut h = DefaultHasher::new();
        serde_json::to_string(&(&p.sequence, &p.job)).unwrap_or_default().hash(&mut h);
        for b in &p.bodies {
            b.offset.map(f64::to_bits).hash(&mut h);
            b.rot.map(f64::to_bits).hash(&mut h);
            b.geo.as_ref().map(|g| std::sync::Arc::as_ptr(g) as usize).hash(&mut h);
        }
        let key = h.finish();
        if key == self.viz_key {
            return;
        }
        self.viz_key = key;
        let home = p.machine.home_position();
        self.viz = p
            .sequence
            .iter()
            .map(|s| match s.kind {
                Kind3::Edge { .. } => StepViz { pts: p.step_points(s), is_path: true },
                Kind3::Home => StepViz { pts: vec![home], is_path: false },
                Kind3::Goto { x, y, z } => StepViz { pts: vec![[x, y, z]], is_path: false },
                Kind3::Pause { .. } => StepViz { pts: vec![], is_path: false },
            })
            .collect();
    }

    pub fn show(&mut self, root: &mut egui::Ui, lib: &Library) {
        self.poll_jobs();
        if !self.jobs.is_empty() {
            root.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.refresh_viz();
        let generated = self.preview.has_path();
        if matches!(self.tab, Tab::Preview | Tab::Gcode) && !generated {
            self.tab = Tab::Prepare;
        }

        egui::Panel::top("p3_tabs").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Prepare, "Prepare");
                ui.selectable_value(&mut self.tab, Tab::PathSelection, "Path selection");
                ui.add_enabled_ui(generated, |ui| {
                    ui.selectable_value(&mut self.tab, Tab::Preview, "Preview").on_disabled_hover_text("Generate G-code first.");
                    ui.selectable_value(&mut self.tab, Tab::Gcode, "G-code").on_disabled_hover_text("Generate G-code first.");
                });
            });
        });

        let mut generate = false;
        egui::Panel::left("p3_left").resizable(false).exact_size(330.0).show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
              self.jobs_ui(ui);
              match self.tab {
                Tab::Prepare => self.prepare_panel(ui, lib),
                Tab::PathSelection => {
                    ui.heading("Job settings");
                    job_grid(ui, &mut self.project.job);
                    ui.separator();
                    sequence_ui(ui, &mut self.project, &mut self.sel_steps, &mut self.seq_drag, &mut self.mode);
                    ui.separator();
                    ui.add_space(4.0);
                    if ui
                        .add_sized([ui.available_width(), 30.0], egui::Button::new(RichText::new("Generate G-code").strong()).fill(theme::ACCENT))
                        .clicked()
                    {
                        generate = true;
                    }
                    for w in &self.warnings {
                        ui.colored_label(Color32::from_rgb(0xff, 0xb0, 0x40), format!("! {w}"));
                    }
                }
                Tab::Gcode => {
                    ui.heading("G-code");
                    ui.label(format!("{} lines", self.gcode.lines().count()));
                    if ui.button("Save...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("G-code", &["gcode", "nc", "gc"])
                            .set_file_name("job.gcode")
                            .save_file()
                        {
                            match std::fs::write(&path, &self.gcode) {
                                Ok(()) => self.msgs.push(format!("Saved {}", path.display())),
                                Err(e) => self.msgs.push(format!("Save failed: {e}")),
                            }
                        }
                    }
                    if ui.button("Copy to clipboard").clicked() {
                        ui.ctx().copy_text(self.gcode.clone());
                    }
                    for w in &self.warnings {
                        ui.colored_label(Color32::from_rgb(0xff, 0xb0, 0x40), format!("! {w}"));
                    }
                }
                Tab::Preview => {
                    let m = &self.project.machine;
                    ui.heading("Simulation");
                    egui::Grid::new("p3_pv").num_columns(2).show(ui, |ui| {
                        ui.label("Machine");
                        ui.label(&m.name);
                        ui.end_row();
                        ui.label("End effector");
                        ui.label(m.effector.kind.label());
                        ui.end_row();
                        ui.label("Path length");
                        ui.label(format!("{:.1} mm", self.preview.path_len));
                        ui.end_row();
                        ui.label("Work moves");
                        ui.label(self.preview.work_moves().to_string());
                        ui.end_row();
                        ui.label("Est. time");
                        ui.label(preview::fmt_time(self.preview.total_time()));
                        ui.end_row();
                        ui.label("Axes");
                        let own = |i: usize| match m.axes[i] {
                            AxisOwner::EndEffector => "tool",
                            AxisOwner::Bed => "bed",
                        };
                        ui.label(format!("X {}  Y {}  Z {}", own(0), own(1), own(2)));
                        ui.end_row();
                    });
                    for w in &self.warnings {
                        ui.colored_label(Color32::from_rgb(0xff, 0xb0, 0x40), format!("! {w}"));
                    }
                }
              }
            });
        });
        if generate {
            self.generate(lib);
        }

        if self.tab == Tab::Preview {
            egui::Panel::bottom("p3_pv_bar").show(root, |ui| {
                let total = self.preview.total_time();
                let pv = &mut self.preview;
                ui.horizontal(|ui| {
                    let label = if pv.playing { "Pause" } else if pv.time >= total { "Replay" } else { "Play" };
                    if ui.button(label).clicked() {
                        if pv.playing {
                            pv.playing = false;
                        } else {
                            if pv.time >= total {
                                pv.time = 0.0;
                            }
                            pv.playing = true;
                        }
                    }
                    if ui.button("Reset").clicked() {
                        pv.time = 0.0;
                        pv.playing = false;
                    }
                    ui.spacing_mut().slider_width = (ui.available_width() - 260.0).max(100.0);
                    let mut t = pv.time;
                    if ui.add(egui::Slider::new(&mut t, 0.0..=total.max(0.001)).show_value(false)).changed() {
                        pv.time = t;
                        pv.playing = false;
                    }
                    ui.monospace(format!("{} / {}", preview::fmt_time(pv.time), preview::fmt_time(total)));
                    egui::ComboBox::from_id_salt("p3_pv_speed")
                        .width(60.0)
                        .selected_text(format!("{}x", preview::SPEEDS[pv.speed_idx]))
                        .show_ui(ui, |ui| {
                            for (i, s) in preview::SPEEDS.iter().enumerate() {
                                ui.selectable_value(&mut pv.speed_idx, i, format!("{s}x"));
                            }
                        });
                });
            });
        }

        let Self { project, view, tab, mode, selected_body, sel_steps, viz, preview, gcode, cursor, msgs, lay_face, .. } = self;
        let mut edge_click = None;
        let mut face_click = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(root, |ui| match *tab {
            Tab::Prepare | Tab::PathSelection => {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    if ui.button("Fit").clicked() {
                        view.fit();
                    }
                    if ui.button("Iso").clicked() {
                        view.iso();
                    }
                    if ui.button("Top").clicked() {
                        view.top();
                    }
                    if ui.button("Front").clicked() {
                        view.front();
                    }
                    if *tab == Tab::Prepare {
                        ui.separator();
                        ui.toggle_value(lay_face, "Lay face on bed")
                            .on_hover_text("Click a flat face of a part to turn it down onto the bed, like in 3D printing software.");
                    }
                    if *tab == Tab::PathSelection {
                        ui.separator();
                        ui.label("Pick");
                        ui.selectable_value(mode, PickMode::Edges, "Edges").on_hover_text("Click an edge to add or remove it as a path.");
                        ui.selectable_value(mode, PickMode::Surface, "Surface")
                            .on_hover_text("Select steps in the list, then click a surface. The Z gap is measured from it.");
                    }
                    let hint = if *tab == Tab::Prepare && *lay_face {
                        "Click a flat face to lay it on the bed."
                    } else if *tab == Tab::Prepare {
                        "Drag a part to place it on the bed.   Drag empty space: orbit   Right drag: pan   Wheel: zoom"
                    } else if *mode == PickMode::Edges {
                        "Parts are locked. Click edges to add paths.   Drag: orbit   Right drag: pan   Wheel: zoom"
                    } else {
                        "Select steps, then click a surface for their Z gap.   Drag: orbit   Right drag: pan"
                    };
                    ui.label(RichText::new(hint).small().weak());
                });
                let tints: Vec<((usize, usize), [u8; 3])> = if *tab == Tab::PathSelection {
                    project
                        .sequence
                        .iter()
                        .enumerate()
                        .filter_map(|(i, s)| {
                            s.surface.map(|f| (f, if sel_steps.contains(&i) { [60, 200, 216] } else { [196, 138, 138] }))
                        })
                        .collect()
                } else {
                    vec![]
                };
                let ov = Overlay {
                    mode: if *tab == Tab::Prepare { if *lay_face { PickMode::LayFace } else { PickMode::Move } } else { *mode },
                    selected_body: if *tab == Tab::Prepare { *selected_body } else { None },
                    tints: &tints,
                    steps: viz,
                    show_steps: *tab == Tab::PathSelection,
                    sel_steps,
                };
                let out = view.show(ui, project, &ov);
                *cursor = out.cursor;
                if let Some(b) = out.body_click {
                    *selected_body = b;
                }
                edge_click = out.edge_click;
                face_click = out.face_click;
            }
            Tab::Preview => {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    if ui.button("Iso").clicked() {
                        preview.view_iso();
                    }
                    if ui.button("Top").clicked() {
                        preview.view_top();
                    }
                    if ui.button("Front").clicked() {
                        preview.view_front();
                    }
                    ui.checkbox(&mut preview.show_travel, "Travel moves");
                    ui.checkbox(&mut preview.show_full, "Whole path");
                });
                preview.show(ui, &project.machine);
            }
            Tab::Gcode => {
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut gcode.as_str()).code_editor().desired_width(f32::INFINITY));
                });
            }
        });

        // Clicks in the view.
        if let Some((b, e)) = edge_click {
            if let Some(i) = project.edge_in_sequence(b, e) {
                project.sequence.remove(i);
                sel_steps.clear();
            } else {
                let at = sel_steps.iter().max().map_or(project.sequence.len(), |m| m + 1).min(project.sequence.len());
                let step = project.new_edge_step(b, e);
                project.sequence.insert(at, step);
                *sel_steps = vec![at];
            }
        }
        if let (Some(f), Tab::Prepare) = (face_click, *tab) {
            if let Some(b) = project.bodies.get_mut(f.0) {
                match b.lay_face_down(f.1) {
                    Ok(()) => {
                        msgs.push(format!("{}: face laid on the bed.", b.name));
                        *lay_face = false;
                        *selected_body = Some(f.0);
                    }
                    Err(e) => msgs.push(e),
                }
            }
        } else if let Some(f) = face_click {
            let targets: Vec<usize> = sel_steps.iter().copied().filter(|&i| project.sequence.get(i).is_some_and(|s| s.is_edge())).collect();
            if targets.is_empty() {
                msgs.push("Select a path step in the list first, then click a surface.".into());
            }
            for i in targets {
                project.sequence[i].surface = Some(f);
            }
        }
    }

    fn prepare_panel(&mut self, ui: &mut egui::Ui, lib: &Library) {
        let mut reload = false;
        ui.horizontal(|ui| {
            ui.heading("Machine");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reload = icons::icon_button(
                    ui,
                    icons::Icon::Reload,
                    "Reload machine settings. The project keeps a copy of the machine. Use this after editing the machine in Machines.",
                )
                .clicked();
            });
        });
        if reload {
            if let Some(m) = lib.machines.iter().find(|m| m.name == self.project.machine.name) {
                self.project.machine = m.clone();
            }
        }
        let m = &self.project.machine;
        ui.label(format!("{}   bed {} x {} x {} mm", m.name, m.bed_x, m.bed_y, m.bed_z));
        let t = m.travel();
        ui.label(
            RichText::new(format!(
                "Travel {} x {} x {} mm   Tool: {} ({})",
                t[0],
                t[1],
                t[2],
                m.effector.kind.label(),
                match m.effector.control {
                    ControlType::Binary => "binary",
                    ControlType::Pwm => "PWM",
                }
            ))
            .small()
            .weak(),
        );
        ui.separator();

        let mut insert = false;
        ui.horizontal(|ui| {
            ui.heading("Parts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Insert STEP...").clicked() {
                    insert = true;
                }
            });
        });
        let mut remove: Option<usize> = None;
        if self.project.bodies.is_empty() {
            ui.label(RichText::new("No STEP file inserted yet.").small().weak());
        } else {
            egui::Grid::new("p3_parts").num_columns(5).striped(true).spacing([6.0, 4.0]).show(ui, |ui| {
                ui.label(RichText::new("STEP").small().weak());
                ui.label(RichText::new("X mm").small().weak());
                ui.label(RichText::new("Y mm").small().weak());
                ui.label(RichText::new("Z mm").small().weak());
                ui.label("");
                ui.end_row();
                for bi in 0..self.project.bodies.len() {
                    let selected = self.selected_body == Some(bi);
                    let b = &mut self.project.bodies[bi];
                    let tris = b.geo.as_ref().map_or(0, |g| g.tri_count());
                    if ui
                        .add(egui::Button::selectable(selected, RichText::new(b.name.as_str())).truncate())
                        .on_hover_text(format!("{}\n{} triangles", b.source, tris))
                        .clicked()
                    {
                        self.selected_body = Some(bi);
                    }
                    let mut c = b.min_corner();
                    let before = c;
                    for k in 0..3 {
                        ui.add(egui::DragValue::new(&mut c[k]).speed(0.1).fixed_decimals(1));
                    }
                    for k in 0..3 {
                        b.offset[k] += c[k] - before[k];
                    }
                    if icons::icon_button(ui, icons::Icon::Remove, "Remove part").clicked() {
                        remove = Some(bi);
                    }
                    ui.end_row();
                }
            });
        }
        if let Some(i) = remove {
            self.project.remove_body(i);
            self.selected_body = None;
            self.sel_steps.clear();
        }
        // Orientation of the selected part.
        if let Some(bi) = self.selected_body.filter(|&i| i < self.project.bodies.len()) {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Orientation of {}", self.project.bodies[bi].name)).strong());
            ui.horizontal(|ui| {
                ui.label("Step");
                ui.add(egui::DragValue::new(&mut self.rot_step).speed(1.0).range(0.1..=180.0).suffix(" deg"));
            });
            let step = self.rot_step.to_radians();
            let mut turn: Option<([f64; 3], f64)> = None;
            ui.horizontal(|ui| {
                for (name, axis) in [("X", [1.0, 0.0, 0.0]), ("Y", [0.0, 1.0, 0.0]), ("Z", [0.0, 0.0, 1.0])] {
                    if ui.button(format!("{name}-")).on_hover_text(format!("Rotate about {name}")).clicked() {
                        turn = Some((axis, -step));
                    }
                    if ui.button(format!("{name}+")).on_hover_text(format!("Rotate about {name}")).clicked() {
                        turn = Some((axis, step));
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.toggle_value(&mut self.lay_face, "Lay face on bed")
                    .on_hover_text("Then click a flat face of a part in the view.");
                if ui.button("Reset").on_hover_text("Back to the orientation of the file").clicked() {
                    self.project.bodies[bi].reset_rotation();
                }
            });
            if let Some((axis, angle)) = turn {
                let r = super::model::axis_rotation(axis, angle);
                self.project.bodies[bi].rotate(&r, axis[2] > 0.5);
            }
        }
        if insert {
            self.insert_step_file();
        }
    }
}

fn job_grid(ui: &mut egui::Ui, j: &mut Job3) {
    egui::Grid::new("p3_job").num_columns(2).show(ui, |ui| {
        ui.label("Work feed");
        ui.add(egui::DragValue::new(&mut j.dispense_feed).speed(5.0).range(1.0..=20000.0).suffix(" mm/min"));
        ui.end_row();
        ui.label("Travel feed");
        ui.add(egui::DragValue::new(&mut j.travel_feed).speed(10.0).range(1.0..=50000.0).suffix(" mm/min"));
        ui.end_row();
        ui.label("Safe Z");
        ui.add(egui::DragValue::new(&mut j.safe_z).speed(0.1).suffix(" mm"));
        ui.end_row();
        ui.label("Z gap").on_hover_text("Distance between the tool tip and the chosen surface.");
        ui.add(egui::DragValue::new(&mut j.z_gap).speed(0.05).suffix(" mm"));
        ui.end_row();
        ui.label("Tool diameter")
            .on_hover_text("Paths are moved sideways by half the tool diameter, to the side each step chooses. 0 = off.");
        ui.add(egui::DragValue::new(&mut j.tool_diameter).speed(0.05).range(0.0..=100.0).suffix(" mm"));
        ui.end_row();
        ui.label("Path step").on_hover_text("Longest piece of a path while following a surface.");
        ui.add(egui::DragValue::new(&mut j.sample_mm).speed(0.05).range(0.05..=50.0).suffix(" mm"));
        ui.end_row();
        ui.label("Start delay");
        ui.add(egui::DragValue::new(&mut j.on_delay_ms).speed(5.0).suffix(" ms"));
        ui.end_row();
        ui.label("Stop delay");
        ui.add(egui::DragValue::new(&mut j.off_delay_ms).speed(5.0).suffix(" ms"));
        ui.end_row();
    });
}

fn step_desc(project: &Project3, s: &Step3) -> String {
    match s.kind {
        Kind3::Edge { body, edge } => {
            let name = project.bodies.get(body).map(|b| b.name.as_str()).unwrap_or("?");
            format!(
                "{} edge {}{}{}{}",
                name,
                edge + 1,
                if s.surface.is_some() { "" } else { "  (no surface)" },
                if s.reverse { "  (reversed)" } else { "" },
                if s.customised() { "  *" } else { "" }
            )
        }
        Kind3::Home => "Home".into(),
        Kind3::Goto { x, y, z } => format!("Go to  X{x:.2}  Y{y:.2}  Z{z:.2}"),
        Kind3::Pause { ms } => format!("Pause  {ms} ms"),
    }
}

fn override_row(ui: &mut egui::Ui, label: &str, ov: &mut Option<f64>, job: f64, speed: f64, suffix: &str) {
    let mut on = ov.is_some();
    if ui.checkbox(&mut on, label).changed() {
        *ov = if on { Some(job) } else { None };
    }
    match ov {
        Some(v) => {
            ui.add(egui::DragValue::new(v).speed(speed).suffix(suffix));
        }
        None => {
            ui.label(RichText::new(format!("{job}{suffix}")).weak());
        }
    }
    ui.end_row();
}

fn step_settings(ui: &mut egui::Ui, project_bodies: &[Body], step: &mut Step3, job: &Job3, mode: &mut PickMode) -> bool {
    ui.set_min_width(260.0);
    let mut pick_surface = false;
    match &mut step.kind {
        Kind3::Edge { .. } => {
            let surf = match step.surface {
                Some((b, f)) => format!("{} face {}", project_bodies.get(b).map(|x| x.name.as_str()).unwrap_or("?"), f + 1),
                None => "none, Z follows the edge".to_string(),
            };
            ui.label(format!("Surface: {surf}"));
            ui.horizontal(|ui| {
                if ui.button("Pick surface").on_hover_text("Switches the view to surface picking for this step.").clicked() {
                    *mode = PickMode::Surface;
                    pick_surface = true;
                }
                if ui.add_enabled(step.surface.is_some(), egui::Button::new("Clear")).clicked() {
                    step.surface = None;
                }
            });
            ui.separator();
            ui.label(RichText::new("Tick a value to change it for this path. Others follow Job settings.").small().weak());
            egui::Grid::new("p3_step_over").num_columns(2).show(ui, |ui| {
                override_row(ui, "Z gap", &mut step.gap, job.z_gap, 0.05, " mm");
                override_row(ui, "Work feed", &mut step.feed, job.dispense_feed, 5.0, " mm/min");
            });
            ui.horizontal(|ui| {
                ui.label("Offset side");
                egui::ComboBox::from_id_salt("p3_side")
                    .selected_text(step.side.label())
                    .show_ui(ui, |ui| {
                        for sd in Side::ALL {
                            ui.selectable_value(&mut step.side, sd, sd.label());
                        }
                    });
            });
            ui.label(RichText::new(format!("Offset: {} mm (half the tool diameter)", job.tool_diameter / 2.0)).small().weak());
            if ui.add_enabled(step.customised(), egui::Button::new("Use job settings")).clicked() {
                step.gap = None;
                step.feed = None;
                step.side = Side::Outside;
            }
        }
        Kind3::Goto { x, y, z } => {
            egui::Grid::new("p3_step_goto").num_columns(2).show(ui, |ui| {
                for (l, v) in [("X", x), ("Y", y), ("Z", z)] {
                    ui.label(l);
                    ui.add(egui::DragValue::new(v).speed(0.1).suffix(" mm"));
                    ui.end_row();
                }
            });
        }
        Kind3::Pause { ms } => {
            ui.horizontal(|ui| {
                ui.label("Wait");
                ui.add(egui::DragValue::new(ms).speed(10.0).range(0..=600_000).suffix(" ms"));
            });
        }
        Kind3::Home => {
            ui.label("Goes to the home position of the machine.");
        }
    }
    pick_surface
}

fn move_steps(project: &mut Project3, sel: &mut Vec<usize>, ins: usize) {
    let mut idx = sel.clone();
    idx.sort_unstable();
    idx.dedup();
    idx.retain(|&i| i < project.sequence.len());
    if idx.is_empty() {
        return;
    }
    let before = idx.iter().filter(|&&i| i < ins).count();
    let items: Vec<Step3> = idx.iter().map(|&i| project.sequence[i]).collect();
    for &i in idx.iter().rev() {
        project.sequence.remove(i);
    }
    let at = (ins - before).min(project.sequence.len());
    for (k, it) in items.iter().enumerate() {
        project.sequence.insert(at + k, *it);
    }
    *sel = (at..at + items.len()).collect();
}

fn sequence_ui(ui: &mut egui::Ui, project: &mut Project3, sel: &mut Vec<usize>, drag: &mut Option<usize>, mode: &mut PickMode) {
    sel.retain(|&i| i < project.sequence.len());
    let mut add: Option<AddStep> = None;
    let mut delete = false;

    ui.horizontal(|ui| {
        ui.heading("Sequence");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::icon_button_ex(ui, icons::Icon::Trash, "Remove the selected steps (Delete)", false, !sel.is_empty()).clicked() {
                delete = true;
            }
            let plus = icons::icon_button(ui, icons::Icon::Plus, "Add a step");
            egui::Popup::menu(&plus).show(|ui| {
                if ui.button("Home").clicked() {
                    add = Some(AddStep::Home);
                }
                if ui.button("Go to coordinate").clicked() {
                    add = Some(AddStep::Goto);
                }
                if ui.button("Pause").clicked() {
                    add = Some(AddStep::Pause);
                }
                ui.separator();
                if ui.button("Clear sequence").clicked() {
                    add = Some(AddStep::Clear);
                }
            });
        });
    });
    ui.label(
        RichText::new("Click edges in the view to add paths. Drag rows to reorder. Ctrl or Shift click selects several.")
            .small()
            .weak(),
    );

    if !sel.is_empty() && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        delete = true;
    }

    if let Some(a) = add {
        let at = sel.iter().max().map_or(project.sequence.len(), |m| m + 1).min(project.sequence.len());
        let home = project.machine.home_position();
        let new = match a {
            AddStep::Home => Some(Step3::new(Kind3::Home)),
            AddStep::Goto => Some(Step3::new(Kind3::Goto { x: home[0], y: home[1], z: home[2] })),
            AddStep::Pause => Some(Step3::new(Kind3::Pause { ms: 1000 })),
            AddStep::Clear => {
                project.sequence.clear();
                sel.clear();
                None
            }
        };
        if let Some(s) = new {
            project.sequence.insert(at, s);
            *sel = vec![at];
        }
    }
    if delete {
        let mut idx = sel.clone();
        idx.sort_unstable();
        idx.dedup();
        for &i in idx.iter().rev() {
            if i < project.sequence.len() {
                project.sequence.remove(i);
            }
        }
        sel.clear();
    }

    let n = project.sequence.len();
    let job = project.job.clone();
    let mut rows: Vec<egui::Rect> = Vec::with_capacity(n);
    if n == 0 {
        ui.label(RichText::new("The sequence is empty.").small().weak());
    }
    for i in 0..n {
        let item = project.sequence[i];
        let desc = step_desc(project, &item);
        let selected = sel.contains(&i);
        let bg = ui.painter().add(egui::Shape::Noop);
        let mut label_rect = egui::Rect::NOTHING;
        ui.horizontal(|ui| {
            let w = (ui.available_width() - if item.is_edge() { 54.0 } else { 28.0 }).max(40.0);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::click_and_drag());
            label_rect = rect;
            if resp.drag_started() {
                *drag = Some(i);
                if !selected {
                    sel.clear();
                    sel.push(i);
                }
            }
            if resp.clicked() {
                let m = ui.input(|inp| inp.modifiers);
                if m.shift && !sel.is_empty() {
                    let anchor = *sel.last().unwrap();
                    *sel = (anchor.min(i)..=anchor.max(i)).collect();
                } else if m.command {
                    if let Some(pos) = sel.iter().position(|&x| x == i) {
                        sel.remove(pos);
                    } else {
                        sel.push(i);
                    }
                } else {
                    sel.clear();
                    sel.push(i);
                }
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(if drag.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            let col = ui.visuals().text_color();
            let painter = ui.painter().with_clip_rect(rect);
            for dx in [0.0_f32, 4.0] {
                for dy in [-4.0_f32, 0.0, 4.0] {
                    painter.circle_filled(rect.left_center() + egui::vec2(5.0 + dx, dy), 1.1, col.gamma_multiply(0.5));
                }
            }
            painter.text(
                rect.left_center() + egui::vec2(18.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{:>2}.  {}", i + 1, desc),
                egui::FontId::proportional(13.0),
                col,
            );
            if item.is_edge() {
                let rev = icons::icon_button_ex(ui, icons::Icon::Reverse, "Reverse direction", item.reverse, true);
                if rev.clicked() {
                    project.sequence[i].reverse = !item.reverse;
                }
            }
            if item.kind != Kind3::Home {
                let b = icons::icon_button_ex(ui, icons::Icon::Settings, "Step settings", item.customised(), true);
                let bodies = project.bodies.clone();
                let mut want_surface = false;
                egui::Popup::from_toggle_button_response(&b)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        want_surface = step_settings(ui, &bodies, &mut project.sequence[i], &job, mode);
                    });
                if want_surface {
                    *sel = vec![i];
                }
            }
        });
        let full = egui::Rect::from_min_max(
            egui::pos2(ui.min_rect().left(), label_rect.top() - 1.0),
            egui::pos2(ui.max_rect().right(), label_rect.bottom() + 1.0),
        );
        rows.push(full);
        let fill = if selected {
            Some(theme::ACCENT.gamma_multiply(0.7))
        } else if i % 2 == 1 {
            Some(ui.visuals().faint_bg_color)
        } else {
            None
        };
        if let Some(f) = fill {
            ui.painter().set(bg, egui::Shape::rect_filled(full, egui::CornerRadius::same(2), f));
        }
    }

    if drag.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(ptr) = ui.input(|i| i.pointer.interact_pos()) {
            let ins = rows.iter().position(|r| ptr.y < r.center().y).unwrap_or(n);
            if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
                let y = if ins < n { rows[ins].top() } else { last.bottom() };
                ui.painter().line_segment(
                    [egui::pos2(first.left(), y), egui::pos2(first.right(), y)],
                    egui::Stroke::new(2.0, theme::PATH_HOVER),
                );
            }
            if ui.input(|i| i.pointer.any_released()) {
                move_steps(project, sel, ins);
                *drag = None;
            }
        } else {
            *drag = None;
        }
    }
}
