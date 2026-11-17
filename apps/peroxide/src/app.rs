//! The main window. One frame: answers from agents, rust-analyzer and cargo come in;
//! menus, toolbar, panes and shortcuts collect commands; everything runs through
//! `PeroxApp::run`, which logs it to Activity.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use eframe::egui;
use perox_core::{CommandBus, CommandError, Origin};
use serde_json::{Value, json};

use crate::commands::{self, AppState};
use crate::ide::{self, LayoutRequest};
use crate::keymap;
use crate::layout::{self, Pane, PaneUi};
use crate::lsp::Effect;
use crate::mcp_host::{Call, McpController};
use crate::mcp_page::{self, McpPage};
use crate::palette::Palette;
use crate::settings::Settings;

pub struct PeroxApp {
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
            (Ok(_), true) => format!("{id}  ✓"),
            (Err(e), _) => format!("{id}  ✗ {e}"),
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

    fn menu_bar(&self, ui: &mut egui::Ui, actions: &mut Vec<&'static str>) {
        let item = |ui: &mut egui::Ui, actions: &mut Vec<&'static str>, id: &'static str| {
            let title = self.bus.spec(id).map_or(id, |s| s.title);
            let shortcut = keymap::shortcut_for(id)
                .map(|s| ui.ctx().format_shortcut(&s))
                .unwrap_or_default();
            if ui
                .add(egui::Button::new(title).shortcut_text(shortcut))
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
            ui.menu_button("View", |ui| {
                item(ui, actions, "view.command_palette");
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
                let mut on = self.state.mcp.is_on();
                if ui.checkbox(&mut on, "MCP Server").clicked() {
                    actions.push("mcp.toggle");
                    ui.close();
                }
                if ui.button("MCP Server Settings…").clicked() {
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
                ("▶ Run", "build.run"),
                ("🔨 Build", "build.build"),
                ("✔ Check", "build.check"),
                ("🧪 Test", "build.test"),
            ] {
                if ui
                    .add_enabled(has_project, egui::Button::new(label))
                    .on_hover_text(tip(id))
                    .clicked()
                {
                    actions.push(id);
                }
            }
            if ui
                .add_enabled(running, egui::Button::new("■ Stop"))
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

        // Agent calls first: they wait on a reply.
        while let Ok(call) = self.calls.try_recv() {
            let result = self.run(&call.id, call.args, call.origin);
            let _ = call.reply.send(result);
        }
        // rust-analyzer answers.
        let effects = self
            .state
            .lsp
            .poll(&mut self.state.docs, &self.state.recency);
        for Effect::Open { path, line, col } in effects {
            self.run_user(
                "file.open",
                json!({ "path": path, "line": line + 1, "column": col + 1 }),
            );
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

        let mut palette_open = self.state.show_palette;
        if let Some(id) = self.palette.ui(&ctx, &self.bus, &mut palette_open) {
            actions.push(id);
        }
        self.state.show_palette = palette_open;

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

        for id in actions {
            self.run_user(id, json!({}));
        }
        for (id, args) in page_actions.into_iter().chain(pane_actions) {
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
        if std::mem::take(&mut self.state.quit_requested) {
            self.state.lsp.stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
