//! Draws laid-out elements with egui widgets.

use crate::egui;
use egui::{Align, Color32, Layout, Rect, RichText, Stroke, StrokeKind, UiBuilder, vec2};
use perox_forms::{Element, File, Value};

use crate::FormEvent;
use crate::layout::{
    Built, Part, Values, flag, is_container, items, label_font, num, prop, selected, text,
};

fn color(v: Option<&Value>) -> Option<Color32> {
    match v? {
        Value::Color([r, g, b, a]) => Some(Color32::from_rgba_unmultiplied(*r, *g, *b, *a)),
        _ => None,
    }
}

fn set(values: &mut Values, key: &str, name: &str, value: Value) {
    values
        .entry(key.to_owned())
        .or_default()
        .insert(name.to_owned(), value);
}

fn event(el: &Element, path: &[usize], name: &'static str) -> FormEvent {
    FormEvent {
        id: el.id.clone(),
        path: path.to_vec(),
        event: name,
        handler: el
            .handlers
            .iter()
            .find(|h| h.event == name)
            .map(|h| h.method.clone()),
    }
}

pub(crate) fn draw(
    ui: &mut egui::Ui,
    file: &File,
    values: &mut Values,
    built: &Built,
    origin: egui::Pos2,
    design: bool,
    events: &mut Vec<FormEvent>,
) {
    let mut enabled: Vec<bool> = Vec::with_capacity(built.entries.len());
    for e in &built.entries {
        let Some(el) = file.at(&e.path) else {
            enabled.push(true);
            continue;
        };
        let on = e.parent.is_none_or(|p| enabled[p])
            && flag(el, values, &e.key, "enabled") != Some(false);
        enabled.push(on);
        let rect = e.rect.translate(origin.to_vec2());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((&e.key, e.part == Part::TabHeader)),
        );
        if !on {
            child.disable();
        }
        let ui = &mut child;
        if e.part == Part::TabHeader {
            tab_header(ui, el, &e.key, &e.path, values, events);
            continue;
        }
        if is_container(&el.kind) {
            container(ui, el, rect, design);
            continue;
        }
        let response = leaf(ui, el, &e.key, &e.path, rect, values, design, events);
        if !design
            && let Some(r) = response
            && let Some(tip) = el.str_prop("tooltip")
        {
            r.on_hover_text(tip);
        }
    }
}

fn tab_header(
    ui: &mut egui::Ui,
    el: &Element,
    key: &str,
    path: &[usize],
    values: &mut Values,
    events: &mut Vec<FormEvent>,
) {
    let current = selected(el, values, key).unwrap_or(0);
    for (i, tab) in el.children.iter().enumerate() {
        let title = tab.str_prop("title").unwrap_or(&tab.kind);
        if ui.selectable_label(i == current, title).clicked() && i != current {
            set(values, key, "selected", Value::Num(i as f64));
            events.push(event(el, path, "changed"));
        }
    }
    let r = ui.max_rect();
    ui.painter().hline(
        r.x_range(),
        r.bottom() - 0.5,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

fn container(ui: &mut egui::Ui, el: &Element, rect: Rect, design: bool) {
    let visuals = ui.visuals().clone();
    match el.kind.as_str() {
        "GroupBox" => {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let title = el.str_prop("title").unwrap_or("");
            let galley = ui
                .painter()
                .layout_no_wrap(title.to_owned(), font, visuals.text_color());
            let top = rect.top() + galley.size().y / 2.0;
            let frame = Rect::from_min_max(egui::pos2(rect.left(), top), rect.max);
            ui.painter().rect_stroke(
                frame,
                4.0,
                visuals.widgets.noninteractive.bg_stroke,
                StrokeKind::Inside,
            );
            if !title.is_empty() {
                let at = egui::pos2(rect.left() + 8.0, rect.top());
                let bg = Rect::from_min_size(at - vec2(3.0, 0.0), galley.size() + vec2(6.0, 0.0));
                ui.painter().rect_filled(bg, 0.0, visuals.panel_fill);
                ui.painter().galley(at, galley, visuals.text_color());
            }
        }
        "VBox" | "HBox" | "Grid" if design => {
            // Layouts are invisible at runtime; the designer shows them faintly.
            ui.painter().rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.35)),
                StrokeKind::Inside,
            );
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn leaf(
    ui: &mut egui::Ui,
    el: &Element,
    key: &str,
    path: &[usize],
    rect: Rect,
    values: &mut Values,
    design: bool,
    events: &mut Vec<FormEvent>,
) -> Option<egui::Response> {
    let size = rect.size();
    let r = match el.kind.as_str() {
        "Label" => {
            let font = label_font(el, values, key, ui.style());
            let mut rt = RichText::new(text(el, values, key, "text")).font(font);
            if let Some(c) = color(prop(el, values, key, "color")) {
                rt = rt.color(c);
            }
            if flag(el, values, key, "bold") == Some(true) {
                rt = rt.strong();
            }
            let wrap = if flag(el, values, key, "wrap") == Some(true) {
                egui::TextWrapMode::Wrap
            } else {
                egui::TextWrapMode::Truncate
            };
            let layout = match el.str_prop("text_align") {
                Some("center") => Layout::centered_and_justified(egui::Direction::TopDown),
                Some("right") => Layout::right_to_left(Align::Center),
                _ => Layout::left_to_right(Align::Center),
            };
            ui.with_layout(layout, |ui| {
                ui.add(egui::Label::new(rt).wrap_mode(wrap).selectable(false))
            })
            .inner
        }
        "Button" => {
            let r = ui.add(egui::Button::new(text(el, values, key, "text")).min_size(size));
            if r.clicked() {
                events.push(event(el, path, "clicked"));
            }
            r
        }
        "CheckBox" => {
            let mut b = flag(el, values, key, "checked").unwrap_or(false);
            let r = ui.checkbox(&mut b, text(el, values, key, "text"));
            if r.changed() {
                set(values, key, "checked", Value::Bool(b));
                events.push(event(el, path, "toggled"));
            }
            r
        }
        "RadioButton" => {
            let b = flag(el, values, key, "checked").unwrap_or(false);
            let r = ui.radio(b, text(el, values, key, "text"));
            if r.clicked() && !b {
                set(values, key, "checked", Value::Bool(true));
                events.push(event(el, path, "toggled"));
            }
            r
        }
        "TextEdit" => {
            let mut s = text(el, values, key, "text");
            let rows = num(el, values, key, "rows").unwrap_or(1.0);
            let multi = flag(el, values, key, "multiline") == Some(true) || rows > 1.0;
            let mut edit = if multi {
                egui::TextEdit::multiline(&mut s).desired_rows(rows.max(1.0) as usize)
            } else {
                egui::TextEdit::singleline(&mut s)
            };
            edit = edit
                .desired_width(size.x)
                .password(flag(el, values, key, "password") == Some(true))
                .interactive(flag(el, values, key, "read_only") != Some(true));
            if let Some(h) = el.str_prop("placeholder") {
                edit = edit.hint_text(h);
            }
            if flag(el, values, key, "monospace") == Some(true) {
                edit = edit.code_editor();
            }
            let r = ui.add_sized(size, edit);
            if r.changed() {
                set(values, key, "text", Value::Str(s));
                events.push(event(el, path, "changed"));
            }
            if !multi && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                events.push(event(el, path, "submitted"));
            }
            r
        }
        "ComboBox" => {
            let list = items(el, values, key);
            let current = selected(el, values, key);
            let shown = current
                .and_then(|i| list.get(i))
                .cloned()
                .unwrap_or_default();
            let mut picked = None;
            let r = egui::ComboBox::from_id_salt(key)
                .width(size.x)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (i, item) in list.iter().enumerate() {
                        if ui.selectable_label(current == Some(i), item).clicked() {
                            picked = Some(i);
                        }
                    }
                })
                .response;
            if let Some(i) = picked
                && Some(i) != current
            {
                set(values, key, "selected", Value::Num(i as f64));
                events.push(event(el, path, "changed"));
            }
            r
        }
        "Slider" => {
            let min = num(el, values, key, "min").unwrap_or(0.0) as f64;
            let max = num(el, values, key, "max").unwrap_or(100.0) as f64;
            let mut v = num(el, values, key, "value").map_or(min, f64::from);
            let show_value = flag(el, values, key, "show_value") != Some(false);
            ui.spacing_mut().slider_width =
                if show_value { size.x - 60.0 } else { size.x }.max(20.0);
            let mut slider = egui::Slider::new(&mut v, min..=max).show_value(show_value);
            if let Some(step) = num(el, values, key, "step") {
                slider = slider.step_by(step as f64);
            }
            let r = ui.add(slider);
            if r.changed() {
                set(values, key, "value", Value::Num(v));
                events.push(event(el, path, "changed"));
            }
            r
        }
        "SpinBox" => {
            let min = num(el, values, key, "min").map_or(f64::NEG_INFINITY, f64::from);
            let max = num(el, values, key, "max").map_or(f64::INFINITY, f64::from);
            let mut v = num(el, values, key, "value").map_or(0.0, f64::from);
            let step = num(el, values, key, "step").map_or(1.0, f64::from);
            let mut dv = egui::DragValue::new(&mut v).range(min..=max).speed(step);
            if let Some(p) = el.str_prop("prefix") {
                dv = dv.prefix(p);
            }
            if let Some(s) = el.str_prop("suffix") {
                dv = dv.suffix(s);
            }
            let r = ui.add_sized(size, dv);
            if r.changed() {
                set(values, key, "value", Value::Num(v));
                events.push(event(el, path, "changed"));
            }
            r
        }
        "ProgressBar" => {
            let min = num(el, values, key, "min").unwrap_or(0.0);
            let max = num(el, values, key, "max").unwrap_or(100.0);
            let v = num(el, values, key, "value").unwrap_or(min);
            let f = if max > min {
                (v - min) / (max - min)
            } else {
                0.0
            };
            let mut bar = egui::ProgressBar::new(f.clamp(0.0, 1.0)).desired_width(size.x);
            let label = text(el, values, key, "text");
            bar = if label.is_empty() {
                bar.show_percentage()
            } else {
                bar.text(label)
            };
            ui.add(bar)
        }
        "Image" => {
            let source = text(el, values, key, "source");
            if source.is_empty() {
                placeholder(ui, rect, "Image")
            } else {
                let uri = if source.contains("://") {
                    source
                } else {
                    format!("file://{source}")
                };
                ui.add(egui::Image::new(uri).fit_to_exact_size(size))
            }
        }
        "Separator" => {
            let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
            if rect.width() > rect.height() {
                ui.painter().hline(rect.x_range(), rect.center().y, stroke);
            } else {
                ui.painter().vline(rect.center().x, rect.y_range(), stroke);
            }
            return None;
        }
        "ListView" => list_view(ui, el, key, path, rect, values, events),
        "Spacer" => {
            if design {
                spacer_mark(ui, rect);
            }
            return None;
        }
        other => placeholder(ui, rect, other),
    };
    Some(r)
}

fn list_view(
    ui: &mut egui::Ui,
    el: &Element,
    key: &str,
    path: &[usize],
    rect: Rect,
    values: &mut Values,
    events: &mut Vec<FormEvent>,
) -> egui::Response {
    let list = items(el, values, key);
    let current = selected(el, values, key);
    let mut picked = None;
    let mut activated = false;
    let r = egui::Frame::new()
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(2.0)
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.set_min_size(rect.size() - vec2(4.0, 4.0));
            ui.set_max_size(rect.size() - vec2(4.0, 4.0));
            egui::ScrollArea::vertical()
                .id_salt(key)
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
                        for (i, item) in list.iter().enumerate() {
                            let r = ui.selectable_label(current == Some(i), item);
                            if r.clicked() {
                                picked = Some(i);
                            }
                            if r.double_clicked() {
                                picked = Some(i);
                                activated = true;
                            }
                        }
                    });
                });
        })
        .response;
    if let Some(i) = picked {
        if Some(i) != current {
            set(values, key, "selected", Value::Num(i as f64));
            events.push(event(el, path, "changed"));
        }
        if activated {
            events.push(event(el, path, "activated"));
        }
    }
    r
}

/// A box with the widget's kind in it: for images without a source and for widgets
/// this runtime doesn't know (a component whose crate isn't loaded).
fn placeholder(ui: &mut egui::Ui, rect: Rect, label: &str) -> egui::Response {
    let v = ui.visuals();
    ui.painter()
        .rect_stroke(rect, 2.0, v.widgets.inactive.bg_stroke, StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Small.resolve(ui.style()),
        v.weak_text_color(),
    );
    ui.allocate_rect(rect, egui::Sense::hover())
}

/// Spacers are invisible; the designer draws a spring so you can find them.
fn spacer_mark(ui: &mut egui::Ui, rect: Rect) {
    let color = ui.visuals().selection.bg_fill.gamma_multiply(0.7);
    let stroke = Stroke::new(1.0, color);
    let horizontal = rect.width() >= rect.height();
    let (len, mid) = if horizontal {
        (rect.width(), rect.center().y)
    } else {
        (rect.height(), rect.center().x)
    };
    let turns = ((len / 8.0) as usize).clamp(1, 64);
    let mut points = Vec::with_capacity(turns + 1);
    for i in 0..=turns {
        let t = i as f32 / turns as f32;
        let off = if i % 2 == 0 { -3.0 } else { 3.0 };
        points.push(if horizontal {
            egui::pos2(rect.left() + t * len, mid + off)
        } else {
            egui::pos2(mid + off, rect.top() + t * len)
        });
    }
    ui.painter().add(egui::Shape::line(points, stroke));
}

/// After a radio button turns on, turn off the others in its group: the same `group`
/// value, or (without one) the same parent.
pub(crate) fn fix_radio_groups(file: &File, values: &mut Values, events: &[FormEvent]) {
    let Some(root) = file.root() else { return };
    for ev in events {
        let Some(el) = file.at(&ev.path) else {
            continue;
        };
        if el.kind != "RadioButton" || ev.event != "toggled" {
            continue;
        }
        let group = el.str_prop("group").map(str::to_owned);
        let parent = &ev.path[..ev.path.len().saturating_sub(1)];
        let mut off = Vec::new();
        root.walk(&mut Vec::new(), &mut |other, p| {
            if other.kind != "RadioButton" || p == ev.path.as_slice() {
                return;
            }
            let same = match &group {
                Some(g) => other.str_prop("group") == Some(g),
                None => {
                    other.str_prop("group").is_none() && &p[..p.len().saturating_sub(1)] == parent
                }
            };
            if same {
                off.push(crate::key(other, p));
            }
        });
        for k in off {
            set(values, &k, "checked", Value::Bool(false));
        }
    }
}
