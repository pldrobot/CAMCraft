use super::{Action, App, NewKind, Screen};
use crate::common::theme;
use eframe::egui;
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Vec2};

const TILE: Vec2 = Vec2::new(146.0, 124.0);
const THUMB: Vec2 = Vec2::new(146.0, 82.0);

impl App {
    /// Start window shown over the empty workspace: start commands on the left, recent projects
    /// on the right, in the style of the SolidWorks welcome dialog.
    pub(crate) fn welcome_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let empty = self.project.is_none() && self.p3.is_none();
        if !empty || self.screen != Screen::Project || self.welcome_closed {
            return;
        }
        for path in self.recent.0.clone() {
            if !self.thumbs.contains_key(&path) {
                let tex = super::thumb::file_for(&path)
                    .and_then(|f| image::open(f).ok())
                    .map(|im| {
                        let im = im.to_rgba8();
                        let img = egui::ColorImage::from_rgba_unmultiplied([im.width() as usize, im.height() as usize], im.as_raw());
                        ctx.load_texture(format!("thumb-{}", path.display()), img, egui::TextureOptions::LINEAR)
                    });
                self.thumbs.insert(path, tex);
            }
        }
        let mut open = true;
        let frame = egui::Frame::window(&ctx.global_style()).fill(theme::UI_PANEL).inner_margin(0.0);
        egui::Window::new("Welcome to CAMCraft")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .frame(frame)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(720.0);
                self.welcome_header(ui);
                ui.horizontal_top(|ui| {
                    ui.add_space(16.0);
                    ui.allocate_ui_with_layout(Vec2::new(210.0, 300.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.add_space(12.0);
                        section_label(ui, "START");
                        if start_item(ui, "New 2D project", "DXF drawings, flat paths").clicked() {
                            actions.push(Action::NewProject(NewKind::D2));
                        }
                        if start_item(ui, "New 3D project", "STEP models, edges and faces").clicked() {
                            actions.push(Action::NewProject(NewKind::D3));
                        }
                        if start_item(ui, "Open project...", "Browse for a project file").clicked() {
                            actions.push(Action::OpenProject);
                        }
                    });
                    ui.add_space(8.0);
                    let (line, _) = ui.allocate_exact_size(Vec2::new(1.0, 300.0), Sense::hover());
                    ui.painter().rect_filled(line, 0.0, theme::UI_BORDER);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            section_label(ui, "RECENT PROJECTS");
                            if !self.recent.0.is_empty() && ui.small_button("Clear").clicked() {
                                self.recent.clear();
                            }
                        });
                        if self.recent.0.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("No recent projects yet.").color(theme::UI_TEXT_WEAK));
                        }
                        egui::ScrollArea::vertical().max_height(270.0).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::splat(8.0);
                                for path in &self.recent.0 {
                                    let tex = self.thumbs.get(path).and_then(|t| t.as_ref());
                                    if recent_tile(ui, path, tex).clicked() {
                                        actions.push(Action::OpenPath(path.clone()));
                                    }
                                }
                            });
                        });
                    });
                });
                ui.add_space(14.0);
            });
        if !open {
            self.welcome_closed = true;
        }
    }

    fn welcome_header(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(720.0, 76.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, theme::UI_DEEP);
        p.rect_filled(egui::Rect::from_min_size(rect.left_bottom() - Vec2::new(0.0, 2.0), Vec2::new(rect.width(), 2.0)), 0.0, theme::ACCENT);
        let mut x = rect.left() + 18.0;
        if let Some(logo) = &self.logo {
            let img = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 12.0), Vec2::splat(52.0));
            p.image(logo.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            x += 66.0;
        }
        p.text(egui::pos2(x, rect.top() + 38.0), Align2::LEFT_BOTTOM, "CAMCraft", FontId::proportional(26.0), Color32::WHITE);
        p.text(
            egui::pos2(x, rect.top() + 42.0),
            Align2::LEFT_TOP,
            "Path programming for CNC machines",
            FontId::proportional(12.5),
            theme::UI_TEXT_WEAK,
        );
        p.text(
            rect.right_center() - Vec2::new(18.0, 0.0),
            Align2::RIGHT_CENTER,
            format!("Version {}", env!("CARGO_PKG_VERSION")),
            FontId::proportional(12.5),
            theme::UI_TEXT_WEAK,
        );
    }
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).strong().color(theme::UI_TEXT_WEAK));
    ui.add_space(2.0);
}

/// Flat list entry: bold title with a weak description, accent bar on hover.
fn start_item(ui: &mut egui::Ui, title: &str, hint: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 2.0, theme::UI_HOVER);
        p.rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), 0.0, theme::ACCENT);
    }
    p.text(rect.left_top() + Vec2::new(14.0, 9.0), Align2::LEFT_TOP, title, FontId::proportional(14.5), Color32::WHITE);
    p.text(rect.left_top() + Vec2::new(14.0, 29.0), Align2::LEFT_TOP, hint, FontId::proportional(11.5), theme::UI_TEXT_WEAK);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn recent_tile(ui: &mut egui::Ui, path: &std::path::Path, thumb: Option<&egui::TextureHandle>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(TILE, Sense::click());
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let is3d = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("camcraft3d"));
    let p = ui.painter();
    let (fill, stroke) = if resp.hovered() { (theme::UI_HOVER, theme::ACCENT) } else { (theme::UI_WIDGET, theme::UI_BORDER) };
    p.rect(rect, 2.0, fill, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    let img = egui::Rect::from_min_size(rect.min + Vec2::splat(1.0), THUMB - Vec2::splat(2.0));
    match thumb {
        Some(t) => {
            p.image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            p.rect_filled(img, 0.0, theme::UI_DEEP);
            p.text(img.center(), Align2::CENTER_CENTER, if is3d { "3D" } else { "2D" }, FontId::proportional(22.0), theme::UI_TEXT_WEAK);
        }
    }
    // Small badge with the project kind.
    let badge = egui::Rect::from_min_size(img.right_top() + Vec2::new(-30.0, 4.0), Vec2::new(26.0, 16.0));
    p.rect_filled(badge, 2.0, if is3d { theme::ACCENT } else { Color32::from_rgb(0x2d, 0x8a, 0x5e) });
    p.text(badge.center(), Align2::CENTER_CENTER, if is3d { "3D" } else { "2D" }, FontId::proportional(11.0), Color32::WHITE);
    let max_chars = 21;
    let name = if name.chars().count() > max_chars { format!("{}...", name.chars().take(max_chars - 3).collect::<String>()) } else { name };
    p.text(egui::pos2(rect.left() + 8.0, rect.top() + 91.0), Align2::LEFT_TOP, name, FontId::proportional(13.0), Color32::WHITE);
    resp.on_hover_text(path.display().to_string()).on_hover_cursor(egui::CursorIcon::PointingHand)
}
