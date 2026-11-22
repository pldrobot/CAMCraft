//! Edit > Preferences window.

use super::settings::Settings;
use crate::common::controls::{Act, Controls, PadMode};
use eframe::egui;

#[derive(Default)]
pub(crate) struct Prefs {
    pub(crate) open: bool,
    /// The action waiting for a key press.
    pub(crate) capture: Option<Act>,
}

impl Prefs {
    /// Shows the window. Returns true when a setting changed (the caller saves it).
    pub(crate) fn show(&mut self, ctx: &egui::Context, s: &mut Settings) -> bool {
        let mut changed = false;
        let mut open = self.open;
        egui::Window::new("Preferences").open(&mut open).default_width(420.0).collapsible(false).show(ctx, |ui| {
            let c = &mut s.controls;
            ui.heading("Keyboard");
            ui.label(egui::RichText::new("Click a key, then press the new one. Click again to cancel.").small().weak());
            egui::Grid::new("pref_keys").num_columns(2).striped(true).show(ui, |ui| {
                for a in Act::ALL {
                    ui.label(a.label());
                    let waiting = self.capture == Some(a);
                    let text = if waiting { "press a key...".to_string() } else { c.key(a).map(|k| k.name().to_string()).unwrap_or("none".into()) };
                    if ui.add_sized([130.0, 22.0], egui::Button::new(text).selected(waiting)).clicked() {
                        self.capture = if waiting { None } else { Some(a) };
                    }
                    ui.end_row();
                }
            });
            if let Some(a) = self.capture {
                let key = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(*key),
                        _ => None,
                    })
                });
                if let Some(k) = key {
                    c.set_key(a, k);
                    self.capture = None;
                    changed = true;
                }
            }
            ui.separator();
            ui.heading("Jog");
            egui::Grid::new("pref_jog").num_columns(2).show(ui, |ui| {
                ui.label("X/Y steps (mm)");
                ui.horizontal(|ui| {
                    for v in c.steps.iter_mut() {
                        changed |= ui.add(egui::DragValue::new(v).speed(0.05).range(0.01..=1000.0)).changed();
                    }
                });
                ui.end_row();
                ui.label("Z steps (mm)");
                ui.horizontal(|ui| {
                    for v in c.steps_z.iter_mut() {
                        changed |= ui.add(egui::DragValue::new(v).speed(0.05).range(0.01..=1000.0)).changed();
                    }
                });
                ui.end_row();
                ui.label("Speed (mm/min)");
                changed |= ui.add(egui::DragValue::new(&mut c.feed).range(10.0..=20000.0)).changed();
                ui.end_row();
                ui.label("Hold for continuous (ms)");
                changed |= ui.add(egui::DragValue::new(&mut c.long_press_ms).range(100..=2000)).changed();
                ui.end_row();
            });
            ui.separator();
            ui.heading("Gamepad");
            ui.horizontal(|ui| {
                changed |= ui.radio_value(&mut c.pad_mode, PadMode::Stick, "Joystick").changed();
                changed |= ui.radio_value(&mut c.pad_mode, PadMode::Buttons, "Buttons").changed();
            });
            let text = match c.pad_mode {
                PadMode::Stick => {
                    "Continuous: left stick X / Y, right stick up/down Z. The further you push, the faster.\n\
                     Y / X: speed up / down.  A: start / pause.  B: stop.  Start: home all."
                }
                PadMode::Buttons => {
                    "Steps: D-pad X / Y, bumpers Z. Hold to move continuously.\n\
                     Y / X: step size up / down.  A: start / pause.  B: stop.  Start: home all."
                }
            };
            ui.label(egui::RichText::new(text).small());
            egui::Grid::new("pref_pad").num_columns(2).show(ui, |ui| {
                ui.label("Dead zone");
                changed |= ui.add(egui::Slider::new(&mut c.dead_zone, 0.0..=0.9)).changed();
                ui.end_row();
            });
            ui.add_space(6.0);
            if ui.button("Reset to defaults").clicked() {
                *c = Controls::default();
                changed = true;
            }
        });
        if !open {
            self.capture = None;
        }
        self.open = open;
        changed
    }
}
