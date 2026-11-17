//! Settings › MCP server. Same layout as SolidCraft's page: turn it on, choose where
//! Claude Code runs, copy one command. Every change runs a command; this module only
//! returns which ones.

use std::time::Instant;

use eframe::egui::{self, RichText};
use perox_core::Access;
use serde_json::{Value, json};

use crate::commands::AppState;
use crate::mcp_host::McpController;
use crate::settings::ConnectFrom;

/// Page state that isn't a setting: what's being typed, what's revealed.
#[derive(Debug, Default)]
pub struct McpPage {
    port_text: Option<String>,
    show_token: bool,
}

pub type Action = (&'static str, Value);

impl McpPage {
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &AppState) -> Vec<Action> {
        let mut actions = Vec::new();
        let s = &state.settings.mcp;

        ui.label(
            "Lets AI agents such as Claude Code use PeroxIDE: run its commands, and later edit \
             code and design forms. Everything an agent does appears in the Activity panel.",
        );
        ui.add_space(8.0);

        let mut on = state.mcp.is_on();
        if ui
            .checkbox(&mut on, RichText::new("Turn on the MCP server").strong())
            .changed()
        {
            actions.push(("mcp.toggle", json!({})));
        }
        status_line(ui, state);
        ui.add_space(8.0);

        ui.label(RichText::new("Connect from").strong());
        let mut from = s.connect_from;
        ui.radio_value(&mut from, ConnectFrom::ThisComputer, "This computer")
            .on_hover_text("Only apps on this computer can connect (127.0.0.1).");
        ui.radio_value(
            &mut from,
            ConnectFrom::AnotherDevice,
            "Another device on the network",
        );
        if from != s.connect_from {
            let value = match from {
                ConnectFrom::ThisComputer => "this_computer",
                ConnectFrom::AnotherDevice => "another_device",
            };
            actions.push(("mcp.set_connect_from", json!({ "value": value })));
        }
        if s.connect_from == ConnectFrom::AnotherDevice {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Use only on networks you trust. The connection is plain HTTP, like any local \
                 development server.",
            );
        }
        ui.add_space(8.0);

        let url = McpController::url(s);
        match &url {
            Ok(url) => {
                ui.label(RichText::new("Server address").strong());
                copy_row(ui, url, url);
                ui.add_space(8.0);
                ui.label(RichText::new("Start Claude Code with it").strong());
                let full = perox_mcp::claude_session_command(url, &s.token);
                copy_row(ui, &self.mask(&full, &s.token), &full);
                ui.weak(
                    "Paste into a terminal where Claude Code runs. Connects for that session only.",
                );
            }
            Err(e) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
        }
        ui.add_space(8.0);

        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            egui::Grid::new("mcp_advanced")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Port");
                    ui.horizontal(|ui| {
                        let text = self.port_text.get_or_insert_with(|| s.port.to_string());
                        ui.add(egui::TextEdit::singleline(text).desired_width(64.0));
                        if ui.button("Use").clicked() {
                            match text.trim().parse::<u16>() {
                                Ok(port) => actions.push(("mcp.set_port", json!({ "port": port }))),
                                Err(_) => *text = s.port.to_string(),
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("Agents may");
                    let mut access = s.max_access;
                    egui::ComboBox::from_id_salt("mcp_access")
                        .selected_text(access_label(access))
                        .show_ui(ui, |ui| {
                            for a in [Access::Read, Access::Edit, Access::Execute] {
                                ui.selectable_value(&mut access, a, access_label(a));
                            }
                        });
                    if access != s.max_access {
                        let value = match access {
                            Access::Read => "read",
                            Access::Edit => "edit",
                            Access::Execute => "execute",
                        };
                        actions.push(("mcp.set_access", json!({ "value": value })));
                    }
                    ui.end_row();

                    ui.label("Access token");
                    ui.horizontal(|ui| {
                        let shown = if self.show_token {
                            s.token.clone()
                        } else {
                            "•".repeat(16)
                        };
                        ui.monospace(shown);
                        if ui
                            .button(if self.show_token { "Hide" } else { "Show" })
                            .clicked()
                        {
                            self.show_token = !self.show_token;
                        }
                        if ui
                            .button("New token")
                            .on_hover_text(
                                "Clients using the old token are refused: copy the command again.",
                            )
                            .clicked()
                        {
                            actions.push(("mcp.new_token", json!({})));
                        }
                    });
                    ui.end_row();

                    ui.label("");
                    let mut on_launch = s.start_on_launch;
                    if ui
                        .checkbox(&mut on_launch, "Turn on when PeroxIDE starts")
                        .changed()
                    {
                        actions.push(("mcp.set_start_on_launch", json!({ "value": on_launch })));
                    }
                    ui.end_row();
                });
            if let Ok(url) = &url {
                ui.add_space(8.0);
                ui.label(RichText::new("Or add it to Claude Code for good").strong());
                let full = perox_mcp::claude_add_command(url, &s.token);
                copy_row(ui, &self.mask(&full, &s.token), &full);
                ui.weak("After this, plain `claude` has PeroxIDE every time.");
            }
        });

        if actions.iter().any(|(id, _)| *id == "mcp.set_port") {
            self.port_text = None;
        }
        actions
    }

    fn mask(&self, text: &str, token: &str) -> String {
        if self.show_token || token.is_empty() {
            text.to_owned()
        } else {
            text.replace(token, &format!("{}…", &token[..6.min(token.len())]))
        }
    }
}

fn access_label(a: Access) -> &'static str {
    match a {
        Access::Read => "Look only",
        Access::Edit => "Look and edit",
        Access::Execute => "Everything, including build and run",
    }
}

fn copy_row(ui: &mut egui::Ui, shown: &str, copied: &str) {
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(copied.to_owned());
        }
        ui.add(egui::Label::new(RichText::new(shown).monospace()).wrap());
    });
}

fn status_line(ui: &mut egui::Ui, state: &AppState) {
    if let Some(e) = &state.mcp.error
        && !state.mcp.is_on()
    {
        ui.colored_label(ui.visuals().error_fg_color, e);
        return;
    }
    let Some(stats) = state.mcp.stats() else {
        ui.weak("Off. Nothing is listening.");
        return;
    };
    let text = match (stats.sessions, stats.last_request) {
        (0, _) => "Listening: waiting for Claude Code to connect".to_owned(),
        (n, Some(t)) => format!(
            "Connected: {n} session{}, last request {} ago",
            if n == 1 { "" } else { "s" },
            ago(t)
        ),
        (n, None) => format!("Connected: {n} session(s)"),
    };
    ui.colored_label(ui.visuals().hyperlink_color, text);
    if stats.refused > 0 {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!(
                "Refused {} request(s) with a missing or wrong token",
                stats.refused
            ),
        );
    }
}

fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h", s / 3600),
    }
}

/// The short indicator in the status bar.
pub fn indicator(state: &AppState) -> String {
    match state.mcp.stats() {
        None => "MCP off".into(),
        Some(s) if s.sessions == 0 => format!("MCP listening :{}", state.settings.mcp.port),
        Some(s) => format!("MCP: {} connected", s.sessions),
    }
}
