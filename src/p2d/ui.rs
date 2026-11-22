use super::canvas::Tool;
use super::dxf_import;
use super::model::*;
use super::sequence_ui::*;
use crate::app::{Action, App, ProjectTab};
use crate::common::{icons, machine::*, preview, theme};
use eframe::egui;
use egui::{Color32, RichText};
use std::collections::HashSet;

impl App {
    pub(crate) fn add_part(&mut self, name: String, source: String, layers: Vec<String>, r: dxf_import::ImportResult, replace: Option<usize>) {
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

    pub(crate) fn layer_window(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn project_screen(&mut self, root: &mut egui::Ui, actions: &mut Vec<Action>) {
        if let Some(p3) = &mut self.p3 {
            p3.show(root, &self.lib, &self.settings.controls, &self.cmds);
            let msgs = std::mem::take(&mut p3.msgs);
            for m in msgs {
                self.say(m);
            }
            return;
        }
        if self.project.is_none() {
            // Empty workspace: the same layout as a project, disabled, with the welcome window on top.
            egui::Panel::left("empty_left").resizable(false).exact_size(330.0).show(root, |ui| {
                ui.add_enabled_ui(false, |ui| {
                    icons::tab_strip(ui, &[
                        (icons::Icon::TabPrepare, "Prepare", true),
                        (icons::Icon::TabPath, "Path selection", true),
                        (icons::Icon::TabSimulation, "Run", true),
                    ], 0);
                    for h in ["Parts", "Job settings", "Sequence"] {
                        ui.collapsing(h, |ui| {
                            ui.label("-");
                        });
                    }
                });
            });
            egui::CentralPanel::default().show(root, |_| {});
            return;
        }

        let Self { lib, project, canvas, gcode, warnings, tab, preview, seq_drag, settings, cmds, .. } = self;
        let project = project.as_mut().unwrap();
        let generated = preview.has_path();
        if *tab == ProjectTab::Run && !generated {
            *tab = ProjectTab::Prepare;
        }

        match *tab {
            ProjectTab::Prepare => canvas.tool = Tool::Move,
            ProjectTab::PathSelection => canvas.tool = Tool::Sequence,
            ProjectTab::Run => {}
        }

        egui::Panel::left("left").resizable(false).exact_size(330.0).show(root, |ui| {
            const ORDER: [ProjectTab; 3] = [ProjectTab::Prepare, ProjectTab::PathSelection, ProjectTab::Run];
            let cur = ORDER.iter().position(|t| *t == *tab).unwrap_or(0);
            let picked = icons::tab_strip(ui, &[
                (icons::Icon::TabPrepare, "Prepare", true),
                (icons::Icon::TabPath, "Path selection", true),
                (icons::Icon::TabSimulation, "Run", generated),
            ], cur);
            if let Some(i) = picked {
                *tab = ORDER[i];
            }
            if *tab == ProjectTab::Run && preview.machine_mode {
                preview::console_panel(ui, preview);
            }
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
                        let mut pick: Option<usize> = None;
                        egui::ComboBox::from_id_salt("project_machine")
                            .selected_text(project.machine.name.clone())
                            .width(ui.available_width() - 8.0)
                            .show_ui(ui, |ui| {
                                for (i, m) in lib.machines.iter().enumerate() {
                                    if ui.selectable_label(m.name == project.machine.name, &m.name).clicked() {
                                        pick = Some(i);
                                    }
                                }
                            });
                        if let Some(m) = pick.and_then(|i| lib.machines.get(i)) {
                            project.job = m.job.clone();
                            project.machine = m.clone();
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
                    ProjectTab::Run => {
                        let m = &project.machine;
                        ui.heading("Run");
                        preview::run_controls(ui, preview, &project.machine, &settings.controls, cmds, gcode);
                        ui.separator();
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
                    }
                }
            });
        });

        if *tab == ProjectTab::Run {
            egui::Panel::right("gcode_right").default_size(320.0).show(root, |ui| {
                match crate::common::gcode_panel::show(ui, gcode, preview.current_line()) {
                    Some(crate::common::gcode_panel::Tool::Save) => actions.push(Action::SaveGcode),
                    Some(crate::common::gcode_panel::Tool::Copy) => {
                        ui.ctx().copy_text(gcode.clone());
                        actions.push(Action::CopyGcode);
                    }
                    None => {}
                }
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
                ProjectTab::Run => {
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
                        ui.checkbox(&mut preview.show_parts, "Parts");
                    });
                    preview.show(ui, &project.machine);
                }
            });
    }
}
