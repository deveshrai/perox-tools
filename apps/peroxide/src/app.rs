//! The main window: menu bar, welcome page, status bar.

use eframe::egui;
use perox_core::{CommandBus, Origin};
use serde_json::Value;

use crate::commands::{self, ShellState};

pub struct PeroxApp {
    bus: CommandBus<ShellState>,
    state: ShellState,
    status: String,
}

impl PeroxApp {
    pub fn new() -> Self {
        let mut bus = CommandBus::new();
        commands::register(&mut bus);
        Self {
            bus,
            state: ShellState::default(),
            status: "Ready".into(),
        }
    }

    /// Runs a command for the user and reports failures in the status bar.
    fn run(&mut self, id: &str) {
        if let Err(e) = self
            .bus
            .execute(&mut self.state, id, Value::Null, Origin::User)
        {
            self.status = e.to_string();
        }
    }

    /// A menu entry wired to a command: the label is the command's title.
    fn menu_item(&mut self, ui: &mut egui::Ui, id: &str) {
        let title = self.bus.spec(id).map_or(id, |s| s.title);
        if ui.button(title).clicked() {
            self.run(id);
            ui.close();
        }
    }
}

impl eframe::App for PeroxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| self.menu_item(ui, "app.quit"));
                ui.menu_button("Tools", |ui| {
                    ui.add_enabled(false, egui::Button::new("MCP Server"))
                        .on_disabled_hover_text(
                            "Arrives with the MCP milestone (docs/roadmap.md).",
                        );
                });
                ui.menu_button("Help", |ui| self.menu_item(ui, "help.about"));
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| ui.label(&self.status));

        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.heading("PeroxIDE");
                ui.label("The Rust RAD IDE. Phase 0: the shell.");
            });
        });

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

        if std::mem::take(&mut self.state.quit_requested) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
