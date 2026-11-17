//! The main window. One frame: answers from agents, rust-analyzer and cargo come in;
//! menus, toolbar, panes and shortcuts collect commands; everything runs through
//! `PeroxApp::run`, which logs it to Activity.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use eframe::egui;
use perox_core::{CommandBus, CommandError, Origin};
use serde_json::{Value, json};

use crate::commands::{self, AppState};
use crate::icons;
use crate::ide::{self, LayoutRequest};
use crate::keymap;
use crate::layout::{self, Pane, PaneUi};
use crate::lsp::Effect;
use crate::mcp_host::{Call, McpController};
use crate::mcp_page::{self, McpPage};
use crate::palette::Palette;
use crate::settings::Settings;
use perox_icons::Icon;

pub struct PeroxApp {
    /// Showing "save before quitting?"; set when quitting with unsaved changes.
    quit_prompt: bool,
    /// The user chose to quit; don't ask again.
    quitting: bool,
    bus: CommandBus<AppState>,
    state: AppState,
    calls: Receiver<Call>,
    tree: egui_tiles::Tree<Pane>,
    palette: Palette,
    mcp_page: McpPage,
}

impl PeroxApp {
    /// `open`: a project folder (or file) to open at start, from the command line.
    pub fn new(ctx: &egui::Context, open: Option<std::path::PathBuf>) -> Self {
        egui_extras::install_image_loaders(ctx);
        let mut bus = CommandBus::new();
        commands::register(&mut bus);
        ide::register(&mut bus);
        let (mcp, calls) = McpController::new(bus.specs().cloned().collect(), ctx.clone());
        let mut state = AppState::new(Settings::load(), mcp, ctx.clone());
        if state.settings.mcp.start_on_launch {
            state.mcp.start(&state.settings.mcp);
            let note = match &state.mcp.error {
                Some(e) => e.clone(),
                None => format!("MCP server listening on port {}", state.settings.mcp.port),
            };
            state.activity.push(false, note.clone());
            state.status = note;
        }
        let mut app = Self {
            quit_prompt: false,
            quitting: false,
            bus,
            state,
            calls,
            tree: layout::default_tree(),
            palette: Palette::default(),
            mcp_page: McpPage::default(),
        };
        if let Some(path) = open {
            app.run_user("project.open", json!({ "path": path }));
            if path.is_file() {
                app.run_user("file.open", json!({ "path": path }));
            }
        }
        app
    }

    /// Runs a command and records it in the Activity panel.
    fn run(&mut self, id: &str, args: Value, origin: Origin) -> Result<Value, CommandError> {
        let agent = matches!(origin, Origin::Agent { .. });
        let result = self.bus.execute(&mut self.state, id, args, origin);
        let title = self.bus.spec(id).map_or(id, |s| s.title);
        let entry = match (&result, agent) {
            (Ok(_), false) => title.to_owned(),
            (Ok(_), true) => format!("{id}: ok"),
            (Err(e), _) => format!("{id} failed: {e}"),
        };
        self.state.activity.push(agent, entry);
        if let (Err(e), false) = (&result, agent) {
            self.state.status = e.to_string();
        }
        result
    }

    fn run_user(&mut self, id: &str, args: Value) {
        let _ = self.run(id, args, Origin::User);
    }

    /// Acts on something rust-analyzer answered.
    fn effect(&mut self, effect: Effect) {
        match effect {
            Effect::Open { path, line, col } => {
                self.run_user(
                    "file.open",
                    json!({ "path": path, "line": line + 1, "column": col + 1 }),
                );
            }
            Effect::References(locs) => {
                let s = &mut self.state;
                s.references = locs
                    .into_iter()
                    .map(|l| {
                        let text = match s.docs.by_path(&l.path).and_then(|id| s.docs.get(id)) {
                            Some(d) => d.editor.core.doc.line_text(l.line),
                            None => std::fs::read_to_string(&l.path)
                                .ok()
                                .and_then(|t| t.lines().nth(l.line).map(str::to_owned))
                                .unwrap_or_default(),
                        };
                        (l.path, l.line + 1, l.col + 1, text)
                    })
                    .collect();
                s.status = format!("{} reference(s)", s.references.len());
                s.layout_requests.push(LayoutRequest::ShowReferences);
            }
            Effect::CodeActions { doc, items } => self.state.code_actions = Some((doc, items)),
            Effect::WorkspaceEdit(edit) => {
                let n = ide::apply_workspace_edit(&mut self.state, &edit);
                self.state.status = format!("Changed {n} file(s). Save to keep the changes.");
            }
            Effect::Save(doc) => {
                if let Ok(path) = self.state.docs.save(doc) {
                    self.state.lsp.did_save(doc, &self.state.docs);
                    self.state.status = format!("Formatted and saved {}", path.display());
                }
            }
            Effect::Message(m) => self.state.status = m,
        }
    }

    /// Quitting (menu, ⌘Q or the window's close button) asks first if there are unsaved changes.
    fn handle_quit(&mut self, ctx: &egui::Context) {
        let close_button = ctx.input(|i| i.viewport().close_requested());
        let asked = std::mem::take(&mut self.state.quit_requested) || close_button;
        let dirty: Vec<String> = self
            .state
            .docs
            .list
            .iter()
            .filter(|d| d.is_dirty())
            .map(|d| {
                d.path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            })
            .collect();
        if asked && !self.quitting {
            if dirty.is_empty() {
                self.quitting = true;
            } else {
                self.quit_prompt = true;
                if close_button {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
            }
        }
        if self.quit_prompt {
            let mut choice = None;
            egui::Modal::new(egui::Id::unique("quit_prompt")).show(ctx, |ui| {
                ui.heading("Save changes before quitting?");
                ui.label(format!("Unsaved: {}", dirty.join(", ")));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save All and Quit").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.run_user("file.save_all", json!({}));
                    self.quit_prompt = false;
                    self.quitting = self.state.docs.list.iter().all(|d| !d.is_dirty());
                }
                Some(1) => {
                    self.quit_prompt = false;
                    self.quitting = true;
                }
                Some(_) => self.quit_prompt = false,
                None => {}
            }
        }
        if self.quitting {
            self.state.save_session();
            self.state.lsp.stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// The rename prompt and the quick-fix menu.
    fn editor_dialogs(&mut self, ctx: &egui::Context) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        if let Some(name) = &mut self.state.rename {
            let mut open = true;
            let mut submit = false;
            egui::Window::new("Rename Symbol")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("New name:");
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(320.0));
                    r.request_focus();
                    submit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.weak("Enter renames everywhere it's used. Esc cancels.");
                });
            if submit {
                out.push(("editor.rename", json!({ "new_name": name.clone() })));
                self.state.rename = None;
            } else if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.state.rename = None;
            }
        }
        if let Some((_, items)) = &self.state.code_actions {
            let mut open = true;
            let mut pick = None;
            egui::Window::new("Quick Fix")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    for (i, (title, _)) in items.iter().enumerate() {
                        if ui.button(title).clicked() {
                            pick = Some(i);
                        }
                    }
                });
            if let Some(i) = pick {
                out.push(("editor.apply_code_action", json!({ "index": i })));
            } else if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.state.code_actions = None;
            }
        }
        out
    }

    fn menu_bar(&self, ui: &mut egui::Ui, actions: &mut Vec<&'static str>) {
        let item = |ui: &mut egui::Ui, actions: &mut Vec<&'static str>, id: &'static str| {
            let title = self.bus.spec(id).map_or(id, |s| s.title);
            let shortcut = keymap::shortcut_for(id)
                .map(|s| ui.ctx().format_shortcut(&s))
                .unwrap_or_default();
            if ui
                .add(perox_icons::button(
                    ui,
                    icons::for_command(id),
                    title,
                    &shortcut,
                ))
                .clicked()
            {
                actions.push(id);
                ui.close();
            }
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                item(ui, actions, "project.open_dialog");
                ui.separator();
                item(ui, actions, "file.save");
                item(ui, actions, "file.save_all");
                item(ui, actions, "file.close");
                ui.separator();
                item(ui, actions, "app.settings");
                ui.separator();
                item(ui, actions, "app.quit");
            });
            ui.menu_button("Go", |ui| {
                item(ui, actions, "view.quick_open");
                item(ui, actions, "editor.goto_line_prompt");
                item(ui, actions, "editor.references");
            });
            ui.menu_button("Code", |ui| {
                item(ui, actions, "editor.code_actions");
                item(ui, actions, "editor.rename_prompt");
                item(ui, actions, "editor.format");
                ui.separator();
                let label = if self.state.settings.format_on_save {
                    "Turn Off Format on Save"
                } else {
                    "Turn On Format on Save"
                };
                if ui
                    .add(perox_icons::button(ui, Icon::FileCheck, label, ""))
                    .clicked()
                {
                    actions.push("editor.toggle_format_on_save");
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                item(ui, actions, "view.command_palette");
                item(ui, actions, "terminal.show");
                item(ui, actions, "view.reset_layout");
            });
            ui.menu_button("Build", |ui| {
                item(ui, actions, "build.check");
                item(ui, actions, "build.build");
                item(ui, actions, "build.run");
                item(ui, actions, "build.test");
                item(ui, actions, "build.clippy");
                ui.separator();
                item(ui, actions, "build.stop");
            });
            ui.menu_button("Tools", |ui| {
                let label = if self.state.mcp.is_on() {
                    "Turn Off MCP Server"
                } else {
                    "Turn On MCP Server"
                };
                if ui
                    .add(perox_icons::button(ui, Icon::Plug, label, ""))
                    .clicked()
                {
                    actions.push("mcp.toggle");
                    ui.close();
                }
                if ui
                    .add(perox_icons::button(
                        ui,
                        Icon::Settings2,
                        "MCP Server Settings…",
                        "",
                    ))
                    .clicked()
                {
                    actions.push("app.settings");
                    ui.close();
                }
                ui.separator();
                item(ui, actions, "lsp.restart");
            });
            ui.menu_button("Help", |ui| item(ui, actions, "help.about"));
        });
    }

    fn toolbar(&self, ui: &mut egui::Ui, actions: &mut Vec<&'static str>) {
        ui.horizontal(|ui| {
            let has_project = self.state.project.is_some();
            let running = self.state.build.running().is_some();
            let ctx = ui.ctx().clone();
            let tip = |id: &str| {
                let s = keymap::shortcut_for(id)
                    .map(|s| ctx.format_shortcut(&s))
                    .unwrap_or_default();
                format!("{} {s}", self.bus.spec(id).map_or(id, |c| c.title))
            };
            for (label, id) in [
                ("Run", "build.run"),
                ("Build", "build.build"),
                ("Check", "build.check"),
                ("Test", "build.test"),
            ] {
                let icon = icons::for_command(id).image(ui);
                if ui
                    .add_enabled(has_project, egui::Button::image_and_text(icon, label))
                    .on_hover_text(tip(id))
                    .clicked()
                {
                    actions.push(id);
                }
            }
            if ui
                .add_enabled(
                    running,
                    egui::Button::image_and_text(Icon::Square.image(ui), "Stop"),
                )
                .on_hover_text(tip("build.stop"))
                .clicked()
            {
                actions.push("build.stop");
            }
            ui.separator();
            match &self.state.project {
                Some(p) => ui.strong(&p.name),
                None => ui.weak("No project"),
            };
        });
    }
}

impl eframe::App for PeroxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Files changed by other programs, before anyone reads them.
        self.state.apply_disk_changes();
        // Agent calls next: they wait on a reply.
        while let Ok(call) = self.calls.try_recv() {
            let result = self.run(&call.id, call.args, call.origin);
            let _ = call.reply.send(result);
        }
        // rust-analyzer answers.
        let effects = self
            .state
            .lsp
            .poll(&mut self.state.docs, &self.state.recency);
        for effect in effects {
            self.effect(effect);
        }
        // cargo.
        if let Some((kind, ok)) = self.state.build.poll() {
            self.state.status = format!(
                "cargo {} {}",
                kind.verb(),
                if ok { "finished" } else { "failed" }
            );
        }

        let mut actions: Vec<&'static str> = keymap::pressed(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui, &mut actions));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, &mut actions));

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.state.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .link(mcp_page::indicator(&self.state))
                        .on_hover_text("MCP server settings")
                        .clicked()
                    {
                        actions.push("app.settings");
                    }
                    ui.separator();
                    if !self.state.lsp.status.is_empty() {
                        if self.state.lsp.busy {
                            ui.spinner();
                        }
                        ui.weak(&self.state.lsp.status);
                        ui.separator();
                    }
                    if let Some(doc) = self.state.active_doc.and_then(|id| self.state.docs.get(id))
                    {
                        let p = doc
                            .editor
                            .core
                            .doc
                            .pos_of(doc.editor.core.sels.primary().head);
                        let n = doc.editor.core.sels.len();
                        let cursors = if n > 1 {
                            format!("  ({n} cursors)")
                        } else {
                            String::new()
                        };
                        ui.weak(format!("Ln {}, Col {}{cursors}", p.line + 1, p.col + 1));
                    }
                });
            });
        });

        let mut pane_actions = Vec::new();
        let mut requests = Vec::new();
        let mut closed = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| {
            let s = &mut self.state;
            let mut panes = PaneUi {
                docs: &mut s.docs,
                recency: &mut s.recency,
                activity: &s.activity,
                lsp: &s.lsp,
                build: &s.build,
                tree: s.tree.as_ref(),
                project_root: s.project.as_ref().map(|p| p.root.as_path()),
                recent: &s.settings.recent_projects,
                references: &s.references,
                terminal: s.terminal.as_mut(),
                active_doc: s.active_doc,
                actions: Vec::new(),
                requests: Vec::new(),
                focused: None,
                closed: Vec::new(),
            };
            self.tree.ui(&mut panes, ui);
            if let Some(f) = panes.focused {
                s.active_doc = Some(f);
            }
            pane_actions = panes.actions;
            requests = panes.requests;
            closed = panes.closed;
        });

        // Keep rust-analyzer in step with every editor, then send their requests.
        let ids: Vec<_> = self.state.docs.list.iter().map(|d| d.id).collect();
        for id in ids {
            let edits = self
                .state
                .docs
                .get_mut(id)
                .map(|d| d.editor.take_lsp_edits())
                .unwrap_or_default();
            self.state.lsp.did_change(id, &self.state.docs, &edits);
        }
        for (id, req) in requests {
            self.state.lsp.request(id, &self.state.docs, &req);
        }
        for id in closed {
            let _ = ide::close_doc(&mut self.state, id, true);
        }

        if let Some(mode) = self.state.palette_request.take() {
            self.palette.open(mode);
            self.state.show_palette = true;
        }
        let mut palette_open = self.state.show_palette;
        let lines = self
            .state
            .active_doc
            .and_then(|id| self.state.docs.get(id))
            .map(|d| d.editor.core.doc.len_lines());
        let picked = self
            .palette
            .ui(&ctx, &self.bus, &self.state.files, lines, &mut palette_open);
        self.state.show_palette = palette_open;

        let dialog_actions = self.editor_dialogs(&ctx);
        let mut page_actions = Vec::new();
        if self.state.show_settings {
            let mut open = true;
            egui::Window::new("Settings")
                .open(&mut open)
                .default_width(560.0)
                .collapsible(false)
                .show(&ctx, |ui| {
                    ui.heading("MCP server");
                    ui.separator();
                    page_actions = self.mcp_page.ui(ui, &self.state);
                });
            self.state.show_settings = open;
            if self.state.mcp.is_on() {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }

        if self.state.show_about {
            egui::Window::new("About PeroxIDE")
                .collapsible(false)
                .resizable(false)
                .open(&mut self.state.show_about)
                .show(&ctx, |ui| {
                    ui.label(format!("PeroxIDE {}", env!("CARGO_PKG_VERSION")));
                    ui.label("GPL-3.0-or-later");
                });
        }

        // Commands change what's on screen: draw the next frame now, not on the next input.
        let ran_any = !actions.is_empty();
        for id in actions {
            self.run_user(id, json!({}));
        }
        if ran_any {
            ctx.request_repaint();
        }
        let more = !page_actions.is_empty()
            || !pane_actions.is_empty()
            || picked.is_some()
            || !dialog_actions.is_empty();
        if more {
            ctx.request_repaint();
        }
        for (id, args) in page_actions
            .into_iter()
            .chain(pane_actions)
            .chain(picked)
            .chain(dialog_actions)
        {
            self.run_user(id, args);
        }
        if std::mem::take(&mut self.state.open_dialog_requested)
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Open a Cargo project")
                .pick_folder()
        {
            self.run_user("project.open", json!({ "path": dir }));
        }
        for req in std::mem::take(&mut self.state.layout_requests) {
            match req {
                LayoutRequest::ShowDoc(id) => layout::show_pane(&mut self.tree, Pane::Editor(id)),
                LayoutRequest::CloseDoc(id) => {
                    layout::remove_pane(&mut self.tree, Pane::Editor(id))
                }
                LayoutRequest::ShowOutput => layout::show_pane(&mut self.tree, Pane::Output),
                LayoutRequest::ShowReferences => {
                    layout::show_pane(&mut self.tree, Pane::References)
                }
                LayoutRequest::ShowTerminal => layout::show_pane(&mut self.tree, Pane::Terminal),
            }
        }
        if std::mem::take(&mut self.state.reset_layout) {
            self.tree = layout::default_tree();
            for d in &self.state.docs.list {
                layout::show_pane(&mut self.tree, Pane::Editor(d.id));
            }
        }
        if self.state.build.running().is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.handle_quit(&ctx);
    }
}
