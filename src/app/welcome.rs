use super::{Action, App, NewKind, Screen};
use eframe::egui;
use egui::RichText;

impl App {
    /// Start window shown over the empty workspace: the three start buttons and recent projects.
    pub(crate) fn welcome_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let empty = self.project.is_none() && self.p3.is_none();
        if !empty || self.screen != Screen::Project || self.welcome_closed {
            return;
        }
        let mut open = true;
        egui::Window::new("Welcome to CAMCraft")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(600.0);
                ui.horizontal(|ui| {
                    if ui.add_sized([186.0, 34.0], egui::Button::new("New 2D project")).clicked() {
                        actions.push(Action::NewProject(NewKind::D2));
                    }
                    if ui.add_sized([186.0, 34.0], egui::Button::new("New 3D project")).clicked() {
                        actions.push(Action::NewProject(NewKind::D3));
                    }
                    if ui.add_sized([186.0, 34.0], egui::Button::new("Open project...")).clicked() {
                        actions.push(Action::OpenProject);
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Recent projects").strong());
                    if !self.recent.0.is_empty() && ui.small_button("Clear").clicked() {
                        self.recent.clear();
                    }
                });
                ui.add_space(4.0);
                if self.recent.0.is_empty() {
                    ui.weak("No recent projects yet.");
                }
                ui.horizontal_wrapped(|ui| {
                    for path in &self.recent.0 {
                        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        let is3d = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("camcraft3d"));
                        let kind = if is3d { "3D project" } else { "2D project" };
                        let tile = egui::Button::new(format!("{name}\n{kind}"));
                        if ui.add_sized([186.0, 56.0], tile).on_hover_text(path.display().to_string()).clicked() {
                            actions.push(Action::OpenPath(path.clone()));
                        }
                    }
                });
            });
        if !open {
            self.welcome_closed = true;
        }
    }
}
