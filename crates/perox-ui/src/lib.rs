//! Runtime for `.pxui` forms. Lays a form out with [taffy] (flexbox for `VBox`/`HBox`,
//! CSS grid for `Grid`) and draws it with egui.
//!
//! The same code draws forms in user apps ([`run`], [`Form::show`]) and in the PeroxIDE
//! designer ([`Form::show_design`]), so the designer is exactly what ships.
//!
//! Every piece of runtime state is a property. A text box's text is its `text`
//! property, a check box's state is `checked`. Code sets properties with [`Form::set`] or
//! the typed helpers on [`Widget`]; values set at runtime override the `.pxui` file.

mod draw;
mod layout;

use std::collections::HashMap;

use perox_forms::{Element, File, Value};

pub use eframe::{self, egui};
pub use perox_forms;
pub use perox_forms::Value as PropValue;

/// Something happened to a widget: a click, a change, Enter in a text box.
#[derive(Debug, Clone, PartialEq)]
pub struct FormEvent {
    /// The widget's id, if it has one.
    pub id: Option<String>,
    /// Child indexes from the root element.
    pub path: Vec<usize>,
    /// `clicked`, `changed`, `submitted`, `toggled`, `activated`.
    pub event: &'static str,
    /// The handler method named in the form (`on clicked: save_clicked`), if any.
    pub handler: Option<String>,
}

/// What the designer gets back each frame.
#[derive(Debug)]
pub struct DesignOutput {
    /// Every laid-out element (containers too), in drawing order: parents before
    /// children. Rects are in screen points.
    pub rects: Vec<(Vec<usize>, egui::Rect)>,
    /// The element under a primary click, deepest first.
    pub clicked: Option<Vec<usize>>,
    /// The element under the pointer.
    pub hovered: Option<Vec<usize>>,
    /// The element a drag started on (the designer moves it).
    pub drag_started: Option<Vec<usize>>,
    /// The whole form's rect.
    pub form_rect: egui::Rect,
}

impl Default for DesignOutput {
    fn default() -> Self {
        Self {
            rects: Vec::new(),
            clicked: None,
            hovered: None,
            drag_started: None,
            form_rect: egui::Rect::NOTHING,
        }
    }
}

/// A parsed form and its runtime state.
#[derive(Debug, Default)]
pub struct Form {
    source: String,
    file: File,
    /// Runtime property values, by widget key (see [`key`]).
    values: HashMap<String, HashMap<String, Value>>,
}

/// The state key for an element: its id, or `#0.1.2` from its path.
pub fn key(element: &Element, path: &[usize]) -> String {
    match &element.id {
        Some(id) => id.clone(),
        None => {
            let parts: Vec<String> = path.iter().map(usize::to_string).collect();
            format!("#{}", parts.join("."))
        }
    }
}

impl Form {
    pub fn new(source: impl Into<String>) -> Self {
        let source = source.into();
        let file = perox_forms::parse(&source);
        Self {
            source,
            file,
            values: HashMap::new(),
        }
    }

    /// Replace the source (the designer calls this on every edit). Runtime values
    /// are kept, so a live preview doesn't lose what you typed.
    pub fn set_source(&mut self, source: &str) {
        if source != self.source {
            self.source = source.to_owned();
            self.file = perox_forms::parse(source);
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    /// The root element's string property, e.g. the window `title`.
    pub fn root_str(&self, name: &str) -> Option<&str> {
        self.file.root()?.str_prop(name)
    }

    /// The window size from `size: 640 x 480`.
    pub fn window_size(&self) -> Option<[f32; 2]> {
        match self.file.root()?.prop("size")? {
            Value::Size(w, h) => Some([*w as f32, *h as f32]),
            _ => None,
        }
    }

    /// The current value of a property: set at runtime, else from the file.
    pub fn get(&self, id: &str, name: &str) -> Option<Value> {
        if let Some(v) = self.values.get(id).and_then(|m| m.get(name)) {
            return Some(v.clone());
        }
        let path = self.file.find(id)?;
        self.file.at(&path)?.prop(name).cloned()
    }

    /// Set a property at runtime. Takes effect next frame.
    pub fn set(&mut self, id: &str, name: &str, value: Value) {
        self.values
            .entry(id.to_owned())
            .or_default()
            .insert(name.to_owned(), value);
    }

    /// Drop all runtime values.
    pub fn reset(&mut self) {
        self.values.clear();
    }

    /// A handle with typed getters and setters for one widget.
    pub fn widget(&mut self, id: &str) -> Widget<'_> {
        Widget {
            form: self,
            id: id.to_owned(),
        }
    }

    /// Draw the form, filling the available space. Returns what the user did.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Vec<FormEvent> {
        let mut events = Vec::new();
        let Some(root) = self.file.root() else {
            self.show_errors(ui);
            return events;
        };
        let viewport = ui.available_rect_before_wrap();
        let built = layout::build(ui, root, &self.values, viewport.size(), false);
        let content = built.content_size().max(viewport.size());
        let (rect, _) = ui.allocate_exact_size(content, egui::Sense::hover());
        draw::draw(
            ui,
            &self.file,
            &mut self.values,
            &built,
            rect.min,
            false,
            &mut events,
        );
        draw::fix_radio_groups(&self.file, &mut self.values, &events);
        events
    }

    /// Draw the form for the designer: widgets look real but don't react; clicks
    /// select. `selected` is outlined.
    pub fn show_design(&mut self, ui: &mut egui::Ui, selected: Option<&[usize]>) -> DesignOutput {
        let mut out = DesignOutput::default();
        let Some(root) = self.file.root() else {
            self.show_errors(ui);
            return out;
        };
        let viewport = ui.available_rect_before_wrap();
        let built = layout::build(ui, root, &self.values, viewport.size(), true);
        let size = built.root_size().max(egui::vec2(40.0, 40.0));
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let mut ignored = Vec::new();
        draw::draw(
            ui,
            &self.file,
            &mut self.values,
            &built,
            rect.min,
            true,
            &mut ignored,
        );
        out.form_rect = rect;
        out.rects = built.element_rects(rect.min);

        // On top of every widget, so widgets never see the pointer.
        let response = ui.interact(
            rect,
            ui.scope_id().with("design_overlay"),
            egui::Sense::click_and_drag(),
        );
        let pointer = response.hover_pos();
        let deepest = |p: egui::Pos2| {
            out.rects
                .iter()
                .rev()
                .find(|(_, r)| r.contains(p))
                .map(|(path, _)| path.clone())
        };
        out.hovered = pointer.and_then(deepest);
        if response.clicked() {
            out.clicked = response.interact_pointer_pos().and_then(deepest);
        }
        if response.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin());
            out.drag_started = origin.and_then(deepest);
        }
        let painter = ui.painter();
        let accent = ui.visuals().selection.bg_fill;
        if let Some(h) = &out.hovered
            && Some(h.as_slice()) != selected
            && let Some((_, r)) = out.rects.iter().find(|(p, _)| p == h)
        {
            painter.rect_stroke(
                *r,
                2.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(sel) = selected
            && let Some((_, r)) = out.rects.iter().find(|(p, _)| p == sel)
        {
            painter.rect_stroke(
                *r,
                2.0,
                egui::Stroke::new(2.0, accent),
                egui::StrokeKind::Outside,
            );
        }
        out
    }

    fn show_errors(&self, ui: &mut egui::Ui) {
        if self.file.errors.is_empty() {
            ui.weak("Empty form");
        }
        for e in &self.file.errors {
            ui.colored_label(ui.visuals().error_fg_color, &e.message);
        }
    }
}

/// Typed access to one widget's properties.
#[derive(Debug)]
pub struct Widget<'a> {
    form: &'a mut Form,
    id: String,
}

impl Widget<'_> {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn text(&self) -> String {
        match self.form.get(&self.id, "text") {
            Some(Value::Str(s) | Value::Ident(s)) => s,
            Some(Value::Num(n)) => n.to_string(),
            _ => String::new(),
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.form.set(&self.id, "text", Value::Str(text.into()));
    }

    pub fn checked(&self) -> bool {
        matches!(self.form.get(&self.id, "checked"), Some(Value::Bool(true)))
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.form.set(&self.id, "checked", Value::Bool(checked));
    }

    /// A slider's, spin box's or progress bar's value.
    pub fn value(&self) -> f64 {
        self.form
            .get(&self.id, "value")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    pub fn set_value(&mut self, value: f64) {
        self.form.set(&self.id, "value", Value::Num(value));
    }

    /// The selected index of a combo box, list or tab set.
    pub fn selected(&self) -> Option<usize> {
        self.form
            .get(&self.id, "selected")
            .and_then(|v| v.as_f64())
            .filter(|n| *n >= 0.0)
            .map(|n| n as usize)
    }

    pub fn set_selected(&mut self, index: Option<usize>) {
        let n = index.map_or(-1.0, |i| i as f64);
        self.form.set(&self.id, "selected", Value::Num(n));
    }

    /// The selected item's text.
    pub fn selected_text(&self) -> Option<String> {
        let i = self.selected()?;
        self.items().into_iter().nth(i)
    }

    pub fn items(&self) -> Vec<String> {
        match self.form.get(&self.id, "items") {
            Some(Value::List(items)) => items
                .iter()
                .map(|v| match v {
                    Value::Str(s) | Value::Ident(s) => s.clone(),
                    other => other.to_source(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn set_items<S: Into<String>>(&mut self, items: impl IntoIterator<Item = S>) {
        let list = items.into_iter().map(|s| Value::Str(s.into())).collect();
        self.form.set(&self.id, "items", Value::List(list));
    }

    pub fn enabled(&self) -> bool {
        !matches!(self.form.get(&self.id, "enabled"), Some(Value::Bool(false)))
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.form.set(&self.id, "enabled", Value::Bool(enabled));
    }

    pub fn visible(&self) -> bool {
        !matches!(self.form.get(&self.id, "visible"), Some(Value::Bool(false)))
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.form.set(&self.id, "visible", Value::Bool(visible));
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.form.get(&self.id, name)
    }

    pub fn set(&mut self, name: &str, value: Value) {
        self.form.set(&self.id, name, value);
    }
}

/// Open a window showing the form and call `on_event` for everything the user does.
/// Window `title`, `size` and `min_size` come from the root `Window` element.
pub fn run(source: &str, on_event: impl FnMut(&mut Form, &FormEvent) + 'static) -> eframe::Result {
    let form = Form::new(source);
    let title = form.root_str("title").unwrap_or("App").to_owned();
    let mut viewport = egui::ViewportBuilder::default().with_title(title.clone());
    if let Some(size) = form.window_size() {
        viewport = viewport.with_inner_size(size);
    }
    if let Some(Value::Size(w, h)) = form.file().root().and_then(|r| r.prop("min_size")) {
        viewport = viewport.with_min_inner_size([*w as f32, *h as f32]);
    }
    if let Some(Value::Bool(false)) = form.file().root().and_then(|r| r.prop("resizable")) {
        viewport = viewport.with_resizable(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(FormApp { form, on_event }))
        }),
    )
}

struct FormApp<F> {
    form: Form,
    on_event: F,
}

impl<F: FnMut(&mut Form, &FormEvent)> eframe::App for FormApp<F> {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0.0))
            .show(ui, |ui| {
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    let events = self.form.show(ui);
                    for e in &events {
                        (self.on_event)(&mut self.form, e);
                    }
                    if !events.is_empty() {
                        ui.ctx().request_repaint();
                    }
                });
            });
    }
}

/// Include a form compiled by `perox_forms::build::compile` in `build.rs`:
/// `perox_ui::include_form!("main_window");`
#[macro_export]
macro_rules! include_form {
    ($name:literal) => {
        include!(concat!(env!("OUT_DIR"), "/", $name, "_ui.rs"));
    };
}
