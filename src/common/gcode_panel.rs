//! Read-only G-code list with line numbers, shared by the 2D and 3D Run tab.

use eframe::egui::{self, RichText};

/// What the user asked for with the header buttons.
#[derive(Clone, Copy, PartialEq)]
pub enum Tool {
    Save,
    Copy,
}

/// One line: line count and the Save and Copy icon buttons, then the list. `current` is a
/// 0-based line to highlight and keep in view while a run is going.
pub fn show(ui: &mut egui::Ui, gcode: &str, current: Option<usize>) -> Option<Tool> {
    let mut tool = None;
    ui.horizontal(|ui| {
        ui.label(format!("{} lines", gcode.lines().count()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::common::icons::icon_button(ui, crate::common::icons::Icon::Copy, "Copy to clipboard").clicked() {
                tool = Some(Tool::Copy);
            }
            if crate::common::icons::icon_button(ui, crate::common::icons::Icon::Save, "Save G-code...").clicked() {
                tool = Some(Tool::Save);
            }
        });
    });
    ui.separator();
    list(ui, gcode, current);
    tool
}

fn list(ui: &mut egui::Ui, gcode: &str, current: Option<usize>) {
    let lines: Vec<&str> = gcode.lines().collect();
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
    let row = row_h + ui.spacing().item_spacing.y;
    let width = lines.len().max(1).to_string().len();
    // Visible rows of the last frame. Scroll only when the line leaves them, so the list does
    // not jump on every line while the simulation runs fast.
    let key = ui.scope_id().with("visible");
    let vis: Option<(usize, usize)> = ui.data(|d| d.get_temp(key));
    let mut area = egui::ScrollArea::vertical().auto_shrink(false);
    if let (Some(c), Some((a, b))) = (current, vis) {
        if c < a + 1 || c + 2 > b {
            let half = (b.saturating_sub(a)) as f32 / 2.0;
            area = area.vertical_scroll_offset((c as f32 - half).max(0.0) * row);
        }
    }
    let out = area.show_rows(ui, row_h, lines.len(), |ui, range| {
        for i in range.clone() {
            let text = RichText::new(format!("{:>width$}  {}", i + 1, lines[i])).monospace();
            // Plain label rows keep the exact height show_rows assumes; the highlight is painted behind.
            let bg = ui.painter().add(egui::Shape::Noop);
            let r = ui.add(egui::Label::new(text).truncate());
            if current == Some(i) {
                let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), r.rect.y_range());
                ui.painter().set(bg, egui::Shape::rect_filled(rect, 0.0, ui.visuals().selection.bg_fill));
            }
        }
        range
    });
    ui.data_mut(|d| d.insert_temp(key, (out.inner.start, out.inner.end)));
}
