use super::machine::*;
use crate::app::App;
use super::preview;
use eframe::egui;
use egui::{Color32, RichText};

#[derive(PartialEq, Clone, Copy)]
pub enum MachineTab {
    Machine,
    Speed,
    EndEffector,
    Scripts,
}

impl App {
    pub(crate) fn machines_screen(&mut self, root: &mut egui::Ui) {
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

pub(crate) fn unique_machine_name(machines: &[Machine], base: &str) -> String {
    if !machines.iter().any(|m| m.name == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|c| !machines.iter().any(|m| &m.name == c))
        .unwrap()
}

pub(crate) fn machine_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
    ui.heading("Machine");
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.strong("Machine construction");
            let mut edited: Vec<(usize, f64, f64)> = vec![];
            egui::Grid::new("m_build").num_columns(5).striped(true).show(ui, |ui| {
                for h in ["Axis", "Min (mm)", "Max (mm)", "Bed size (mm)", "Moves the"] {
                    ui.strong(h);
                }
                ui.end_row();
                let names = ["X", "Y", "Z"];
                let ranges = [0, 1, 2].map(|i| m.travel_range(i));
                let bed = [&mut m.bed_x, &mut m.bed_y, &mut m.bed_z];
                for (i, b) in bed.into_iter().enumerate() {
                    ui.label(names[i]);
                    let (mut lo, mut hi) = ranges[i];
                    let before = (lo, hi);
                    ui.add(egui::DragValue::new(&mut lo).speed(1.0).range(-5000.0..=hi - 1.0));
                    ui.add(egui::DragValue::new(&mut hi).speed(1.0).range(lo + 1.0..=5000.0));
                    if (lo, hi) != before {
                        edited.push((i, lo, hi));
                    }
                    *ch |= ui.add(egui::DragValue::new(b).speed(1.0).range(1.0..=5000.0)).changed();
                    ui.horizontal(|ui| {
                        *ch |= ui.radio_value(&mut m.axes[i], AxisOwner::EndEffector, "End effector").changed();
                        *ch |= ui.radio_value(&mut m.axes[i], AxisOwner::Bed, "Bed").changed();
                    });
                    ui.label("");
                    ui.end_row();
                }
            });
            for (i, lo, hi) in edited {
                m.travel_min[i] = lo;
                m.travel_max[i] = hi;
                *ch = true;
            }
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

pub(crate) fn speed_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
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

pub(crate) fn end_effector_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
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

pub(crate) fn scripts_section(ui: &mut egui::Ui, m: &mut Machine, ch: &mut bool) {
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
    egui::Grid::new("m_fw").num_columns(2).show(ui, |ui| {
        ui.label("Firmware");
        let mut fw = m.firmware;
        egui::ComboBox::from_id_salt("m_fw_sel").selected_text(fw.label()).show_ui(ui, |ui| {
            for f in crate::common::machine::Firmware::ALL {
                ui.selectable_value(&mut fw, f, f.label());
            }
        });
        if fw != m.firmware {
            m.set_firmware(fw);
            *ch = true;
        }
        ui.end_row();
        ui.label("Baud rate");
        *ch |= ui.add(egui::DragValue::new(&mut m.baud).range(1200..=2_000_000)).changed();
        ui.end_row();
    });
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
