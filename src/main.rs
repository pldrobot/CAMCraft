mod canvas;
mod dxf_import;
mod gcode;
mod model;
mod preview;
mod theme;

use canvas::{Canvas, Tool};
use eframe::egui;
use egui::{Color32, RichText};
use model::*;
use preview::Preview;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(PartialEq, Clone, Copy)]
enum Screen {
    Project,
    Machines,
}

#[derive(PartialEq, Clone, Copy)]
enum MachineTab {
    Machine,
    Speed,
    EndEffector,
    Scripts,
}

#[derive(PartialEq, Clone, Copy)]
enum ProjectTab {
    Prepare,
    PathSelection,
    Preview,
}

enum Action {
    NewProject,
    OpenProject,
    SaveProject(bool), // true = save as
    InsertDxf,
    Generate,
    SaveGcode,
    CopyGcode,
}

struct PendingDxf {
    name: String,
    drawing: dxf::Drawing,
    /// Layer name, entity count, selected for import.
    layers: Vec<(String, usize, bool)>,
}

struct App {
    lib: Library,
    lib_dirty: bool,
    screen: Screen,
    project: Option<Project>,
    project_path: Option<PathBuf>,
    canvas: Canvas,
    gcode: String,
    gcode_open: bool,
    warnings: Vec<String>,
    log: Vec<String>,
    show_new: bool,
    new_machine_idx: usize,
    machine_tab: MachineTab,
    rename: Option<String>,
    machine_sel: usize,
    tab: ProjectTab,
    preview: Preview,
    pending_dxf: Option<PendingDxf>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let lib = Library::load();
        Self {
            lib,
            lib_dirty: false,
            screen: Screen::Project,
            project: None,
            project_path: None,
            canvas: Canvas::default(),
            gcode: String::new(),
            gcode_open: false,
            warnings: vec![],
            log: vec!["CAMCraft ready. Command: New Project".into()],
            show_new: false,
            new_machine_idx: 0,
            machine_tab: MachineTab::Machine,
            rename: None,
            machine_sel: 0,
            tab: ProjectTab::Prepare,
            preview: Preview::default(),
            pending_dxf: None,
        }
    }

    fn say(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    // ---------- actions ----------

    fn run(&mut self, a: Action) {
        match a {
            Action::NewProject => {
                self.show_new = true;
            }
            Action::OpenProject => {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("CAMCraft project", &["camcraft"])
                    .pick_file()
                else {
                    return;
                };
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<Project>(&s).map_err(|e| e.to_string()))
                {
                    Ok(p) => {
                        self.project = Some(p);
                        self.project_path = Some(path.clone());
                        self.canvas = Canvas::default();
                        self.gcode.clear();
                        self.warnings.clear();
                        self.preview.clear();
                        self.tab = ProjectTab::Prepare;
                        self.screen = Screen::Project;
                        self.say(format!("Opened {}", path.display()));
                    }
                    Err(e) => self.say(format!("Open failed: {e}")),
                }
            }
            Action::SaveProject(as_new) => {
                let Some(project) = &self.project else { return };
                let path = if as_new || self.project_path.is_none() {
                    rfd::FileDialog::new()
                        .add_filter("CAMCraft project", &["camcraft"])
                        .set_file_name("project.camcraft")
                        .save_file()
                } else {
                    self.project_path.clone()
                };
                let Some(path) = path else { return };
                let res = serde_json::to_string_pretty(project)
                    .map_err(|e| e.to_string())
                    .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
                match res {
                    Ok(()) => {
                        self.say(format!("Saved {}", path.display()));
                        self.project_path = Some(path);
                    }
                    Err(e) => self.say(format!("Save failed: {e}")),
                }
            }
            Action::InsertDxf => {
                let Some(path) = rfd::FileDialog::new().add_filter("DXF", &["dxf"]).pick_file() else {
                    return;
                };
                let drawing = match dxf_import::load_drawing(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        self.say(format!("DXF import failed: {e}"));
                        return;
                    }
                };
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "part".into());
                let layers: Vec<(String, usize, bool)> = dxf_import::layers(&drawing)
                    .into_iter()
                    .map(|(n, c)| (n, c, true))
                    .collect();
                if layers.len() <= 1 {
                    let r = dxf_import::import(&drawing);
                    self.add_part(name, r);
                } else {
                    self.pending_dxf = Some(PendingDxf { name, drawing, layers });
                }
            }
            Action::Generate => {
                // The project holds a copy of its machine. Take the current
                // settings from the library so edits are not missed.
                if let Some(project) = &mut self.project {
                    if let Some(m) = self.lib.machines.iter().find(|m| m.name == project.machine.name) {
                        project.machine = m.clone();
                    }
                }
                let Some(project) = &self.project else { return };
                let r = gcode::generate(project);
                let lines = r.text.lines().count();
                self.preview.set_moves(r.moves, &project.job);
                self.gcode = r.text;
                self.warnings = r.warnings;
                self.gcode_open = true;
                self.say(format!("G-code generated: {lines} lines, {} warning(s).", self.warnings.len()));
            }
            Action::SaveGcode => {
                let ext = "gcode";
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("G-code", &[ext, "nc", "gc"])
                    .set_file_name("job.gcode")
                    .save_file()
                else {
                    return;
                };
                match std::fs::write(&path, &self.gcode) {
                    Ok(()) => self.say(format!("Saved {}", path.display())),
                    Err(e) => self.say(format!("Save failed: {e}")),
                }
            }
            Action::CopyGcode => {
                // Handled in the UI through the egui context.
            }
        }
    }

    fn add_part(&mut self, name: String, r: dxf_import::ImportResult) {
        if r.paths.is_empty() {
            self.say("No usable geometry found in the DXF.");
            return;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in r.paths.iter().flat_map(|p| p.pts.iter()) {
            minx = minx.min(p[0]);
            miny = miny.min(p[1]);
            maxx = maxx.max(p[0]);
            maxy = maxy.max(p[1]);
        }
        let n = r.paths.len();
        let (w, h) = (maxx - minx, maxy - miny);
        let Some(project) = &mut self.project else { return };
        project.parts.push(Part {
            name: name.clone(),
            paths: r.paths,
            // Put the lower-left corner of the drawing 5 mm inside the workable area.
            offset: [
                project.machine.workable()[0].0 + 5.0 - minx,
                project.machine.workable()[1].0 + 5.0 - miny,
            ],
        });
        let wk = project.machine.workable();
        let big = w > wk[0].1 - wk[0].0 || h > wk[1].1 - wk[1].0;
        self.canvas.selected_part = Some(project.parts.len() - 1);
        self.canvas.tool = Tool::Move;
        self.say(format!("Inserted {name}: {n} paths, {:.1} x {:.1} mm (assumed mm)", w, h));
        for s in r.skipped {
            self.say(s);
        }
        if big {
            self.say("Warning: drawing is larger than the workable area.");
        }
    }

    fn layer_window(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.pending_dxf else { return };
        let mut open = true;
        let mut import = false;
        let mut cancel = false;
        egui::Window::new("Select DXF layers")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{}: choose the layers to import", p.name));
                ui.horizontal(|ui| {
                    if ui.small_button("All").clicked() {
                        p.layers.iter_mut().for_each(|l| l.2 = true);
                    }
                    if ui.small_button("None").clicked() {
                        p.layers.iter_mut().for_each(|l| l.2 = false);
                    }
                });
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for (name, count, on) in p.layers.iter_mut() {
                        let label = if name.is_empty() { "(no layer name)" } else { name.as_str() };
                        ui.checkbox(on, format!("{label}  ({count})"));
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let any = p.layers.iter().any(|l| l.2);
                    if ui.add_enabled(any, egui::Button::new("Import")).clicked() {
                        import = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if import {
            if let Some(p) = self.pending_dxf.take() {
                let set: HashSet<String> =
                    p.layers.iter().filter(|l| l.2).map(|l| l.0.clone()).collect();
                let r = dxf_import::import_layers(&p.drawing, Some(&set));
                self.add_part(p.name, r);
            }
        } else if cancel || !open {
            self.pending_dxf = None;
        }
    }

    // ---------- panels ----------

    fn top_bar(&mut self, root: &mut egui::Ui, actions: &mut Vec<Action>) {
        egui::Panel::top("top").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Project...").clicked() {
                        actions.push(Action::NewProject);
                        ui.close();
                    }
                    if ui.button("Open Project...").clicked() {
                        actions.push(Action::OpenProject);
                        ui.close();
                    }
                    ui.add_enabled_ui(self.project.is_some(), |ui| {
                        if ui.button("Save").clicked() {
                            actions.push(Action::SaveProject(false));
                            ui.close();
                        }
                        if ui.button("Save As...").clicked() {
                            actions.push(Action::SaveProject(true));
                            ui.close();
                        }
                    });
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.separator();
                ui.selectable_value(&mut self.screen, Screen::Project, "Project");
                ui.selectable_value(&mut self.screen, Screen::Machines, "Machines");
            });
        });
    }

    fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                let coords = match (self.screen, self.canvas.cursor_world) {
                    (Screen::Project, Some(p)) => format!("X {:8.2}   Y {:8.2}", p[0], p[1]),
                    _ => "X          Y".to_string(),
                };
                ui.label(RichText::new(coords).monospace().color(Color32::from_rgb(0x4f, 0xb6, 0xff)));
                ui.separator();
                ui.label(RichText::new(format!("> {}", self.log.last().cloned().unwrap_or_default())).monospace());
            });
        });
    }

    fn new_project_window(&mut self, ctx: &egui::Context) {
        if !self.show_new {
            return;
        }
        let mut open = true;
        let mut create = false;
        let mut goto_machines = false;
        egui::Window::new("New Project")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if self.lib.machines.is_empty() {
                    ui.label("No machines yet. Add one first.");
                    if ui.button("Open Machine management").clicked() {
                        goto_machines = true;
                    }
                } else {
                    self.new_machine_idx = self.new_machine_idx.min(self.lib.machines.len() - 1);
                    ui.label("Select machine");
                    egui::ComboBox::from_id_salt("new_machine")
                        .selected_text(self.lib.machines[self.new_machine_idx].name.clone())
                        .show_ui(ui, |ui| {
                            for (i, m) in self.lib.machines.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.new_machine_idx,
                                    i,
                                    format!("{}  (bed {} x {} x {} mm)", m.name, m.bed_x, m.bed_y, m.bed_z),
                                );
                            }
                        });
                    ui.add_space(6.0);
                    if ui.button("Create").clicked() {
                        create = true;
                    }
                }
            });
        if goto_machines {
            self.screen = Screen::Machines;
            self.show_new = false;
        }
        if create {
            let m = self.lib.machines[self.new_machine_idx].clone();
            self.project = Some(Project::new(m.clone()));
            self.project_path = None;
            self.canvas = Canvas::default();
            self.gcode.clear();
            self.warnings.clear();
            self.preview.clear();
            self.tab = ProjectTab::Prepare;
            self.screen = Screen::Project;
            self.show_new = false;
            self.say(format!("New project on {} ({} x {} mm)", m.name, m.bed_x, m.bed_y));
        }
        if !open {
            self.show_new = false;
        }
    }

    fn project_screen(&mut self, root: &mut egui::Ui, actions: &mut Vec<Action>) {
        if self.project.is_none() {
            egui::CentralPanel::default().show(root, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.35);
                    ui.heading("CAMCraft");
                    ui.label("Path programming for CNC machines");
                    ui.add_space(12.0);
                    if ui.button("New Project...").clicked() {
                        actions.push(Action::NewProject);
                    }
                    if ui.button("Open Project...").clicked() {
                        actions.push(Action::OpenProject);
                    }
                });
            });
            return;
        }

        let Self { lib, project, canvas, gcode, gcode_open, warnings, tab, preview, .. } = self;
        let project = project.as_mut().unwrap();
        let generated = preview.has_path();
        if *tab == ProjectTab::Preview && !generated {
            *tab = ProjectTab::Prepare;
        }

        egui::Panel::top("project_tabs").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(tab, ProjectTab::Prepare, "Prepare");
                ui.selectable_value(tab, ProjectTab::PathSelection, "Path selection");
                ui.add_enabled_ui(generated, |ui| {
                    ui.selectable_value(tab, ProjectTab::Preview, "Preview")
                        .on_disabled_hover_text("Generate G-code first.");
                });
            });
        });

        match *tab {
            ProjectTab::Prepare => canvas.tool = Tool::Move,
            ProjectTab::PathSelection => canvas.tool = Tool::Sequence,
            ProjectTab::Preview => {}
        }

        egui::Panel::left("left").default_size(310.0).show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                match *tab {
                    ProjectTab::Prepare => {
                        let m = &project.machine;
                        ui.heading("Machine");
                        ui.label(format!(
                            "{}   bed {} x {} x {} mm",
                            m.name, m.bed_x, m.bed_y, m.bed_z
                        ));
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
                        if ui
                            .button("Reload machine settings")
                            .on_hover_text("The project keeps a copy of the machine and its speeds. Use this after editing the machine in Machines.")
                            .clicked()
                        {
                            if let Some(m) = lib.machines.iter().find(|m| m.name == project.machine.name) {
                                project.job = m.job.clone();
                                project.machine = m.clone();
                            }
                        }
                        ui.separator();

                        ui.heading("Parts");
                        if ui.button("Insert DXF...").clicked() {
                            actions.push(Action::InsertDxf);
                        }
                        let mut remove: Option<usize> = None;
                        for pi in 0..project.parts.len() {
                            let selected = canvas.selected_part == Some(pi);
                            ui.horizontal(|ui| {
                                let name = format!(
                                    "{}  ({} paths)",
                                    project.parts[pi].name,
                                    project.parts[pi].paths.len()
                                );
                                if ui.selectable_label(selected, name).clicked() {
                                    canvas.selected_part = Some(pi);
                                }
                                if ui.small_button("x").on_hover_text("Remove part").clicked() {
                                    remove = Some(pi);
                                }
                            });
                            if selected {
                                let part = &mut project.parts[pi];
                                // Show the lower-left corner of the drawing on the bed, not the
                                // DXF origin, which can be far from the geometry.
                                let mut c = part.min_corner();
                                let before = c;
                                ui.horizontal(|ui| {
                                    ui.label("   X");
                                    ui.add(egui::DragValue::new(&mut c[0]).speed(0.1).suffix(" mm"));
                                    ui.label("Y");
                                    ui.add(egui::DragValue::new(&mut c[1]).speed(0.1).suffix(" mm"));
                                });
                                part.offset[0] += c[0] - before[0];
                                part.offset[1] += c[1] - before[1];
                            }
                        }
                        if let Some(i) = remove {
                            project.remove_part(i);
                            canvas.selected_part = None;
                        }
                        ui.separator();

                        ui.heading("Job settings");
                        let j = &mut project.job;
                        egui::Grid::new("job").num_columns(2).show(ui, |ui| {
                            ui.label("Work feed");
                            ui.add(egui::DragValue::new(&mut j.dispense_feed).speed(5.0).range(1.0..=20000.0).suffix(" mm/min"));
                            ui.end_row();
                            ui.label("Travel feed");
                            ui.add(egui::DragValue::new(&mut j.travel_feed).speed(10.0).range(1.0..=50000.0).suffix(" mm/min"));
                            ui.end_row();
                            ui.label("Use Z axis");
                            ui.checkbox(&mut j.use_z, "");
                            ui.end_row();
                            ui.add_enabled_ui(j.use_z, |ui| ui.label("Safe Z"));
                            ui.add_enabled(j.use_z, egui::DragValue::new(&mut j.safe_z).speed(0.1).suffix(" mm"));
                            ui.end_row();
                            ui.add_enabled_ui(j.use_z, |ui| ui.label("Work Z"));
                            ui.add_enabled(j.use_z, egui::DragValue::new(&mut j.dispense_z).speed(0.1).suffix(" mm"));
                            ui.end_row();
                            ui.label("Start delay");
                            ui.add(egui::DragValue::new(&mut j.on_delay_ms).speed(5.0).suffix(" ms"));
                            ui.end_row();
                            ui.label("Stop delay");
                            ui.add(egui::DragValue::new(&mut j.off_delay_ms).speed(5.0).suffix(" ms"));
                            ui.end_row();
                        });
                        ui.add_space(8.0);
                        if ui
                            .add_sized([ui.available_width(), 30.0], egui::Button::new(RichText::new("Generate G-code").strong()).fill(theme::ACCENT))
                            .clicked()
                        {
                            actions.push(Action::Generate);
                        }
                        for w in warnings.iter() {
                            ui.colored_label(Color32::from_rgb(0xff, 0xb0, 0x40), format!("! {w}"));
                        }
                    }
                    ProjectTab::PathSelection => {
                        ui.heading("Sequence");
                        ui.horizontal(|ui| {
                            if ui.button("Add all").clicked() {
                                for (pi, part) in project.parts.iter().enumerate() {
                                    for ci in 0..part.paths.len() {
                                        if project.seq_index(pi, ci).is_none() {
                                            project.sequence.push(SeqItem { part: pi, path: ci, reverse: false });
                                        }
                                    }
                                }
                            }
                            if ui.button("Clear").clicked() {
                                project.sequence.clear();
                            }
                        });
                        ui.label(
                            RichText::new("Click paths on the bed to add them in order. Click again to remove.")
                                .small()
                                .weak(),
                        );
                        let mut del: Option<usize> = None;
                        let mut up: Option<usize> = None;
                        let mut down: Option<usize> = None;
                        let n = project.sequence.len();
                        for i in 0..n {
                            let item = project.sequence[i];
                            ui.horizontal(|ui| {
                                let pname = project.parts.get(item.part).map(|p| p.name.clone()).unwrap_or_default();
                                ui.label(format!("{:>2}. {} #{}", i + 1, pname, item.path + 1));
                                if ui.small_button("^").clicked() && i > 0 {
                                    up = Some(i);
                                }
                                if ui.small_button("v").clicked() && i + 1 < n {
                                    down = Some(i);
                                }
                                if ui
                                    .selectable_label(item.reverse, "rev")
                                    .on_hover_text("Reverse direction")
                                    .clicked()
                                {
                                    project.sequence[i].reverse = !item.reverse;
                                }
                                if ui.small_button("x").clicked() {
                                    del = Some(i);
                                }
                            });
                        }
                        if let Some(i) = up {
                            project.sequence.swap(i, i - 1);
                        }
                        if let Some(i) = down {
                            project.sequence.swap(i, i + 1);
                        }
                        if let Some(i) = del {
                            project.sequence.remove(i);
                        }
                        ui.separator();
                        ui.add_space(4.0);
                        if ui
                            .add_sized([ui.available_width(), 30.0], egui::Button::new(RichText::new("Generate G-code").strong()).fill(theme::ACCENT))
                            .clicked()
                        {
                            actions.push(Action::Generate);
                        }
                    }
                    ProjectTab::Preview => {
                        let m = &project.machine;
                        ui.heading("Simulation");
                        egui::Grid::new("pv_stats").num_columns(2).show(ui, |ui| {
                            ui.label("Machine");
                            ui.label(&m.name);
                            ui.end_row();
                            ui.label("End effector");
                            ui.label(m.effector.kind.label());
                            ui.end_row();
                            ui.label("Path length");
                            ui.label(format!("{:.1} mm", preview.path_len));
                            ui.end_row();
                            ui.label("Work moves");
                            ui.label(preview.work_moves().to_string());
                            ui.end_row();
                            ui.label("Est. time");
                            ui.label(preview::fmt_time(preview.total_time()));
                            ui.end_row();
                            ui.label("Axes");
                            let own = |i: usize| match m.axes[i] {
                                AxisOwner::EndEffector => "tool",
                                AxisOwner::Bed => "bed",
                            };
                            ui.label(format!("X {}  Y {}  Z {}", own(0), own(1), own(2)));
                            ui.end_row();
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Time is estimated from the feed rates. Delays and acceleration are not included.")
                                .small()
                                .weak(),
                        );
                        for w in warnings.iter() {
                            ui.colored_label(Color32::from_rgb(0xff, 0xb0, 0x40), format!("! {w}"));
                        }
                    }
                }
            });
        });

        if *tab == ProjectTab::Prepare && !gcode.is_empty() && *gcode_open {
            egui::Panel::bottom("gcode_panel")
                .resizable(true)
                .default_size(180.0)
                .show(root, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("G-code");
                        if ui.button("Save...").clicked() {
                            actions.push(Action::SaveGcode);
                        }
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(gcode.clone());
                            actions.push(Action::CopyGcode);
                        }
                        if ui.button("Close").clicked() {
                            *gcode_open = false;
                        }
                        ui.label(format!("{} lines", gcode.lines().count()));
                    });
                    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut gcode.as_str())
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
                });
        }

        if *tab == ProjectTab::Preview {
            egui::Panel::bottom("preview_bar").show(root, |ui| {
                let total = preview.total_time();
                ui.horizontal(|ui| {
                    let label = if preview.playing { "Pause" } else if preview.time >= total { "Replay" } else { "Play" };
                    if ui.button(label).clicked() {
                        if preview.playing {
                            preview.playing = false;
                        } else {
                            if preview.time >= total {
                                preview.time = 0.0;
                            }
                            preview.playing = true;
                        }
                    }
                    if ui.button("Reset").clicked() {
                        preview.time = 0.0;
                        preview.playing = false;
                    }
                    let w = (ui.available_width() - 260.0).max(100.0);
                    ui.spacing_mut().slider_width = w;
                    let mut t = preview.time;
                    if ui
                        .add(egui::Slider::new(&mut t, 0.0..=total.max(0.001)).show_value(false))
                        .changed()
                    {
                        preview.time = t;
                        preview.playing = false;
                    }
                    ui.monospace(format!("{} / {}", preview::fmt_time(preview.time), preview::fmt_time(total)));
                    egui::ComboBox::from_id_salt("pv_speed")
                        .width(60.0)
                        .selected_text(format!("{}x", preview::SPEEDS[preview.speed_idx]))
                        .show_ui(ui, |ui| {
                            for (i, s) in preview::SPEEDS.iter().enumerate() {
                                ui.selectable_value(&mut preview.speed_idx, i, format!("{s}x"));
                            }
                        });
                });
            });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| match *tab {
                ProjectTab::Prepare | ProjectTab::PathSelection => {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        if ui.button("Zoom fit").clicked() {
                            canvas.zoom_fit();
                        }
                        let hint = if *tab == ProjectTab::Prepare {
                            "Drag a part to place it.   Wheel: zoom   Right or middle drag: pan"
                        } else {
                            "Click paths to build the order.   Wheel: zoom   Right or middle drag: pan"
                        };
                        ui.label(RichText::new(hint).small().weak());
                    });
                    canvas.show(ui, project);
                }
                ProjectTab::Preview => {
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
            });
    }

    fn machines_screen(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let Self { lib, lib_dirty, machine_sel, machine_tab, rename, .. } = self;
        *machine_sel = (*machine_sel).min(lib.machines.len().saturating_sub(1));

        egui::Panel::left("machine_list").default_size(360.0).show(root, |ui| {
            // Section 1: pick a machine, add, clone, rename.
            let selected = lib
                .machines
                .get(*machine_sel)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "(no machines)".into());
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("machine_pick")
                    .width(130.0)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (i, m) in lib.machines.iter().enumerate() {
                            ui.selectable_value(machine_sel, i, &m.name);
                        }
                    });
                if ui.button("Add").clicked() {
                    let name = unique_machine_name(&lib.machines, "New machine");
                    lib.machines.push(Machine::new(&name));
                    *machine_sel = lib.machines.len() - 1;
                    *lib_dirty = true;
                }
                let has = !lib.machines.is_empty();
                if ui.add_enabled(has, egui::Button::new("Clone")).clicked() {
                    let mut m = lib.machines[*machine_sel].clone();
                    m.name = unique_machine_name(&lib.machines, &format!("{} copy", m.name));
                    lib.machines.push(m);
                    *machine_sel = lib.machines.len() - 1;
                    *lib_dirty = true;
                }
                if ui.add_enabled(has, egui::Button::new("Rename")).clicked() {
                    *rename = Some(lib.machines[*machine_sel].name.clone());
                }
                if ui.add_enabled(has, egui::Button::new("Delete")).clicked() {
                    lib.machines.remove(*machine_sel);
                    *machine_sel = (*machine_sel).min(lib.machines.len().saturating_sub(1));
                    *lib_dirty = true;
                }
            });

            // Section 2: menu.
            ui.separator();
            for (tab, label) in [
                (MachineTab::Machine, "Machine"),
                (MachineTab::Speed, "Speed"),
                (MachineTab::EndEffector, "End-Effector"),
                (MachineTab::Scripts, "Scripts"),
            ] {
                ui.selectable_value(machine_tab, tab, label);
            }
        });

        egui::CentralPanel::default().show(root, |ui| {
            let Some(m) = lib.machines.get_mut(*machine_sel) else {
                ui.heading("Machines");
                ui.label("No machines yet. Use Add.");
                return;
            };
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut ch = false;
                match *machine_tab {
                    MachineTab::Machine => machine_section(ui, m, &mut ch),
                    MachineTab::Speed => speed_section(ui, m, &mut ch),
                    MachineTab::EndEffector => end_effector_section(ui, m, &mut ch),
                    MachineTab::Scripts => scripts_section(ui, m, &mut ch),
                }
                *lib_dirty |= ch;
            });
        });

        // Rename dialog.
        if let Some(name) = rename.as_mut() {
            let mut open = true;
            let mut ok = false;
            let mut cancel = false;
            let taken = lib
                .machines
                .iter()
                .enumerate()
                .any(|(i, m)| i != *machine_sel && m.name == name.trim());
            egui::Window::new("Rename machine")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.text_edit_singleline(name);
                    if taken {
                        ui.colored_label(Color32::from_rgb(0xff, 0xb0, 0x40), "Another machine has this name.");
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!name.trim().is_empty() && !taken, egui::Button::new("OK"))
                            .clicked()
                        {
                            ok = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if ok {
                if let (Some(n), Some(m)) = (rename.take(), lib.machines.get_mut(*machine_sel)) {
                    m.name = n.trim().to_string();
                    *lib_dirty = true;
                }
            } else if cancel || !open {
                *rename = None;
            }
        }
    }
}

fn unique_machine_name(machines: &[Machine], base: &str) -> String {
    if !machines.iter().any(|m| m.name == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|c| !machines.iter().any(|m| &m.name == c))
        .unwrap()
}

fn machine_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
    ui.heading("Machine");
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.strong("Machine construction");
            egui::Grid::new("m_build").num_columns(5).striped(true).show(ui, |ui| {
                for h in ["Axis", "Travel (mm)", "Bed size (mm)", "Moves the", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                let names = ["X", "Y", "Z"];
                let travel = [&mut m.travel_x, &mut m.travel_y, &mut m.travel_z];
                let bed = [&mut m.bed_x, &mut m.bed_y, &mut m.bed_z];
                for (i, (t, b)) in travel.into_iter().zip(bed).enumerate() {
                    ui.label(names[i]);
                    *ch |= ui.add(egui::DragValue::new(t).speed(1.0).range(1.0..=5000.0)).changed();
                    *ch |= ui.add(egui::DragValue::new(b).speed(1.0).range(1.0..=5000.0)).changed();
                    ui.horizontal(|ui| {
                        *ch |= ui.radio_value(&mut m.axes[i], AxisOwner::EndEffector, "End effector").changed();
                        *ch |= ui.radio_value(&mut m.axes[i], AxisOwner::Bed, "Bed").changed();
                    });
                    ui.label("");
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.strong("Bed & End-Effector Positions");
            let (bp, ep) = (m.bed_position(), m.effector_position());
            egui::Grid::new("m_fixed").num_columns(4).striped(true).show(ui, |ui| {
                for h in ["Axis", "End effector (mm)", "Bed (mm)", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(*name);
                    // Only the part that stays fixed on an axis has a position.
                    let fixed_z = i == 2;
                    if m.axes[i] == AxisOwner::Bed {
                        let mut v = ep[i];
                        let (lo, hi) = m.effector_range(i);
                        let r = ui.add_enabled(!fixed_z, egui::DragValue::new(&mut v).speed(0.5).range(lo..=hi));
                        if r.changed() {
                            m.effector_pos[i] = v;
                            *ch = true;
                        }
                    } else {
                        ui.label("-");
                    }
                    if m.axes[i] == AxisOwner::EndEffector {
                        let mut v = bp[i];
                        let (lo, hi) = m.bed_range(i);
                        let r = ui.add_enabled(!fixed_z, egui::DragValue::new(&mut v).speed(0.5).range(lo..=hi));
                        if r.changed() {
                            m.bed_pos[i] = v;
                            *ch = true;
                        }
                    } else {
                        ui.label("-");
                    }
                    if ui.small_button("Reset").clicked() {
                        m.effector_pos[i] = -1.0;
                        m.bed_pos[i] = -1.0;
                        *ch = true;
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.strong("Home Positions");
            let hm = m.home_position();
            egui::Grid::new("m_home").num_columns(3).striped(true).show(ui, |ui| {
                for h in ["Axis", "Homing (mm)", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(*name);
                    let mut hv = hm[i];
                    if ui.add(egui::DragValue::new(&mut hv).speed(0.5).range(-5000.0..=5000.0)).changed() {
                        m.home[i] = Some(hv);
                        *ch = true;
                    }
                    if ui.small_button("Reset").clicked() {
                        m.home[i] = None;
                        *ch = true;
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.strong("Z heights");
            egui::Grid::new("m_z").num_columns(2).show(ui, |ui| {
                let j = &mut m.job;
                ui.label("Use Z axis");
                *ch |= ui.checkbox(&mut j.use_z, "").changed();
                ui.end_row();
                ui.add_enabled_ui(j.use_z, |ui| ui.label("Safe Z"));
                *ch |= ui.add_enabled(j.use_z, egui::DragValue::new(&mut j.safe_z).speed(0.1).suffix(" mm")).changed();
                ui.end_row();
                ui.add_enabled_ui(j.use_z, |ui| ui.label("Work Z"));
                *ch |= ui.add_enabled(j.use_z, egui::DragValue::new(&mut j.dispense_z).speed(0.1).suffix(" mm")).changed();
                ui.end_row();
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            preview::Preview::show_schematic(ui, &*m, egui::vec2(400.0, 330.0));
        });
    });
}

fn speed_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
    ui.heading("Speed");
    ui.add_space(4.0);
    egui::Grid::new("m_speed").num_columns(2).show(ui, |ui| {
        ui.label("Work feed");
        *ch |= ui
            .add(egui::DragValue::new(&mut m.job.dispense_feed).speed(5.0).range(1.0..=20000.0).suffix(" mm/min"))
            .changed();
        ui.end_row();
        ui.label("Travel feed");
        *ch |= ui
            .add(egui::DragValue::new(&mut m.job.travel_feed).speed(10.0).range(1.0..=50000.0).suffix(" mm/min"))
            .changed();
        ui.end_row();
        ui.label("F on rapid moves");
        *ch |= ui.checkbox(&mut m.gcode.rapid_with_feed, "").changed();
        ui.end_row();
    });
}

fn end_effector_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
    ui.heading("End-Effector");
    ui.add_space(4.0);
    let e = &mut m.effector;
    egui::Grid::new("m_ee").num_columns(2).show(ui, |ui| {
        ui.label("Type");
        egui::ComboBox::from_id_salt("ee_kind")
            .selected_text(e.kind.label())
            .show_ui(ui, |ui| {
                for k in EffectorKind::ALL {
                    *ch |= ui.selectable_value(&mut e.kind, k, k.label()).changed();
                }
            });
        ui.end_row();
        ui.label("");
        ui.label(RichText::new("Used for the simulation drawing only.").small().weak());
        ui.end_row();
        ui.label("Control");
        ui.horizontal(|ui| {
            *ch |= ui.radio_value(&mut e.control, ControlType::Binary, "Binary").changed();
            *ch |= ui.radio_value(&mut e.control, ControlType::Pwm, "PWM").changed();
        });
        ui.end_row();
        match e.control {
            ControlType::Binary => {
                ui.label("On command");
                *ch |= ui.text_edit_singleline(&mut e.on_cmd).changed();
                ui.end_row();
                ui.label("Off command");
                *ch |= ui.text_edit_singleline(&mut e.off_cmd).changed();
                ui.end_row();
            }
            ControlType::Pwm => {
                ui.label("PWM command");
                *ch |= ui.text_edit_singleline(&mut e.pwm_cmd).changed();
                ui.end_row();
                ui.label("");
                ui.label(RichText::new("{v} is replaced by the PWM value.").small().weak());
                ui.end_row();
                ui.label("PWM min (off)");
                *ch |= ui.add(egui::DragValue::new(&mut e.pwm_min).speed(1.0)).changed();
                ui.end_row();
                ui.label("PWM max (on)");
                *ch |= ui.add(egui::DragValue::new(&mut e.pwm_max).speed(1.0)).changed();
                ui.end_row();
            }
        }
        ui.label("Start delay");
        *ch |= ui.add(egui::DragValue::new(&mut m.job.on_delay_ms).speed(5.0).suffix(" ms")).changed();
        ui.end_row();
        ui.label("Stop delay");
        *ch |= ui.add(egui::DragValue::new(&mut m.job.off_delay_ms).speed(5.0).suffix(" ms")).changed();
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "Output: on = \"{}\"   off = \"{}\"",
            m.effector.on_line(),
            m.effector.off_line()
        ))
        .small()
        .monospace(),
    );
}

fn scripts_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
    ui.heading("Scripts");
    ui.add_space(4.0);
    ui.label("Pre script (written at the start)");
    *ch |= ui
        .add(egui::TextEdit::multiline(&mut m.pre_script).code_editor().desired_rows(5).desired_width(f32::INFINITY))
        .changed();
    ui.add_space(6.0);
    ui.label("Post script (written at the end)");
    *ch |= ui
        .add(egui::TextEdit::multiline(&mut m.post_script).code_editor().desired_rows(4).desired_width(f32::INFINITY))
        .changed();
    ui.add_space(8.0);
    ui.separator();
    ui.strong("G-code format");
    let g = &mut m.gcode;
    egui::Grid::new("m_fmt").num_columns(2).show(ui, |ui| {
        ui.label("Dwell template");
        *ch |= ui.text_edit_singleline(&mut g.dwell).changed();
        ui.end_row();
        ui.label("");
        ui.label(RichText::new("{ms} = milliseconds, {s} = seconds").small().weak());
        ui.end_row();
        ui.label("Rapid command");
        *ch |= ui.text_edit_singleline(&mut g.rapid_cmd).changed();
        ui.end_row();
        ui.label("Linear command");
        *ch |= ui.text_edit_singleline(&mut g.linear_cmd).changed();
        ui.end_row();
        ui.label("Arc output (G2/G3)");
        *ch |= ui.checkbox(&mut g.arcs, "").changed();
        ui.end_row();
        ui.label("Clockwise arc");
        *ch |= ui.text_edit_singleline(&mut g.cw_cmd).changed();
        ui.end_row();
        ui.label("Counter-clockwise arc");
        *ch |= ui.text_edit_singleline(&mut g.ccw_cmd).changed();
        ui.end_row();
        ui.label("Decimals");
        *ch |= ui.add(egui::DragValue::new(&mut g.decimals).range(0..=6)).changed();
        ui.end_row();
        ui.label("Comments in output");
        *ch |= ui.checkbox(&mut g.comments, "").changed();
        ui.end_row();
    });
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let mut actions: Vec<Action> = vec![];

        self.top_bar(root, &mut actions);
        self.status_bar(root);
        match self.screen {
            Screen::Project => self.project_screen(root, &mut actions),
            Screen::Machines => self.machines_screen(root),
        }
        self.new_project_window(&ctx);
        self.layer_window(&ctx);

        for a in actions {
            self.run(a);
        }

        if self.lib_dirty && !ctx.input(|i| i.pointer.any_down()) {
            match self.lib.save() {
                Ok(()) => {}
                Err(e) => self.say(format!("Could not save library: {e}")),
            }
            self.lib_dirty = false;
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("CAMCraft"),
        ..Default::default()
    };
    eframe::run_native("CAMCraft", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
