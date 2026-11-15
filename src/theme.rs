use eframe::egui;
use egui::{Color32, Context, CornerRadius, Stroke};

// Colours follow the dark AutoCAD workspace.
pub const CANVAS_BG: Color32 = Color32::from_rgb(0x21, 0x28, 0x30);
pub const BED_FILL: Color32 = Color32::from_rgb(0x1a, 0x20, 0x27);
pub const BED_EDGE: Color32 = Color32::from_rgb(0x3f, 0xc1, 0x6b);
pub const GRID_MINOR: Color32 = Color32::from_rgb(0x2a, 0x32, 0x3b);
pub const GRID_MAJOR: Color32 = Color32::from_rgb(0x38, 0x42, 0x4d);
pub const PATH_NORMAL: Color32 = Color32::from_rgb(0xe6, 0xec, 0xf2);
pub const PATH_PART_SELECTED: Color32 = Color32::from_rgb(0x4f, 0xb6, 0xff);
pub const PATH_HOVER: Color32 = Color32::from_rgb(0xff, 0xdc, 0x3c);
pub const PATH_SEQ: Color32 = Color32::from_rgb(0x50, 0xe0, 0x64);
pub const PATH_OUTSIDE: Color32 = Color32::from_rgb(0xff, 0x55, 0x55);
pub const TRAVEL: Color32 = Color32::from_rgb(0x7a, 0x86, 0x92);
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0x78, 0xd4);

pub fn apply(ctx: &Context) {
    let mut v = egui::Visuals::dark();
    let panel = Color32::from_rgb(0x2b, 0x31, 0x39);
    let panel_dark = Color32::from_rgb(0x23, 0x28, 0x2f);
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = panel_dark;
    v.faint_bg_color = Color32::from_rgb(0x30, 0x36, 0x3f);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = Color32::from_rgb(0x4f, 0xb6, 0xff);
    let r = CornerRadius::same(2);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3d, 0x45, 0x4f));
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x3a, 0x41, 0x4b);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x3a, 0x41, 0x4b);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x4a, 0x53, 0x5f);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x4a, 0x53, 0x5f);
    ctx.set_visuals(v);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    ctx.set_global_style(style);
}
