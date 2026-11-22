use crate::common::machine::*;
use crate::common::machine_ui::MachineTab;
use crate::common::{preview::Preview, theme};
use crate::p2d::canvas::Canvas;
use crate::p2d::model::Project;
use crate::p2d::{dxf_import, gcode};
use crate::p3d;
use eframe::egui;
use egui::{Color32, RichText};
use std::path::PathBuf;

mod recent;
mod welcome;

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("CAMCraft")
            .with_maximized(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icons/png/camcraft-256.png"))
                    .expect("icon png"),
            ),
        ..Default::default()
    };
    eframe::run_native("CAMCraft", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}


#[derive(PartialEq, Clone, Copy)]
pub(crate) enum NewKind {
    D2,
    D3,
}

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Screen {
    Project,
    Machines,
}


#[derive(PartialEq, Clone, Copy)]
pub(crate) enum ProjectTab {
    Prepare,
    PathSelection,
    Preview,
    Gcode,
}

pub(crate) enum Action {
    NewProject(NewKind),
    OpenProject,
    OpenPath(PathBuf),
    SaveProject(bool), // true = save as
    InsertDxf,
    ChangeLayers(usize),
    Generate,
    SaveGcode,
    CopyGcode,
}

pub(crate) struct PendingDxf {
    pub(crate) name: String,
    pub(crate) source: String,
    /// Part whose layers are being changed; None for a new part.
    pub(crate) replace: Option<usize>,
    pub(crate) drawing: dxf::Drawing,
    /// Layer name, entity count, selected for import.
    pub(crate) layers: Vec<(String, usize, bool)>,
}

/// How long the splash screen stays up.
const SPLASH_SECS: f32 = 2.0;

pub(crate) struct App {
    /// Splash image and the time it was first drawn; None once it is gone.
    pub(crate) splash: Option<(egui::TextureHandle, Option<std::time::Instant>)>,
    pub(crate) lib: Library,
    pub(crate) lib_dirty: bool,
    pub(crate) screen: Screen,
    pub(crate) project: Option<Project>,
    pub(crate) project_path: Option<PathBuf>,
    pub(crate) canvas: Canvas,
    pub(crate) gcode: String,
    pub(crate) warnings: Vec<String>,
    pub(crate) log: Vec<String>,
    pub(crate) machine_tab: MachineTab,
    pub(crate) rename: Option<String>,
    pub(crate) machine_sel: usize,
    pub(crate) tab: ProjectTab,
    pub(crate) preview: Preview,
    pub(crate) pending_dxf: Option<PendingDxf>,
    /// Sequence step being dragged to a new place.
    pub(crate) seq_drag: Option<usize>,
    /// The open 3D project. Only one of `project` and `p3` is Some.
    pub(crate) p3: Option<p3d::ui::P3App>,
    pub(crate) recent: recent::Recent,
    /// The welcome window was closed with its X.
    pub(crate) welcome_closed: bool,
}


impl App {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let lib = Library::load();
        let splash = image::load_from_memory(include_bytes!("../../assets/splash/camcraft-splash-1280x720.png"))
            .ok()
            .map(|im| {
                let im = im.to_rgba8();
                let size = [im.width() as usize, im.height() as usize];
                let img = egui::ColorImage::from_rgba_unmultiplied(size, im.as_raw());
                (cc.egui_ctx.load_texture("splash", img, egui::TextureOptions::LINEAR), None)
            });
        Self {
            splash,
            lib,
            lib_dirty: false,
            screen: Screen::Project,
            project: None,
            project_path: None,
            canvas: Canvas::default(),
            gcode: String::new(),
            warnings: vec![],
            log: vec!["CAMCraft ready.".into()],
            machine_tab: MachineTab::Machine,
            rename: None,
            machine_sel: 0,
            tab: ProjectTab::Prepare,
            preview: Preview::default(),
            pending_dxf: None,
            seq_drag: None,
            p3: None,
            recent: recent::Recent::load(),
            welcome_closed: false,
        }
    }

    pub(crate) fn say(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    // ---------- actions ----------

    pub(crate) fn run(&mut self, a: Action) {
        match a {
            Action::NewProject(kind) => self.new_project(kind),
            Action::OpenProject => {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("CAMCraft project", &["camcraft", "camcraft3d"])
                    .add_filter("2D project", &["camcraft"])
                    .add_filter("3D project", &["camcraft3d"])
                    .pick_file()
                else {
                    return;
                };
                self.run(Action::OpenPath(path));
            }
            Action::OpenPath(path) => {
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("camcraft3d")) {
                    match p3d::ui::P3App::open(path.clone()) {
                        Ok(app) => {
                            self.recent.add(path.clone());
                            self.p3 = Some(app);
                            self.project = None;
                            self.project_path = None;
                            self.screen = Screen::Project;
                        }
                        Err(e) => self.say(format!("Open failed: {e}")),
                    }
                    return;
                }
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<Project>(&s).map_err(|e| e.to_string()))
                {
                    Ok(p) => {
                        self.recent.add(path.clone());
                        self.p3 = None;
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
                if let Some(p3) = &mut self.p3 {
                    p3.save(as_new);
                    if let Some(path) = p3.path.clone() {
                        self.recent.add(path);
                    }
                    return;
                }
                let Some(project) = &self.project else { return };
                let path = if as_new || self.project_path.is_none() {
                    rfd::FileDialog::new()
                        .add_filter("CAMCraft 2D project", &["camcraft"])
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
                        self.recent.add(path.clone());
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

    pub(crate) fn top_bar(&mut self, root: &mut egui::Ui, actions: &mut Vec<Action>) {
        egui::Panel::top("top").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New 2D Project").clicked() {
                        actions.push(Action::NewProject(NewKind::D2));
                        ui.close();
                    }
                    if ui.button("New 3D Project").clicked() {
                        actions.push(Action::NewProject(NewKind::D3));
                        ui.close();
                    }
                    if ui.button("Open Project...").clicked() {
                        actions.push(Action::OpenProject);
                        ui.close();
                    }
                    ui.add_enabled_ui(!self.recent.0.is_empty(), |ui| {
                        ui.menu_button("Open Recent", |ui| {
                            for path in self.recent.0.clone() {
                                let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                                if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                                    actions.push(Action::OpenPath(path));
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.button("Clear list").clicked() {
                                self.recent.clear();
                                ui.close();
                            }
                        });
                    });
                    ui.add_enabled_ui(self.project.is_some() || self.p3.is_some(), |ui| {
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

    pub(crate) fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                let cursor = match &self.p3 {
                    Some(p3) => p3.cursor,
                    None => self.canvas.cursor_world,
                };
                let coords = match (self.screen, cursor) {
                    (Screen::Project, Some(p)) => format!("X {:8.2}   Y {:8.2}", p[0], p[1]),
                    _ => "X          Y".to_string(),
                };
                ui.label(RichText::new(coords).monospace().color(Color32::from_rgb(0x4f, 0xb6, 0xff)));
                ui.separator();
                ui.label(RichText::new(format!("> {}", self.log.last().cloned().unwrap_or_default())).monospace());
            });
        });
    }

    /// Start an empty project on the first machine. The machine is changed in the Prepare panel.
    pub(crate) fn new_project(&mut self, kind: NewKind) {
        let Some(m) = self.lib.machines.first().cloned() else {
            self.say("No machines yet. Add one first.");
            self.screen = Screen::Machines;
            return;
        };
        self.screen = Screen::Project;
        if kind == NewKind::D3 {
            self.p3 = Some(p3d::ui::P3App::new(m.clone()));
            self.project = None;
            self.project_path = None;
            self.say(format!("New 3D project on {}", m.name));
            return;
        }
        self.p3 = None;
        self.project = Some(Project::new(m.clone()));
        self.project_path = None;
        self.canvas = Canvas::default();
        self.gcode.clear();
        self.warnings.clear();
        self.preview.clear();
        self.tab = ProjectTab::Prepare;
        self.say(format!("New 2D project on {}", m.name));
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        if let Some((tex, started)) = &mut self.splash {
            if started.is_none() {
                // eframe shows the window after the first frame; keep it hidden behind the splash.
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            let t0 = *started.get_or_insert_with(std::time::Instant::now);
            if t0.elapsed().as_secs_f32() < SPLASH_SECS {
                let size = egui::vec2(640.0, 360.0);
                let mon = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1920.0, 1080.0));
                let builder = egui::ViewportBuilder::default()
                    .with_title("CAMCraft")
                    .with_inner_size(size)
                    .with_position(((mon - size) / 2.0).to_pos2())
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_taskbar(false)
                    .with_always_on_top();
                ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("splash"), builder, |ui, _| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                        let rect = ui.max_rect();
                        ui.put(rect, egui::Image::new(&*tex).fit_to_exact_size(rect.size()));
                        ui.painter().text(
                            rect.right_bottom() - egui::vec2(14.0, 10.0),
                            egui::Align2::RIGHT_BOTTOM,
                            format!("Version {}", env!("CARGO_PKG_VERSION")),
                            egui::FontId::proportional(13.0),
                            Color32::from_rgb(190, 205, 235),
                        );
                    });
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                return;
            }
            self.splash = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        let mut actions: Vec<Action> = vec![];

        self.top_bar(root, &mut actions);
        self.status_bar(root);
        match self.screen {
            Screen::Project => self.project_screen(root, &mut actions),
            Screen::Machines => self.machines_screen(root),
        }
        self.welcome_window(&ctx, &mut actions);
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
