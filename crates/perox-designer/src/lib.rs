//! PeroxIDE's form designer for `.slint` files (ADR 0011).
//!
//! The `.slint` text is the only state: every gesture (drop an element, drag it
//! elsewhere, change a property, wire a callback) becomes a minimal [`TextEdit`] that the
//! IDE applies to the open document. So undo, the Source view, git diffs and agents all
//! see the same thing, and hand-written formatting and comments survive.
//!
//! The canvas is drawn by Slint itself ([`preview`]), so what you see is what runs.

pub mod actions;
pub mod catalog;
pub mod data;
pub mod drop;
pub mod free;
pub mod keys;
pub mod preview;
pub mod resources;
pub mod translations;

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

/// Colours of Slint's `Palette` (std-widgets): they follow the style and colour scheme.
const PALETTE_COLORS: &[&str] = &[
    "background",
    "foreground",
    "alternate-background",
    "alternate-foreground",
    "control-background",
    "control-foreground",
    "accent-background",
    "accent-foreground",
    "selection-background",
    "selection-foreground",
    "border",
];

/// A base form: the component and its file.
type Base = (Component, PathBuf);

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
    /// Preview in the dark colour scheme (default: like the IDE's theme).
    pub dark: Option<bool>,
    /// The selected element's path (`[]` is the root): the primary selection.
    pub selected: Option<Vec<usize>>,
    /// More selected elements (Shift- or Cmd-click), besides `selected`.
    pub multi: Vec<Vec<usize>>,
    /// `selected` as of the last frame: a change not made by a modifier-click starts a
    /// new single selection.
    last_selected: Option<Vec<usize>>,
    keep_multi: bool,
    /// What Copy put on the clipboard (`.slint` element source).
    clipboard: String,
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
    /// The inspector key of a shortcut field waiting for a key press.
    recording: Option<String>,
    /// The `.slint` file being designed (images and fonts are relative to its folder).
    file: PathBuf,
    /// Images in the form's folder, and when they were listed.
    images: Option<(std::time::Instant, Vec<String>)>,
    /// Extra properties for the element a palette drag will add (an image's `source`).
    drag_extra: Vec<(String, String)>,
    /// An import to add along with the property edit being applied: a font file
    /// (`(None, "fonts/X.ttf")`) or a name (`(Some("Palette"), "std-widgets.slint")`).
    pending_import: Option<(Option<String>, String)>,
    /// The Translations window.
    pub translations: translations::Editor,
    /// The component the designed one inherits, if it's one of ours, and when it was
    /// looked up.
    base: Option<(std::time::Instant, u64, Option<Base>)>,
    /// Bound queries' results, for showing data while designing.
    data_cache: data::Cache,
    /// What the last Run in the Data section gave.
    data_status: Option<String>,
    /// Preview the form in this language (`None`: as written).
    pub preview_lang: Option<String>,
    /// The app's style settings, and when they were read.
    app_ui: Option<(std::time::Instant, perox_slint::project::UiSettings)>,
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
    /// Non-visual elements in the component tray.
    pub tray: Vec<(Vec<usize>, Rect)>,
    /// The project's images in the palette (paths relative to the form).
    pub images: Vec<(String, Rect)>,
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

    /// The style the preview uses: picked in the toolbar, else the app's, else this OS's.
    pub fn style(&self) -> String {
        self.style
            .clone()
            .or_else(|| self.app_ui().style)
            .unwrap_or_else(|| preview::native_style().into())
    }

    /// The app's style and colour scheme (`slint-project.json`), as last read.
    pub fn app_ui(&self) -> perox_slint::project::UiSettings {
        self.app_ui
            .as_ref()
            .map(|(_, s)| s.clone())
            .unwrap_or_default()
    }

    /// Re-reads the app's style settings every few seconds (they can change on disk).
    fn refresh_app_ui(&mut self) {
        let fresh = self
            .app_ui
            .as_ref()
            .is_some_and(|(t, _)| t.elapsed() < std::time::Duration::from_secs(3));
        if !fresh {
            let s = perox_slint::project::settings_for(&self.form_dir());
            self.app_ui = Some((std::time::Instant::now(), s));
        }
    }

    /// Sets the app's style and colour scheme in `slint-project.json` (made next to the
    /// form if there's none), and previews them.
    pub fn set_app_ui(&mut self, s: &perox_slint::project::UiSettings) -> std::io::Result<PathBuf> {
        let file = perox_slint::project::find(&self.form_dir())
            .unwrap_or_else(|| self.form_dir().join(perox_slint::project::FILE_NAME));
        let old = std::fs::read_to_string(&file).unwrap_or_default();
        std::fs::write(&file, perox_slint::project::write(&old, s))?;
        self.app_ui = Some((std::time::Instant::now(), s.clone()));
        self.style = None;
        self.dark = None;
        Ok(file)
    }

    /// Dark preview: toggled in the toolbar, else the app's fixed scheme, else the IDE's.
    fn preview_dark(&self, ide_dark: bool) -> bool {
        self.dark
            .unwrap_or_else(|| match self.app_ui().color_scheme.as_deref() {
                Some("dark") => true,
                Some("light") => false,
                _ => ide_dark,
            })
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
        if self.file != path {
            self.file = path.to_owned();
            self.images = None;
            self.app_ui = None;
        }
        self.refresh_app_ui();
        if DragAndDrop::payload::<Drag>(ui.ctx()).is_none() {
            self.drag_extra.clear();
        }
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
            self.tray(ui, base);
            self.canvas(ui, source, path, &mut actions);
        });
        self.translations.show(ui.ctx());
        self.keyboard(ui, source, &mut actions);
        self.drag_preview(ui);
        if self.selected != self.last_selected && !self.keep_multi {
            self.multi.clear();
        }
        self.keep_multi = false;
        self.last_selected = self.selected.clone();
        actions
    }

    /// Everything selected, primary first, without elements whose parent is also
    /// selected.
    pub fn selection(&self) -> Vec<Vec<usize>> {
        let mut all: Vec<Vec<usize>> = self.selected.iter().cloned().collect();
        for p in &self.multi {
            if !all.contains(p) && self.element(p).is_some() {
                all.push(p.clone());
            }
        }
        let snapshot = all.clone();
        all.retain(|p| !snapshot.iter().any(|q| q != p && p.starts_with(q)));
        all
    }

    /// Shift/Cmd-click: add `p` to the selection, or take it out.
    fn toggle(&mut self, p: Vec<usize>) {
        self.keep_multi = true;
        if self.selected.as_ref() == Some(&p) {
            self.selected = if self.multi.is_empty() {
                None
            } else {
                Some(self.multi.remove(0))
            };
        } else if let Some(i) = self.multi.iter().position(|q| *q == p) {
            self.multi.remove(i);
        } else if let Some(old) = self.selected.replace(p) {
            self.multi.insert(0, old);
        }
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
                    ("Menus", catalog::Group::Menu),
                    ("Non-visual", catalog::Group::NonVisual),
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
                let images = self.project_images();
                self.layout.images.clear();
                if !images.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Images").small().weak())
                        .on_hover_text("Images in this form's folder. Drag one onto the form.");
                    for rel in images {
                        let name = rel.rsplit('/').next().unwrap_or(&rel).to_owned();
                        if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                            continue;
                        }
                        let r = palette_row(ui, Icon::FileImage, &name);
                        self.layout.images.push((rel.clone(), r.rect));
                        if r.drag_started() {
                            DragAndDrop::set_payload(ui.ctx(), Drag::New("Image"));
                            self.drag_extra = vec![("source".into(), resources::image_url(&rel))];
                        }
                        if r.double_clicked() {
                            self.drag_extra = vec![("source".into(), resources::image_url(&rel))];
                            self.insert_at_selection(source, "Image", actions);
                        }
                        r.on_hover_ui(|ui| {
                            ui.label(&rel);
                            let full = self.form_dir().join(&rel);
                            ui.add(
                                egui::Image::new(format!("file://{}", full.display()))
                                    .max_size(vec2(160.0, 120.0)),
                            );
                        });
                    }
                }
            });
    }

    /// The folder of the `.slint` file being designed.
    fn form_dir(&self) -> PathBuf {
        self.file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    /// Images in the form's folder, listed again every few seconds.
    fn project_images(&mut self) -> Vec<String> {
        let fresh = self
            .images
            .as_ref()
            .is_some_and(|(t, _)| t.elapsed() < std::time::Duration::from_secs(3));
        if !fresh {
            let dir = self.form_dir();
            let list = if dir.as_os_str().is_empty() {
                Vec::new()
            } else {
                resources::images(&dir)
            };
            self.images = Some((std::time::Instant::now(), list));
        }
        self.images
            .as_ref()
            .map(|(_, l)| l.clone())
            .unwrap_or_default()
    }

    /// Double-click in the palette: into the selected container, else after the selected
    /// element, else into the root's single layout.
    fn insert_at_selection(&mut self, source: &str, kind: &str, actions: &mut Vec<Action>) {
        if catalog::is_non_visual(kind) {
            self.add(source, kind, &[], usize::MAX, &[], actions);
            return;
        }
        let Some(c) = self.current() else { return };
        let mut sel = self.selected.clone().unwrap_or_default();
        let visual: Vec<usize> = (0..c.root.children.len())
            .filter(|&i| catalog::has_geometry(&c.root.children[i].kind))
            .collect();
        if sel.is_empty()
            && visual.len() == 1
            && catalog::accepts(&c.root.children[visual[0]].kind, kind)
        {
            sel = vec![visual[0]];
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
        // Non-visual elements live at the top of the root: the menu bar first, then
        // timers, before anything drawn.
        let (parent, index) = if catalog::is_non_visual(kind) {
            if kind == "MenuBar"
                && (c.root.kind != "Window" || c.root.children.iter().any(|e| e.kind == "MenuBar"))
            {
                self.error = Some(if c.root.kind == "Window" {
                    "This window already has a menu bar".into()
                } else {
                    "Only a Window can have a menu bar".into()
                });
                return;
            }
            let lead = c
                .root
                .children
                .iter()
                .take_while(|e| catalog::is_non_visual(&e.kind))
                .count();
            (&[][..], if kind == "MenuBar" { 0 } else { lead })
        } else {
            (parent, index)
        };
        let mut props = catalog::defaults(kind);
        props.extend_from_slice(extra);
        if kind == "Image" {
            props.extend(self.drag_extra.iter().cloned());
        }
        let (kind, id) = match catalog::template(kind) {
            Some((real, name)) => (
                real,
                Some(if c.find(name).is_none() {
                    name.to_owned()
                } else {
                    unique_id(c, name)
                }),
            ),
            None => (
                kind,
                catalog::wants_id(kind).then(|| unique_id(c, &catalog::snake(kind))),
            ),
        };
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
        ui.horizontal_wrapped(|ui| {
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
            let selection = self.selection();
            let can_align = selection.len() >= 2
                && self
                    .current()
                    .is_some_and(|c| free::common_free_parent(c, &selection).is_some());
            for (how, icon, tip, min) in [
                (free::Arrange::Left, Icon::AlignStartVertical, "Align left edges (to the first selected)", 2),
                (free::Arrange::CenterH, Icon::AlignCenterVertical, "Align centres horizontally", 2),
                (free::Arrange::Right, Icon::AlignEndVertical, "Align right edges", 2),
                (free::Arrange::Top, Icon::AlignStartHorizontal, "Align top edges", 2),
                (free::Arrange::CenterV, Icon::AlignCenterHorizontal, "Align centres vertically", 2),
                (free::Arrange::Bottom, Icon::AlignEndHorizontal, "Align bottom edges", 2),
                (free::Arrange::DistributeH, Icon::AlignHorizontalDistributeCenter, "Space evenly across", 3),
                (free::Arrange::DistributeV, Icon::AlignVerticalDistributeCenter, "Space evenly down", 3),
            ] {
                let enabled = can_align && selection.len() >= min;
                let tip = format!("{tip}. Shift-click to select several freely placed elements.");
                if ui
                    .add_enabled_ui(enabled, |ui| icon_button(ui, icon, &tip))
                    .inner
                    .clicked()
                {
                    let rects = self.layout.elements.clone();
                    let comp = self.component_name();
                    let result = free::arrange(source, &comp, free::Drawn { rects: &rects }, &selection, how);
                    self.keep_multi = true;
                    self.apply_text(actions, source, result);
                }
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
                .on_hover_text("Widget style for the preview (try others here). What the app uses is set in App Style.");
            let dark = self.preview_dark(ui.visuals().dark_mode);
            let (icon, tip) = if dark {
                (Icon::Moon, "Previewing the dark scheme. Click for light.")
            } else {
                (Icon::Sun, "Previewing the light scheme. Click for dark.")
            };
            if icon_button(ui, icon, tip).clicked() {
                self.dark = Some(!dark);
            }
            self.app_style_menu(ui);
            self.language_menu(ui, source, actions);
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
                ui.add(Icon::TriangleAlert.image(ui));
                ui.colored_label(ui.visuals().warn_fg_color, e);
            } else if let Some(err) = self
                .doc
                .errors
                .first()
                .map(|e| e.message.clone())
                .or_else(|| self.preview.diagnostics.first().cloned())
            {
                ui.separator();
                ui.add(Icon::CircleX.image(ui));
                ui.colored_label(ui.visuals().error_fg_color, format!("{err} (fix it in Source)"));
            }
        });
        ui.add_space(2.0);
    }

    fn delete_selected(&mut self, source: &str, actions: &mut Vec<Action>) {
        let mut paths: Vec<Vec<usize>> = self
            .selection()
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        let Some(first) = paths.first().cloned() else {
            return;
        };
        // Later and deeper first, so earlier paths stay valid.
        paths.sort();
        paths.reverse();
        let comp = self.component_name();
        let mut work = source.to_owned();
        let result = (|| {
            for p in &paths {
                let s = work.clone();
                free::step(&mut work, edit::remove_element(&s, &comp, p))?;
            }
            Ok(work)
        })();
        if self.apply_text(actions, source, result) {
            let mut parent = first;
            parent.pop();
            self.selected = Some(parent);
            self.multi.clear();
        }
    }

    /// The selection's source, one element after another: what Copy puts on the
    /// clipboard.
    fn copy_selection(&self, source: &str) -> Option<String> {
        let paths: Vec<Vec<usize>> = self
            .selection()
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        let c = self.current()?;
        let blocks: Vec<String> = paths
            .iter()
            .filter_map(|p| c.at(p))
            .map(|el| edit::element_block(source, el))
            .collect();
        (!blocks.is_empty()).then(|| blocks.join("\n"))
    }

    /// Pastes `.slint` elements: into the selected container, else after the selected
    /// element. Ids that already exist get new ones; freely placed copies move 16 px so
    /// they don't hide the originals. Text that isn't elements is ignored.
    pub fn paste(&mut self, source: &str, text: &str) -> Vec<Action> {
        self.sync(source);
        let mut actions = Vec::new();
        let Some(c) = self.current().cloned() else {
            return actions;
        };
        // Is it elements? Parse it inside a throwaway component.
        let probe = perox_slint::parse(&format!("component PeroxPaste {{\n{text}\n}}"));
        let Some(pc) = probe.components.first().filter(|_| probe.errors.is_empty()) else {
            return actions;
        };
        let pasted = pc.root.children.clone();
        if pasted.is_empty() {
            return actions;
        }
        let sel = self.selected.clone().unwrap_or_default();
        let fits = |el: &Element| pasted.iter().all(|p| catalog::accepts(&el.kind, &p.kind));
        let (parent, index) = match c.at(&sel) {
            Some(el) if fits(el) => (sel.clone(), usize::MAX),
            _ if !sel.is_empty() => {
                let mut parent = sel.clone();
                let i = parent.pop().unwrap_or(0);
                (parent, i + 1)
            }
            _ => {
                self.error = Some("Select where to paste".into());
                return actions;
            }
        };
        let Some(target) = c.at(&parent).cloned() else {
            return actions;
        };
        // New ids for clashes (and for repeats within the paste).
        let mut block = text.trim().to_owned();
        let mut taken: Vec<String> = Vec::new();
        c.root
            .walk(&mut Vec::new(), &mut |el, _| taken.extend(el.id.clone()));
        let mut renames: Vec<(String, String)> = Vec::new();
        pc.root.walk(&mut Vec::new(), &mut |el, _| {
            if let Some(id) = &el.id
                && taken.contains(id)
            {
                let base = id.trim_end_matches(|ch: char| ch.is_ascii_digit());
                let new = (1..)
                    .map(|n| format!("{base}{n}"))
                    .find(|n| !taken.contains(n))
                    .unwrap_or_default();
                taken.push(new.clone());
                renames.push((id.clone(), new));
            }
        });
        for (old, new) in &renames {
            block = replace_word(&block, old, new);
        }
        let comp = c.name.clone();
        let count = target.children.len();
        let first = if index == usize::MAX {
            count
        } else {
            index.min(count)
        };
        let mut work = source.to_owned();
        let result = (|| {
            free::step(
                &mut work,
                edit::insert_source(source, &comp, &parent, index, &block),
            )?;
            let doc = perox_slint::parse(&work);
            let nc = doc
                .component(&comp)
                .ok_or(EditError::NoSuchElement)?
                .clone();
            for k in 0..pasted.len() {
                let mut p = parent.clone();
                p.push(first + k);
                if let Some(el) = nc.at(&p) {
                    let mut kinds = Vec::new();
                    el.walk(&mut Vec::new(), &mut |e, _| kinds.push(e.kind.clone()));
                    for kind in kinds {
                        free::import(&mut work, &kind);
                    }
                    if catalog::is_free(&target.kind)
                        && let (Some(x), Some(y)) = (el.num_prop("x"), el.num_prop("y"))
                    {
                        free::set_position(
                            &mut work,
                            &comp,
                            &p,
                            Pos2::new(x as f32 + 16.0, y as f32 + 16.0),
                        )?;
                    }
                }
            }
            Ok(work)
        })();
        if self.apply_text(&mut actions, source, result) {
            let mut p = parent.clone();
            p.push(first);
            self.selected = Some(p);
            self.multi = (1..pasted.len())
                .map(|k| {
                    let mut q = parent.clone();
                    q.push(first + k);
                    q
                })
                .collect();
            self.keep_multi = true;
        }
        actions
    }

    fn keyboard(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        // Only when no text field has the keyboard.
        if ui.memory(|m| m.focused().is_some()) {
            return;
        }
        // Clipboard: egui turns Cmd+C/X/V into these events.
        let (copy, cut, paste) = ui.input(|i| {
            let mut out = (false, false, None);
            for e in &i.events {
                match e {
                    egui::Event::Copy => out.0 = true,
                    egui::Event::Cut => out.1 = true,
                    egui::Event::Paste(t) => out.2 = Some(t.clone()),
                    _ => {}
                }
            }
            out
        });
        let duplicate = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D));
        if copy || cut || duplicate {
            if let Some(text) = self.copy_selection(source) {
                ui.ctx().copy_text(text.clone());
                self.clipboard = text;
                if cut {
                    self.delete_selected(source, actions);
                } else if duplicate {
                    let text = self.clipboard.clone();
                    actions.extend(self.paste(source, &text));
                }
            }
            return;
        }
        if let Some(text) = paste {
            actions.extend(self.paste(source, &text));
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
        egui::Image::new(catalog::icon(&kind).source(ui.visuals().dark_mode))
            .paint_at(ui, icon_rect);
        painter.galley(rect.min + vec2(24.0, 4.0), galley, Color32::WHITE);
    }

    // ---- canvas ----

    /// The source with live (being-dragged) values applied, for the preview.
    /// The component the designed one inherits (`inherits MainWindow`) and its file,
    /// when it's a component in this file or an imported one, not a built-in.
    pub fn base(&mut self) -> Option<(Component, PathBuf)> {
        let key = hash(&format!("{}|{}", self.doc_key, self.component_name()));
        let fresh = self
            .base
            .as_ref()
            .is_some_and(|(t, k, _)| *k == key && t.elapsed() < std::time::Duration::from_secs(3));
        if !fresh {
            let found = self.find_base();
            self.base = Some((std::time::Instant::now(), key, found));
        }
        self.base.as_ref().and_then(|(_, _, b)| b.clone())
    }

    fn find_base(&self) -> Option<(Component, PathBuf)> {
        let c = self.current()?;
        let name = &c.root.kind;
        if let Some(b) = self.doc.component(name).filter(|b| b.name != c.name) {
            return Some((b.clone(), self.file.clone()));
        }
        let import = self
            .doc
            .imports
            .iter()
            .find(|i| i.names.iter().any(|n| n == name))?;
        if import.from.ends_with("std-widgets.slint") {
            return None;
        }
        let file = self.form_dir().join(&import.from);
        let src = std::fs::read_to_string(&file).ok()?;
        let doc = perox_slint::parse(&src);
        Some((doc.component(name)?.clone(), file))
    }

    /// The app's design-time database file, if one is set.
    fn database_file(&self) -> Option<PathBuf> {
        let db = self.app_ui().database?;
        Some(data::crate_root(&self.form_dir())?.join(db))
    }

    /// Results of the component's bound queries, as the preview shows them.
    fn design_data(&mut self, source: &str) -> Vec<(String, String, Vec<Vec<String>>)> {
        let comp = self.component_name();
        let Some(db) = self.database_file() else {
            return Vec::new();
        };
        data::design_rows(&mut self.data_cache, &db, source, &comp)
    }

    /// The Data section of a list, table or combo box: the database, the query, Run and
    /// Bind.
    fn data_rows(
        &mut self,
        ui: &mut egui::Ui,
        c: &Component,
        el: &Element,
        path: &[usize],
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        let bound = el.id.as_deref().and_then(|id| {
            perox_slint::data::bindings(c, Some(source))
                .into_iter()
                .find(|b| b.id == id)
        });
        ui.horizontal(|ui| {
            label_cell(ui, RichText::new("Database"))
                .on_hover_text("A SQLite file the designer reads to show data while you design");
            let shown = self
                .app_ui()
                .database
                .clone()
                .unwrap_or_else(|| "none".into());
            ui.label(RichText::new(shown).monospace());
            if ui.button("Choose…").clicked()
                && let Some(root) = data::crate_root(&self.form_dir())
                && let Some(file) = rfd::FileDialog::new()
                    .add_filter("SQLite", &["db", "sqlite", "sqlite3"])
                    .set_directory(&root)
                    .pick_file()
            {
                match resources::relative(&root, &file) {
                    Some(rel) => {
                        let mut s = self.app_ui();
                        s.database = Some(rel);
                        if let Err(e) = self.set_app_ui(&s) {
                            self.error = Some(e.to_string());
                        }
                    }
                    None => self.error = Some("Pick a database inside the project".into()),
                }
            }
        });
        let key = format!("{path:?}/#sql");
        let initial = bound
            .as_ref()
            .and_then(|b| b.sql.clone())
            .unwrap_or_default();
        let buf = self.buffers.entry(key.clone()).or_insert(initial);
        ui.add(
            egui::TextEdit::multiline(buf)
                .code_editor()
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("SELECT name, email FROM people"),
        );
        let sql = buf.trim().to_owned();
        ui.horizontal(|ui| {
            let can = !sql.is_empty();
            let run = ui.add_enabled(can, egui::Button::new("Run"));
            let bind = ui
                .add_enabled(can, egui::Button::new(if bound.is_some() { "Update" } else { "Bind" }))
                .on_hover_text("Show this query's rows here: adds a load_<name>(&db) method to the window for your code");
            let result = (run.clicked() || bind.clicked())
                .then(|| match self.database_file() {
                    Some(db) => self.data_cache.run(&db, &sql),
                    None => Err("Choose a database first".into()),
                });
            if let Some(r) = &result {
                self.data_status = Some(match r {
                    Ok(t) => format!("{} rows: {}", t.len(), t.columns.join(", ")),
                    Err(e) => e.clone(),
                });
            }
            if bind.clicked() {
                let cols = result.as_ref().and_then(|r| r.as_ref().ok()).map(|t| t.columns.clone());
                let r = data::bind(source, &c.name, path, &sql, cols.as_deref());
                if self.apply_text(actions, source, r) {
                    self.buffers.remove(&key);
                    self.enable_sqlite();
                }
            }
        });
        if let Some(s) = &self.data_status {
            ui.weak(s);
        }
        if let Some(b) = &bound {
            ui.weak(format!(
                "In code: ui.load_{}(&db) fills it (perox_ui::data::Database).",
                b.id.replace('-', "_")
            ));
        }
    }

    /// Turns on perox-ui's `sqlite` feature in the project's Cargo.toml.
    fn enable_sqlite(&mut self) {
        let Some(root) = data::crate_root(&self.form_dir()) else {
            return;
        };
        let file = root.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&file)
            && let Some(new) = data::with_sqlite_feature(&text)
            && let Err(e) = std::fs::write(&file, new)
        {
            self.error = Some(format!("Couldn't update Cargo.toml: {e}"));
        }
    }

    /// The language menu: preview in a translation, open the Translations window, make
    /// the form's text translatable.
    fn language_menu(&mut self, ui: &mut egui::Ui, source: &str, actions: &mut Vec<Action>) {
        let label = self
            .preview_lang
            .clone()
            .unwrap_or_else(|| "As written".into());
        let r = ui.menu_button(label, |ui| {
            let (dir, _) = translations::location(&self.form_dir());
            let langs = perox_slint::i18n::languages(&dir);
            ui.label(RichText::new("Preview in").small().weak());
            if ui
                .radio(self.preview_lang.is_none(), "The text as written")
                .clicked()
            {
                self.preview_lang = None;
            }
            for l in &langs {
                if ui
                    .radio(self.preview_lang.as_deref() == Some(l.as_str()), l)
                    .clicked()
                {
                    self.preview_lang = Some(l.clone());
                }
            }
            ui.separator();
            if ui.button("Translations…").clicked() {
                self.translations.open(&self.form_dir());
                ui.close();
            }
            if ui
                .button("Make all text translatable")
                .on_hover_text("Wraps every text, title and placeholder in this form in @tr(…)")
                .clicked()
            {
                let comp = self.component_name();
                match translations::make_translatable(source, &comp) {
                    Ok((text, n)) => {
                        self.apply_text(actions, source, Ok(text));
                        self.error = (n == 0).then(|| "No plain text left to translate".into());
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
                ui.close();
            }
        });
        r.response
            .on_hover_text("Languages: preview a translation, edit translations");
    }

    /// The App Style menu: the style and colour scheme the app is built with.
    fn app_style_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.app_ui();
        let r = ui.menu_button(
            RichText::new(format!(
                "App: {}{}",
                current.style.as_deref().unwrap_or("OS style"),
                current
                    .color_scheme
                    .as_deref()
                    .map(|c| format!(", {c}"))
                    .unwrap_or_default()
            )),
            |ui| {
                let mut next = current.clone();
                ui.label(RichText::new("Style").small().weak());
                if ui
                    .radio(
                        next.style.is_none(),
                        "The OS's own (Cupertino on macOS, Fluent elsewhere)",
                    )
                    .clicked()
                {
                    next.style = None;
                }
                for st in perox_slint::project::STYLES {
                    if ui.radio(next.style.as_deref() == Some(*st), *st).clicked() {
                        next.style = Some((*st).to_owned());
                    }
                }
                ui.separator();
                ui.label(RichText::new("Colours").small().weak());
                if ui
                    .radio(
                        next.color_scheme.is_none(),
                        "Follow the system (light or dark)",
                    )
                    .clicked()
                {
                    next.color_scheme = None;
                }
                for c in perox_slint::project::COLOR_SCHEMES {
                    if ui
                        .radio(
                            next.color_scheme.as_deref() == Some(*c),
                            format!("Always {c}"),
                        )
                        .clicked()
                    {
                        next.color_scheme = Some((*c).to_owned());
                    }
                }
                if next != current {
                    match self.set_app_ui(&next) {
                        Ok(_) => self.error = None,
                        Err(e) => self.error = Some(format!("Couldn't save the app style: {e}")),
                    }
                }
            },
        );
        r.response.on_hover_text(
            "The style and colours the app is built with (ui/slint-project.json). Rebuild to see it in the running app.",
        );
    }

    /// Asks for an image or font file and brings it into the form's folder (`sub`:
    /// `images` or `fonts`). Returns its path relative to the form.
    fn choose_resource(&mut self, sub: &str) -> Option<String> {
        let dir = self.form_dir();
        let exts = if sub == "fonts" {
            resources::FONT_EXTENSIONS
        } else {
            resources::IMAGE_EXTENSIONS
        };
        let file = rfd::FileDialog::new()
            .add_filter(if sub == "fonts" { "Fonts" } else { "Images" }, exts)
            .set_directory(&dir)
            .pick_file()?;
        match resources::import_file(&file, &dir, sub) {
            Ok(rel) => {
                self.images = None;
                Some(rel)
            }
            Err(e) => {
                self.error = Some(format!("Couldn't copy {}: {e}", file.display()));
                None
            }
        }
    }

    /// The component tray: elements not drawn on the form (timers, the menu bar), as
    /// chips to select. Shown when there are any, or while one is being dragged in.
    fn tray(&mut self, ui: &mut egui::Ui, base: Id) {
        self.layout.tray.clear();
        let Some(c) = self.current().cloned() else {
            return;
        };
        let mut items: Vec<(Vec<usize>, String, String)> = Vec::new();
        c.root.walk(&mut Vec::new(), &mut |el, p| {
            if catalog::is_non_visual(&el.kind) {
                items.push((
                    p.to_vec(),
                    el.kind.clone(),
                    el.id.clone().unwrap_or_default(),
                ));
            }
        });
        let incoming = DragAndDrop::payload::<Drag>(ui.ctx()).and_then(|d| match &*d {
            Drag::New(k) if catalog::is_non_visual(k) => Some(*k),
            _ => None,
        });
        if items.is_empty() && incoming.is_none() {
            return;
        }
        egui::Panel::bottom(base.with("tray"))
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Components").small().weak())
                        .on_hover_text("Parts of the form that aren't drawn on it");
                    for (path, kind, id) in &items {
                        let selected = self.selected.as_deref() == Some(path.as_slice());
                        let label = if id.is_empty() {
                            kind.clone()
                        } else {
                            id.clone()
                        };
                        let r = ui
                            .add(
                                egui::Button::image_and_text(catalog::icon(kind).image(ui), label)
                                    .selected(selected),
                            )
                            .on_hover_text(kind.as_str());
                        self.layout.tray.push((path.clone(), r.rect));
                        if r.clicked() {
                            self.selected = Some(path.clone());
                            self.error = None;
                        }
                    }
                    if let Some(k) = incoming {
                        ui.label(RichText::new(format!("Drop to add a {k}")).italics().weak());
                    }
                });
                ui.add_space(3.0);
            });
    }

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
        let dark = self.preview_dark(ui.visuals().dark_mode);
        self.preview.design_data = self.design_data(&shown);
        self.preview.language = self.preview_lang.as_ref().map(|l| {
            let (dir, domain) = translations::location(&self.form_dir());
            (dir.join(perox_slint::i18n::DIR), domain, l.clone())
        });
        self.preview.update(&shown, &path, &c.name, &style, dark);

        let size = self.window_size();
        // A form inheriting a window is a window too.
        let base_kind = self.base().map(|(b, _)| b.root.kind);
        let is_window = c.is_window()
            || base_kind
                .as_deref()
                .is_some_and(|k| matches!(k, "Window" | "Dialog"));
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
                if p.is_empty() || !catalog::has_geometry(&el.kind) {
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
            // A window filled by one layout isn't placing anything freely.
            let filled = el.children.len() == 1 && catalog::is_layout(&el.children[0].kind);
            if catalog::is_free(&el.kind)
                && !filled
                && self.selected.as_deref() == Some(p.as_slice())
            {
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
        // Text that's empty here (bound to code, or blank): a faint dashed box with what
        // it shows, so it can be found and selected.
        for (p, r) in rects {
            let Some(el) = c.at(p) else { continue };
            if el.kind != "Text" {
                continue;
            }
            let hint = match el.prop("text") {
                Some(Value::Str(t)) if !t.is_empty() => continue,
                Some(Value::Str(_)) | None => "(empty text)".to_owned(),
                Some(v) => v.to_source(),
            };
            let box_ = if r.height() < 4.0 {
                Rect::from_min_size(r.min, vec2(r.width().max(60.0), 16.0))
            } else {
                *r
            };
            let faint = ui.visuals().weak_text_color().gamma_multiply(0.7);
            let pts = [
                box_.left_top(),
                box_.right_top(),
                box_.right_bottom(),
                box_.left_bottom(),
                box_.left_top(),
            ];
            painter.extend(egui::Shape::dashed_line(
                &pts,
                Stroke::new(1.0, faint),
                3.0,
                3.0,
            ));
            painter.text(
                box_.left_center() + vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                hint,
                egui::FontId::proportional(11.0),
                faint,
            );
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
        for (i, sel) in self.selection().iter().enumerate() {
            if sel.is_empty() {
                continue;
            }
            if let Some((_, r)) = rects.iter().find(|(p, _)| p == sel) {
                // The primary selection is solid; the others lighter.
                let c = if i == 0 {
                    accent
                } else {
                    accent.gamma_multiply(0.6)
                };
                painter.rect_stroke(*r, 0.0, Stroke::new(2.0, c), StrokeKind::Outside);
            }
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
            let m = ui.input(|i| i.modifiers);
            if (m.shift || m.command) && !p.is_empty() {
                self.toggle(p);
            } else {
                self.selected = Some(p);
            }
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
        if let Drag::New(k) = &*drag
            && catalog::is_non_visual(k)
        {
            // Not placed anywhere in particular: it joins the tray.
            ui.painter().rect_stroke(
                content,
                2.0,
                Stroke::new(2.0, ui.visuals().selection.bg_fill),
                StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                DragAndDrop::clear_payload(ui.ctx());
                self.add(source, k, &[], usize::MAX, &[], actions);
            }
            return;
        }
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
        if let Some((base, file)) = self.base() {
            let file = file
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            egui::CollapsingHeader::new(
                RichText::new(format!("Inherited from {}", base.name)).weak(),
            )
            .id_salt("inherited")
            .default_open(false)
            .show(ui, |ui| {
                ui.weak(format!("Edit these in {file}"));
                for child in &base.root.children {
                    inherited_node(ui, child);
                }
            });
        }
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
        let base_kind = if path.is_empty() {
            self.base().map(|(b, _)| b.root.kind)
        } else {
            None
        };
        let spec = catalog::kind(&el.kind).or_else(|| base_kind.as_deref().and_then(catalog::kind));
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

        self.menu_buttons(ui, &el, &path, source, actions);
        self.action_rows(ui, &c, &el, &path, source, actions);
        if perox_slint::data::target(&el.kind).is_some() {
            egui::CollapsingHeader::new("Data")
                .default_open(true)
                .show(ui, |ui| self.data_rows(ui, &c, &el, &path, source, actions));
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
        let geometry = catalog::has_geometry(&el.kind);
        let free_child = geometry && self.in_free_parent(&path);
        let mut layout: Vec<PropSpec> = Vec::new();
        if free_child {
            layout.extend_from_slice(catalog::FREE_CHILD);
        }
        if !path.is_empty() && geometry {
            layout.extend_from_slice(catalog::COMMON);
        }
        if geometry && parent_kind.as_deref().is_some_and(catalog::is_layout) {
            layout.extend_from_slice(catalog::LAYOUT_CHILD);
        }
        if geometry && parent_kind.as_deref().is_some_and(catalog::is_grid) {
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

        if path.is_empty()
            && let Some((base, _)) = self.base()
        {
            let specs: Vec<PropSpec> = base
                .properties
                .iter()
                .filter(|p| matches!(p.visibility.as_str(), "in" | "in-out"))
                .map(|p| PropSpec {
                    name: p.name.clone().into(),
                    kind: PropKind::Source,
                    help: "From the base form; edited as a .slint expression",
                })
                .collect();
            if !specs.is_empty() {
                egui::CollapsingHeader::new(format!("From {}", base.name))
                    .default_open(true)
                    .show(ui, |ui| {
                        self.prop_rows(ui, &c.name, &el, &path, &specs, source, actions)
                    });
            }
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

    /// Actions (see [`actions`]): on the root, the list with "can be disabled" and a
    /// field for a new one; on a button or menu item, which action it runs; on a menu or
    /// a toolbar, "add an entry for this action".
    fn action_rows(
        &mut self,
        ui: &mut egui::Ui,
        c: &Component,
        el: &Element,
        path: &[usize],
        source: &str,
        out: &mut Vec<Action>,
    ) {
        let list = actions::list(c);
        let comp = c.name.clone();
        let apply = |this: &mut Self, out: &mut Vec<Action>, r: Result<String, EditError>| {
            this.apply_text(out, source, r);
        };
        if path.is_empty() {
            egui::CollapsingHeader::new("Actions")
                .default_open(true)
                .show(ui, |ui| {
                    if list.is_empty() {
                        ui.weak("Commands that menu items and buttons share.");
                    }
                    for a in &list {
                        ui.horizontal(|ui| {
                            label_cell(ui, RichText::new(&a.name).strong()).on_hover_text(
                                format!("callback {}(); used by {} element(s)", a.name, a.users.len()),
                            );
                            let mut on = a.enabled.is_some();
                            if ui
                                .checkbox(&mut on, "can be disabled")
                                .on_hover_text(format!(
                                    "Adds `in-out property <bool> {}` and binds `enabled` of everything that runs it",
                                    actions::enabled_name(&a.name)
                                ))
                                .changed()
                            {
                                apply(self, out, actions::set_can_disable(source, &comp, &a.name, on));
                            }
                            ui.weak(match a.users.len() {
                                1 => "1 use".to_owned(),
                                n => format!("{n} uses"),
                            });
                        });
                    }
                    ui.horizontal(|ui| {
                        label_cell(ui, RichText::new("New action").weak());
                        if let Some(name) = self.text_field(ui, "#new-action", "", false) {
                            let name = name.trim().replace('-', "_");
                            if !name.is_empty() {
                                apply(self, out, actions::add(source, &comp, &name, false));
                            }
                        }
                    });
                });
            return;
        }
        if let Some(event) = actions::trigger(&el.kind)
            && !list.is_empty()
            && matches!(el.kind.as_str(), "Button" | "MenuItem")
        {
            let current = el
                .handler(event)
                .and_then(|h| h.forwards_to())
                .filter(|f| list.iter().any(|a| a.name == *f))
                .map(str::to_owned);
            ui.horizontal(|ui| {
                label_cell(ui, RichText::new("Action"))
                    .on_hover_text("Run one of the form's actions (and follow its enabled flag)");
                egui::ComboBox::from_id_salt(format!("{path:?}/action"))
                    .selected_text(current.clone().unwrap_or_else(|| "none".into()))
                    .show_ui(ui, |ui| {
                        for a in &list {
                            if ui
                                .selectable_label(
                                    current.as_deref() == Some(a.name.as_str()),
                                    &a.name,
                                )
                                .clicked()
                            {
                                apply(self, out, actions::use_action(source, &comp, path, &a.name));
                            }
                        }
                    });
            });
        }
        let entry = match el.kind.as_str() {
            "Menu" => Some(("MenuItem", "Add item for")),
            k if catalog::is_layout(k) && catalog::is_horizontal(k) => {
                Some(("Button", "Add button for"))
            }
            _ => None,
        };
        if let Some((kind, label)) = entry
            && !list.is_empty()
        {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(format!("{path:?}/add-for"))
                    .selected_text(format!("{label}…"))
                    .show_ui(ui, |ui| {
                        for a in &list {
                            if ui.selectable_label(false, &a.name).clicked() {
                                match actions::add_user(source, &comp, path, kind, &a.name) {
                                    Ok((text, new)) => {
                                        if self.apply_text(out, source, Ok(text)) {
                                            self.selected = Some(new);
                                        }
                                    }
                                    Err(e) => self.error = Some(e.to_string()),
                                }
                            }
                        }
                    });
            });
        }
    }

    /// Building menus: a menu bar takes menus; a menu takes items, separators and
    /// submenus; a context menu area takes its one menu. New entries go last and are
    /// selected, ready to be named.
    fn menu_buttons(
        &mut self,
        ui: &mut egui::Ui,
        el: &Element,
        path: &[usize],
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        let offers: &[(&'static str, &str, Icon)] = match el.kind.as_str() {
            "MenuBar" => &[("Menu", "Add menu", Icon::SquareMenu)],
            "Menu" => &[
                ("MenuItem", "Add item", Icon::Menu),
                ("MenuSeparator", "Add separator", Icon::SeparatorHorizontal),
                ("Menu", "Add submenu", Icon::SquareMenu),
            ],
            "ContextMenuArea" if !el.children.iter().any(|c| c.kind == "Menu") => {
                &[("Menu", "Add its menu", Icon::SquareMenu)]
            }
            "Window" if path.is_empty() && !el.children.iter().any(|c| c.kind == "MenuBar") => {
                &[("MenuBar", "Add menu bar", Icon::PanelTop)]
            }
            _ => &[],
        };
        if offers.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for (kind, label, icon) in offers {
                if ui
                    .add(egui::Button::image_and_text(icon.image(ui), *label))
                    .clicked()
                {
                    // A context menu's menu goes first, before the content it covers.
                    let index = if el.kind == "ContextMenuArea" {
                        0
                    } else {
                        usize::MAX
                    };
                    self.add(source, kind, path, index, &[], actions);
                }
            }
        });
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
                self.expose_toggle(ui, comp, el, path, spec, source, actions);
                if let Some(v) = self.value_editor(ui, path, spec, value.as_ref()) {
                    match self.pending_import.take() {
                        // A new font: the property and its `import` in one undo step.
                        Some((name, from)) => {
                            let mut work = source.to_owned();
                            let result = free::step(
                                &mut work,
                                edit::set_property(source, comp, path, &spec.name, &v),
                            )
                            .map(|()| {
                                let e = match &name {
                                    Some(n) => edit::ensure_import(&work, n, &from),
                                    None => edit::ensure_file_import(&work, &from),
                                };
                                edit::apply(&work, &e)
                            });
                            self.apply_text(actions, source, result);
                        }
                        None => {
                            self.apply(
                                actions,
                                edit::set_property(source, comp, path, &spec.name, &v),
                            );
                        }
                    }
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

    /// The "use from code" toggle on a property row: exposes it as a root property
    /// (`in-out property <t> id_prop <=> id.prop;`), so Rust gets `get_`/`set_`.
    #[allow(clippy::too_many_arguments)]
    fn expose_toggle(
        &mut self,
        ui: &mut egui::Ui,
        comp: &str,
        el: &Element,
        path: &[usize],
        spec: &PropSpec,
        source: &str,
        actions: &mut Vec<Action>,
    ) {
        let width = 16.0;
        if path.is_empty() || catalog::slint_type(spec).is_none() {
            ui.add_space(width + ui.spacing().item_spacing.x);
            return;
        }
        let exposed = el.id.as_ref().and_then(|id| {
            self.current()?
                .exposing(&format!("{id}.{}", spec.name))
                .map(|p| p.name.clone())
        });
        let tip = match &exposed {
            Some(n) => {
                let r = perox_slint::codegen::rust_name(n);
                format!("Used from code: ui.get_{r}() and ui.set_{r}(…). Click to stop.")
            }
            None => "Use from code: adds get_/set_ methods for this property".to_owned(),
        };
        // In full colour when code can use it, washed out when not.
        let icon = if exposed.is_some() {
            Icon::Code.image(ui)
        } else {
            Icon::Code.faded(ui)
        }
        .fit_to_exact_size(Vec2::splat(13.0));
        let b = egui::Button::image(icon)
            .frame(false)
            .min_size(vec2(width, 0.0));
        if ui.add(b).on_hover_text(tip).clicked() {
            let result = match exposed {
                Some(_) => unexpose_property(source, comp, path, &spec.name),
                None => expose_property(source, comp, path, &spec.name),
            }
            .map(|new| edit::diff(source, &new));
            self.apply(actions, result);
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
            (PropKind::Keys, Some(Value::Expr(e))) if keys::inner(e).is_some() => PropKind::Keys,
            (PropKind::Keys, None) => PropKind::Keys,
            (PropKind::Text, Some(Value::Expr(e))) if perox_slint::i18n::tr_text(e).is_some() => {
                PropKind::Text
            }
            (PropKind::Image, Some(Value::Expr(e))) if resources::image_path(e).is_some() => {
                PropKind::Image
            }
            (_, Some(Value::Expr(_))) => PropKind::Source,
            (k, _) => k,
        };
        match kind {
            PropKind::Keys => {
                let current = match value {
                    Some(Value::Expr(e)) => keys::inner(e).unwrap_or_default().to_owned(),
                    _ => String::new(),
                };
                if self.recording.as_deref() == Some(key.as_str()) {
                    let press = ui.input(|i| {
                        i.events.iter().find_map(|e| match e {
                            egui::Event::Key {
                                key,
                                pressed: true,
                                modifiers,
                                ..
                            } => Some((*key, *modifiers)),
                            _ => None,
                        })
                    });
                    let r = ui.add(
                        egui::Button::new(RichText::new("Press a shortcut…").italics())
                            .selected(true),
                    );
                    if let Some((k, m)) = press {
                        self.recording = None;
                        if k == egui::Key::Escape && m.is_none() {
                            return None;
                        }
                        // Don't let the press also act in the designer (Delete, arrows).
                        ui.input_mut(|i| {
                            i.events.retain(|e| !matches!(e, egui::Event::Key { .. }))
                        });
                        return keys::from_press(k, m).map(|t| keys::expr(&t));
                    }
                    if r.clicked() || r.clicked_elsewhere() {
                        self.recording = None;
                    }
                    return None;
                }
                let shown = if current.is_empty() {
                    "none".to_owned()
                } else {
                    keys::display(&current, cfg!(target_os = "macos"))
                };
                let r = ui.button(shown).on_hover_text(format!(
                    "Click, then press the shortcut (Esc cancels). In the file: {}",
                    if current.is_empty() {
                        "unset".into()
                    } else {
                        keys::expr(&current)
                    }
                ));
                if r.clicked() {
                    self.recording = Some(key.clone());
                }
                None
            }
            PropKind::Image => {
                let current = match value {
                    Some(Value::Expr(e)) => resources::image_path(e),
                    _ => None,
                };
                let shown = current
                    .as_deref()
                    .map(|c| c.rsplit('/').next().unwrap_or(c).to_owned())
                    .unwrap_or_else(|| "none".into());
                let mut picked = None;
                egui::ComboBox::from_id_salt(&key)
                    .selected_text(shown)
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for rel in self.project_images() {
                            if ui
                                .selectable_label(current.as_deref() == Some(rel.as_str()), &rel)
                                .clicked()
                            {
                                picked = Some(resources::image_url(&rel));
                            }
                        }
                        ui.separator();
                        if ui.button("Choose a file…").clicked() {
                            picked = self
                                .choose_resource("images")
                                .map(|rel| resources::image_url(&rel));
                        }
                    });
                picked
            }
            PropKind::Font => {
                let current = value.and_then(Value::as_str).map(str::to_owned);
                let fonts = resources::imported_fonts(&self.doc, &self.form_dir());
                let mut picked = None;
                egui::ComboBox::from_id_salt(&key)
                    .selected_text(current.clone().unwrap_or_else(|| "default".into()))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for (family, file) in &fonts {
                            if ui
                                .selectable_label(
                                    current.as_deref() == Some(family.as_str()),
                                    family,
                                )
                                .on_hover_text(file.as_str())
                                .clicked()
                            {
                                picked = Some(perox_slint::syntax::quote(family));
                            }
                        }
                        if !fonts.is_empty() {
                            ui.separator();
                        }
                        if ui.button("Add a font file…").clicked()
                            && let Some(rel) = self.choose_resource("fonts")
                        {
                            match resources::font_family(&self.form_dir().join(&rel)) {
                                Some(family) => {
                                    self.pending_import = Some((None, rel));
                                    picked = Some(perox_slint::syntax::quote(&family));
                                }
                                None => {
                                    self.error =
                                        Some(format!("{rel} isn't a font file Slint can read"))
                                }
                            }
                        }
                    });
                picked
            }
            PropKind::Duration => {
                // Shown and edited in milliseconds; written as `2s` when it's whole seconds.
                let current = match value {
                    Some(Value::Num(n, unit)) => Some(match unit.as_str() {
                        "s" => n * 1000.0,
                        _ => *n,
                    }),
                    _ => None,
                };
                let mut n = current.unwrap_or(1000.0);
                let r = ui.add(
                    egui::DragValue::new(&mut n)
                        .speed(10.0)
                        .range(0.0..=f64::MAX)
                        .fixed_decimals(0)
                        .suffix(" ms"),
                );
                (r.changed() && !r.dragged() || r.drag_stopped()).then(|| {
                    let ms = n.round().max(0.0) as i64;
                    if ms % 1000 == 0 && ms > 0 {
                        format!("{}s", ms / 1000)
                    } else {
                        format!("{ms}ms")
                    }
                })
            }
            PropKind::Text => {
                let tr = match value {
                    Some(Value::Expr(e)) => perox_slint::i18n::tr_text(e),
                    _ => None,
                };
                let translatable = tr.is_some();
                let current = tr
                    .or_else(|| value.and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_default();
                let wrap = |t: &str, on: bool| {
                    if on {
                        perox_slint::i18n::tr(t)
                    } else {
                        perox_slint::syntax::quote(t)
                    }
                };
                let mut out = self
                    .text_field(ui, &key, &current, false)
                    .map(|t| wrap(&t, translatable));
                let r = ui
                    .add(
                        egui::Button::image(if translatable {
                            Icon::Languages.image(ui)
                        } else {
                            Icon::Languages.faded(ui)
                        })
                        .selected(translatable)
                        .frame(false),
                    )
                    .on_hover_text(if translatable {
                        "Translatable (@tr): click to make it plain text"
                    } else {
                        "Make it translatable (@tr)"
                    });
                if r.clicked() && (value.is_some() || !current.is_empty()) {
                    out = Some(wrap(&current, !translatable));
                }
                out
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
                let current = live.or(value.and_then(Value::as_f64));
                let unset = current.is_none();
                let mut n = current.unwrap_or(0.0);
                let mut dv = egui::DragValue::new(&mut n).speed(if kind == PropKind::Float {
                    0.05
                } else {
                    0.5
                });
                if kind == PropKind::Int {
                    dv = dv.fixed_decimals(0);
                }
                if unset {
                    // Not set in the file: Slint decides (layout, style default).
                    dv = dv.custom_formatter(|_, _| "auto".to_owned());
                } else if !unit.is_empty() {
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
                // Unset: the documented default, e.g. "(true)" for `enabled`.
                let default = spec.help.starts_with("(true)");
                let mut b = value.and_then(Value::as_bool).unwrap_or(default);
                ui.checkbox(&mut b, "").changed().then(|| b.to_string())
            }
            PropKind::Color => {
                let palette = match value {
                    Some(Value::Ident(i)) => i.strip_prefix("Palette.").map(str::to_owned),
                    _ => None,
                };
                let mut picked = None;
                if let Some(name) = &palette {
                    // A theme colour: follows the style and light/dark.
                    ui.label(RichText::new(name).italics())
                        .on_hover_text(format!(
                            "Palette.{name}: the style's colour, light or dark"
                        ));
                } else {
                    let mut c = match value {
                        Some(Value::Color([r, g, b, a])) => {
                            Color32::from_rgba_unmultiplied(*r, *g, *b, *a)
                        }
                        _ => ui.visuals().text_color(),
                    };
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut c,
                        egui::color_picker::Alpha::OnlyBlend,
                    )
                    .changed()
                    {
                        picked = Some(Value::Color(c.to_srgba_unmultiplied()).to_source());
                    }
                }
                ui.menu_image_button(Icon::Paintbrush.image(ui), |ui| {
                    ui.label(
                        RichText::new("Theme colours: follow the style and light/dark")
                            .small()
                            .weak(),
                    );
                    for name in PALETTE_COLORS {
                        if ui
                            .selectable_label(palette.as_deref() == Some(*name), *name)
                            .clicked()
                        {
                            self.pending_import =
                                Some((Some("Palette".into()), catalog::STD_WIDGETS.into()));
                            picked = Some(format!("Palette.{name}"));
                        }
                    }
                })
                .response
                .on_hover_text("Use a theme colour");
                picked
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
            PropKind::Source | PropKind::Data(_) => {
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

/// Exposes property `prop` of the element at `path` to code: names the element if
/// needed, then declares `in-out property <type> <id>_<prop> <=> <id>.<prop>;` on the
/// component. Returns the new source.
pub fn expose_property(
    source: &str,
    comp: &str,
    path: &[usize],
    prop: &str,
) -> Result<String, EditError> {
    let doc = perox_slint::parse(source);
    let c = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.into()))?;
    let el = c.at(path).ok_or(EditError::NoSuchElement)?;
    if path.is_empty() {
        return Err(EditError::Root);
    }
    let spec = catalog::kind(&el.kind)
        .into_iter()
        .flat_map(|k| k.props.iter())
        .chain(catalog::COMMON)
        .chain(catalog::FREE_CHILD)
        .find(|p| p.name == prop);
    let ty = spec.and_then(catalog::slint_type).ok_or_else(|| {
        EditError::InvalidName(format!("{prop} (can't expose this property yet)"))
    })?;
    let mut work = source.to_owned();
    let id = match &el.id {
        Some(id) => id.clone(),
        None => {
            let id = unique_id(c, &catalog::snake(&el.kind));
            free::step(&mut work, edit::set_id(source, comp, path, Some(&id)))?;
            id
        }
    };
    let name = format!("{id}_{prop}").replace('-', "_");
    let s = work.clone();
    if catalog::binding_only(&el.kind, prop) {
        // The root property takes over the current value, and the element binds to it.
        let current = el
            .props
            .iter()
            .find(|p| p.name == prop)
            .map(|p| p.value.to_source());
        free::step(
            &mut work,
            edit::add_property(&s, comp, &name, ty, current.as_deref()),
        )?;
        let s = work.clone();
        free::step(
            &mut work,
            edit::set_property(&s, comp, path, prop, &format!("root.{name}")),
        )?;
    } else {
        free::step(
            &mut work,
            edit::expose_property(&s, comp, &name, ty, &format!("{id}.{prop}")),
        )?;
    }
    Ok(work)
}

/// Stops using a property from code: removes the root property, and for a bound one
/// (see [`catalog::binding_only`]) gives the element back the property's value.
pub fn unexpose_property(
    source: &str,
    comp: &str,
    path: &[usize],
    prop: &str,
) -> Result<String, EditError> {
    let doc = perox_slint::parse(source);
    let c = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.into()))?;
    let el = c.at(path).ok_or(EditError::NoSuchElement)?;
    let Some(id) = &el.id else {
        return Ok(source.to_owned());
    };
    let Some(decl) = c.exposing(&format!("{id}.{prop}")).cloned() else {
        return Ok(source.to_owned());
    };
    let mut work = source.to_owned();
    if decl.alias.is_none() {
        let value = edit::property_initial_value(source, comp, &decl.name);
        let s = work.clone();
        match value {
            Some(v) => free::step(&mut work, edit::set_property(&s, comp, path, prop, &v))?,
            None => free::step(&mut work, edit::remove_property(&s, comp, path, prop))?,
        }
    }
    let s = work.clone();
    free::step(&mut work, edit::remove_property_decl(&s, comp, &decl.name))?;
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

/// Replaces whole-word `old` (a Slint identifier) with `new`.
fn replace_word(text: &str, old: &str, new: &str) -> String {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(old) {
        let before_ok = rest[..i].chars().last().is_none_or(|c| !is_ident(c));
        let after = &rest[i + old.len()..];
        let after_ok = after.chars().next().is_none_or(|c| !is_ident(c));
        out.push_str(&rest[..i]);
        out.push_str(if before_ok && after_ok { new } else { old });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `button1`, `button2`, …: the first id not already in the component.
pub fn unique_id(c: &Component, base: &str) -> String {
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|id| c.find(id).is_none())
        .unwrap_or_else(|| base.to_owned())
}

/// An element of the base form in the Objects tree: shown, not selectable.
fn inherited_node(ui: &mut egui::Ui, el: &Element) {
    let label = match (&el.id, el.str_prop("text").or_else(|| el.str_prop("title"))) {
        (Some(id), _) => format!("{id}  {}", el.kind),
        (None, Some(t)) => format!("\u{201c}{t}\u{201d}  {}", el.kind),
        (None, None) => el.kind.clone(),
    };
    ui.horizontal(|ui| {
        ui.add(catalog::icon(&el.kind).faded(ui));
        ui.label(RichText::new(label).weak());
    });
    if !el.children.is_empty() {
        ui.indent(el.kind_span.start, |ui| {
            for c in &el.children {
                inherited_node(ui, c);
            }
        });
    }
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
    } else if let Some(caption) = el.str_prop("title").or_else(|| el.str_prop("text")) {
        // Unnamed menus, tabs and labels: their caption tells them apart.
        RichText::new(format!("\u{201c}{caption}\u{201d}  ")).append_to(
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
        // Centred, but never over the three window buttons.
        let galley = painter.layout_no_wrap(
            title.to_owned(),
            egui::TextStyle::Body.resolve(ui.style()),
            v.text_color(),
        );
        let min_x = bar.left() + 64.0;
        let x = (bar.center().x - galley.size().x / 2.0).max(min_x);
        painter.with_clip_rect(bar).galley(
            Pos2::new(x, bar.center().y - galley.size().y / 2.0),
            galley,
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
