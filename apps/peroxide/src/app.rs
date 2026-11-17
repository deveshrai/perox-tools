//! The main window: menu bar, docking layout, status bar, palette and dialogs.
//! Every action, including agent calls arriving over MCP, runs through `Self::run`.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use eframe::egui;
use perox_core::{CommandBus, CommandError, Origin};
use serde_json::{Value, json};

use crate::commands::{self, AppState};
use crate::keymap;
use crate::layout::{self, Pane, PaneUi};
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
    pub fn new(ctx: &egui::Context) -> Self {
        let mut bus = CommandBus::new();
        commands::register(&mut bus);
        let (mcp, calls) = McpController::new(bus.specs().cloned().collect(), ctx.clone());
        let mut state = AppState::new(Settings::load(), mcp);
        if state.settings.mcp.start_on_launch {
            state.mcp.start(&state.settings.mcp);
            let note = match &state.mcp.error {
                Some(e) => e.clone(),
                None => format!("MCP server listening on port {}", state.settings.mcp.port),
            };
            state.activity.push(false, note.clone());
            state.status = note;
        }
        Self {
            bus,
            state,
            calls,
            tree: layout::default_tree(),
            palette: Palette::default(),
            mcp_page: McpPage::default(),
        }
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
                item(ui, actions, "app.settings");
                ui.separator();
                item(ui, actions, "app.quit");
            });
            ui.menu_button("View", |ui| {
                item(ui, actions, "view.command_palette");
                item(ui, actions, "view.reset_layout");
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
            });
            ui.menu_button("Help", |ui| item(ui, actions, "help.about"));
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

        let mut actions: Vec<&'static str> = keymap::pressed(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui, &mut actions));

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
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let mut panes = PaneUi {
                activity: &self.state.activity,
            };
            self.tree.ui(&mut panes, ui);
        });

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
        for (id, args) in page_actions {
            self.run_user(id, args);
        }
        if std::mem::take(&mut self.state.reset_layout) {
            self.tree = layout::default_tree();
        }
        if std::mem::take(&mut self.state.quit_requested) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
