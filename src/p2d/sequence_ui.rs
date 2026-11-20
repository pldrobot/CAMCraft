use super::model::*;
use crate::common::machine::*;
use crate::common::{icons, theme};
use eframe::egui;
use egui::RichText;

pub(crate) fn job_settings_grid(ui: &mut egui::Ui, j: &mut JobSettings) {
    egui::Grid::new("job").num_columns(2).show(ui, |ui| {
        ui.label("Work feed");
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
        ui.add_enabled_ui(j.use_z, |ui| ui.label("Work Z"));
        ui.add_enabled(j.use_z, egui::DragValue::new(&mut j.dispense_z).speed(0.1).suffix(" mm"));
        ui.end_row();
        ui.label("Start delay");
        ui.add(egui::DragValue::new(&mut j.on_delay_ms).speed(5.0).suffix(" ms"));
        ui.end_row();
        ui.label("Stop delay");
        ui.add(egui::DragValue::new(&mut j.off_delay_ms).speed(5.0).suffix(" ms"));
        ui.end_row();
    });
}

pub(crate) enum AddStep {
    AllPaths,
    Home,
    Goto,
    Pause,
    Clear,
}

pub(crate) fn step_desc(project: &Project, item: &SeqItem) -> String {
    match item.kind {
        StepKind::Path => {
            let name = project.parts.get(item.part).map(|p| p.name.as_str()).unwrap_or("?");
            format!(
                "{} #{}{}{}",
                name,
                item.path + 1,
                if item.reverse { "  (reversed)" } else { "" },
                if item.over.is_empty() { "" } else { "  *" }
            )
        }
        StepKind::Home => "Home".to_string(),
        StepKind::Goto { x, y, z } => match z {
            Some(z) => format!("Go to  X{x:.2}  Y{y:.2}  Z{z:.2}"),
            None => format!("Go to  X{x:.2}  Y{y:.2}"),
        },
        StepKind::Pause { ms } => format!("Pause  {ms} ms"),
    }
}

/// A row with a tick to override a job value and the value itself.
pub(crate) fn override_row(ui: &mut egui::Ui, label: &str, ov: &mut Option<f64>, job: f64, speed: f64, suffix: &str, enabled: bool) {
    ui.add_enabled_ui(enabled, |ui| {
        let mut on = ov.is_some();
        if ui.checkbox(&mut on, label).changed() {
            *ov = if on { Some(job) } else { None };
        }
    });
    match ov {
        Some(v) if enabled => {
            ui.add(egui::DragValue::new(v).speed(speed).suffix(suffix));
        }
        _ => {
            ui.label(RichText::new(format!("{job}{suffix}")).weak());
        }
    }
    ui.end_row();
}

pub(crate) fn step_settings(ui: &mut egui::Ui, item: &mut SeqItem, job: &JobSettings) {
    ui.set_min_width(250.0);
    match &mut item.kind {
        StepKind::Path => {
            ui.label(RichText::new("Tick a value to change it for this path. Others follow Job settings.").small().weak());
            let o = &mut item.over;
            egui::Grid::new("step_over").num_columns(2).show(ui, |ui| {
                override_row(ui, "Work feed", &mut o.dispense_feed, job.dispense_feed, 5.0, " mm/min", true);
                override_row(ui, "Travel feed", &mut o.travel_feed, job.travel_feed, 10.0, " mm/min", true);
                override_row(ui, "Safe Z", &mut o.safe_z, job.safe_z, 0.1, " mm", job.use_z);
                override_row(ui, "Work Z", &mut o.dispense_z, job.dispense_z, 0.1, " mm", job.use_z);
                let mut on_ms = o.on_delay_ms.map(|v| v as f64);
                override_row(ui, "Start delay", &mut on_ms, job.on_delay_ms as f64, 5.0, " ms", true);
                o.on_delay_ms = on_ms.map(|v| v.max(0.0) as u32);
                let mut off_ms = o.off_delay_ms.map(|v| v as f64);
                override_row(ui, "Stop delay", &mut off_ms, job.off_delay_ms as f64, 5.0, " ms", true);
                o.off_delay_ms = off_ms.map(|v| v.max(0.0) as u32);
            });
            if ui.add_enabled(!item.over.is_empty(), egui::Button::new("Use job settings")).clicked() {
                item.over = JobOverride::default();
            }
        }
        StepKind::Goto { x, y, z } => {
            egui::Grid::new("step_goto").num_columns(2).show(ui, |ui| {
                ui.label("X");
                ui.add(egui::DragValue::new(x).speed(0.1).suffix(" mm"));
                ui.end_row();
                ui.label("Y");
                ui.add(egui::DragValue::new(y).speed(0.1).suffix(" mm"));
                ui.end_row();
                let mut use_z = z.is_some();
                if ui.checkbox(&mut use_z, "Z").changed() {
                    *z = if use_z { Some(job.safe_z) } else { None };
                }
                match z {
                    Some(v) => {
                        ui.add(egui::DragValue::new(v).speed(0.1).suffix(" mm"));
                    }
                    None => {
                        ui.label(RichText::new("stays at safe Z").weak());
                    }
                }
                ui.end_row();
            });
        }
        StepKind::Pause { ms } => {
            ui.horizontal(|ui| {
                ui.label("Wait");
                ui.add(egui::DragValue::new(ms).speed(10.0).range(0..=600_000).suffix(" ms"));
            });
        }
        StepKind::Home => {
            ui.label("Goes to the home position of the machine.");
        }
    }
}

/// Move the selected steps so the block starts at `ins` (an index into the
/// list before the move).
pub(crate) fn move_steps(project: &mut Project, sel: &mut Vec<usize>, ins: usize) {
    let mut idx = sel.clone();
    idx.sort_unstable();
    idx.dedup();
    idx.retain(|&i| i < project.sequence.len());
    if idx.is_empty() {
        return;
    }
    let before = idx.iter().filter(|&&i| i < ins).count();
    let items: Vec<SeqItem> = idx.iter().map(|&i| project.sequence[i]).collect();
    for &i in idx.iter().rev() {
        project.sequence.remove(i);
    }
    let at = (ins - before).min(project.sequence.len());
    for (k, it) in items.iter().enumerate() {
        project.sequence.insert(at + k, *it);
    }
    *sel = (at..at + items.len()).collect();
}

pub(crate) fn sequence_ui(ui: &mut egui::Ui, project: &mut Project, sel: &mut Vec<usize>, drag: &mut Option<usize>) {
    sel.retain(|&i| i < project.sequence.len());
    let mut add: Option<AddStep> = None;
    let mut delete = false;

    ui.horizontal(|ui| {
        ui.heading("Sequence");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Right to left: delete first so it sits right of add.
            if icons::icon_button_ex(ui, icons::Icon::Trash, "Remove the selected steps (Delete)", false, !sel.is_empty()).clicked() {
                delete = true;
            }
            let plus = icons::icon_button(ui, icons::Icon::Plus, "Add a step");
            egui::Popup::menu(&plus).show(|ui| {
                if ui.button("All paths").clicked() {
                    add = Some(AddStep::AllPaths);
                }
                ui.separator();
                if ui.button("Home").clicked() {
                    add = Some(AddStep::Home);
                }
                if ui.button("Go to coordinate").clicked() {
                    add = Some(AddStep::Goto);
                }
                if ui.button("Pause").clicked() {
                    add = Some(AddStep::Pause);
                }
                ui.separator();
                if ui.button("Clear sequence").clicked() {
                    add = Some(AddStep::Clear);
                }
            });
        });
    });
    ui.label(
        RichText::new("Click paths on the bed to add them. Drag rows to reorder. Ctrl or Shift click selects several.")
            .small()
            .weak(),
    );

    // Delete key, when no text field is being edited.
    if !sel.is_empty() && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        delete = true;
    }

    // Adding steps.
    if let Some(a) = add {
        let at = sel.iter().max().map_or(project.sequence.len(), |m| m + 1).min(project.sequence.len());
        let home = project.machine.home_position();
        let new: Vec<SeqItem> = match a {
            AddStep::AllPaths => {
                let mut v = vec![];
                for (pi, part) in project.parts.iter().enumerate() {
                    for ci in 0..part.paths.len() {
                        if project.seq_index(pi, ci).is_none() {
                            v.push(SeqItem::path(pi, ci));
                        }
                    }
                }
                v
            }
            AddStep::Home => vec![SeqItem::other(StepKind::Home)],
            AddStep::Goto => vec![SeqItem::other(StepKind::Goto { x: home[0], y: home[1], z: None })],
            AddStep::Pause => vec![SeqItem::other(StepKind::Pause { ms: 1000 })],
            AddStep::Clear => {
                project.sequence.clear();
                sel.clear();
                vec![]
            }
        };
        let count = new.len();
        for (k, it) in new.into_iter().enumerate() {
            project.sequence.insert(at + k, it);
        }
        if count > 0 {
            *sel = (at..at + count).collect();
        }
    }
    if delete {
        let mut idx = sel.clone();
        idx.sort_unstable();
        idx.dedup();
        for &i in idx.iter().rev() {
            if i < project.sequence.len() {
                project.sequence.remove(i);
            }
        }
        sel.clear();
    }

    // The list.
    let n = project.sequence.len();
    let job = project.job.clone();
    let mut rows: Vec<egui::Rect> = Vec::with_capacity(n);
    if n == 0 {
        ui.label(RichText::new("The sequence is empty.").small().weak());
    }
    for i in 0..n {
        let item = project.sequence[i];
        let desc = step_desc(project, &item);
        let selected = sel.contains(&i);
        let bg = ui.painter().add(egui::Shape::Noop);
        let mut label_rect = egui::Rect::NOTHING;
        ui.horizontal(|ui| {
            let w = (ui.available_width() - if item.is_path() { 54.0 } else { 28.0 }).max(40.0);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::click_and_drag());
            label_rect = rect;
            if resp.drag_started() {
                *drag = Some(i);
                if !selected {
                    sel.clear();
                    sel.push(i);
                }
            }
            if resp.clicked() {
                let m = ui.input(|inp| inp.modifiers);
                if m.shift && !sel.is_empty() {
                    let anchor = *sel.last().unwrap();
                    let (lo, hi) = (anchor.min(i), anchor.max(i));
                    *sel = (lo..=hi).collect();
                } else if m.command {
                    if let Some(pos) = sel.iter().position(|&x| x == i) {
                        sel.remove(pos);
                    } else {
                        sel.push(i);
                    }
                } else {
                    sel.clear();
                    sel.push(i);
                }
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(if drag.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            // Grip dots and text.
            let col = ui.visuals().text_color();
            let painter = ui.painter().with_clip_rect(rect);
            for dx in [0.0_f32, 4.0] {
                for dy in [-4.0_f32, 0.0, 4.0] {
                    painter.circle_filled(rect.left_center() + egui::vec2(5.0 + dx, dy), 1.1, col.gamma_multiply(0.5));
                }
            }
            painter.text(
                rect.left_center() + egui::vec2(18.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{:>2}.  {}", i + 1, desc),
                egui::FontId::proportional(13.0),
                col,
            );
            if item.is_path() {
                let rev = icons::icon_button_ex(ui, icons::Icon::Reverse, "Reverse direction", item.reverse, true);
                if rev.clicked() {
                    project.sequence[i].reverse = !item.reverse;
                }
            }
            if item.kind != StepKind::Home {
                let b = icons::icon_button_ex(ui, icons::Icon::Settings, "Step settings", !item.over.is_empty(), true);
                egui::Popup::from_toggle_button_response(&b)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| step_settings(ui, &mut project.sequence[i], &job));
            }
        });
        // Row background spans the whole width.
        let full = egui::Rect::from_min_max(
            egui::pos2(ui.min_rect().left(), label_rect.top() - 1.0),
            egui::pos2(ui.max_rect().right(), label_rect.bottom() + 1.0),
        );
        rows.push(full);
        let fill = if selected {
            Some(theme::ACCENT.gamma_multiply(0.7))
        } else if i % 2 == 1 {
            Some(ui.visuals().faint_bg_color)
        } else {
            None
        };
        if let Some(f) = fill {
            ui.painter().set(bg, egui::Shape::rect_filled(full, egui::CornerRadius::same(2), f));
        }
    }

    // Drag and drop reordering.
    if let Some(_from) = *drag {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(ptr) = ui.input(|i| i.pointer.interact_pos()) {
            let ins = rows.iter().position(|r| ptr.y < r.center().y).unwrap_or(n);
            if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
                let y = if ins < n { rows[ins].top() } else { last.bottom() };
                ui.painter().line_segment(
                    [egui::pos2(first.left(), y), egui::pos2(first.right(), y)],
                    egui::Stroke::new(2.0, theme::PATH_HOVER),
                );
            }
            if ui.input(|i| i.pointer.any_released()) {
                move_steps(project, sel, ins);
                *drag = None;
            }
        } else {
            *drag = None;
        }
    }
}
