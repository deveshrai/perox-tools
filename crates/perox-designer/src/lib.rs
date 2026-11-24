//! PeroxIDE's form designer for `.slint` files (ADR 0011).
//!
//! The `.slint` text is the only state: every gesture (drop an element, drag it
//! elsewhere, change a property, wire a callback) becomes a minimal [`TextEdit`] that the
//! IDE applies to the open document. So undo, the Source view, git diffs and agents all
//! see the same thing, and hand-written formatting and comments survive.
//!
//! The canvas is drawn by Slint itself ([`preview`]), so what you see is what runs.

pub mod catalog;
pub mod drop;
pub mod free;
pub mod preview;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Color32, DragAndDrop, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Vec2, vec2,
};
use perox_icons::Icon;
use perox_slint::edit::{self, EditError, TextEdit};
use perox_slint::{Component, Document, Element, Value};

use catalog::{PropKind, PropSpec};

const DEFAULT_SIZE: Vec2 = Vec2::new(480.0, 320.0);
const TITLE_BAR: f32 = 28.0;
const CANVAS_MARGIN: f32 = 24.0;
const LABEL_WIDTH: f32 = 112.0;

/// What the IDE should do after a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Apply these edits to the document (one undo step).
    Edit(Vec<TextEdit>),
    /// Show the handler `method` of `<form>Handlers` in the code-behind, adding it (with
    /// these Rust parameters) if it's missing.
    OpenHandler {
        /// The component, e.g. `MainWindow` (the trait is `MainWindowHandlers`).
        form: String,
        method: String,
        /// `(name, Rust type)`.
        params: Vec<(String, String)>,
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
    /// The parsed source, and its hash.
    doc: Document,
    doc_key: u64,
    /// The component being designed (default: the file's main one).
    pub component: Option<String>,
    pub preview: preview::Preview,
    /// Widget style for the preview (default: this OS's native style).
    pub style: Option<String>,
    /// The selected element's path (`[]` is the root).
    pub selected: Option<Vec<usize>>,
    /// Window size while the corner handle is being dragged.
    resize: Option<Vec2>,
    resize_from: Option<Vec2>,
    /// Properties being dragged (inspector numbers, resize handles): previewed live,
    /// written on release. `(path, name, expression)`.
    live: Vec<(Vec<usize>, String, String)>,
    /// Where in the element the pointer grabbed it (free placement keeps that offset).
    grab: Vec2,
    /// An element being resized by its handles: path, which sides move, start rect
    /// (relative to its parent).
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
    /// Every element on the canvas that's showing, parents first.
    pub elements: Vec<(Vec<usize>, Rect)>,
    pub palette: Vec<(&'static str, Rect)>,
    /// The window frame, title bar included.
    pub window: Option<Rect>,
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl Designer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The document as the designer last saw it.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The component being designed.
    pub fn current(&self) -> Option<&Component> {
        match &self.component {
            Some(name) => self.doc.component(name),
            None => self.doc.main_component().map(|i| &self.doc.components[i]),
        }
    }

    /// Its name.
    pub fn component_name(&self) -> String {
        self.current().map(|c| c.name.clone()).unwrap_or_default()
    }

    fn element(&self, path: &[usize]) -> Option<&Element> {
        self.current()?.at(path)
    }

    pub fn style(&self) -> String {
        self.style
            .clone()
            .unwrap_or_else(|| preview::native_style().into())
    }

    fn sync(&mut self, source: &str) {
        let key = hash(source);
        if key != self.doc_key {
            self.doc = perox_slint::parse(source);
            self.doc_key = key;
        }
        if let Some(name) = &self.component
            && self.doc.component(name).is_none()
        {
            self.component = None;
        }
        if let Some(sel) = &self.selected
            && self.element(sel).is_none()
        {
            self.selected = None;
        }
    }

    /// Draw the whole designer (palette, canvas, tree, inspector) for `source`, the text
    /// of the `.slint` file at `path` (used to resolve its imports).
    pub fn show(&mut self, ui: &mut egui::Ui, source: &str, path: &Path) -> Vec<Action> {
        self.sync(source);
        let mut actions = Vec::new();
        let base = ui.scope_id().with("designer");

        egui::Panel::left(base.with("palette"))
            .resizable(true)
            .default_size(160.0)
            .show(ui, |ui| self.palette(ui, source, &mut actions));
        egui::Panel::right(base.with("inspector"))
            .resizable(true)
            .default_size(300.0)
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
                    .show(ui, |ui| self.inspector(ui, source, &mut actions));
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui, source, &mut actions);
            self.canvas(ui, source, path, &mut actions);
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
                for (title, group) in [
                    ("Layouts", catalog::Group::Layout),
                    ("Containers", catalog::Group::Container),
                    ("Widgets", catalog::Group::Widget),
                ] {
                    ui.add_space(6.0);
                    ui.label(RichText::new(title).small().weak());
                    for k in catalog::KINDS.iter().filter(|k| k.group == group) {
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

    /// Double-click in the palette: into the selected container, else after the selected
    /// element, else into the root's single layout.
    fn insert_at_selection(&mut self, source: &str, kind: &str, actions: &mut Vec<Action>) {
        let Some(c) = self.current() else { return };
        let mut sel = self.selected.clone().unwrap_or_default();
        if sel.is_empty()
            && c.root.children.len() == 1
            && catalog::accepts(&c.root.children[0].kind, kind)
        {
            sel = vec![0];
        }
        let (parent, index) = match c.at(&sel) {
            Some(el) if catalog::accepts(&el.kind, kind) => (sel.clone(), el.children.len()),
            Some(_) if !sel.is_empty() => {
                let mut parent = sel.clone();
                let last = parent.pop().unwrap_or(0);
                (parent, last + 1)
            }
            _ => return,
        };
        if c.at(&parent)
            .is_some_and(|p| catalog::accepts(&p.kind, kind))
        {
            self.add(source, kind, &parent, index, &[], actions);
        } else {
            self.error = Some(format!("A {kind} can't go there"));
        }
    }

    /// Adds a new element (with the catalog's defaults plus `extra`), importing its type
    /// if needed, and selects it.
    fn add(
        &mut self,
        source: &str,
        kind: &str,
        parent: &[usize],
        index: usize,
        extra: &[(String, String)],
        actions: &mut Vec<Action>,
    ) {
        let Some(c) = self.current() else { return };
        let comp = c.name.clone();
        let id = catalog::wants_id(kind).then(|| unique_id(c, &catalog::snake(kind)));
        let mut props = catalog::defaults(kind);
        props.extend_from_slice(extra);
        let count = c.at(parent).map_or(0, |p| p.children.len());
        let mut work = source.to_owned();
        let result = free::step(
            &mut work,
            edit::add_element(source, &comp, parent, index, kind, id.as_deref(), &props),
        )
        .map(|()| {
            free::import(&mut work, kind);
            work
        });
        if self.apply_text(actions, source, result) {
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
                .add_enabled_ui(can_delete, |ui| icon_button(ui, Icon::Trash2, "Delete (Del)"))
                .inner
                .clicked()
            {
                self.delete_selected(source, actions);
            }
            ui.separator();
            let sel = self.selected.clone().unwrap_or_default();
            let (can_break, can_lay_out) = self
                .current()
                .map(|c| (free::can_break(c, &sel), free::can_lay_out(c, &sel)))
                .unwrap_or_default();
            if ui
                .add_enabled_ui(can_break, |ui| {
                    icon_button(ui, Icon::Ungroup, "Break layout: place the children freely, where they are")
                })
                .inner
                .clicked()
            {
                self.relayout(source, None, actions);
            }
            for (dir, icon, tip) in [
                (free::Direction::Vertical, Icon::Rows3, "Lay out vertically"),
                (free::Direction::Horizontal, Icon::Columns3, "Lay out horizontally"),
                (free::Direction::Grid, Icon::Grid3x3, "Lay out in a grid"),
            ] {
                if ui
                    .add_enabled_ui(can_lay_out, |ui| icon_button(ui, icon, tip))
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
                .on_hover_text("Snap free placement to the grid (8 px)")
                .clicked()
            {
                self.snap_off = !self.snap_off;
            }
            ui.separator();
            // Style and component pickers.
            let style = self.style();
            egui::ComboBox::from_id_salt("designer_style")
                .selected_text(&style)
                .width(90.0)
                .show_ui(ui, |ui| {
                    for s in preview::STYLES {
                        let label = if *s == preview::native_style() {
                            format!("{s} (native)")
                        } else {
                            (*s).to_owned()
                        };
                        if ui.selectable_label(style == *s, label).clicked() {
                            self.style = Some((*s).to_owned());
                        }
                    }
                })
                .response
                .on_hover_text("Widget style for the preview. Apps use the native style of the OS they run on.");
            let names: Vec<String> = self.doc.components.iter().map(|c| c.name.clone()).collect();
            if names.len() > 1 {
                let current = self.component_name();
                egui::ComboBox::from_id_salt("designer_component")
                    .selected_text(&current)
                    .show_ui(ui, |ui| {
                        for n in &names {
                            if ui.selectable_label(*n == current, n).clicked() {
                                self.component = Some(n.clone());
                                self.selected = None;
                            }
                        }
                    });
            }
            if let Some(e) = &self.error {
                ui.separator();
                ui.add(Icon::TriangleAlert.colored(ui.visuals().warn_fg_color));
                ui.colored_label(ui.visuals().warn_fg_color, e);
            } else if let Some(err) = self
                .doc
                .errors
                .first()
                .map(|e| e.message.clone())
                .or_else(|| self.preview.diagnostics.first().cloned())
            {
                ui.separator();
                ui.add(Icon::CircleX.colored(ui.visuals().error_fg_color));
                ui.colored_label(ui.visuals().error_fg_color, format!("{err} (fix it in Source)"));
            }
        });
        ui.add_space(2.0);
    }

    fn delete_selected(&mut self, source: &str, actions: &mut Vec<Action>) {
        let Some(sel) = self.selected.clone().filter(|s| !s.is_empty()) else {
            return;
        };
        let comp = self.component_name();
        if self.apply(actions, edit::remove_element(source, &comp, &sel)) {
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

    /// Arrow keys move a freely placed element 1 px (a grid step with Shift).
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
                    d += dir * free::GRID;
                }
            }
        });
        if d == Vec2::ZERO {
            return;
        }
        // Start from where it is on screen (x/y may be unset or an expression).
        let (Some(rect), Some(parent)) = (
            self.drawn_rect(&path),
            path.split_last().and_then(|(_, p)| self.drawn_rect(p)),
        ) else {
            return;
        };
        let at = (rect.min - parent.min + d).max(Vec2::ZERO).to_pos2();
        let comp = self.component_name();
        let mut work = source.to_owned();
        let result = free::set_position(&mut work, &comp, &path, at).map(|()| work);
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
            Drag::Move(p) => self.element(p).map_or_else(String::new, |e| e.kind.clone()),
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

    /// The source with live (being-dragged) values applied, for the preview.
    fn shown_source(&self, source: &str) -> String {
        let comp = self.component_name();
        let mut work = source.to_owned();
        for (path, name, value) in &self.live {
            let s = work.clone();
            if let Ok(e) = edit::set_property(&s, &comp, path, name, value) {
                work = edit::apply(&s, &e);
            }
        }
        work
    }

    /// The window's size: being resized, else from its properties, else its natural size.
    fn window_size(&self) -> Vec2 {
        if let Some(s) = self.resize {
            return s;
        }
        let from_props = self.current().and_then(|c| {
            let get = |a: &str, b: &str| c.root.num_prop(a).or_else(|| c.root.num_prop(b));
            Some(vec2(
                get("width", "preferred-width")? as f32,
                get("height", "preferred-height")? as f32,
            ))
        });
        let natural = self.preview.preferred_size();
        match (from_props, natural) {
            (Some(s), _) => s,
            (None, Some(n)) => {
                // Only one of them set: take it, the other from the layout.
                let c = self.current();
                let w = c.and_then(|c| {
                    c.root
                        .num_prop("width")
                        .or_else(|| c.root.num_prop("preferred-width"))
                });
                let h = c.and_then(|c| {
                    c.root
                        .num_prop("height")
                        .or_else(|| c.root.num_prop("preferred-height"))
                });
                vec2(w.map_or(n.x, |w| w as f32), h.map_or(n.y, |h| h as f32))
            }
            (None, None) => DEFAULT_SIZE,
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, source: &str, path: &Path, actions: &mut Vec<Action>) {
        let Some(c) = self.current().cloned() else {
            ui.weak("This file has no component to design. Add `export component Name inherits Window { }`.");
            return;
        };
        // The preview is built from the source plus live values; element offsets come
        // from that same text.
        let shown = self.shown_source(source);
        let shown_doc = if self.live.is_empty() {
            self.doc.clone()
        } else {
            perox_slint::parse(&shown)
        };
        let path = if path.as_os_str().is_empty() {
            PathBuf::from("form.slint")
        } else {
            path.to_owned()
        };
        let style = self.style();
        self.preview.update(&shown, &path, &c.name, &style);

        let size = self.window_size();
        let is_window = c.is_window();
        let title = c.root.str_prop("title").unwrap_or("").to_owned();
        let bar = if is_window { TITLE_BAR } else { 0.0 };
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            let total = size + vec2(2.0 * CANVAS_MARGIN, 2.0 * CANVAS_MARGIN + bar);
            let (outer, _) = ui.allocate_exact_size(total, Sense::hover());
            let window = Rect::from_min_size(
                outer.min + Vec2::splat(CANVAS_MARGIN),
                size + vec2(0.0, bar),
            );
            let content = Rect::from_min_size(window.min + vec2(0.0, bar), size);
            window_chrome(
                ui,
                window,
                &title,
                is_window,
                self.selected.as_deref() == Some(&[]),
            );

            let scale = ui.ctx().pixels_per_point();
            if let Some(tex) = self.preview.texture(ui.ctx(), size, scale) {
                ui.painter().image(
                    tex.id(),
                    content,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter()
                    .rect_filled(content, 0.0, ui.visuals().extreme_bg_color);
            }

            // Where each element was drawn.
            let shown_c = shown_doc
                .component(&c.name)
                .cloned()
                .unwrap_or_else(|| c.clone());
            let mut rects: Vec<(Vec<usize>, Rect)> = vec![(Vec::new(), content)];
            shown_c.root.walk(&mut Vec::new(), &mut |el, p| {
                if p.is_empty() {
                    return;
                }
                if let Some(r) = self.preview.rect_at_offset(el.kind_span.start) {
                    rects.push((p.to_vec(), r.translate(content.min.to_vec2())));
                }
            });
            self.layout.elements = rects.clone();
            self.layout.window = Some(window);

            self.overlays(ui, content, &c, &rects);
            self.pointer(ui, content, &rects);
            self.drop_onto(ui, content, &rects, source, actions);
            self.resize_widget(ui, source, actions);
            self.resize_handle(ui, window, size, source, actions);
        });
        // Keep animations (spinners, indeterminate progress) moving.
        if self.preview.is_ready() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Selection, hover, layout outlines and the free-placement grid.
    fn overlays(&self, ui: &egui::Ui, content: Rect, c: &Component, rects: &[(Vec<usize>, Rect)]) {
        let accent = ui.visuals().selection.bg_fill;
        let painter = ui.painter().with_clip_rect(content.expand(4.0));
        for (p, r) in rects {
            let Some(el) = c.at(p) else { continue };
            if catalog::is_free(&el.kind) && self.selected.as_deref() == Some(p.as_slice()) {
                grid_dots(&painter, *r, accent.gamma_multiply(0.35));
            }
            if catalog::is_layout(&el.kind) && el.kind != "GroupBox" {
                painter.rect_stroke(
                    *r,
                    0.0,
                    Stroke::new(1.0, accent.gamma_multiply(0.25)),
                    StrokeKind::Inside,
                );
            }
        }
        if let Some(hover) = ui
            .ctx()
            .pointer_hover_pos()
            .filter(|p| content.contains(*p))
            .and_then(|p| hit(rects, p, None))
            .and_then(|p| rects.iter().find(|(q, _)| *q == p))
            && !hover.0.is_empty()
        {
            painter.rect_stroke(
                hover.1,
                0.0,
                Stroke::new(1.0, accent.gamma_multiply(0.6)),
                StrokeKind::Outside,
            );
        }
        if let Some(sel) = &self.selected
            && !sel.is_empty()
            && let Some((_, r)) = rects.iter().find(|(p, _)| p == sel)
        {
            painter.rect_stroke(*r, 0.0, Stroke::new(2.0, accent), StrokeKind::Outside);
        }
    }

    /// Click to select; drag a non-root element to move it.
    fn pointer(&mut self, ui: &mut egui::Ui, content: Rect, rects: &[(Vec<usize>, Rect)]) {
        let r = ui.interact(
            content,
            ui.scope_id().with("canvas"),
            Sense::click_and_drag(),
        );
        if r.clicked()
            && let Some(p) = r.interact_pointer_pos().and_then(|p| hit(rects, p, None))
        {
            self.selected = Some(p);
            self.error = None;
        }
        if r.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && let Some(p) = hit(rects, origin, None).filter(|p| !p.is_empty())
        {
            if let Some(rect) = self.drawn_rect(&p) {
                self.grab = origin - rect.min;
            }
            self.selected = Some(p.clone());
            DragAndDrop::set_payload(ui.ctx(), Drag::Move(p));
        }
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
        let Some(c) = self.current().cloned() else {
            return;
        };
        let (kind, moving) = match &*drag {
            Drag::New(k) => ((*k).to_owned(), None),
            Drag::Move(p) => match c.at(p) {
                Some(el) => (el.kind.clone(), Some(p.clone())),
                None => return,
            },
        };
        let Some(target) = drop::target(&c, rects, pointer, &kind, moving.as_deref()) else {
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
            None => self.add(source, &kind, &target.parent, target.index, &[], actions),
            Some(from) => {
                if drop::is_noop(&from, &target.parent, target.index) {
                    return;
                }
                // Into a layout: a position means nothing there.
                let new_path = drop::moved_path(&from, &target.parent, target.index);
                let comp = c.name.clone();
                let mut work = source.to_owned();
                let result = free::step(
                    &mut work,
                    edit::move_element(source, &comp, &from, &target.parent, target.index),
                )
                .and_then(|()| free::strip(&mut work, &comp, &new_path, free::GEOMETRY))
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

    fn snap(&self, v: f32) -> f32 {
        free::snap(v, !self.snap_off)
    }

    /// Is the element at `path` placed freely (its parent isn't a layout)?
    fn in_free_parent(&self, path: &[usize]) -> bool {
        let Some((_, parent)) = path.split_last() else {
            return false;
        };
        self.element(parent)
            .is_some_and(|p| catalog::is_free(&p.kind))
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
        let Some(origin) = self.drawn_rect(&target.parent) else {
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
        let rel = pointer - grab - origin.min;
        let at = Pos2::new(self.snap(rel.x), self.snap(rel.y));
        let accent = ui.visuals().selection.bg_fill;
        let ghost = Rect::from_min_size(origin.min + at.to_vec2(), size);
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
        let comp = self.component_name();
        match moving {
            None => {
                let mut extra = vec![
                    ("x".to_owned(), free::px(at.x)),
                    ("y".to_owned(), free::px(at.y)),
                ];
                if let Some((w, h)) = catalog::free_default_size(kind) {
                    extra.push(("width".into(), free::px(w as f32)));
                    extra.push(("height".into(), free::px(h as f32)));
                }
                self.add(source, kind, &parent, usize::MAX, &extra, actions);
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
                            edit::move_element(source, &comp, &from, &parent, target.index),
                        )?;
                        free::strip(&mut work, &comp, &new_path, free::LAYOUT_CHILD)?;
                    }
                    free::set_position(&mut work, &comp, &new_path, at)?;
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
        let (Some(rect), Some(parent)) = (
            self.drawn_rect(&path),
            path.split_last().and_then(|(_, p)| self.drawn_rect(p)),
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
                    let start = rect.translate(-parent.min.to_vec2());
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
                        (p.clone(), "x".into(), free::px(nr.min.x)),
                        (p.clone(), "y".into(), free::px(nr.min.y)),
                        (p.clone(), "width".into(), free::px(nr.width())),
                        (p, "height".into(), free::px(nr.height())),
                    ];
                }
                if r.drag_stopped()
                    && let Some((p, _, _)) = self.widget_resize.take()
                {
                    let get = |name: &str| {
                        self.live
                            .iter()
                            .find(|(_, n, _)| n == name)
                            .and_then(|(_, _, v)| Value::parse(v).as_f64())
                            .unwrap_or(0.0) as f32
                    };
                    let nr = Rect::from_min_size(
                        Pos2::new(get("x"), get("y")),
                        vec2(get("width"), get("height")),
                    );
                    self.live.clear();
                    let comp = self.component_name();
                    let mut work = source.to_owned();
                    let result = free::set_geometry(&mut work, &comp, &p, nr).map(|()| work);
                    self.apply_text(actions, source, result);
                }
            }
        }
    }

    /// Break Layout, or Lay Out Vertically / Horizontally / in a Grid, on the selection.
    fn relayout(&mut self, source: &str, dir: Option<free::Direction>, actions: &mut Vec<Action>) {
        let path = self.selected.clone().unwrap_or_default();
        let rects = self.layout.elements.clone();
        let drawn = free::Drawn { rects: &rects };
        let comp = self.component_name();
        let result = match dir {
            None => free::break_layout(source, &comp, drawn, &path),
            Some(d) => free::lay_out(source, &comp, drawn, &path, d),
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
            && let Some(c) = self.current()
        {
            // Fixed size if the form has one, else the preferred (initial) size.
            let (wn, hn) = if c.root.prop("width").is_some() || c.root.prop("height").is_some() {
                ("width", "height")
            } else {
                ("preferred-width", "preferred-height")
            };
            let comp = c.name.clone();
            let mut work = source.to_owned();
            let result = (|| {
                let s1 = work.clone();
                free::step(
                    &mut work,
                    edit::set_property(&s1, &comp, &[], wn, &free::px(s.x)),
                )?;
                let s2 = work.clone();
                free::step(
                    &mut work,
                    edit::set_property(&s2, &comp, &[], hn, &free::px(s.y)),
                )?;
                Ok(work)
            })();
            self.apply_text(actions, source, result);
        }
    }

    // ---- object tree ----

    fn object_tree(&mut self, ui: &mut egui::Ui) {
        let Some(c) = self.current().cloned() else {
            ui.weak("No component");
            return;
        };
        let mut path = Vec::new();
        let mut clicked = None;
        tree_node(
            ui,
            &c.root,
            &c.name,
            &mut path,
            self.selected.as_deref(),
            &mut clicked,
        );
        if let Some(p) = clicked {
            self.selected = Some(p);
            self.error = None;
        }
    }

    // ---- inspector ----

    fn inspector(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        let Some(path) = self.selected.clone() else {
            ui.weak("Select an element to edit it.");
            return;
        };
        let Some(c) = self.current().cloned() else {
            return;
        };
        let Some(el) = c.at(&path).cloned() else {
            return;
        };
        let spec = catalog::kind(&el.kind);
        let parent_kind = path
            .split_last()
            .and_then(|(_, p)| c.at(p))
            .map(|p| p.kind.clone());

        ui.horizontal(|ui| {
            ui.add(catalog::icon(&el.kind).image(ui));
            ui.strong(&el.kind);
            if path.is_empty() {
                ui.weak(format!("component {}", c.name));
            }
        });
        if !path.is_empty() {
            ui.horizontal(|ui| {
                label_cell(ui, RichText::new("Name"))
                    .on_hover_text("The id other elements use (`name.text`)");
                let current = el.id.clone().unwrap_or_default();
                if let Some(new) = self.text_field(ui, &format!("{path:?}/#id"), &current, false) {
                    let id = new.trim();
                    let id = (!id.is_empty()).then_some(id);
                    self.apply(actions, edit::set_id(source, &c.name, &path, id));
                }
            });
        }

        let known: Vec<PropSpec> = spec.map(|k| k.props.to_vec()).unwrap_or_default();
        ui.add_space(4.0);
        if !known.is_empty() {
            egui::CollapsingHeader::new("Properties")
                .default_open(true)
                .show(ui, |ui| {
                    self.prop_rows(ui, &c.name, &el, &path, &known, source, actions)
                });
        }
        let free_child = self.in_free_parent(&path);
        let mut layout: Vec<PropSpec> = Vec::new();
        if free_child {
            layout.extend_from_slice(catalog::FREE_CHILD);
        }
        if !path.is_empty() {
            layout.extend_from_slice(catalog::COMMON);
        }
        if parent_kind.as_deref().is_some_and(catalog::is_layout) {
            layout.extend_from_slice(catalog::LAYOUT_CHILD);
        }
        if parent_kind.as_deref().is_some_and(catalog::is_grid) {
            layout.extend_from_slice(catalog::GRID_CHILD);
        }
        if !layout.is_empty() {
            egui::CollapsingHeader::new("Size and layout")
                .default_open(free_child)
                .show(ui, |ui| {
                    self.prop_rows(ui, &c.name, &el, &path, &layout, source, actions)
                });
        }
        // Properties in the file the catalog doesn't know (components, expressions).
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
                help: "Edited as a .slint expression",
            })
            .collect();
        if !other.is_empty() {
            egui::CollapsingHeader::new("Other")
                .default_open(true)
                .show(ui, |ui| {
                    self.prop_rows(ui, &c.name, &el, &path, &other, source, actions)
                });
        }

        let events = catalog::events(&el.kind);
        if !events.is_empty() {
            egui::CollapsingHeader::new("Events")
                .default_open(true)
                .show(ui, |ui| {
                    self.events(ui, &c, &el, &path, events, source, actions)
                });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prop_rows(
        &mut self,
        ui: &mut egui::Ui,
        comp: &str,
        el: &Element,
        path: &[usize],
        specs: &[PropSpec],
        source: &str,
        actions: &mut Vec<Action>,
    ) {
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
                if let Some(v) = self.value_editor(ui, path, spec, value.as_ref()) {
                    self.apply(
                        actions,
                        edit::set_property(source, comp, path, &spec.name, &v),
                    );
                }
                if value.is_some()
                    && small_icon_button(ui, Icon::X, "Reset to the default").clicked()
                {
                    self.apply(
                        actions,
                        edit::remove_property(source, comp, path, &spec.name),
                    );
                }
            });
        }
    }

    /// An editor for one property. Returns a new expression to write.
    fn value_editor(
        &mut self,
        ui: &mut egui::Ui,
        path: &[usize],
        spec: &PropSpec,
        value: Option<&Value>,
    ) -> Option<String> {
        let key = format!("{path:?}/{}", spec.name);
        // A value the editor can't show (an expression) is edited as source.
        let kind = match (spec.kind, value) {
            (_, Some(Value::Expr(_))) => PropKind::Source,
            (k, _) => k,
        };
        match kind {
            PropKind::Text => {
                let current = value.and_then(Value::as_str).unwrap_or_default().to_owned();
                self.text_field(ui, &key, &current, false)
                    .map(|t| perox_slint::syntax::quote(&t))
            }
            PropKind::Length | PropKind::Int | PropKind::Float => {
                let unit = match kind {
                    PropKind::Length => "px",
                    _ => "",
                };
                let live = self
                    .live
                    .iter()
                    .find(|(p, n, _)| p == path && *n == spec.name)
                    .and_then(|(_, _, v)| Value::parse(v).as_f64());
                let mut n = live.or(value.and_then(Value::as_f64)).unwrap_or(0.0);
                let mut dv = egui::DragValue::new(&mut n).speed(if kind == PropKind::Float {
                    0.05
                } else {
                    0.5
                });
                if kind == PropKind::Int {
                    dv = dv.fixed_decimals(0);
                }
                if !unit.is_empty() {
                    dv = dv.suffix(" px");
                }
                let r = ui.add(dv);
                let text = |n: f64| {
                    if kind == PropKind::Float {
                        let s = format!("{n:.3}");
                        s.trim_end_matches('0').trim_end_matches('.').to_owned()
                    } else {
                        format!("{}{unit}", n.round() as i64)
                    }
                };
                if r.dragged() && r.changed() {
                    self.live = vec![(path.to_vec(), spec.name.to_string(), text(n))];
                    None
                } else if r.drag_stopped() {
                    self.live.clear();
                    Some(text(n))
                } else if r.changed() {
                    Some(text(n))
                } else {
                    None
                }
            }
            PropKind::Bool => {
                let mut b = value.and_then(Value::as_bool).unwrap_or(false);
                ui.checkbox(&mut b, "").changed().then(|| b.to_string())
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
                .then(|| Value::Color(c.to_srgba_unmultiplied()).to_source())
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
                    .to_source()
                })
            }
            PropKind::Choice(options) => {
                let current = match value {
                    Some(Value::Ident(i)) => Some(i.rsplit('.').next().unwrap_or(i).to_owned()),
                    _ => None,
                };
                let mut picked = None;
                egui::ComboBox::from_id_salt(&key)
                    .selected_text(current.clone().unwrap_or_else(|| "default".into()))
                    .show_ui(ui, |ui| {
                        for o in options {
                            if ui
                                .selectable_label(current.as_deref() == Some(o), *o)
                                .clicked()
                            {
                                picked = Some((*o).to_owned());
                            }
                        }
                    });
                picked
            }
            PropKind::Source => {
                let current = value.map(Value::to_source).unwrap_or_default();
                self.text_field(ui, &key, &current, false)
                    .filter(|t| !t.trim().is_empty())
            }
        }
    }

    /// Each callback the element offers, with the root callback it forwards to. The
    /// arrow opens (or creates) the Rust handler.
    #[allow(clippy::too_many_arguments)]
    fn events(
        &mut self,
        ui: &mut egui::Ui,
        c: &Component,
        el: &Element,
        path: &[usize],
        events: &[catalog::Event],
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        for event in events {
            ui.horizontal(|ui| {
                label_cell(ui, RichText::new(event.name));
                let handler = el.handler(event.name);
                let forwards = handler.and_then(|h| h.forwards_to()).map(str::to_owned);
                if handler.is_some() && forwards.is_none() {
                    // Hand-written code in the callback: leave it to the Source view.
                    ui.weak("code in .slint");
                    return;
                }
                let key = format!("{path:?}/on {}", event.name);
                if let Some(new) =
                    self.text_field(ui, &key, forwards.as_deref().unwrap_or(""), false)
                {
                    let new = new.trim();
                    if new.is_empty() {
                        self.apply(
                            actions,
                            edit::remove_handler(source, &c.name, path, event.name),
                        );
                    } else {
                        self.wire(source, c, path, event, new, actions);
                    }
                }
                if small_icon_button(
                    ui,
                    Icon::ArrowRight,
                    "Go to the handler (adds it if needed)",
                )
                .clicked()
                {
                    actions.extend(self.go_to_handler(source, path, event.name));
                }
            });
        }
    }

    /// "Go to handler" for `event` of the element at `path`: wires it to a root callback
    /// (`<id>_<event>`) if it isn't yet, and asks the IDE to open the Rust method.
    pub fn go_to_handler(&mut self, source: &str, path: &[usize], event: &str) -> Vec<Action> {
        self.sync(source);
        let mut actions = Vec::new();
        let Some(c) = self.current().cloned() else {
            return actions;
        };
        let Some(el) = c.at(path).cloned() else {
            self.error = Some("no such element".into());
            return actions;
        };
        let Some(ev) = catalog::events(&el.kind)
            .iter()
            .find(|e| e.name == event)
            .copied()
        else {
            self.error = Some(format!("{} has no `{event}` event", el.kind));
            return actions;
        };
        let handler = el.handler(event);
        let forwards = handler.and_then(|h| h.forwards_to()).map(str::to_owned);
        if handler.is_some() && forwards.is_none() {
            self.error = Some(format!(
                "`{event}` has code in the .slint file; edit it in Source"
            ));
            return actions;
        }
        let method = forwards.clone().unwrap_or_else(|| {
            let who = el.id.clone().unwrap_or_else(|| catalog::kebab(&el.kind));
            format!("{who}-{event}").replace('-', "_")
        });
        if forwards.is_some() || self.wire(source, &c, path, &ev, &method, &mut actions) {
            actions.push(Action::OpenHandler {
                form: c.name.clone(),
                method: perox_slint::codegen::rust_name(&method),
                params: ev
                    .params
                    .iter()
                    .map(|(n, t)| {
                        (
                            perox_slint::codegen::rust_name(n),
                            perox_slint::codegen::rust_type(t)
                                .unwrap_or("()")
                                .to_owned(),
                        )
                    })
                    .collect(),
            });
        }
        actions
    }

    /// `event(args) => { root.method(args); }` plus `callback method(…);` on the root.
    fn wire(
        &mut self,
        source: &str,
        c: &Component,
        path: &[usize],
        event: &catalog::Event,
        method: &str,
        actions: &mut Vec<Action>,
    ) -> bool {
        let result = wire_event(source, &c.name, path, event.name, method);
        self.apply_text(actions, source, result)
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

/// Wires `event` of the element at `path` to the root callback `method`:
/// `event(args) => { root.method(args); }`, declaring `callback method(args);` on the
/// component if needed. Returns the new source.
pub fn wire_event(
    source: &str,
    comp: &str,
    path: &[usize],
    event: &str,
    method: &str,
) -> Result<String, EditError> {
    let doc = perox_slint::parse(source);
    let kind = doc
        .component(comp)
        .and_then(|c| c.at(path))
        .map(|e| e.kind.clone())
        .ok_or(EditError::NoSuchElement)?;
    let params: &[(&str, &str)] = catalog::events(&kind)
        .iter()
        .find(|e| e.name == event)
        .map_or(&[], |e| e.params);
    let names: Vec<&str> = params.iter().map(|(n, _)| *n).collect();
    let decl: Vec<(Option<&str>, &str)> = params.iter().map(|(n, t)| (Some(*n), *t)).collect();
    let body = format!("root.{method}({});", names.join(", "));
    let mut work = source.to_owned();
    free::step(
        &mut work,
        edit::set_handler(source, comp, path, event, &names, &body),
    )?;
    let s = work.clone();
    free::step(&mut work, edit::add_callback(&s, comp, method, &decl))?;
    Ok(work)
}

/// The deepest element under `p` (rects are parents first), skipping `excluded` and its
/// children.
fn hit(rects: &[(Vec<usize>, Rect)], p: Pos2, excluded: Option<&[usize]>) -> Option<Vec<usize>> {
    rects
        .iter()
        .rev()
        .find(|(path, r)| r.contains(p) && !excluded.is_some_and(|e| path.starts_with(e)))
        .map(|(path, _)| path.clone())
}

/// `button1`, `button2`, …: the first id not already in the component.
pub fn unique_id(c: &Component, base: &str) -> String {
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|id| c.find(id).is_none())
        .unwrap_or_else(|| base.to_owned())
}

fn tree_node(
    ui: &mut egui::Ui,
    el: &Element,
    comp: &str,
    path: &mut Vec<usize>,
    selected: Option<&[usize]>,
    clicked: &mut Option<Vec<usize>>,
) {
    let is_sel = selected == Some(path.as_slice());
    let mut text = egui::text::LayoutJob::default();
    let style = ui.style();
    let name = if path.is_empty() {
        Some(comp.to_owned())
    } else {
        el.id.clone()
    };
    if let Some(id) = name {
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
                tree_node(ui, child, comp, path, selected, clicked);
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

/// Dots every grid step: where free placement snaps.
fn grid_dots(painter: &egui::Painter, r: Rect, color: Color32) {
    let step = free::GRID * 2.0;
    let mut y = r.top() + step;
    while y < r.bottom() {
        let mut x = r.left() + step;
        while x < r.right() {
            painter.circle_filled(Pos2::new(x, y), 0.8, color);
            x += step;
        }
        y += step;
    }
}

/// A neutral window frame around the form: title bar, border, shadow.
fn window_chrome(ui: &egui::Ui, window: Rect, title: &str, is_window: bool, selected: bool) {
    let v = ui.visuals();
    let painter = ui.painter();
    let radius = if is_window { 8.0 } else { 0.0 };
    painter.add(
        egui::epaint::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(90),
        }
        .as_shape(window, radius),
    );
    painter.rect_filled(window, radius, v.panel_fill);
    if is_window {
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
    }
    let stroke = if selected {
        Stroke::new(2.0, v.selection.bg_fill)
    } else {
        v.window_stroke
    };
    painter.rect_stroke(window, radius, stroke, StrokeKind::Outside);
}
