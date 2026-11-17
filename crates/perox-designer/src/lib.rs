//! PeroxIDE's form designer.
//!
//! The `.pxui` text is the only state: every gesture (drop a widget, drag it elsewhere,
//! change a property, wire an event) becomes a minimal [`TextEdit`] that the IDE applies
//! to the open document. So undo, the Source view, git diffs and agents all see the same
//! thing, and hand-written formatting and comments survive.
//!
//! The canvas is drawn by `perox-ui`, the runtime that ships in the app, so what you see
//! is what runs.

pub mod catalog;
pub mod drop;
pub mod free;

use std::collections::HashMap;

use eframe::egui::{
    self, Align, Color32, DragAndDrop, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    UiBuilder, Vec2, vec2,
};
use perox_forms::edit::{self, EditError, TextEdit};
use perox_forms::{Element, File, Value};
use perox_icons::Icon;
use perox_ui::Form;

use catalog::{PropKind, PropSpec};

const DEFAULT_SIZE: Vec2 = Vec2::new(640.0, 480.0);
const TITLE_BAR: f32 = 28.0;
const CANVAS_MARGIN: f32 = 24.0;
const LABEL_WIDTH: f32 = 96.0;

/// What the IDE should do after a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Apply these edits to the document (one undo step).
    Edit(Vec<TextEdit>),
    /// Show the handler `method` in the code-behind, adding it if it's missing.
    OpenHandler {
        /// The generated type, e.g. `MainWindow` (the trait is `MainWindowHandlers`).
        form: String,
        method: String,
    },
    Undo,
    Redo,
}

/// What's being dragged.
#[derive(Debug, Clone)]
enum Drag {
    New(&'static str),
    Move(Vec<usize>),
}

#[derive(Debug, Default)]
pub struct Designer {
    form: Form,
    /// The selected element's path (`[]` is the window).
    pub selected: Option<Vec<usize>>,
    /// Window size while the corner handle is being dragged.
    resize: Option<Vec2>,
    resize_from: Option<Vec2>,
    /// Values being dragged (inspector numbers, widget resizing): shown live, written
    /// on release.
    live: Vec<(Vec<usize>, String, Value)>,
    /// Where in the widget the pointer grabbed it (free placement keeps that offset).
    grab: Vec2,
    /// A widget being resized by its handles: path, which sides move, start rect
    /// (relative to the parent's content box).
    widget_resize: Option<(Vec<usize>, [i8; 2], Rect)>,
    /// Snap free placement to the grid. On unless turned off.
    snap_off: bool,
    /// Inspector text fields while they're being edited.
    buffers: HashMap<String, String>,
    palette_filter: String,
    /// The last edit that couldn't be made.
    pub error: Option<String>,
    /// Where things were drawn last frame (for tests and agents).
    pub layout: DrawnLayout,
}

/// Screen rects from the last frame.
#[derive(Debug, Default, Clone)]
pub struct DrawnLayout {
    /// Every element on the canvas, parents first.
    pub elements: Vec<(Vec<usize>, Rect)>,
    pub palette: Vec<(&'static str, Rect)>,
    pub window: Option<Rect>,
    /// Content boxes, same order as `elements`.
    pub contents: Vec<Rect>,
}

impl Designer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The form as the designer last saw it.
    pub fn file(&self) -> &File {
        self.form.file()
    }

    /// Draw the whole designer (palette, canvas, tree, inspector) for `source`.
    /// `stem` is the file name without `.pxui`, the fallback form name.
    pub fn show(&mut self, ui: &mut egui::Ui, source: &str, stem: &str) -> Vec<Action> {
        self.form.set_source(source);
        if let Some(sel) = &self.selected
            && self.form.file().at(sel).is_none()
        {
            self.selected = None;
        }
        let mut actions = Vec::new();
        let base = ui.scope_id().with("designer");

        egui::Panel::left(base.with("palette"))
            .resizable(true)
            .default_size(150.0)
            .show(ui, |ui| self.palette(ui, source, &mut actions));
        egui::Panel::right(base.with("inspector"))
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.strong("Objects");
                egui::ScrollArea::vertical()
                    .id_salt("objects")
                    .max_height(ui.available_height() * 0.45)
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.object_tree(ui));
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("inspector")
                    .auto_shrink(false)
                    .show(ui, |ui| self.inspector(ui, source, stem, &mut actions));
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui, source, &mut actions);
            self.canvas(ui, source, &mut actions);
        });
        self.keyboard(ui, source, &mut actions);
        self.drag_preview(ui);
        actions
    }

    fn apply(
        &mut self,
        actions: &mut Vec<Action>,
        result: Result<Vec<TextEdit>, EditError>,
    ) -> bool {
        match result {
            Ok(edits) if edits.is_empty() => false,
            Ok(edits) => {
                self.error = None;
                actions.push(Action::Edit(edits));
                true
            }
            Err(e) => {
                self.error = Some(e.to_string());
                false
            }
        }
    }

    // ---- palette ----

    fn palette(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.palette_filter)
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
        let filter = self.palette_filter.to_lowercase();
        self.layout.palette.clear();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (title, layouts) in [("Layouts", true), ("Widgets", false)] {
                    ui.add_space(6.0);
                    ui.label(RichText::new(title).small().weak());
                    for k in catalog::KINDS.iter().filter(|k| k.layout == layouts) {
                        if !filter.is_empty() && !k.name.to_lowercase().contains(&filter) {
                            continue;
                        }
                        let r = palette_row(ui, k.icon, k.name);
                        self.layout.palette.push((k.name, r.rect));
                        if r.drag_started() {
                            DragAndDrop::set_payload(ui.ctx(), Drag::New(k.name));
                        }
                        if r.double_clicked() {
                            self.insert_at_selection(source, k.name, actions);
                        }
                        r.on_hover_text(
                            "Drag onto the form, or double-click to add to the selection",
                        );
                    }
                }
            });
    }

    /// Double-click in the palette: into the selected container, else after the
    /// selected widget, else into the window's layout.
    fn insert_at_selection(&mut self, source: &str, kind: &str, actions: &mut Vec<Action>) {
        let file = self.form.file();
        let Some(root) = file.root() else { return };
        let mut sel = self.selected.clone().unwrap_or_default();
        if sel.is_empty()
            && root.children.len() == 1
            && catalog::accepts(&root.children[0].kind, kind)
        {
            sel = vec![0];
        }
        let (parent, index) = match file.at(&sel) {
            Some(el) if catalog::accepts(&el.kind, kind) => (sel.clone(), el.children.len()),
            Some(_) if !sel.is_empty() => {
                let mut parent = sel.clone();
                let last = parent.pop().unwrap_or(0);
                (parent, last + 1)
            }
            _ => return,
        };
        if file
            .at(&parent)
            .is_some_and(|p| catalog::accepts(&p.kind, kind))
        {
            self.add(source, kind, &parent, index, actions);
        } else {
            self.error = Some(format!("A {kind} can't go there"));
        }
    }

    fn add(
        &mut self,
        source: &str,
        kind: &str,
        parent: &[usize],
        index: usize,
        actions: &mut Vec<Action>,
    ) {
        let file = self.form.file();
        let id = catalog::wants_id(kind).then(|| unique_id(file, &catalog::snake(kind)));
        let props = catalog::defaults(kind);
        let count = file.at(parent).map_or(0, |p| p.children.len());
        if self.apply(
            actions,
            edit::add_widget(source, parent, index, kind, id.as_deref(), &props),
        ) {
            let mut path = parent.to_vec();
            path.push(index.min(count));
            self.selected = Some(path);
        }
    }

    // ---- toolbar and keys ----

    fn toolbar(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if icon_button(ui, Icon::Undo2, "Undo (Cmd+Z)").clicked() {
                actions.push(Action::Undo);
            }
            if icon_button(ui, Icon::Redo2, "Redo (Shift+Cmd+Z)").clicked() {
                actions.push(Action::Redo);
            }
            ui.separator();
            let can_delete = self.selected.as_ref().is_some_and(|s| !s.is_empty());
            if ui
                .add_enabled_ui(can_delete, |ui| {
                    icon_button(ui, Icon::Trash2, "Delete (Del)")
                })
                .inner
                .clicked()
            {
                self.delete_selected(source, actions);
            }
            ui.separator();
            let sel = self.selected.clone().unwrap_or_default();
            let sel_el = self.form.file().at(&sel);
            let free_sel = sel_el.is_some_and(perox_ui::is_free);
            let can_break = sel_el.is_some_and(|e| {
                (matches!(e.kind.as_str(), "VBox" | "HBox" | "Grid") && !sel.is_empty())
                    || (matches!(e.kind.as_str(), "Window" | "GroupBox" | "Tab")
                        && !perox_ui::is_free(e))
            });
            if ui
                .add_enabled_ui(can_break, |ui| {
                    icon_button(
                        ui,
                        Icon::Ungroup,
                        "Break layout: place the children freely, where they are",
                    )
                })
                .inner
                .clicked()
            {
                self.relayout(source, None, actions);
            }
            for (dir, icon, tip) in [
                (free::Direction::Vertical, Icon::Rows3, "Lay out vertically"),
                (
                    free::Direction::Horizontal,
                    Icon::Columns3,
                    "Lay out horizontally",
                ),
                (free::Direction::Grid, Icon::Grid3x3, "Lay out in a grid"),
            ] {
                if ui
                    .add_enabled_ui(free_sel, |ui| icon_button(ui, icon, tip))
                    .inner
                    .clicked()
                {
                    self.relayout(source, Some(dir), actions);
                }
            }
            let snap = egui::Button::image(Icon::Magnet.image(ui))
                .selected(!self.snap_off)
                .frame(false);
            if ui
                .add(snap)
                .on_hover_text("Snap free placement to the grid (8 pt)")
                .clicked()
            {
                self.snap_off = !self.snap_off;
            }
            if let Some(e) = &self.error {
                ui.separator();
                ui.add(perox_icons::Icon::TriangleAlert.colored(ui.visuals().warn_fg_color));
                ui.colored_label(ui.visuals().warn_fg_color, e);
            } else if let Some(err) = self.form.file().errors.first() {
                ui.separator();
                ui.add(perox_icons::Icon::CircleX.colored(ui.visuals().error_fg_color));
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{} (fix it in Source)", err.message),
                );
            }
        });
        ui.add_space(2.0);
    }

    fn delete_selected(&mut self, source: &str, actions: &mut Vec<Action>) {
        let Some(sel) = self.selected.clone().filter(|s| !s.is_empty()) else {
            return;
        };
        if self.apply(actions, edit::remove_widget(source, &sel)) {
            let mut parent = sel;
            parent.pop();
            self.selected = Some(parent);
        }
    }

    fn keyboard(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        // Only when no text field has the keyboard.
        if ui.memory(|m| m.focused().is_some()) {
            return;
        }
        let (delete, escape, undo, redo) = ui.input_mut(|i| {
            let cmd = egui::Modifiers::COMMAND;
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                i.consume_key(cmd, egui::Key::Z),
                i.consume_key(cmd | egui::Modifiers::SHIFT, egui::Key::Z),
            )
        });
        if delete {
            self.delete_selected(source, actions);
        }
        self.nudge(ui, source, actions);
        if escape && let Some(sel) = &mut self.selected {
            sel.pop();
        }
        if undo {
            actions.push(Action::Undo);
        }
        if redo {
            actions.push(Action::Redo);
        }
    }

    /// Arrow keys move a freely placed widget 1 pt (a grid step with Shift).
    fn nudge(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        let Some(path) = self.selected.clone().filter(|p| self.in_free_parent(p)) else {
            return;
        };
        let mut d = Vec2::ZERO;
        ui.input_mut(|i| {
            for (key, dir) in [
                (egui::Key::ArrowLeft, vec2(-1.0, 0.0)),
                (egui::Key::ArrowRight, vec2(1.0, 0.0)),
                (egui::Key::ArrowUp, vec2(0.0, -1.0)),
                (egui::Key::ArrowDown, vec2(0.0, 1.0)),
            ] {
                if i.consume_key(egui::Modifiers::NONE, key) {
                    d += dir;
                }
                if i.consume_key(egui::Modifiers::SHIFT, key) {
                    d += dir * perox_ui::GRID;
                }
            }
        });
        if d == Vec2::ZERO {
            return;
        }
        let Some(el) = self.form.file().at(&path) else {
            return;
        };
        let x = el.num_prop("x").unwrap_or(0.0) as f32 + d.x;
        let y = el.num_prop("y").unwrap_or(0.0) as f32 + d.y;
        let mut work = source.to_owned();
        let result =
            free::set_position(&mut work, &path, Pos2::new(x.max(0.0), y.max(0.0))).map(|()| work);
        self.apply_text(actions, source, result);
    }

    fn drag_preview(&self, ui: &egui::Ui) {
        let Some(drag) = DragAndDrop::payload::<Drag>(ui.ctx()) else {
            return;
        };
        let Some(pos) = ui.ctx().pointer_interact_pos() else {
            return;
        };
        let kind = match &*drag {
            Drag::New(k) => (*k).to_owned(),
            Drag::Move(p) => self
                .form
                .file()
                .at(p)
                .map_or_else(String::new, |e| e.kind.clone()),
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            Id::unique("designer_drag"),
        ));
        let galley = painter.layout_no_wrap(
            kind.clone(),
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().strong_text_color(),
        );
        let rect = Rect::from_min_size(pos + vec2(14.0, 10.0), galley.size() + vec2(30.0, 8.0));
        painter.rect(
            rect,
            4.0,
            ui.visuals().window_fill,
            ui.visuals().window_stroke,
            StrokeKind::Outside,
        );
        let icon_rect =
            Rect::from_min_size(rect.min + vec2(6.0, 4.0), Vec2::splat(galley.size().y));
        egui::Image::new(catalog::icon(&kind).source())
            .tint(ui.visuals().text_color())
            .paint_at(ui, icon_rect);
        painter.galley(rect.min + vec2(24.0, 4.0), galley, Color32::WHITE);
    }

    // ---- canvas ----

    fn canvas(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        // Runtime values in the designer: only the live inspector value and which tab
        // page is showing (the one holding the selection).
        self.form.reset();
        for (path, name, value) in &self.live {
            if let Some(el) = self.form.file().at(path) {
                let key = perox_ui::key(el, path);
                self.form.set(&key, name, value.clone());
            }
        }
        if let Some(sel) = self.selected.clone() {
            for depth in 0..sel.len() {
                let tabs_path = &sel[..depth];
                if let Some(el) = self.form.file().at(tabs_path)
                    && el.kind == "Tabs"
                {
                    let key = perox_ui::key(el, tabs_path);
                    self.form
                        .set(&key, "selected", Value::Num(sel[depth] as f64));
                }
            }
        }

        let file_size = self.form.window_size().map(|[w, h]| vec2(w, h));
        let size = self.resize.or(file_size).unwrap_or(DEFAULT_SIZE);
        let title = self.form.root_str("title").unwrap_or("").to_owned();
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            let total = size + vec2(2.0 * CANVAS_MARGIN, 2.0 * CANVAS_MARGIN + TITLE_BAR);
            let (outer, bg) = ui.allocate_exact_size(total, Sense::click());
            if bg.clicked() {
                self.selected = Some(Vec::new());
            }
            let window = Rect::from_min_size(
                outer.min + Vec2::splat(CANVAS_MARGIN),
                size + vec2(0.0, TITLE_BAR),
            );
            let content = Rect::from_min_size(window.min + vec2(0.0, TITLE_BAR), size);
            window_chrome(ui, window, &title, self.selected.as_deref() == Some(&[]));

            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(content)
                    .layout(Layout::top_down(Align::Min))
                    .id_salt("form"),
            );
            child.set_clip_rect(content.intersect(ui.clip_rect()));
            let out = self.form.show_design(&mut child, self.selected.as_deref());
            self.layout.elements = out.rects.clone();
            self.layout.contents = out.contents.clone();
            self.layout.window = Some(window);
            if let Some(p) = out.clicked {
                self.selected = Some(p);
                self.error = None;
            }
            if let Some(p) = out.drag_started.filter(|p| !p.is_empty()) {
                let origin = ui.input(|i| i.pointer.press_origin());
                if let (Some(o), Some(r)) = (origin, self.drawn_rect(&p)) {
                    self.grab = o - r.min;
                }
                self.selected = Some(p.clone());
                DragAndDrop::set_payload(ui.ctx(), Drag::Move(p));
            }
            self.drop_onto(ui, content, &out.rects, source, actions);
            self.resize_widget(ui, source, actions);
            self.resize_handle(ui, window, size, source, actions);
        });
    }

    fn drop_onto(
        &mut self,
        ui: &mut egui::Ui,
        content: Rect,
        rects: &[(Vec<usize>, Rect)],
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        let Some(drag) = DragAndDrop::payload::<Drag>(ui.ctx()) else {
            return;
        };
        let Some(pointer) = ui
            .ctx()
            .pointer_hover_pos()
            .filter(|p| content.contains(*p))
        else {
            return;
        };
        let file = self.form.file();
        let (kind, moving) = match &*drag {
            Drag::New(k) => ((*k).to_owned(), None),
            Drag::Move(p) => match file.at(p) {
                Some(el) => (el.kind.clone(), Some(p.clone())),
                None => return,
            },
        };
        let Some(target) = drop::target(file, rects, pointer, &kind, moving.as_deref()) else {
            return;
        };
        let accent = ui.visuals().selection.bg_fill;
        if target.free {
            self.drop_free(ui, pointer, &kind, moving, &target, source, actions);
            return;
        }
        ui.painter()
            .line_segment(target.line, Stroke::new(3.0, accent));
        if let Some(r) = rects
            .iter()
            .find(|(p, _)| *p == target.parent)
            .map(|(_, r)| *r)
        {
            ui.painter().rect_stroke(
                r,
                2.0,
                Stroke::new(1.0, accent.gamma_multiply(0.5)),
                StrokeKind::Inside,
            );
        }
        if !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        DragAndDrop::clear_payload(ui.ctx());
        match moving {
            None => self.add(source, &kind, &target.parent, target.index, actions),
            Some(from) => {
                if drop::is_noop(&from, &target.parent, target.index) {
                    return;
                }
                // Out of a free container: its position means nothing in a layout.
                let new_path = drop::moved_path(&from, &target.parent, target.index);
                let mut work = source.to_owned();
                let result = free::step(
                    &mut work,
                    edit::move_widget(source, &from, &target.parent, target.index),
                )
                .and_then(|()| free::strip(&mut work, &new_path, &["x", "y"]))
                .map(|()| work);
                if self.apply_text(actions, source, result) {
                    self.selected = Some(new_path);
                }
            }
        }
    }

    fn drawn_rect(&self, path: &[usize]) -> Option<Rect> {
        self.layout
            .elements
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, r)| *r)
    }

    fn drawn_content(&self, path: &[usize]) -> Option<Rect> {
        let i = self.layout.elements.iter().position(|(p, _)| p == path)?;
        self.layout.contents.get(i).copied()
    }

    /// Apply a multi-step result (the final text) as one edit.
    fn apply_text(
        &mut self,
        actions: &mut Vec<Action>,
        source: &str,
        result: Result<String, EditError>,
    ) -> bool {
        let edits = result.map(|new| edit::diff(source, &new));
        self.apply(actions, edits)
    }

    fn snap(&self, v: f32) -> f32 {
        free::snap(v, !self.snap_off)
    }

    /// Is the element at `path` placed freely (its parent has no layout)?
    fn in_free_parent(&self, path: &[usize]) -> bool {
        let Some((_, parent)) = path.split_last() else {
            return false;
        };
        self.form.file().at(parent).is_some_and(perox_ui::is_free)
    }

    /// Dropping into a free container: a ghost where it will land, then `x`, `y`.
    #[allow(clippy::too_many_arguments)]
    fn drop_free(
        &mut self,
        ui: &mut egui::Ui,
        pointer: Pos2,
        kind: &str,
        moving: Option<Vec<usize>>,
        target: &drop::Target,
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        let Some(content) = self.drawn_content(&target.parent) else {
            return;
        };
        let size = match &moving {
            Some(p) => self.drawn_rect(p).map_or(vec2(80.0, 24.0), |r| r.size()),
            None => catalog::free_default_size(kind)
                .map_or(vec2(80.0, 24.0), |(w, h)| vec2(w as f32, h as f32)),
        };
        let grab = if moving.is_some() {
            self.grab
        } else {
            Vec2::ZERO
        };
        let rel = pointer - grab - content.min;
        let at = Pos2::new(self.snap(rel.x), self.snap(rel.y));
        let accent = ui.visuals().selection.bg_fill;
        let ghost = Rect::from_min_size(content.min + at.to_vec2(), size);
        ui.painter().rect(
            ghost,
            2.0,
            accent.gamma_multiply(0.15),
            Stroke::new(1.5, accent),
            StrokeKind::Inside,
        );
        ui.painter().text(
            ghost.left_top() + vec2(0.0, -2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{}, {}", at.x, at.y),
            egui::TextStyle::Small.resolve(ui.style()),
            accent,
        );
        if !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        DragAndDrop::clear_payload(ui.ctx());
        let parent = target.parent.clone();
        match moving {
            None => {
                let file = self.form.file();
                let id = catalog::wants_id(kind).then(|| unique_id(file, &catalog::snake(kind)));
                let mut props = catalog::defaults(kind);
                props.push(("x".into(), Value::Num(at.x as f64)));
                props.push(("y".into(), Value::Num(at.y as f64)));
                if let Some((w, h)) = catalog::free_default_size(kind) {
                    props.push(("width".into(), Value::Num(w)));
                    props.push(("height".into(), Value::Num(h)));
                }
                let count = file.at(&parent).map_or(0, |p| p.children.len());
                if self.apply(
                    actions,
                    edit::add_widget(source, &parent, usize::MAX, kind, id.as_deref(), &props),
                ) {
                    let mut path = parent;
                    path.push(count);
                    self.selected = Some(path);
                }
            }
            Some(from) => {
                let same_parent = from
                    .split_last()
                    .is_some_and(|(_, p)| p == parent.as_slice());
                let mut work = source.to_owned();
                let new_path = if same_parent {
                    from.clone()
                } else {
                    drop::moved_path(&from, &parent, target.index)
                };
                let result = (|| {
                    if !same_parent {
                        free::step(
                            &mut work,
                            edit::move_widget(source, &from, &parent, target.index),
                        )?;
                        free::strip(
                            &mut work,
                            &new_path,
                            &["row", "column", "row_span", "column_span", "stretch"],
                        )?;
                    }
                    free::set_position(&mut work, &new_path, at)?;
                    Ok(work)
                })();
                if self.apply_text(actions, source, result) {
                    self.selected = Some(new_path);
                }
            }
        }
    }

    /// Eight handles around a freely placed selection; dragging one resizes it.
    fn resize_widget(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        let Some(path) = self.selected.clone().filter(|p| self.in_free_parent(p)) else {
            return;
        };
        let (Some(rect), Some(parent_content)) = (
            self.drawn_rect(&path),
            path.split_last().and_then(|(_, p)| self.drawn_content(p)),
        ) else {
            return;
        };
        let accent = ui.visuals().selection.bg_fill;
        for sx in [-1i8, 0, 1] {
            for sy in [-1i8, 0, 1] {
                if sx == 0 && sy == 0 {
                    continue;
                }
                let at = Pos2::new(
                    [rect.left(), rect.center().x, rect.right()][(sx + 1) as usize],
                    [rect.top(), rect.center().y, rect.bottom()][(sy + 1) as usize],
                );
                let handle = Rect::from_center_size(at, Vec2::splat(7.0));
                let id = ui.scope_id().with(("resize_widget", sx, sy));
                let r = ui.interact(handle.expand(2.0), id, Sense::drag());
                ui.painter().rect(
                    handle,
                    1.0,
                    ui.visuals().panel_fill,
                    Stroke::new(1.0, accent),
                    StrokeKind::Inside,
                );
                if r.hovered() || r.dragged() {
                    ui.ctx().set_cursor_icon(match (sx, sy) {
                        (0, _) => egui::CursorIcon::ResizeVertical,
                        (_, 0) => egui::CursorIcon::ResizeHorizontal,
                        (-1, -1) | (1, 1) => egui::CursorIcon::ResizeNwSe,
                        _ => egui::CursorIcon::ResizeNeSw,
                    });
                }
                if r.drag_started() {
                    let start = rect.translate(-parent_content.min.to_vec2());
                    self.widget_resize = Some((path.clone(), [sx, sy], start));
                }
                if r.dragged()
                    && let Some((p, [sx, sy], start)) = self.widget_resize.clone()
                    && let (Some(o), Some(now)) = (
                        ui.input(|i| i.pointer.press_origin()),
                        r.interact_pointer_pos(),
                    )
                {
                    let d = now - o;
                    let mut nr = start;
                    if sx < 0 {
                        nr.min.x = self.snap(start.min.x + d.x).min(start.max.x - 8.0);
                    } else if sx > 0 {
                        nr.max.x = self.snap(start.max.x + d.x).max(start.min.x + 8.0);
                    }
                    if sy < 0 {
                        nr.min.y = self.snap(start.min.y + d.y).min(start.max.y - 8.0);
                    } else if sy > 0 {
                        nr.max.y = self.snap(start.max.y + d.y).max(start.min.y + 8.0);
                    }
                    self.live = vec![
                        (p.clone(), "x".into(), Value::Num(nr.min.x as f64)),
                        (p.clone(), "y".into(), Value::Num(nr.min.y as f64)),
                        (p.clone(), "width".into(), Value::Num(nr.width() as f64)),
                        (p, "height".into(), Value::Num(nr.height() as f64)),
                    ];
                }
                if r.drag_stopped()
                    && let Some((p, _, _)) = self.widget_resize.take()
                {
                    let get = |name: &str| {
                        self.live
                            .iter()
                            .find(|(_, n, _)| n == name)
                            .and_then(|(_, _, v)| v.as_f64())
                            .unwrap_or(0.0) as f32
                    };
                    let nr = Rect::from_min_size(
                        Pos2::new(get("x"), get("y")),
                        vec2(get("width"), get("height")),
                    );
                    self.live.clear();
                    let mut work = source.to_owned();
                    let result = free::set_geometry(&mut work, &p, nr).map(|()| work);
                    self.apply_text(actions, source, result);
                }
            }
        }
    }

    /// Break Layout, or Lay Out Vertically / Horizontally / in a Grid, on the selection.
    fn relayout(&mut self, source: &str, dir: Option<free::Direction>, actions: &mut Vec<Action>) {
        let path = self.selected.clone().unwrap_or_default();
        let rects = self.layout.elements.clone();
        let contents = self.layout.contents.clone();
        let drawn = free::Drawn {
            rects: &rects,
            contents: &contents,
        };
        let result = match dir {
            None => free::break_layout(source, drawn, &path),
            Some(d) => free::lay_out(source, drawn, &path, d),
        };
        if self.apply_text(actions, source, result) {
            self.selected = Some(path);
        }
    }

    fn resize_handle(
        &mut self,
        ui: &mut egui::Ui,
        window: Rect,
        size: Vec2,
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        let handle = Rect::from_center_size(window.right_bottom(), Vec2::splat(14.0));
        let r = ui.interact(handle, ui.scope_id().with("resize_window"), Sense::drag());
        let color = if r.hovered() || r.dragged() {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().weak_text_color()
        };
        for i in 1..=3 {
            let d = i as f32 * 4.0;
            ui.painter().line_segment(
                [
                    window.right_bottom() - vec2(d, 1.0),
                    window.right_bottom() - vec2(1.0, d),
                ],
                Stroke::new(1.0, color),
            );
        }
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        }
        if r.drag_started() {
            self.resize_from = Some(size);
        }
        if r.dragged()
            && let (Some(start), Some(origin), Some(now)) = (
                self.resize_from,
                ui.input(|i| i.pointer.press_origin()),
                r.interact_pointer_pos(),
            )
        {
            let new = (start + (now - origin)).max(vec2(120.0, 80.0));
            self.resize = Some(new.round());
        }
        if r.drag_stopped()
            && let Some(s) = self.resize.take()
        {
            let value = Value::Size(s.x as f64, s.y as f64);
            self.apply(actions, edit::set_property(source, &[], "size", &value));
        }
    }

    // ---- object tree ----

    fn object_tree(&mut self, ui: &mut egui::Ui) {
        let Some(root) = self.form.file().root().cloned() else {
            ui.weak("No form");
            return;
        };
        let mut path = Vec::new();
        let mut clicked = None;
        tree_node(ui, &root, &mut path, self.selected.as_deref(), &mut clicked);
        if let Some(p) = clicked {
            self.selected = Some(p);
            self.error = None;
        }
    }

    // ---- inspector ----

    fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        source: &str,
        stem: &str,
        actions: &mut Vec<Action>,
    ) {
        let Some(path) = self.selected.clone() else {
            ui.weak("Select a widget to edit it.");
            return;
        };
        let Some(el) = self.form.file().at(&path).cloned() else {
            return;
        };
        let spec = catalog::kind(&el.kind);
        let parent_is_grid = !path.is_empty()
            && self
                .form
                .file()
                .at(&path[..path.len() - 1])
                .is_some_and(|p| p.kind == "Grid");

        ui.horizontal(|ui| {
            ui.add(catalog::icon(&el.kind).image(ui));
            ui.strong(&el.kind);
        });
        ui.horizontal(|ui| {
            label_cell(ui, RichText::new("Name")).on_hover_text("The id your code uses: ui.name()");
            let current = el.id.clone().unwrap_or_default();
            if let Some(new) = self.text_field(ui, &format!("{path:?}/#id"), &current, false) {
                let id = new.trim();
                let id = (!id.is_empty()).then_some(id);
                self.apply(actions, edit::set_id(source, &path, id));
            }
        });

        let known: Vec<PropSpec> = spec.map(|k| k.props.to_vec()).unwrap_or_default();
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Properties")
            .default_open(true)
            .show(ui, |ui| {
                self.prop_grid(ui, "props", &el, &path, &known, source, actions)
            });
        let free_child = self.in_free_parent(&path);
        let mut layout: Vec<PropSpec> = Vec::new();
        if free_child {
            layout.extend_from_slice(catalog::FREE_CHILD);
        }
        layout.extend_from_slice(catalog::COMMON);
        if parent_is_grid {
            layout.extend_from_slice(catalog::GRID_CHILD);
        }
        if !path.is_empty() {
            egui::CollapsingHeader::new("Size and layout")
                .default_open(free_child)
                .show(ui, |ui| {
                    self.prop_grid(ui, "layout", &el, &path, &layout, source, actions)
                });
        }
        // Properties in the file the catalog doesn't know (components, typos).
        let other: Vec<PropSpec> = el
            .props
            .iter()
            .filter(|p| {
                !known
                    .iter()
                    .chain(&layout)
                    .any(|k| k.name == p.name.as_str())
            })
            .map(|p| PropSpec {
                name: p.name.clone().into(),
                kind: PropKind::Source,
                help: "Not a known property of this widget",
            })
            .collect();
        if !other.is_empty() {
            egui::CollapsingHeader::new("Other")
                .default_open(true)
                .show(ui, |ui| {
                    self.prop_grid(ui, "other", &el, &path, &other, source, actions)
                });
        }

        let events = spec.map(|k| k.events).unwrap_or_default();
        if !events.is_empty() {
            egui::CollapsingHeader::new("Events")
                .default_open(true)
                .show(ui, |ui| {
                    self.events(ui, &el, &path, events, source, stem, actions)
                });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prop_grid(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        el: &Element,
        path: &[usize],
        specs: &[PropSpec],
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        let _ = id;
        for spec in specs {
            let value = el.prop(&spec.name).cloned();
            let name = if value.is_some() {
                RichText::new(spec.name.as_ref()).strong()
            } else {
                RichText::new(spec.name.as_ref()).weak()
            };
            ui.horizontal(|ui| {
                let r = label_cell(ui, name);
                if !spec.help.is_empty() {
                    r.on_hover_text(spec.help);
                }
                let new = self.value_editor(ui, path, spec, value.as_ref());
                match new {
                    // Switching a container between stack and free converts its
                    // children too, like Break Layout / Lay Out.
                    Some(Value::Ident(mode)) if spec.name == "layout" => {
                        let was_free = el.str_prop("layout") == Some("free");
                        let dir = match (mode.as_str(), was_free) {
                            ("free", false) => Some(None),
                            ("stack", true) => Some(Some(free::Direction::Vertical)),
                            _ => None,
                        };
                        if let Some(dir) = dir {
                            self.selected = Some(path.to_vec());
                            self.relayout(source, dir, actions);
                        }
                    }
                    Some(v) => {
                        self.apply(actions, edit::set_property(source, path, &spec.name, &v));
                    }
                    None => {}
                }
                if value.is_some()
                    && small_icon_button(ui, Icon::X, "Reset to the default").clicked()
                {
                    self.apply(actions, edit::remove_property(source, path, &spec.name));
                }
            });
        }
    }

    /// An editor for one property. Returns a new value to write.
    fn value_editor(
        &mut self,
        ui: &mut egui::Ui,
        path: &[usize],
        spec: &PropSpec,
        value: Option<&Value>,
    ) -> Option<Value> {
        let key = format!("{path:?}/{}", spec.name);
        match spec.kind {
            PropKind::Text => {
                let current = value.and_then(Value::as_str).unwrap_or_default().to_owned();
                self.text_field(ui, &key, &current, false).map(Value::Str)
            }
            PropKind::Number => {
                let live = self
                    .live
                    .iter()
                    .find(|(p, n, _)| p == path && *n == spec.name)
                    .and_then(|(_, _, v)| v.as_f64());
                let mut n = live.or(value.and_then(Value::as_f64)).unwrap_or(0.0);
                let r = ui.add(egui::DragValue::new(&mut n).speed(0.5));
                if r.dragged() && r.changed() {
                    self.live = vec![(path.to_vec(), spec.name.to_string(), Value::Num(n))];
                    None
                } else if r.drag_stopped() {
                    self.live.clear();
                    Some(Value::Num(n))
                } else if r.changed() {
                    Some(Value::Num(n))
                } else {
                    None
                }
            }
            PropKind::Bool => {
                let mut b = value.and_then(Value::as_bool).unwrap_or(false);
                ui.checkbox(&mut b, "").changed().then_some(Value::Bool(b))
            }
            PropKind::Color => {
                let mut c = match value {
                    Some(Value::Color([r, g, b, a])) => {
                        Color32::from_rgba_unmultiplied(*r, *g, *b, *a)
                    }
                    _ => ui.visuals().text_color(),
                };
                egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut c,
                    egui::color_picker::Alpha::OnlyBlend,
                )
                .changed()
                .then(|| Value::Color(c.to_srgba_unmultiplied()))
            }
            PropKind::Size => {
                let (mut w, mut h) = match value {
                    Some(Value::Size(w, h)) => (*w, *h),
                    _ => (0.0, 0.0),
                };
                ui.horizontal(|ui| {
                    let a = ui.add(egui::DragValue::new(&mut w).range(0.0..=10000.0));
                    ui.label("x");
                    let b = ui.add(egui::DragValue::new(&mut h).range(0.0..=10000.0));
                    let done =
                        |r: &egui::Response| r.drag_stopped() || (r.changed() && !r.dragged());
                    (done(&a) || done(&b)).then_some(Value::Size(w, h))
                })
                .inner
            }
            PropKind::Items => {
                let current = match value {
                    Some(Value::List(items)) => items
                        .iter()
                        .map(|v| v.as_str().map_or_else(|| v.to_source(), str::to_owned))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                };
                self.text_field(ui, &key, &current, true).map(|t| {
                    Value::List(
                        t.lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(|l| Value::Str(l.to_owned()))
                            .collect(),
                    )
                })
            }
            PropKind::Choice(options) => {
                let current = value.and_then(Value::as_str).map(str::to_owned);
                let mut picked = None;
                egui::ComboBox::from_id_salt(&key)
                    .selected_text(current.clone().unwrap_or_else(|| "default".into()))
                    .show_ui(ui, |ui| {
                        for o in options {
                            if ui
                                .selectable_label(current.as_deref() == Some(o), *o)
                                .clicked()
                            {
                                picked = Some(Value::Ident((*o).to_owned()));
                            }
                        }
                    });
                picked
            }
            PropKind::Source => {
                let current = value.map(Value::to_source).unwrap_or_default();
                let new = self.text_field(ui, &key, &current, false)?;
                match perox_forms::parse_value(&new) {
                    Some(v) => Some(v),
                    None => {
                        self.error = Some(format!("`{new}` isn't a valid value"));
                        None
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn events(
        &mut self,
        ui: &mut egui::Ui,
        el: &Element,
        path: &[usize],
        events: &[&str],
        source: &str,
        stem: &str,
        actions: &mut Vec<Action>,
    ) {
        let form_name = perox_forms::codegen::form_name(self.form.file(), stem);
        for event in events {
            ui.horizontal(|ui| {
                label_cell(ui, RichText::new(*event));
                let current = el
                    .handlers
                    .iter()
                    .find(|h| h.event == *event)
                    .map(|h| h.method.clone());
                let key = format!("{path:?}/on {event}");
                if let Some(new) =
                    self.text_field(ui, &key, current.as_deref().unwrap_or(""), false)
                {
                    let new = new.trim();
                    let result = if new.is_empty() {
                        edit::remove_handler(source, path, event)
                    } else {
                        edit::set_handler(source, path, event, new)
                    };
                    self.apply(actions, result);
                }
                if small_icon_button(
                    ui,
                    Icon::ArrowRight,
                    "Go to the handler (adds it if needed)",
                )
                .clicked()
                {
                    let method = current.clone().unwrap_or_else(|| {
                        let who = el.id.clone().unwrap_or_else(|| catalog::snake(&el.kind));
                        format!("{who}_{event}")
                    });
                    if current.is_some()
                        || self.apply(actions, edit::set_handler(source, path, event, &method))
                    {
                        actions.push(Action::OpenHandler {
                            form: form_name.clone(),
                            method,
                        });
                    }
                }
            });
        }
    }

    /// A text field that writes back when you press Enter or leave it.
    fn text_field(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        current: &str,
        multiline: bool,
    ) -> Option<String> {
        let buf = self
            .buffers
            .entry(key.to_owned())
            .or_insert_with(|| current.to_owned());
        let edit = if multiline {
            egui::TextEdit::multiline(buf).desired_rows(3)
        } else {
            egui::TextEdit::singleline(buf)
        };
        // What's left of the row, less room for the reset button.
        let width = (ui.available_width() - 24.0).max(60.0);
        let r = ui.add(edit.desired_width(width).id_salt(key));
        if r.has_focus() {
            return None;
        }
        let text = self.buffers.remove(key)?;
        (r.lost_focus() && text != current).then_some(text)
    }
}

/// `button1`, `button2`, …: the first id not already in the form.
pub fn unique_id(file: &File, base: &str) -> String {
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|id| file.find(id).is_none())
        .unwrap_or_else(|| base.to_owned())
}

fn tree_node(
    ui: &mut egui::Ui,
    el: &Element,
    path: &mut Vec<usize>,
    selected: Option<&[usize]>,
    clicked: &mut Option<Vec<usize>>,
) {
    let is_sel = selected == Some(path.as_slice());
    let mut text = egui::text::LayoutJob::default();
    let style = ui.style();
    if let Some(id) = &el.id {
        RichText::new(format!("{id}  ")).strong().append_to(
            &mut text,
            style,
            egui::FontSelection::Default,
            Align::Center,
        );
    }
    RichText::new(&el.kind).weak().append_to(
        &mut text,
        style,
        egui::FontSelection::Default,
        Align::Center,
    );
    let row = |ui: &mut egui::Ui| {
        ui.add(egui::Button::selectable(
            is_sel,
            (catalog::icon(&el.kind).image(ui), text.clone()),
        ))
    };
    if el.children.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            if row(ui).clicked() {
                *clicked = Some(path.clone());
            }
        });
        return;
    }
    let id = ui.make_persistent_id(("tree", path.clone()));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            if row(ui).clicked() {
                *clicked = Some(path.clone());
            }
        })
        .body(|ui| {
            for (i, child) in el.children.iter().enumerate() {
                path.push(i);
                tree_node(ui, child, path, selected, clicked);
                path.pop();
            }
        });
}

fn palette_row(ui: &mut egui::Ui, icon: Icon, name: &str) -> egui::Response {
    let height = ui.spacing().interact_size.y + 2.0;
    let (rect, r) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    if r.hovered() {
        ui.painter()
            .rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let icon_rect = Rect::from_min_size(
        Pos2::new(rect.left() + 6.0, rect.center().y - perox_icons::SIZE / 2.0),
        Vec2::splat(perox_icons::SIZE),
    );
    icon.image(ui).paint_at(ui, icon_rect);
    ui.painter().text(
        Pos2::new(icon_rect.right() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    r
}

/// A fixed-width property name, so editors line up.
fn label_cell(ui: &mut egui::Ui, text: RichText) -> egui::Response {
    let h = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        vec2(LABEL_WIDTH, h),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(LABEL_WIDTH);
            ui.add(egui::Label::new(text).truncate())
        },
    )
    .inner
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    ui.add(egui::Button::image(icon.image(ui)).frame(false))
        .on_hover_text(tip)
}

fn small_icon_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    ui.add(egui::Button::image(icon.sized(ui, 12.0)).frame(false))
        .on_hover_text(tip)
}

/// A neutral window frame around the form: title bar, border, shadow.
fn window_chrome(ui: &egui::Ui, window: Rect, title: &str, selected: bool) {
    let v = ui.visuals();
    let painter = ui.painter();
    painter.add(
        egui::epaint::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(90),
        }
        .as_shape(window, 8.0),
    );
    painter.rect_filled(window, 8.0, v.panel_fill);
    let bar = Rect::from_min_size(window.min, vec2(window.width(), TITLE_BAR));
    painter.rect_filled(
        bar,
        egui::CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        },
        v.faint_bg_color,
    );
    for i in 0..3 {
        painter.circle_filled(
            bar.left_center() + vec2(14.0 + i as f32 * 16.0, 0.0),
            5.0,
            v.widgets.inactive.bg_fill,
        );
    }
    painter.text(
        bar.center(),
        egui::Align2::CENTER_CENTER,
        title,
        egui::TextStyle::Body.resolve(ui.style()),
        v.text_color(),
    );
    let stroke = if selected {
        Stroke::new(2.0, v.selection.bg_fill)
    } else {
        v.window_stroke
    };
    painter.rect_stroke(window, 8.0, stroke, StrokeKind::Outside);
}
