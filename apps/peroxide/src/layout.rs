//! The docking layout (egui_tiles): Explorer, editor tabs, Problems, Output, Activity.
//! Panes never change state directly: they collect commands in `actions`.

use eframe::egui::{self, RichText, WidgetText};
use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse};
use perox_editor::Severity;
use perox_editor::completion::Recency;
use perox_editor::view::{self, Request};
use perox_icons::Icon;
use serde_json::{Value, json};

use crate::commands::Activity;
use crate::docs::{DocId, Docs};
use crate::ide::BuildState;
use crate::keymap;
use crate::lsp::Lsp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Explorer,
    Welcome,
    Editor(DocId),
    Problems,
    Output,
    Activity,
    References,
    Terminal,
    SourceControl,
}

/// Explorer on the left; editors above, Problems / Output / Activity below.
pub fn default_tree() -> Tree<Pane> {
    let mut tiles = Tiles::default();
    let explorer = tiles.insert_pane(Pane::Explorer);
    let welcome = tiles.insert_pane(Pane::Welcome);
    let problems = tiles.insert_pane(Pane::Problems);
    let output = tiles.insert_pane(Pane::Output);
    let activity = tiles.insert_pane(Pane::Activity);
    let left = tiles.insert_tab_tile(vec![explorer]);
    let center = tiles.insert_tab_tile(vec![welcome]);
    let bottom = tiles.insert_tab_tile(vec![problems, output, activity]);
    let right = tiles.insert_vertical_tile(vec![center, bottom]);
    let root = tiles.insert_horizontal_tile(vec![left, right]);
    set_shares(&mut tiles, right, &[(center, 3.0), (bottom, 1.0)]);
    set_shares(&mut tiles, root, &[(left, 1.0), (right, 4.5)]);
    Tree::new(egui::Id::unique("main_layout"), root, tiles)
}

fn set_shares(tiles: &mut Tiles<Pane>, container: TileId, shares: &[(TileId, f32)]) {
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(container) {
        for (id, share) in shares {
            linear.shares.set_share(*id, *share);
        }
    }
}

fn find_pane(tree: &Tree<Pane>, want: impl Fn(&Pane) -> bool) -> Option<TileId> {
    tree.tiles
        .iter()
        .find(|(_, t)| matches!(t, Tile::Pane(p) if want(p)))
        .map(|(id, _)| *id)
}

/// Shows a pane in its tab group; adds an editor tab next to the other editors.
pub fn show_pane(tree: &mut Tree<Pane>, pane: Pane) {
    let tile = match find_pane(tree, |p| *p == pane) {
        Some(t) => t,
        None => {
            // Editors go next to editors (or Welcome); panels next to the other panels.
            let bottom = |p: &Pane| {
                matches!(
                    p,
                    Pane::Problems
                        | Pane::Output
                        | Pane::Activity
                        | Pane::References
                        | Pane::Terminal
                        | Pane::SourceControl
                )
            };
            let sibling = if bottom(&pane) {
                find_pane(tree, bottom)
            } else {
                find_pane(tree, |p| matches!(p, Pane::Editor(_)))
                    .or_else(|| find_pane(tree, |p| *p == Pane::Welcome))
            }
            .or_else(|| find_pane(tree, |p| *p != Pane::Explorer));
            let new = tree.tiles.insert_pane(pane);
            match sibling.and_then(|s| tree.tiles.parent_of(s)) {
                Some(parent) => {
                    if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(parent)
                    {
                        tabs.add_child(new);
                    } else if let Some(Tile::Container(c)) = tree.tiles.get_mut(parent) {
                        c.add_child(new);
                    }
                }
                None => {
                    *tree = default_tree();
                    return show_pane(tree, pane);
                }
            }
            new
        }
    };
    tree.make_active(|id, _| id == tile);
}

pub fn remove_pane(tree: &mut Tree<Pane>, pane: Pane) {
    if let Some(t) = find_pane(tree, |p| *p == pane) {
        tree.remove_recursively(t);
    }
}

/// Draws the panes for one frame.
pub struct PaneUi<'a> {
    pub docs: &'a mut Docs,
    pub recency: &'a mut Recency,
    pub activity: &'a Activity,
    pub lsp: &'a Lsp,
    pub build: &'a BuildState,
    pub tree: Option<&'a perox_project::Node>,
    pub project_root: Option<&'a std::path::Path>,
    pub recent: &'a [std::path::PathBuf],
    pub references: &'a [(std::path::PathBuf, usize, usize, String)],
    pub terminal: Option<&'a mut perox_terminal::Terminal>,
    pub git: &'a crate::git::Git,
    pub commit_message: &'a mut String,
    pub active_doc: Option<DocId>,
    /// Commands to run after drawing.
    pub actions: Vec<(&'static str, Value)>,
    /// Language-server requests from editors.
    pub requests: Vec<(DocId, Request)>,
    /// The editor that has focus (becomes the active document).
    pub focused: Option<DocId>,
    /// Tabs closed with their ✕.
    pub closed: Vec<DocId>,
}

impl egui_tiles::Behavior<Pane> for PaneUi<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        match pane {
            Pane::Explorer => "Explorer".into(),
            Pane::Welcome => "Welcome".into(),
            Pane::Editor(id) => self
                .docs
                .get(*id)
                .map_or("(closed)".into(), |d| d.title())
                .into(),
            Pane::Problems => {
                let n: usize = self.lsp.diagnostics.values().map(Vec::len).sum::<usize>()
                    + self.build.diagnostics.len();
                if n == 0 {
                    "Problems".into()
                } else {
                    format!("Problems ({n})").into()
                }
            }
            Pane::Output => "Output".into(),
            Pane::Activity => "Activity".into(),
            Pane::References => format!("References ({})", self.references.len()).into(),
            Pane::Terminal => self
                .terminal
                .as_ref()
                .map_or("Terminal".to_owned(), |t| t.title.clone())
                .into(),
            Pane::SourceControl => match self.git.status.files.len() {
                0 => "Source Control".into(),
                n => format!("Source Control ({n})").into(),
            },
        }
    }

    fn is_tab_closable(&self, tiles: &Tiles<Pane>, tile_id: TileId) -> bool {
        matches!(tiles.get(tile_id), Some(Tile::Pane(Pane::Editor(_))))
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<Pane>, tile_id: TileId) -> bool {
        if let Some(Tile::Pane(Pane::Editor(id))) = tiles.get(tile_id) {
            let id = *id;
            if self.docs.get(id).is_some_and(|d| d.is_dirty()) {
                // Ask through the command, which explains how to proceed.
                let path = self
                    .docs
                    .get(id)
                    .map(|d| d.path.display().to_string())
                    .unwrap_or_default();
                self.actions.push(("file.close", json!({ "path": path })));
                return false;
            }
            self.closed.push(id);
        }
        true
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile: TileId, pane: &mut Pane) -> UiResponse {
        match pane {
            Pane::Terminal => match self.terminal.as_deref_mut() {
                Some(t) if !t.exited => t.show(ui, "terminal"),
                _ => {
                    egui::Frame::new().inner_margin(8).show(ui, |ui| {
                        ui.weak("The shell has exited.");
                        if ui.button("Start a New Shell").clicked() {
                            self.actions.push(("terminal.show", json!({})));
                        }
                    });
                }
            },
            Pane::Editor(id) => {
                let id = *id;
                if let Some(doc) = self.docs.get_mut(id)
                    && doc.designer.is_some()
                {
                    form_header(ui, doc);
                    if doc.design_view {
                        self.design(ui, id);
                        return UiResponse::None;
                    }
                }
                if let Some(doc) = self.docs.get_mut(id) {
                    let out = view::show(ui, &mut doc.editor, self.recency, ("editor", id.0));
                    for r in out.requests {
                        self.requests.push((id, r));
                    }
                    if out.has_focus {
                        self.focused = Some(id);
                    }
                } else {
                    ui.weak("This file was closed.");
                }
            }
            other => {
                egui::Frame::new().inner_margin(8).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    match other {
                        Pane::Explorer => self.explorer(ui),
                        Pane::Welcome => self.welcome(ui),
                        Pane::Problems => self.problems(ui),
                        Pane::Output => self.output(ui),
                        Pane::Activity => activity(ui, self.activity),
                        Pane::References => self.references_ui(ui),
                        Pane::SourceControl => self.source_control(ui),
                        Pane::Editor(_) | Pane::Terminal => {}
                    }
                });
            }
        }
        UiResponse::None
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

/// The Design / Source switch above a form.
fn form_header(ui: &mut egui::Ui, doc: &mut crate::docs::OpenDoc) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (design, icon, label, tip) in [
                    (true, Icon::PencilRuler, "Design", "Edit the form visually"),
                    (false, Icon::Code, "Source", "Edit the .pxui text"),
                ] {
                    let b = egui::Button::selectable(
                        doc.design_view == design,
                        (icon.image(ui), label),
                    );
                    if ui.add(b).on_hover_text(tip).clicked() {
                        doc.design_view = design;
                        if !design {
                            doc.editor.focus_requested = true;
                        }
                    }
                }
            });
        });
    ui.separator();
}

impl PaneUi<'_> {
    /// The form designer for a `.pxui` document. Its edits go into the document.
    fn design(&mut self, ui: &mut egui::Ui, id: DocId) {
        let Some(doc) = self.docs.get_mut(id) else {
            return;
        };
        let Some(designer) = doc.designer.as_mut() else {
            return;
        };
        let source = doc.editor.core.text();
        let stem = doc
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_pressed()) {
            self.focused = Some(id);
        }
        let actions = designer.show(ui, &source, &stem);
        for action in actions {
            match action {
                perox_designer::Action::Edit(edits) => {
                    let edits: Vec<_> = edits.into_iter().map(|e| (e.range, e.text)).collect();
                    doc.editor.apply_byte_edits(&edits);
                }
                perox_designer::Action::Undo => doc.editor.undo(),
                perox_designer::Action::Redo => doc.editor.redo(),
                perox_designer::Action::OpenHandler { form, method } => {
                    self.actions.push((
                        "designer.open_handler",
                        json!({ "form": form, "method": method }),
                    ));
                }
            }
        }
    }

    fn explorer(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.tree else {
            ui.weak("No project open.");
            if ui.button("Open Project…").clicked() {
                self.actions.push(("project.open_dialog", json!({})));
            }
            return;
        };
        let active = self
            .active_doc
            .and_then(|id| self.docs.get(id))
            .map(|d| d.path.clone());
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            ui.strong(&tree.name);
            for child in tree.children.iter().flatten() {
                self.node(ui, child, active.as_deref());
            }
        });
    }

    fn node(
        &mut self,
        ui: &mut egui::Ui,
        node: &perox_project::Node,
        active: Option<&std::path::Path>,
    ) {
        match &node.children {
            Some(children) => {
                let id = ui.make_persistent_id(("tree", &node.path));
                let default_open = node.name == "src";
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    default_open,
                )
                .show_header(ui, |ui| {
                    ui.add(perox_icons::Icon::Folder.image(ui));
                    ui.label(&node.name);
                })
                .body(|ui| {
                    for c in children {
                        self.node(ui, c, active);
                    }
                });
            }
            None => {
                let is_active = active == Some(node.path.as_path());
                let state = self.git.state_of(&node.path);
                let icon = crate::icons::for_file(&node.path);
                if ui
                    .add(egui::Button::selectable(
                        is_active,
                        egui::Atoms::new((
                            icon.image(ui),
                            match state {
                                Some(st) => {
                                    egui::RichText::new(node.name.as_str()).color(git_color(ui, st))
                                }
                                None => egui::RichText::new(node.name.as_str()),
                            },
                        )),
                    ))
                    .clicked()
                {
                    self.actions
                        .push(("file.open", json!({ "path": node.path })));
                }
            }
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let shortcut = |id| {
            keymap::shortcut_for(id)
                .map(|s| ctx.format_shortcut(&s))
                .unwrap_or_default()
        };
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.15);
            ui.heading(RichText::new("PeroxIDE").size(32.0));
            ui.label("The Rust RAD IDE");
            ui.add_space(16.0);
            if ui
                .button(format!(
                    "Open Project…   {}",
                    shortcut("project.open_dialog")
                ))
                .clicked()
            {
                self.actions.push(("project.open_dialog", json!({})));
            }
            if !self.recent.is_empty() && self.project_root.is_none() {
                ui.add_space(12.0);
                ui.weak("Recent");
                for p in self.recent.iter().take(6) {
                    if ui.link(p.display().to_string()).clicked() {
                        self.actions.push(("project.open", json!({ "path": p })));
                    }
                }
            }
            ui.add_space(16.0);
            egui::Grid::new("welcome_keys")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    for (label, id) in [
                        ("Command palette", "view.command_palette"),
                        ("Save", "file.save"),
                        ("Build", "build.build"),
                        ("Run", "build.run"),
                        ("Settings", "app.settings"),
                    ] {
                        ui.label(label);
                        ui.monospace(shortcut(id));
                        ui.end_row();
                    }
                    ui.label("Connect Claude Code");
                    ui.label("Tools › MCP Server");
                    ui.end_row();
                });
        });
    }

    fn problems(&mut self, ui: &mut egui::Ui) {
        let root = self.project_root;
        let show = |p: &std::path::Path| {
            root.and_then(|r| p.strip_prefix(r).ok())
                .unwrap_or(p)
                .display()
                .to_string()
        };
        let mut rows: Vec<(
            Severity,
            String,
            usize,
            usize,
            String,
            String,
            std::path::PathBuf,
        )> = Vec::new();
        for p in self.lsp.diagnostics.values().flatten() {
            rows.push((
                p.severity,
                show(&p.path),
                p.line + 1,
                p.col + 1,
                p.message.clone(),
                p.source.clone(),
                p.path.clone(),
            ));
        }
        for d in &self.build.diagnostics {
            let sev = match d.level {
                perox_build::Level::Error => Severity::Error,
                perox_build::Level::Warning => Severity::Warning,
                perox_build::Level::Note => Severity::Info,
            };
            if let Some(f) = &d.file {
                rows.push((
                    sev,
                    show(f),
                    d.line,
                    d.column,
                    d.message.clone(),
                    "cargo".into(),
                    f.clone(),
                ));
            }
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        if rows.is_empty() {
            ui.weak("No problems.");
            return;
        }
        let theme = perox_editor::theme::Theme::for_visuals(ui.visuals());
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (sev, file, line, col, msg, source, path) in rows {
                    let icon = match sev {
                        Severity::Error => perox_icons::Icon::CircleX,
                        Severity::Warning => perox_icons::Icon::TriangleAlert,
                        _ => perox_icons::Icon::Info,
                    };
                    let first = msg.lines().next().unwrap_or_default().to_owned();
                    let resp = ui
                        .horizontal(|ui| {
                            ui.add(icon.colored(theme.severity(sev)));
                            let r = ui.add(
                                egui::Label::new(first)
                                    .sense(egui::Sense::click())
                                    .truncate(),
                            );
                            ui.weak(format!("{file}:{line}:{col}  {source}"));
                            r
                        })
                        .inner;
                    if resp.on_hover_text(&msg).clicked() {
                        self.actions.push((
                            "file.open",
                            json!({ "path": path, "line": line, "column": col }),
                        ));
                    }
                }
            });
    }

    fn source_control(&mut self, ui: &mut egui::Ui) {
        if self.git.root.is_none() {
            ui.weak("This project isn't in a git repository (or git isn't installed).");
            return;
        }
        let st = &self.git.status;
        ui.horizontal(|ui| {
            ui.add(perox_icons::Icon::GitBranch.image(ui));
            ui.strong(&st.branch);
            if st.ahead + st.behind > 0 {
                ui.weak(format!("{} ahead, {} behind", st.ahead, st.behind));
            }
        });
        ui.add(
            egui::TextEdit::multiline(self.commit_message)
                .hint_text("Commit message")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        let can = !st.files.is_empty() && !self.commit_message.trim().is_empty();
        if ui
            .add_enabled(
                can,
                egui::Button::image_and_text(
                    perox_icons::Icon::GitCommitHorizontal.image(ui),
                    "Commit All",
                ),
            )
            .on_hover_text("Saves all files, stages every change and commits")
            .clicked()
        {
            self.actions.push((
                "git.commit",
                json!({ "message": self.commit_message.clone() }),
            ));
        }
        ui.separator();
        if st.files.is_empty() {
            ui.weak("No changes.");
            return;
        }
        let root = self.git.root.clone().unwrap_or_default();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (path, state) in &st.files {
                    let shown = path
                        .strip_prefix(&root)
                        .unwrap_or(path)
                        .display()
                        .to_string();
                    let resp = ui
                        .horizontal(|ui| {
                            ui.add(crate::icons::for_file(path).image(ui));
                            let r = ui.add(egui::Label::new(&shown).sense(egui::Sense::click()));
                            ui.colored_label(git_color(ui, *state), state.letter());
                            r
                        })
                        .inner;
                    if resp.clicked() && path.is_file() {
                        self.actions.push(("file.open", json!({ "path": path })));
                    }
                }
            });
    }

    fn references_ui(&mut self, ui: &mut egui::Ui) {
        if self.references.is_empty() {
            ui.weak("No references. Put the cursor on a name and press ⇧F12.");
            return;
        }
        let root = self.project_root;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (path, line, col, text) in self.references {
                    let shown = root
                        .and_then(|r| path.strip_prefix(r).ok())
                        .unwrap_or(path)
                        .display()
                        .to_string();
                    let resp = ui
                        .horizontal(|ui| {
                            ui.weak(format!("{shown}:{line}"));
                            ui.add(
                                egui::Label::new(RichText::new(text.trim()).monospace())
                                    .sense(egui::Sense::click())
                                    .truncate(),
                            )
                        })
                        .inner;
                    if resp.clicked() {
                        self.actions.push((
                            "file.open",
                            json!({ "path": path, "line": line, "column": col }),
                        ));
                    }
                }
            });
    }

    fn output(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match self.build.running() {
            Some(k) => {
                ui.spinner();
                ui.label(format!("cargo {}", k.verb()));
                if ui
                    .add(egui::Button::image_and_text(
                        perox_icons::Icon::Square.image(ui),
                        "Stop",
                    ))
                    .clicked()
                {
                    self.actions.push(("build.stop", json!({})));
                }
            }
            None => match self.build.last {
                Some((k, true)) => {
                    ui.add(
                        perox_icons::Icon::CircleCheck
                            .colored(egui::Color32::from_rgb(0x4e, 0xc9, 0x6b)),
                    );
                    ui.label(format!("cargo {} finished", k.verb()));
                }
                Some((k, false)) => {
                    ui.add(perox_icons::Icon::CircleX.colored(ui.visuals().error_fg_color));
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("cargo {} failed", k.verb()),
                    );
                }
                None => {
                    ui.weak("Build output appears here.");
                }
            },
        });
        ui.separator();
        egui::ScrollArea::both()
            .stick_to_bottom(true)
            .auto_shrink(false)
            .show(ui, |ui| {
                for line in &self.build.output {
                    ui.add(egui::Label::new(RichText::new(line).monospace()).extend());
                }
            });
    }
}

fn activity(ui: &mut egui::Ui, log: &Activity) {
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink(false)
        .show(ui, |ui| {
            for e in log.iter() {
                ui.horizontal(|ui| {
                    ui.weak(&e.time);
                    if e.agent {
                        ui.colored_label(ui.visuals().warn_fg_color, "agent");
                    } else {
                        ui.weak("you");
                    }
                    ui.monospace(&e.text);
                });
            }
        });
}

/// Colour for a file's git state (Explorer, Source Control).
fn git_color(ui: &egui::Ui, state: crate::git::FileState) -> egui::Color32 {
    use crate::git::FileState::*;
    let dark = ui.visuals().dark_mode;
    let c = |d: u32, l: u32| {
        let h = if dark { d } else { l };
        egui::Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    };
    match state {
        Modified | Renamed => c(0xe2c08d, 0x9a6700),
        Added | Untracked => c(0x73c991, 0x1a7f37),
        Deleted | Conflicted => c(0xf85149, 0xcf222e),
    }
}
