mod canvas;
mod dxf_import;
mod gcode;
mod model;
mod theme;

use canvas::{Canvas, Tool};
use eframe::egui;
use egui::{Color32, RichText};
use model::*;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(PartialEq, Clone, Copy)]
enum Screen {
    Project,
    Machines,
    Flavors,
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
    flavor_sel: usize,
    machine_draft: Machine,
    pending_dxf: Option<PendingDxf>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let lib = Library::load();
        let flavor = lib.flavors.first().map(|f| f.name.clone()).unwrap_or_default();
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
            flavor_sel: 0,
            pending_dxf: None,
            machine_draft: Machine {
                name: "New machine".into(),
                size_x: 150.0,
                size_y: 100.0,
                size_z: 20.0,
                flavor,
            },
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
                let Some(project) = &self.project else { return };
                let Some(flavor) = self.lib.flavor(&project.flavor).cloned() else {
                    self.say(format!("Flavor '{}' not found in flavor management.", project.flavor));
                    return;
                };
                let r = gcode::generate(project, &flavor);
                let lines = r.text.lines().count();
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
            // Put the lower-left corner of the drawing 5 mm inside the bed.
            offset: [5.0 - minx, 5.0 - miny],
        });
        let big = w > project.machine.size_x || h > project.machine.size_y;
        self.canvas.selected_part = Some(project.parts.len() - 1);
        self.canvas.tool = Tool::Move;
        self.say(format!("Inserted {name}: {n} paths, {:.1} x {:.1} mm (assumed mm)", w, h));
        for s in r.skipped {
            self.say(s);
        }
        if big {
            self.say("Warning: drawing is larger than the bed.");
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
                ui.selectable_value(&mut self.screen, Screen::Flavors, "Flavors");
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
                                    format!("{}  ({} x {} x {} mm, {})", m.name, m.size_x, m.size_y, m.size_z, m.flavor),
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
            let flavor = self
                .lib
                .flavor(&m.flavor)
                .or_else(|| self.lib.flavors.first())
                .cloned()
                .unwrap_or_else(Flavor::grbl);
            let mut p = Project::new(m.clone(), &flavor);
            p.flavor = flavor.name.clone();
            self.project = Some(p);
            self.project_path = None;
            self.canvas = Canvas::default();
            self.gcode.clear();
            self.warnings.clear();
            self.screen = Screen::Project;
            self.show_new = false;
            self.say(format!("New project on {} ({} x {} mm)", m.name, m.size_x, m.size_y));
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
                    ui.label("Glue path programming for dispensing machines");
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

        let Self { lib, project, canvas, gcode, gcode_open, warnings, .. } = self;
        let project = project.as_mut().unwrap();

        egui::Panel::left("left").default_size(310.0).show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Machine");
                ui.label(format!(
                    "{}   {} x {} x {} mm",
                    project.machine.name, project.machine.size_x, project.machine.size_y, project.machine.size_z
                ));
                ui.horizontal(|ui| {
                    ui.label("Flavor");
                    egui::ComboBox::from_id_salt("proj_flavor")
                        .selected_text(project.flavor.clone())
                        .show_ui(ui, |ui| {
                            for f in &lib.flavors {
                                ui.selectable_value(&mut project.flavor, f.name.clone(), &f.name);
                            }
                        });
                });
                ui.separator();

                ui.heading("Parts");
                if ui.button("Insert DXF...").clicked() {
                    actions.push(Action::InsertDxf);
                }
                let mut remove: Option<usize> = None;
                for pi in 0..project.parts.len() {
                    let selected = canvas.selected_part == Some(pi);
                    ui.horizontal(|ui| {
                        let name = format!("{}  ({} paths)", project.parts[pi].name, project.parts[pi].paths.len());
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
                    RichText::new("Use the Sequence tool and click paths on the bed. Click again to remove.")
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

                ui.heading("Job settings");
                let j = &mut project.job;
                egui::Grid::new("job").num_columns(2).show(ui, |ui| {
                    ui.label("Dispense feed");
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
                    ui.add_enabled_ui(j.use_z, |ui| ui.label("Dispense Z"));
                    ui.add_enabled(j.use_z, egui::DragValue::new(&mut j.dispense_z).speed(0.1).suffix(" mm"));
                    ui.end_row();
                    ui.label("Pre-flow wait");
                    ui.add(egui::DragValue::new(&mut j.pre_flow_ms).speed(5.0).suffix(" ms"));
                    ui.end_row();
                    ui.label("Post-flow wait");
                    ui.add(egui::DragValue::new(&mut j.post_flow_ms).speed(5.0).suffix(" ms"));
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
            });
        });

        if !gcode.is_empty() && *gcode_open {
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

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.selectable_value(&mut canvas.tool, Tool::Move, "Move part");
                    ui.selectable_value(&mut canvas.tool, Tool::Sequence, "Sequence paths");
                    if ui.button("Zoom fit").clicked() {
                        canvas.zoom_fit();
                    }
                    ui.label(
                        RichText::new("Wheel: zoom   Right or middle drag: pan")
                            .small()
                            .weak(),
                    );
                });
                canvas.show(ui, project);
            });
    }

    fn machines_screen(&mut self, root: &mut egui::Ui) {
        let Self { lib, lib_dirty, machine_draft, .. } = self;
        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("Machine management");
            ui.add_space(6.0);
            let flavor_names: Vec<String> = lib.flavors.iter().map(|f| f.name.clone()).collect();
            let mut delete: Option<usize> = None;
            egui::Grid::new("machines").striped(true).num_columns(6).show(ui, |ui| {
                for h in ["Name", "X (mm)", "Y (mm)", "Z (mm)", "Flavor", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, m) in lib.machines.iter_mut().enumerate() {
                    *lib_dirty |= ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(160.0)).changed();
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut m.size_x).speed(1.0).range(1.0..=5000.0)).changed();
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut m.size_y).speed(1.0).range(1.0..=5000.0)).changed();
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut m.size_z).speed(1.0).range(1.0..=2000.0)).changed();
                    egui::ComboBox::from_id_salt(("mflavor", i))
                        .selected_text(m.flavor.clone())
                        .show_ui(ui, |ui| {
                            for f in &flavor_names {
                                *lib_dirty |= ui.selectable_value(&mut m.flavor, f.clone(), f).changed();
                            }
                        });
                    if ui.button("Delete").clicked() {
                        delete = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = delete {
                lib.machines.remove(i);
                *lib_dirty = true;
            }
            ui.add_space(12.0);
            ui.separator();
            ui.strong("Add machine");
            egui::Grid::new("add_machine").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut machine_draft.name);
                ui.end_row();
                ui.label("Size X (mm)");
                ui.add(egui::DragValue::new(&mut machine_draft.size_x).speed(1.0).range(1.0..=5000.0));
                ui.end_row();
                ui.label("Size Y (mm)");
                ui.add(egui::DragValue::new(&mut machine_draft.size_y).speed(1.0).range(1.0..=5000.0));
                ui.end_row();
                ui.label("Size Z (mm)");
                ui.add(egui::DragValue::new(&mut machine_draft.size_z).speed(1.0).range(1.0..=2000.0));
                ui.end_row();
                ui.label("Flavor");
                egui::ComboBox::from_id_salt("draft_flavor")
                    .selected_text(machine_draft.flavor.clone())
                    .show_ui(ui, |ui| {
                        for f in &flavor_names {
                            ui.selectable_value(&mut machine_draft.flavor, f.clone(), f);
                        }
                    });
                ui.end_row();
            });
            if ui.button("Add machine").clicked() && !machine_draft.name.trim().is_empty() {
                lib.machines.push(machine_draft.clone());
                *lib_dirty = true;
            }
        });
    }

    fn flavors_screen(&mut self, root: &mut egui::Ui) {
        let Self { lib, lib_dirty, flavor_sel, log, .. } = self;
        *flavor_sel = (*flavor_sel).min(lib.flavors.len().saturating_sub(1));

        egui::Panel::left("flavor_list").default_size(200.0).show(root, |ui| {
            ui.heading("Flavors");
            for (i, f) in lib.flavors.iter().enumerate() {
                ui.selectable_value(flavor_sel, i, &f.name);
            }
            ui.separator();
            if ui.button("New from selected").clicked() {
                let mut f = lib.flavors[*flavor_sel].clone();
                f.name = format!("{} copy", f.name);
                lib.flavors.push(f);
                *flavor_sel = lib.flavors.len() - 1;
                *lib_dirty = true;
            }
            if ui.button("New GRBL preset").clicked() {
                let mut f = Flavor::grbl();
                f.name = unique_name(&lib.flavors, "GRBL");
                lib.flavors.push(f);
                *flavor_sel = lib.flavors.len() - 1;
                *lib_dirty = true;
            }
            if ui.button("New Marlin preset").clicked() {
                let mut f = Flavor::marlin();
                f.name = unique_name(&lib.flavors, "Marlin");
                lib.flavors.push(f);
                *flavor_sel = lib.flavors.len() - 1;
                *lib_dirty = true;
            }
            let used = lib.machines.iter().any(|m| m.flavor == lib.flavors[*flavor_sel].name);
            let can_delete = lib.flavors.len() > 1 && !used;
            let del = ui
                .add_enabled(can_delete, egui::Button::new("Delete selected"))
                .on_disabled_hover_text("Keep at least one flavor, and not one used by a machine.");
            if del.clicked() {
                lib.flavors.remove(*flavor_sel);
                *lib_dirty = true;
            }
        });

        egui::CentralPanel::default().show(root, |ui| {
            if lib.flavors.is_empty() {
                return;
            }
            let Library { machines, flavors } = lib;
            let f = &mut flavors[*flavor_sel];
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Flavor editor");
                ui.label(
                    RichText::new("A flavor is plain text. Change any command to match your firmware.")
                        .small()
                        .weak(),
                );
                let old_name = f.name.clone();
                ui.horizontal(|ui| {
                    ui.label("Name");
                    if ui.text_edit_singleline(&mut f.name).changed() {
                        for m in machines.iter_mut().filter(|m| m.flavor == old_name) {
                            m.flavor = f.name.clone();
                        }
                        *lib_dirty = true;
                    }
                });
                ui.label("Header lines");
                *lib_dirty |= ui
                    .add(egui::TextEdit::multiline(&mut f.header).code_editor().desired_rows(3).desired_width(f32::INFINITY))
                    .changed();
                ui.label("Footer lines");
                *lib_dirty |= ui
                    .add(egui::TextEdit::multiline(&mut f.footer).code_editor().desired_rows(2).desired_width(f32::INFINITY))
                    .changed();
                egui::Grid::new("fl_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Valve on");
                    *lib_dirty |= ui.text_edit_singleline(&mut f.valve_on).changed();
                    ui.end_row();
                    ui.label("Valve off");
                    *lib_dirty |= ui.text_edit_singleline(&mut f.valve_off).changed();
                    ui.end_row();
                    ui.label("Dwell template");
                    *lib_dirty |= ui.text_edit_singleline(&mut f.dwell).changed();
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new("{ms} = milliseconds, {s} = seconds").small().weak());
                    ui.end_row();
                    ui.label("Rapid command");
                    *lib_dirty |= ui.text_edit_singleline(&mut f.rapid_cmd).changed();
                    ui.end_row();
                    ui.label("Linear command");
                    *lib_dirty |= ui.text_edit_singleline(&mut f.linear_cmd).changed();
                    ui.end_row();
                    ui.label("Arc output (G2/G3)");
                    *lib_dirty |= ui.checkbox(&mut f.arcs, "").changed();
                    ui.end_row();
                    ui.label("Clockwise arc");
                    *lib_dirty |= ui.text_edit_singleline(&mut f.cw_cmd).changed();
                    ui.end_row();
                    ui.label("Counter-clockwise arc");
                    *lib_dirty |= ui.text_edit_singleline(&mut f.ccw_cmd).changed();
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new("Arcs use I/J (center relative to start). Marlin needs ARC_SUPPORT.").small().weak());
                    ui.end_row();
                    ui.label("F on rapid moves");
                    *lib_dirty |= ui.checkbox(&mut f.rapid_with_feed, "").changed();
                    ui.end_row();
                    ui.label("Decimals");
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut f.decimals).range(0..=6)).changed();
                    ui.end_row();
                    ui.label("Comments in output");
                    *lib_dirty |= ui.checkbox(&mut f.comments, "").changed();
                    ui.end_row();
                });
                ui.separator();
                ui.strong("Default job settings for new projects");
                let j = &mut f.defaults;
                egui::Grid::new("fl_job").num_columns(2).show(ui, |ui| {
                    ui.label("Dispense feed");
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut j.dispense_feed).speed(5.0).suffix(" mm/min")).changed();
                    ui.end_row();
                    ui.label("Travel feed");
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut j.travel_feed).speed(10.0).suffix(" mm/min")).changed();
                    ui.end_row();
                    ui.label("Use Z axis");
                    *lib_dirty |= ui.checkbox(&mut j.use_z, "").changed();
                    ui.end_row();
                    ui.label("Safe Z");
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut j.safe_z).speed(0.1).suffix(" mm")).changed();
                    ui.end_row();
                    ui.label("Dispense Z");
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut j.dispense_z).speed(0.1).suffix(" mm")).changed();
                    ui.end_row();
                    ui.label("Pre-flow wait");
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut j.pre_flow_ms).speed(5.0).suffix(" ms")).changed();
                    ui.end_row();
                    ui.label("Post-flow wait");
                    *lib_dirty |= ui.add(egui::DragValue::new(&mut j.post_flow_ms).speed(5.0).suffix(" ms")).changed();
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Presets are untested on real machines. Check every command against your firmware before running.")
                        .small()
                        .color(Color32::from_rgb(0xff, 0xb0, 0x40)),
                );
            });
            let _ = log;
        });
    }
}

fn unique_name(flavors: &[Flavor], base: &str) -> String {
    if !flavors.iter().any(|f| f.name == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|c| !flavors.iter().any(|f| &f.name == c))
        .unwrap()
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
            Screen::Flavors => self.flavors_screen(root),
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
