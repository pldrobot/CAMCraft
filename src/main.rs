mod canvas;
mod dxf_import;
mod gcode;
mod icons;
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
    Gcode,
}

enum Action {
    NewProject,
    OpenProject,
    SaveProject(bool), // true = save as
    InsertDxf,
    ChangeLayers(usize),
    Generate,
    SaveGcode,
    CopyGcode,
}

struct PendingDxf {
    name: String,
    source: String,
    /// Part whose layers are being changed; None for a new part.
    replace: Option<usize>,
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
    /// Sequence step being dragged to a new place.
    seq_drag: Option<usize>,
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
            seq_drag: None,
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
                let source = path.to_string_lossy().to_string();
                if layers.len() <= 1 {
                    let names = layers.into_iter().map(|l| l.0).collect();
                    let r = dxf_import::import(&drawing);
                    self.add_part(name, source, names, r, None);
                } else {
                    self.pending_dxf = Some(PendingDxf { name, source, replace: None, drawing, layers });
                }
            }
            Action::ChangeLayers(pi) => {
                let Some(part) = self.project.as_ref().and_then(|p| p.parts.get(pi)) else { return };
                if part.source.is_empty() {
                    self.say("This part has no DXF source, so its layers cannot be changed.");
                    return;
                }
                let (name, source, current) = (part.name.clone(), part.source.clone(), part.layers.clone());
                let drawing = match dxf_import::load_drawing(std::path::Path::new(&source)) {
                    Ok(d) => d,
                    Err(e) => {
                        self.say(format!("Cannot reload {source}: {e}"));
                        return;
                    }
                };
                let layers: Vec<(String, usize, bool)> = dxf_import::layers(&drawing)
                    .into_iter()
                    .map(|(n, c)| {
                        let on = current.is_empty() || current.contains(&n);
                        (n, c, on)
                    })
                    .collect();
                self.pending_dxf = Some(PendingDxf { name, source, replace: Some(pi), drawing, layers });
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

    fn add_part(&mut self, name: String, source: String, layers: Vec<String>, r: dxf_import::ImportResult, replace: Option<usize>) {
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
        if let Some(pi) = replace.filter(|&i| i < project.parts.len()) {
            // Same DXF origin and position, new set of paths. The old
            // sequence refers to paths that no longer exist.
            let part = &mut project.parts[pi];
            part.paths = r.paths;
            part.layers = layers;
            project.clear_part_sequence(pi);
            self.say(format!("Layers of {name} changed: {n} paths."));
            return;
        }
        project.parts.push(Part {
            name: name.clone(),
            source,
            layers,
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
                ui.label(format!("{}: choose the layers to use", p.name));
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
                let names: Vec<String> = p.layers.iter().filter(|l| l.2).map(|l| l.0.clone()).collect();
                self.add_part(p.name, p.source, names, r, p.replace);
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

        let Self { lib, project, canvas, gcode, warnings, tab, preview, seq_drag, .. } = self;
        let project = project.as_mut().unwrap();
        let generated = preview.has_path();
        if matches!(*tab, ProjectTab::Preview | ProjectTab::Gcode) && !generated {
            *tab = ProjectTab::Prepare;
        }

        egui::Panel::top("project_tabs").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(tab, ProjectTab::Prepare, "Prepare");
                ui.selectable_value(tab, ProjectTab::PathSelection, "Path selection");
                ui.add_enabled_ui(generated, |ui| {
                    ui.selectable_value(tab, ProjectTab::Preview, "Preview")
                        .on_disabled_hover_text("Generate G-code first.");
                    ui.selectable_value(tab, ProjectTab::Gcode, "G-code")
                        .on_disabled_hover_text("Generate G-code first.");
                });
            });
        });

        match *tab {
            ProjectTab::Prepare => canvas.tool = Tool::Move,
            ProjectTab::PathSelection => canvas.tool = Tool::Sequence,
            ProjectTab::Preview | ProjectTab::Gcode => {}
        }

        egui::Panel::left("left").resizable(false).exact_size(330.0).show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                match *tab {
                    ProjectTab::Prepare => {
                        let mut reload = false;
                        ui.horizontal(|ui| {
                            ui.heading("Machine");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                reload = icons::icon_button(
                                    ui,
                                    icons::Icon::Reload,
                                    "Reload machine settings. The project keeps a copy of the machine and its speeds. Use this after editing the machine in Machines.",
                                )
                                .clicked();
                            });
                        });
                        if reload {
                            if let Some(m) = lib.machines.iter().find(|m| m.name == project.machine.name) {
                                project.job = m.job.clone();
                                project.machine = m.clone();
                            }
                        }
                        let m = &project.machine;
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
                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.heading("Parts");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Insert DXF...").clicked() {
                                    actions.push(Action::InsertDxf);
                                }
                            });
                        });
                        let mut remove: Option<usize> = None;
                        if project.parts.is_empty() {
                            ui.label(RichText::new("No DXF inserted yet.").small().weak());
                        } else {
                            egui::Grid::new("parts_table").num_columns(5).striped(true).spacing([6.0, 4.0]).show(ui, |ui| {
                                ui.label(RichText::new("DXF").small().weak());
                                ui.label(RichText::new("X mm").small().weak());
                                ui.label(RichText::new("Y mm").small().weak());
                                ui.label("");
                                ui.label("");
                                ui.end_row();
                                for pi in 0..project.parts.len() {
                                    let selected = canvas.selected_part == Some(pi);
                                    let part = &mut project.parts[pi];
                                    let name = RichText::new(part.name.as_str());
                                    if ui
                                        .add(egui::Button::selectable(selected, name).truncate())
                                        .on_hover_text(format!("{}\n{} paths", part.name, part.paths.len()))
                                        .clicked()
                                    {
                                        canvas.selected_part = Some(pi);
                                    }
                                    // Lower-left corner of the drawing on the bed, not the
                                    // DXF origin, which can be far from the geometry.
                                    let mut c = part.min_corner();
                                    let before = c;
                                    ui.add(egui::DragValue::new(&mut c[0]).speed(0.1).fixed_decimals(2));
                                    ui.add(egui::DragValue::new(&mut c[1]).speed(0.1).fixed_decimals(2));
                                    part.offset[0] += c[0] - before[0];
                                    part.offset[1] += c[1] - before[1];
                                    if icons::icon_button(ui, icons::Icon::Layers, "Change layers").clicked() {
                                        actions.push(Action::ChangeLayers(pi));
                                    }
                                    if icons::icon_button(ui, icons::Icon::Remove, "Remove DXF").clicked() {
                                        remove = Some(pi);
                                    }
                                    ui.end_row();
                                }
                            });
                        }
                        if let Some(i) = remove {
                            project.remove_part(i);
                            canvas.selected_part = None;
                        }
                    }
                    ProjectTab::PathSelection => {
                        ui.heading("Job settings");
                        job_settings_grid(ui, &mut project.job);
                        ui.separator();
                        sequence_ui(ui, project, &mut canvas.sel_steps, seq_drag);
                        ui.separator();
                        ui.add_space(4.0);
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
                    ProjectTab::Gcode => {
                        ui.heading("G-code");
                        ui.label(format!("{} lines", gcode.lines().count()));
                        if ui.button("Save...").clicked() {
                            actions.push(Action::SaveGcode);
                        }
                        if ui.button("Copy to clipboard").clicked() {
                            ui.ctx().copy_text(gcode.clone());
                            actions.push(Action::CopyGcode);
                        }
                        for w in warnings.iter() {
                            ui.colored_label(Color32::from_rgb(0xff, 0xb0, 0x40), format!("! {w}"));
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
                        if *tab == ProjectTab::Prepare {
                            ui.checkbox(&mut canvas.snap, "Snap")
                                .on_hover_text("Snap to bed corners, the origin and points of other DXFs while moving. Hold Alt to ignore.");
                        }
                        let hint = if *tab == ProjectTab::Prepare {
                            "Drag a DXF to place it (Alt: no snap).   Wheel: zoom   Right or middle drag: pan"
                        } else {
                            "DXFs are locked. Click paths to build the order.   Wheel: zoom   Right or middle drag: pan"
                        };
                        ui.label(RichText::new(hint).small().weak());
                    });
                    let steps = project.sequence.len();
                    canvas.show(ui, project);
                    if project.sequence.len() != steps {
                        canvas.sel_steps.clear();
                    }
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
                ProjectTab::Gcode => {
                    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut gcode.as_str())
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
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


fn job_settings_grid(ui: &mut egui::Ui, j: &mut JobSettings) {
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
}

enum AddStep {
    AllPaths,
    Home,
    Goto,
    Pause,
    Clear,
}

fn step_desc(project: &Project, item: &SeqItem) -> String {
    match item.kind {
        StepKind::Path => {
            let name = project.parts.get(item.part).map(|p| p.name.as_str()).unwrap_or("?");
            format!(
                "{} #{}{}{}",
                name,
                item.path + 1,
                if item.reverse { "  (reversed)" } else { "" },
                if item.over.is_empty() { "" } else { "  *" }
            )
        }
        StepKind::Home => "Home".to_string(),
        StepKind::Goto { x, y, z } => match z {
            Some(z) => format!("Go to  X{x:.2}  Y{y:.2}  Z{z:.2}"),
            None => format!("Go to  X{x:.2}  Y{y:.2}"),
        },
        StepKind::Pause { ms } => format!("Pause  {ms} ms"),
    }
}

/// A row with a tick to override a job value and the value itself.
fn override_row(ui: &mut egui::Ui, label: &str, ov: &mut Option<f64>, job: f64, speed: f64, suffix: &str, enabled: bool) {
    ui.add_enabled_ui(enabled, |ui| {
        let mut on = ov.is_some();
        if ui.checkbox(&mut on, label).changed() {
            *ov = if on { Some(job) } else { None };
        }
    });
    match ov {
        Some(v) if enabled => {
            ui.add(egui::DragValue::new(v).speed(speed).suffix(suffix));
        }
        _ => {
            ui.label(RichText::new(format!("{job}{suffix}")).weak());
        }
    }
    ui.end_row();
}

fn step_settings(ui: &mut egui::Ui, item: &mut SeqItem, job: &JobSettings) {
    ui.set_min_width(250.0);
    match &mut item.kind {
        StepKind::Path => {
            ui.label(RichText::new("Tick a value to change it for this path. Others follow Job settings.").small().weak());
            let o = &mut item.over;
            egui::Grid::new("step_over").num_columns(2).show(ui, |ui| {
                override_row(ui, "Work feed", &mut o.dispense_feed, job.dispense_feed, 5.0, " mm/min", true);
                override_row(ui, "Travel feed", &mut o.travel_feed, job.travel_feed, 10.0, " mm/min", true);
                override_row(ui, "Safe Z", &mut o.safe_z, job.safe_z, 0.1, " mm", job.use_z);
                override_row(ui, "Work Z", &mut o.dispense_z, job.dispense_z, 0.1, " mm", job.use_z);
                let mut on_ms = o.on_delay_ms.map(|v| v as f64);
                override_row(ui, "Start delay", &mut on_ms, job.on_delay_ms as f64, 5.0, " ms", true);
                o.on_delay_ms = on_ms.map(|v| v.max(0.0) as u32);
                let mut off_ms = o.off_delay_ms.map(|v| v as f64);
                override_row(ui, "Stop delay", &mut off_ms, job.off_delay_ms as f64, 5.0, " ms", true);
                o.off_delay_ms = off_ms.map(|v| v.max(0.0) as u32);
            });
            if ui.add_enabled(!item.over.is_empty(), egui::Button::new("Use job settings")).clicked() {
                item.over = JobOverride::default();
            }
        }
        StepKind::Goto { x, y, z } => {
            egui::Grid::new("step_goto").num_columns(2).show(ui, |ui| {
                ui.label("X");
                ui.add(egui::DragValue::new(x).speed(0.1).suffix(" mm"));
                ui.end_row();
                ui.label("Y");
                ui.add(egui::DragValue::new(y).speed(0.1).suffix(" mm"));
                ui.end_row();
                let mut use_z = z.is_some();
                if ui.checkbox(&mut use_z, "Z").changed() {
                    *z = if use_z { Some(job.safe_z) } else { None };
                }
                match z {
                    Some(v) => {
                        ui.add(egui::DragValue::new(v).speed(0.1).suffix(" mm"));
                    }
                    None => {
                        ui.label(RichText::new("stays at safe Z").weak());
                    }
                }
                ui.end_row();
            });
        }
        StepKind::Pause { ms } => {
            ui.horizontal(|ui| {
                ui.label("Wait");
                ui.add(egui::DragValue::new(ms).speed(10.0).range(0..=600_000).suffix(" ms"));
            });
        }
        StepKind::Home => {
            ui.label("Goes to the home position of the machine.");
        }
    }
}

/// Move the selected steps so the block starts at `ins` (an index into the
/// list before the move).
fn move_steps(project: &mut Project, sel: &mut Vec<usize>, ins: usize) {
    let mut idx = sel.clone();
    idx.sort_unstable();
    idx.dedup();
    idx.retain(|&i| i < project.sequence.len());
    if idx.is_empty() {
        return;
    }
    let before = idx.iter().filter(|&&i| i < ins).count();
    let items: Vec<SeqItem> = idx.iter().map(|&i| project.sequence[i]).collect();
    for &i in idx.iter().rev() {
        project.sequence.remove(i);
    }
    let at = (ins - before).min(project.sequence.len());
    for (k, it) in items.iter().enumerate() {
        project.sequence.insert(at + k, *it);
    }
    *sel = (at..at + items.len()).collect();
}

fn sequence_ui(ui: &mut egui::Ui, project: &mut Project, sel: &mut Vec<usize>, drag: &mut Option<usize>) {
    sel.retain(|&i| i < project.sequence.len());
    let mut add: Option<AddStep> = None;
    let mut delete = false;

    ui.horizontal(|ui| {
        ui.heading("Sequence");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Right to left: delete first so it sits right of add.
            if icons::icon_button_ex(ui, icons::Icon::Trash, "Remove the selected steps (Delete)", false, !sel.is_empty()).clicked() {
                delete = true;
            }
            let plus = icons::icon_button(ui, icons::Icon::Plus, "Add a step");
            egui::Popup::menu(&plus).show(|ui| {
                if ui.button("All paths").clicked() {
                    add = Some(AddStep::AllPaths);
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
    ui.label(
        RichText::new("Click paths on the bed to add them. Drag rows to reorder. Ctrl or Shift click selects several.")
            .small()
            .weak(),
    );

    // Delete key, when no text field is being edited.
    if !sel.is_empty() && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        delete = true;
    }

    // Adding steps.
    if let Some(a) = add {
        let at = sel.iter().max().map_or(project.sequence.len(), |m| m + 1).min(project.sequence.len());
        let home = project.machine.home_position();
        let new: Vec<SeqItem> = match a {
            AddStep::AllPaths => {
                let mut v = vec![];
                for (pi, part) in project.parts.iter().enumerate() {
                    for ci in 0..part.paths.len() {
                        if project.seq_index(pi, ci).is_none() {
                            v.push(SeqItem::path(pi, ci));
                        }
                    }
                }
                v
            }
            AddStep::Home => vec![SeqItem::other(StepKind::Home)],
            AddStep::Goto => vec![SeqItem::other(StepKind::Goto { x: home[0], y: home[1], z: None })],
            AddStep::Pause => vec![SeqItem::other(StepKind::Pause { ms: 1000 })],
            AddStep::Clear => {
                project.sequence.clear();
                sel.clear();
                vec![]
            }
        };
        let count = new.len();
        for (k, it) in new.into_iter().enumerate() {
            project.sequence.insert(at + k, it);
        }
        if count > 0 {
            *sel = (at..at + count).collect();
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

    // The list.
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
            let w = (ui.available_width() - if item.is_path() { 54.0 } else { 28.0 }).max(40.0);
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
                    let (lo, hi) = (anchor.min(i), anchor.max(i));
                    *sel = (lo..=hi).collect();
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
            // Grip dots and text.
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
            if item.kind != StepKind::Home {
                let b = icons::icon_button_ex(ui, icons::Icon::Settings, "Step settings", !item.over.is_empty(), true);
                egui::Popup::from_toggle_button_response(&b)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| step_settings(ui, &mut project.sequence[i], &job));
            }
        });
        // Row background spans the whole width.
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

    // Drag and drop reordering.
    if let Some(_from) = *drag {
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
        ui.label("F on every G0-G3 move");
        *ch |= ui.checkbox(&mut m.gcode.feed_every_move, "").changed();
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
