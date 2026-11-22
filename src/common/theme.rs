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

// UI colours: neutral dark grey with a blue accent, like the SolidWorks dark theme.
pub const UI_PANEL: Color32 = Color32::from_rgb(0x2b, 0x2e, 0x33);
pub const UI_DEEP: Color32 = Color32::from_rgb(0x1f, 0x21, 0x25);
pub const UI_WIDGET: Color32 = Color32::from_rgb(0x36, 0x3a, 0x40);
pub const UI_HOVER: Color32 = Color32::from_rgb(0x43, 0x48, 0x50);
pub const UI_BORDER: Color32 = Color32::from_rgb(0x44, 0x48, 0x4f);
pub const UI_TEXT_WEAK: Color32 = Color32::from_rgb(0x9a, 0xa1, 0xab);

pub fn apply(ctx: &Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = UI_PANEL;
    v.window_fill = UI_PANEL;
    v.extreme_bg_color = UI_DEEP;
    v.faint_bg_color = Color32::from_rgb(0x30, 0x33, 0x39);
    v.window_stroke = Stroke::new(1.0, UI_BORDER);
    v.window_shadow.blur = 28;
    v.window_shadow.color = Color32::from_black_alpha(140);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = PATH_PART_SELECTED;
    let r = CornerRadius::same(2);
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = r;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, UI_BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xd4, 0xd8, 0xde));
    v.widgets.inactive.bg_fill = UI_WIDGET;
    v.widgets.inactive.weak_bg_fill = UI_WIDGET;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, UI_BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xe2, 0xe5, 0xea));
    v.widgets.hovered.bg_fill = UI_HOVER;
    v.widgets.hovered.weak_bg_fill = UI_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = ACCENT;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    ctx.set_visuals(v);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.menu_margin = egui::Margin::same(6);
    use egui::{FontId, TextStyle};
    style.text_styles.insert(TextStyle::Body, FontId::proportional(13.5));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(13.5));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(16.0));
    ctx.set_global_style(style);
}
