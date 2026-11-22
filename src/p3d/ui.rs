//! Screen of the 3D project. Same layout as the 2D project: Prepare,
//! Path selection, Preview and G-code tabs.

use super::gcode;
use super::model::*;
use super::step::{self, ImportJob, V3};
use super::view::{Overlay, PickMode, StepViz, View3};
use crate::common::icons;
use crate::common::machine::{AxisOwner, ControlType, JobSettings, Library};
use crate::common::preview::{self, Preview};
use crate::common::theme;
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
    /// What a Pick button of a step is choosing in the view.
    pick: Pick,
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
    Path,
    Home,
    Goto,
    Pause,
    Clear,
}

impl P3App {
    pub fn new(machine: crate::common::machine::Machine) -> Self {
        Self::from_project(Project3::new(machine))
    }

    fn from_project(project: Project3) -> Self {
        Self {
            project,
            path: None,
            view: View3::default(),
            tab: Tab::Prepare,
            pick: None,
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
            .enumerate()
            .map(|(i, s)| match s.kind {
                Kind3::Path => StepViz { pts: p.step_points(s), lead: p.lead_in_points(i), is_path: true },
                Kind3::Home => StepViz { pts: vec![home], lead: vec![], is_path: false },
                Kind3::Goto { x, y, z } => StepViz { pts: vec![[x, y, z]], lead: vec![], is_path: false },
                Kind3::Pause { .. } => StepViz { pts: vec![], lead: vec![], is_path: false },
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

        let mut generate = false;
        egui::Panel::left("p3_left").resizable(false).exact_size(330.0).show(root, |ui| {
            const ORDER: [Tab; 4] = [Tab::Prepare, Tab::PathSelection, Tab::Preview, Tab::Gcode];
            let cur = ORDER.iter().position(|t| *t == self.tab).unwrap_or(0);
            let picked = icons::tab_strip(ui, &[
                (icons::Icon::TabPrepare, "Prepare", true),
                (icons::Icon::TabPath, "Path selection", true),
                (icons::Icon::TabSimulation, "Simulation", generated),
                (icons::Icon::TabGcode, "G-code", generated),
            ], cur);
            if let Some(i) = picked {
                self.tab = ORDER[i];
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
              self.jobs_ui(ui);
              match self.tab {
                Tab::Prepare => self.prepare_panel(ui, lib),
                Tab::PathSelection => {
                    ui.heading("Job settings");
                    job_grid(ui, &mut self.project.job);
                    ui.separator();
                    sequence_ui(ui, &mut self.project, &mut self.sel_steps, &mut self.seq_drag, &mut self.pick, &self.viz);
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

        let Self { project, view, tab, pick, selected_body, sel_steps, viz, preview, gcode, cursor, msgs, lay_face, .. } = self;
        let mut edge_click = None;
        let mut face_click = None;
        let mut point_click = None;
        if *tab != Tab::PathSelection {
            *pick = None;
        }
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
                        match *pick {
                            Some((i, t)) => {
                                ui.label(format!("Click {} for step {}", t.label(), i + 1));
                                if ui.button("Cancel").clicked() {
                                    *pick = None;
                                }
                            }
                            None => {
                                ui.label(RichText::new("Open a step and press Pick to choose an edge, surface or point.").small().weak());
                            }
                        }
                    }
                    let hint = if *tab == Tab::Prepare && *lay_face {
                        "Click a flat face to lay it on the bed."
                    } else if *tab == Tab::Prepare {
                        "Drag a part to place it on the bed.   Drag empty space: orbit   Right drag: pan   Wheel: zoom"
                    } else {
                        "Parts are locked.   Drag: orbit   Right drag: pan   Wheel: zoom   Esc: cancel a pick"
                    };
                    ui.label(RichText::new(hint).small().weak());
                });
                let tints: Vec<((usize, usize), [u8; 3])> = if *tab == Tab::PathSelection {
                    project
                        .sequence
                        .iter()
                        .enumerate()
                        .flat_map(|(i, s)| {
                            let main = s.surface.map(|f| (f, if sel_steps.contains(&i) { [60, 200, 216] } else { [196, 138, 138] }));
                            main.into_iter().chain(s.lead_surfaces.iter().map(|f| (*f, [128, 190, 120])))
                        })
                        .collect()
                } else {
                    vec![]
                };
                let ov = Overlay {
                    mode: if *tab == Tab::Prepare {
                        if *lay_face { PickMode::LayFace } else { PickMode::Move }
                    } else {
                        match pick.map(|p| p.1) {
                            Some(Target::Path) => PickMode::Edges,
                            Some(Target::Surface | Target::LeadSurfaces) => PickMode::Surface,
                            Some(Target::Start | Target::Stop) => PickMode::Any,
                            None => PickMode::Idle,
                        }
                    },
                    selected_body: if *tab == Tab::Prepare { *selected_body } else { None },
                    tints: &tints,
                    steps: viz,
                    show_steps: *tab == Tab::PathSelection,
                    sel_steps,
                };
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    *pick = None;
                    *lay_face = false;
                }
                let out = view.show(ui, project, &ov);
                *cursor = out.cursor;
                if let Some(b) = out.body_click {
                    *selected_body = b;
                }
                edge_click = out.edge_click;
                face_click = out.face_click;
                point_click = out.point_click;
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
        } else if let Some((si, t)) = *pick {
            // The thing picked goes to the step that asked for it.
            let Some(st) = project.sequence.get_mut(si) else {
                *pick = None;
                return;
            };
            let done = match t {
                Target::Path => edge_click.map(|e| st.path = Some(e)).is_some(),
                Target::Surface => face_click.map(|f| st.surface = Some(f)).is_some(),
                Target::LeadSurfaces => {
                    // Stays active so several surfaces can be picked.
                    if let Some(f) = face_click {
                        match st.lead_surfaces.iter().position(|x| *x == f) {
                            Some(k) => {
                                st.lead_surfaces.remove(k);
                            }
                            None => st.lead_surfaces.push(f),
                        }
                    }
                    false
                }
                Target::Start | Target::Stop => {
                    // One click gives an edge, a surface or a point.
                    let l = edge_click
                        .map(|(b, e)| Limit::Edge(b, e))
                        .or(face_click.map(|(b, f)| Limit::Surface(b, f)))
                        .or(point_click.map(Limit::Point));
                    if let Some(l) = l {
                        if t == Target::Start {
                            st.start = Some(l);
                        } else {
                            st.stop = Some(l);
                        }
                    }
                    l.is_some()
                }
            };
            if done {
                *pick = None;
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
        let mut pick: Option<usize> = None;
        egui::ComboBox::from_id_salt("project_machine")
            .selected_text(self.project.machine.name.clone())
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                for (i, m) in lib.machines.iter().enumerate() {
                    if ui.selectable_label(m.name == self.project.machine.name, &m.name).clicked() {
                        pick = Some(i);
                    }
                }
            });
        if let Some(m) = pick.and_then(|i| lib.machines.get(i)) {
            self.project.machine = m.clone();
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

/// What a Pick button on a step is choosing in the view.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Path,
    Surface,
    Start,
    Stop,
    /// Surfaces the lead in follows; each click adds or removes one.
    LeadSurfaces,
}

impl Target {
    fn label(self) -> &'static str {
        match self {
            Target::Path => "the path edge",
            Target::Surface => "the surface",
            Target::Start => "the start (an edge or a surface; Ctrl+click for a point)",
            Target::Stop => "the end (an edge or a surface; Ctrl+click for a point)",
            Target::LeadSurfaces => "the surfaces the lead in follows (click one again to remove it, Esc when done)",
        }
    }
}

/// The step and target being picked in the view, if any.
type Pick = Option<(usize, Target)>;

fn toggle(pick: &mut Pick, i: usize, t: Target) {
    *pick = if *pick == Some((i, t)) { None } else { Some((i, t)) };
}

fn step_desc(project: &Project3, s: &Step3) -> String {
    match s.kind {
        Kind3::Path => match s.path {
            Some((b, e)) => format!(
                "{} edge {}{}",
                project.bodies.get(b).map(|x| x.name.as_str()).unwrap_or("?"),
                e + 1,
                if s.reverse { "  (reversed)" } else { "" }
            ),
            None => "Path: pick an edge".into(),
        },
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

const BLUE: egui::Color32 = egui::Color32::from_rgb(0x3b, 0x82, 0xd6);

/// A section like in a SolidWorks property page: bold title, a chevron at the
/// right and a rule under the header. Click the header to open or close it.
fn section(ui: &mut egui::Ui, id: egui::Id, title: &str, default_open: bool, add: impl FnOnce(&mut egui::Ui)) {
    let open = ui.data_mut(|d| *d.get_temp_mut_or(id, default_open));
    let head = ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(if open { "^" } else { "v" }).weak());
        });
    });
    if ui.interact(head.response.rect, id.with("head"), egui::Sense::click()).clicked() {
        ui.data_mut(|d| d.insert_temp(id, !open));
    }
    let y = head.response.rect.bottom() + 1.0;
    let (l, r) = (head.response.rect.left(), head.response.rect.right());
    ui.painter().line_segment([egui::pos2(l, y), egui::pos2(r, y)], egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color));
    ui.add_space(3.0);
    if open {
        add(ui);
        ui.add_space(4.0);
    }
}

/// An icon and a box that shows the selected element. Click the box to pick in
/// the view (it turns blue while active); the x clears it.
/// Returns (box clicked, cleared).
fn select_box(ui: &mut egui::Ui, icon: icons::Icon, text: Option<String>, hint: &str, active: bool) -> (bool, bool) {
    let (mut clicked, mut cleared) = (false, false);
    ui.horizontal(|ui| {
        icons::icon_label(ui, icon, BLUE);
        let w = (ui.available_width() - 28.0).max(60.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::click());
        let fill = if active { BLUE.gamma_multiply(0.30) } else { ui.visuals().extreme_bg_color };
        let stroke = if active || resp.hovered() { egui::Stroke::new(1.5, BLUE) } else { ui.visuals().widgets.noninteractive.bg_stroke };
        ui.painter().rect(rect, egui::CornerRadius::same(2), fill, stroke, egui::StrokeKind::Inside);
        let (t, col) = match &text {
            Some(t) => (t.as_str(), ui.visuals().text_color()),
            None => (hint, ui.visuals().weak_text_color()),
        };
        ui.painter().with_clip_rect(rect.shrink(2.0)).text(rect.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, t, egui::FontId::proportional(13.0), col);
        clicked = resp.clicked();
        cleared = icons::icon_button_ex(ui, icons::Icon::Remove, "Clear", false, text.is_some()).clicked();
    });
    (clicked, cleared)
}

/// An offset in mm with the icon and a direction toggle. It shows the default
/// (the tool offset) until it is changed; x puts the default back.
fn offset_row(ui: &mut egui::Ui, val: &mut Option<f64>, default: f64, flip: Option<(&mut bool, (&str, &str), &str)>) {
    ui.horizontal(|ui| {
        icons::icon_label(ui, icons::Icon::Offset, BLUE);
        let mut v = val.unwrap_or(default);
        if ui.add(egui::DragValue::new(&mut v).speed(0.05).range(0.0..=10000.0).suffix(" mm")).changed() {
            *val = Some(v);
        }
        if val.is_some() && ui.small_button("x").on_hover_text("Back to the default").clicked() {
            *val = None;
        }
        if let Some((flip, names, tip)) = flip {
            if icons::icon_button_ex(ui, icons::Icon::Reverse, tip, *flip, true).clicked() {
                *flip = !*flip;
            }
            ui.label(RichText::new(if *flip { names.1 } else { names.0 }).small().weak());
        }
    });
}

fn limit_txt(names: &[String], l: Option<Limit>) -> Option<String> {
    let part = |b: usize| names.get(b).map(String::as_str).unwrap_or("?");
    match l? {
        Limit::Point(p) => Some(format!("point {:.1}, {:.1}, {:.1}", p[0], p[1], p[2])),
        Limit::Edge(b, e) => Some(format!("{} edge {}", part(b), e + 1)),
        Limit::Surface(b, f) => Some(format!("{} face {}", part(b), f + 1)),
    }
}

/// The Start or End section of a path step.
#[allow(clippy::too_many_arguments)]
fn limit_section(
    ui: &mut egui::Ui,
    i: usize,
    start: bool,
    limit: &mut Option<Limit>,
    offset: &mut Option<f64>,
    flip: &mut bool,
    tool_r: f64,
    names: &[String],
    pick: &mut Pick,
) {
    let title = if start { "Start" } else { "End" };
    let id = ui.make_persistent_id(("p3_sec", i, title));
    section(ui, id, title, true, |ui| {
        let icon = match limit {
            Some(Limit::Point(_)) => icons::Icon::SelPoint,
            Some(Limit::Surface(..)) => icons::Icon::SelFace,
            _ => icons::Icon::SelEdge,
        };
        let target = if start { Target::Start } else { Target::Stop };
        let hint = format!("Select the {} edge, surface or point", if start { "start" } else { "end" });
        let (c, x) = select_box(ui, icon, limit_txt(names, *limit), &hint, *pick == Some((i, target)));
        if c {
            toggle(pick, i, target);
        }
        if x {
            *limit = None;
        }
        offset_row(ui, offset, tool_r, Some((flip, ("Inward", "Outward"), "Direction of the offset: inward shortens the path from this end, outward lengthens it.")));
    });
}

/// The options of an expanded step.
fn step_body(ui: &mut egui::Ui, i: usize, s: &mut Step3, job: &Job3, names: &[String], pick: &mut Pick, around: (Option<V3>, Option<V3>)) {
    let part = |b: usize| names.get(b).map(String::as_str).unwrap_or("?").to_string();
    match s.kind {
        Kind3::Path => {
            let tool_r = job.with(&s.over).tool_diameter.max(0.0) / 2.0;
            let cur = *pick;
            let on = move |t: Target| cur == Some((i, t));
            let (id_path, id_over, id_lead) = (ui.make_persistent_id(("p3_sec", i, "path")), ui.make_persistent_id(("p3_sec", i, "over")), ui.make_persistent_id(("p3_sec", i, "lead")));
            section(ui, id_path, "Path", true, |ui| {
                let txt = s.path.map(|(b, e)| format!("{} edge {}", part(b), e + 1));
                let (c, x) = select_box(ui, icons::Icon::SelEdge, txt, "Select the path edge", on(Target::Path));
                if c {
                    toggle(pick, i, Target::Path);
                }
                if x {
                    s.path = None;
                }
                let txt = s.surface.map(|(b, f)| format!("{} face {}", part(b), f + 1));
                let (c, x) = select_box(ui, icons::Icon::SelFace, txt, "Select the surface (Z gap)", on(Target::Surface));
                if c {
                    toggle(pick, i, Target::Surface);
                }
                if x {
                    s.surface = None;
                }
                offset_row(ui, &mut s.offset, tool_r, Some((&mut s.offset_flip, ("Outside", "Inside"), "Side of the path offset: outside is away from the surface, inside is toward it.")));
            });
            limit_section(ui, i, true, &mut s.start, &mut s.start_offset, &mut s.start_flip, tool_r, names, pick);
            limit_section(ui, i, false, &mut s.stop, &mut s.stop_offset, &mut s.stop_flip, tool_r, names, pick);
            section(ui, id_lead, "Lead in", true, |ui| {
                ui.checkbox(&mut s.lead_in, "From the end of the previous step")
                    .on_hover_text("The tool goes straight from where the previous step ended to the start of this path, without lifting. Needs a previous step.");
                if s.lead_in {
                    let n = s.lead_surfaces.len();
                    let txt = (n > 0).then(|| format!("{n} surface{}", if n == 1 { "" } else { "s" }));
                    let (c, x) = select_box(ui, icons::Icon::SelFace, txt, "Select the surfaces to follow (optional)", on(Target::LeadSurfaces));
                    if c {
                        toggle(pick, i, Target::LeadSurfaces);
                    }
                    if x {
                        s.lead_surfaces.clear();
                    }
                    let mut remove = None;
                    for (k, (b, f)) in s.lead_surfaces.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_space(26.0);
                            ui.label(RichText::new(format!("{} face {}", part(*b), f + 1)).small());
                            if ui.small_button("x").on_hover_text("Remove").clicked() {
                                remove = Some(k);
                            }
                        });
                    }
                    if let Some(k) = remove {
                        s.lead_surfaces.remove(k);
                    }
                    // How far the lead in keeps from those surfaces, on the side it is on.
                    offset_row(ui, &mut s.lead_offset, tool_r, Some((&mut s.lead_flip, ("Same side", "Other side"), "If the lead in is not right, switch the side: it keeps to the other side of a wall, or goes round a post (or any solid in its way) on the other hand, right instead of left of its travel direction.")));
                }
            });
            section(ui, id_over, "Overrides of the job settings", false, |ui| {
                let o = &mut s.over;
                egui::Grid::new(("p3_over", i)).num_columns(2).show(ui, |ui| {
                    override_row(ui, "Work feed", &mut o.feed, job.dispense_feed, 5.0, " mm/min");
                    override_row(ui, "Travel feed", &mut o.travel_feed, job.travel_feed, 10.0, " mm/min");
                    override_row(ui, "Z gap", &mut o.z_gap, job.z_gap, 0.05, " mm");
                    override_row(ui, "Tool diameter", &mut o.tool_diameter, job.tool_diameter, 0.05, " mm");
                    let mut on_ms = o.on_delay_ms.map(|v| v as f64);
                    override_row(ui, "Start delay", &mut on_ms, job.on_delay_ms as f64, 5.0, " ms");
                    o.on_delay_ms = on_ms.map(|v| v.max(0.0) as u32);
                    let mut off_ms = o.off_delay_ms.map(|v| v as f64);
                    override_row(ui, "Stop delay", &mut off_ms, job.off_delay_ms as f64, 5.0, " ms");
                    o.off_delay_ms = off_ms.map(|v| v.max(0.0) as u32);
                });
                if ui.add_enabled(!o.is_empty(), egui::Button::new("Use job settings")).clicked() {
                    *o = Over3::default();
                }
            });
        }
        Kind3::Goto { .. } => {
            if let Kind3::Goto { x, y, z } = &mut s.kind {
                egui::Grid::new(("p3_goto", i)).num_columns(2).show(ui, |ui| {
                    for (l, v) in [("X", x), ("Y", y), ("Z", z)] {
                        ui.label(l);
                        ui.add(egui::DragValue::new(v).speed(0.1).suffix(" mm"));
                        ui.end_row();
                    }
                });
                // For reference: where the steps around this one are.
                let at = |p: V3| format!("X {:.2}   Y {:.2}   Z {:.2}", p[0], p[1], p[2]);
                if let Some(p) = around.0 {
                    ui.label(RichText::new(format!("Previous step ends at  {}", at(p))).small().weak());
                }
                if let Some(p) = around.1 {
                    ui.label(RichText::new(format!("Next step starts at  {}", at(p))).small().weak());
                }
            }
        }
        Kind3::Pause { .. } => {
            if let Kind3::Pause { ms } = &mut s.kind {
                ui.horizontal(|ui| {
                    ui.label("Wait");
                    ui.add(egui::DragValue::new(ms).speed(10.0).range(0..=600_000).suffix(" ms"));
                });
            }
        }
        Kind3::Home => {}
    }
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
    let items: Vec<Step3> = idx.iter().map(|&i| project.sequence[i].clone()).collect();
    for &i in idx.iter().rev() {
        project.sequence.remove(i);
    }
    let at = (ins - before).min(project.sequence.len());
    for (k, it) in items.iter().enumerate() {
        project.sequence.insert(at + k, it.clone());
    }
    *sel = (at..at + items.len()).collect();
}

fn sequence_ui(ui: &mut egui::Ui, project: &mut Project3, sel: &mut Vec<usize>, drag: &mut Option<usize>, pick: &mut Pick, viz: &[StepViz]) {
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
                if ui.button("Path").clicked() {
                    add = Some(AddStep::Path);
                }
                ui.separator();
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
    ui.label(RichText::new("Add a Path, then open its row and use Pick to choose the edge and surface. Drag rows to reorder.").small().weak());

    if !sel.is_empty() && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        delete = true;
    }

    if let Some(a) = add {
        *pick = None;
        let at = sel.iter().max().map_or(project.sequence.len(), |m| m + 1).min(project.sequence.len());
        let home = project.machine.home_position();
        let new = match a {
            AddStep::Path => Some(Step3::new(Kind3::Path)),
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
        *pick = None;
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
    let names: Vec<String> = project.bodies.iter().map(|b| b.name.clone()).collect();
    let mut rows: Vec<egui::Rect> = Vec::with_capacity(n);
    if n == 0 {
        ui.label(RichText::new("The sequence is empty.").small().weak());
    }
    for i in 0..n {
        let item = project.sequence[i].clone();
        let desc = step_desc(project, &item);
        let selected = sel.contains(&i);
        let bg = ui.painter().add(egui::Shape::Noop);
        let block = ui
            .vertical(|ui| {
                ui.horizontal(|ui| {
                    if item.kind == Kind3::Home {
                        ui.add_space(24.0);
                    } else if ui.small_button(if item.open { "v" } else { ">" }).on_hover_text("Show or hide the options").clicked() {
                        project.sequence[i].open = !item.open;
                    }
                    let w = (ui.available_width() - if item.is_path() { 26.0 } else { 0.0 }).max(40.0);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::click_and_drag());
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
                    if item.is_path() {
                        let rev = icons::icon_button_ex(ui, icons::Icon::Reverse, "Reverse direction", item.reverse, true);
                        if rev.clicked() {
                            project.sequence[i].reverse = !item.reverse;
                        }
                    }
                });
                if item.open && item.kind != Kind3::Home {
                    // For a go to: where the neighbouring steps are, for reference.
                    let before = (0..i).rev().find_map(|k| viz.get(k).and_then(|v| v.pts.last().copied()));
                    let after = (i + 1..n).find_map(|k| viz.get(k).and_then(|v| v.pts.first().copied()));
                    ui.indent(("p3_body", i), |ui| step_body(ui, i, &mut project.sequence[i], &job, &names, pick, (before, after)));
                }
            })
            .response
            .rect;
        let full = egui::Rect::from_min_max(
            egui::pos2(ui.min_rect().left(), block.top() - 1.0),
            egui::pos2(ui.max_rect().right(), block.bottom() + 1.0),
        );
        rows.push(full);
        let fill = if selected {
            Some(theme::ACCENT.gamma_multiply(0.45))
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
                *pick = None;
                move_steps(project, sel, ins);
                *drag = None;
            }
        } else {
            *drag = None;
        }
    }
}
