//! The app's commands and the state they act on. Every menu item, shortcut,
//! palette entry, settings control and agent call runs one of these.

use std::collections::VecDeque;

use perox_core::{Access, CommandBus, CommandError, CommandSpec, Invocation};
use serde_json::{Value, json};

use eframe::egui;
use perox_editor::completion::Recency;
use perox_project::{Node, Project};

use crate::docs::{DocId, Docs};
use crate::ide::{BuildState, LayoutRequest};
use crate::lsp::Lsp;
use crate::mcp_host::McpController;
use crate::settings::{ConnectFrom, Settings};

/// Everything the commands can change.
#[derive(Debug)]
pub struct AppState {
    pub settings: Settings,
    pub mcp: McpController,
    pub activity: Activity,
    pub status: String,
    pub quit_requested: bool,
    pub show_about: bool,
    pub show_settings: bool,
    pub show_palette: bool,
    /// Open the palette in this mode on the next frame.
    pub palette_request: Option<crate::palette::Mode>,
    pub reset_layout: bool,
    /// Project files (relative), for quick open.
    pub files: Vec<std::path::PathBuf>,
    /// Results of the last Find References: (path, 1-based line, column, line text).
    pub references: Vec<(std::path::PathBuf, usize, usize, String)>,
    /// The quick-fix menu: document and (title, action).
    pub code_actions: Option<(DocId, Vec<(String, Value)>)>,
    /// The rename prompt's text while it's open.
    pub rename: Option<String>,
    pub watcher: Option<crate::watcher::Watcher>,
    pub terminal: Option<perox_terminal::Terminal>,
    pub project: Option<Project>,
    /// The Explorer's file tree.
    pub tree: Option<Node>,
    pub docs: Docs,
    pub active_doc: Option<DocId>,
    pub lsp: Lsp,
    pub build: BuildState,
    pub recency: Recency,
    pub layout_requests: Vec<LayoutRequest>,
    pub open_dialog_requested: bool,
    pub ctx: egui::Context,
}

impl AppState {
    pub fn new(settings: Settings, mcp: McpController, ctx: egui::Context) -> Self {
        Self {
            project: None,
            tree: None,
            docs: Docs::default(),
            active_doc: None,
            lsp: Lsp::new(ctx.clone()),
            build: BuildState::default(),
            recency: Recency::default(),
            layout_requests: Vec::new(),
            open_dialog_requested: false,
            ctx,
            settings,
            mcp,
            activity: Activity::default(),
            status: "Ready".into(),
            quit_requested: false,
            show_about: false,
            show_settings: false,
            show_palette: false,
            palette_request: None,
            reset_layout: false,
            files: Vec::new(),
            references: Vec::new(),
            code_actions: None,
            rename: None,
            watcher: None,
            terminal: None,
        }
    }
}

impl AppState {
    /// Remembers the open project's open files, for next time.
    pub fn save_session(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        let open: Vec<_> = self
            .docs
            .list
            .iter()
            .filter(|d| d.path.starts_with(&root))
            .map(|d| d.path.clone())
            .collect();
        let active = self
            .active_doc
            .and_then(|id| self.docs.get(id))
            .map(|d| d.path.clone());
        self.settings
            .sessions
            .insert(root, crate::settings::Session { open, active });
        self.settings.save();
    }

    /// Acts on files changed by other programs.
    pub fn apply_disk_changes(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        let Some(changes) = self.watcher.as_mut().and_then(|w| w.poll(&root)) else {
            return;
        };
        for path in &changes.files {
            let Some(id) = self.docs.by_path(path) else {
                continue;
            };
            let Ok(disk) = std::fs::read_to_string(path) else {
                continue;
            };
            let Some(doc) = self.docs.get_mut(id) else {
                continue;
            };
            if doc.editor.core.text() == disk {
                continue; // our own save, or no real change
            }
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            if doc.is_dirty() {
                self.status = format!(
                    "{name} changed on disk; your unsaved edits are kept. Save to overwrite."
                );
                continue;
            }
            doc.editor.set_text(&disk);
            doc.editor.core.doc.mark_saved();
            let edits = doc.editor.take_lsp_edits();
            self.lsp.did_change(id, &self.docs, &edits);
            self.status = format!("Reloaded {name} (changed on disk)");
        }
        if changes.structure {
            self.refresh_tree();
        }
    }

    /// Re-reads the Explorer tree (after files are added or removed).
    pub fn refresh_tree(&mut self) {
        if let Some(p) = &self.project {
            self.tree = Some(p.file_tree(20_000));
            self.files = p.files(50_000);
        }
    }
}

/// The recent-activity list shown in the Activity panel.
#[derive(Debug, Default)]
pub struct Activity {
    entries: VecDeque<ActivityEntry>,
}

#[derive(Debug, Clone)]
pub struct ActivityEntry {
    pub time: String,
    pub agent: bool,
    pub text: String,
}

impl Activity {
    const MAX: usize = 500;

    pub fn push(&mut self, agent: bool, text: String) {
        if self.entries.len() == Self::MAX {
            self.entries.pop_front();
        }
        let time = chrono::Local::now().format("%H:%M:%S").to_string();
        self.entries.push_back(ActivityEntry { time, agent, text });
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, ActivityEntry> {
        self.entries.iter()
    }
}

type Handler = fn(&mut AppState, &Invocation) -> Result<Value, CommandError>;

pub fn register(bus: &mut CommandBus<AppState>) {
    let commands: [(CommandSpec, Handler); 17] = [
        (
            CommandSpec::new("app.info", "About This IDE", Access::Read)
                .description("Name and version of the running PeroxIDE, and whether a project is open."),
            |s, _| {
                Ok(json!({
                    "name": "PeroxIDE",
                    "version": env!("CARGO_PKG_VERSION"),
                    "project": s.project.as_ref().map(|p| p.root.display().to_string())
                }))
            },
        ),
        (
            CommandSpec::new("status.show_message", "Show Status Message", Access::Edit)
                .description("Shows a short message in the IDE's status bar, where the user can see it.")
                .params(json!({ "type": "object", "required": ["text"], "properties": {
                    "text": { "type": "string", "description": "The message, one line." }
                }})),
            |s, inv| {
                s.status = inv.str_arg("text")?.lines().next().unwrap_or("").to_owned();
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("activity.recent", "Recent Activity", Access::Read)
                .description("The most recent entries of the IDE's Activity panel, newest last: commands run by the user and by agents.")
                .params(json!({ "type": "object", "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 }
                }})),
            |s, inv| {
                let limit = inv.args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
                let entries: Vec<Value> = s
                    .activity
                    .iter()
                    .rev()
                    .take(limit)
                    .rev()
                    .map(|e| json!({ "time": e.time, "by": if e.agent { "agent" } else { "user" }, "text": e.text }))
                    .collect();
                Ok(json!(entries))
            },
        ),
        (
            CommandSpec::new("view.screenshot", "Screenshot of the IDE", Access::Read)
                .description("Saves a PNG of the IDE window and returns its path, so you can look at what's on screen. The file is written within a moment of the call; read it after that.")
                .params(json!({ "type": "object", "properties": {
                    "path": { "type": "string", "description": "Where to save; default: a temporary file." }
                }})),
            |s, inv| {
                let path = match inv.args.get("path").and_then(Value::as_str) {
                    Some(p) => std::path::PathBuf::from(p),
                    None => std::env::temp_dir().join(format!("peroxide-screenshot-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
                };
                let target = path.clone();
                let save = egui::ScreenshotCallback::new(move |img| {
                    let [w, h] = img.size;
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    if let Some(png) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                        let _ = png.save(&target);
                    }
                });
                s.ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(save));
                s.ctx.request_repaint();
                Ok(json!({ "path": path }))
            },
        ),
        (
            CommandSpec::new("app.quit", "Quit PeroxIDE", Access::Execute).ui_only(),
            |s, _| {
                s.quit_requested = true;
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("help.about", "About PeroxIDE", Access::Read).ui_only(),
            |s, _| {
                s.show_about = true;
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("app.settings", "Settings…", Access::Read).ui_only(),
            |s, _| {
                s.show_settings = true;
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("view.command_palette", "Command Palette…", Access::Read).ui_only(),
            |s, _| {
                s.palette_request = Some(crate::palette::Mode::Commands);
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("view.quick_open", "Go to File…", Access::Read).ui_only(),
            |s, _| {
                s.palette_request = Some(crate::palette::Mode::Files);
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("editor.goto_line_prompt", "Go to Line…", Access::Read).ui_only(),
            |s, _| {
                s.palette_request = Some(crate::palette::Mode::Line);
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("view.reset_layout", "Reset Layout", Access::Edit).ui_only(),
            |s, _| {
                s.reset_layout = true;
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("mcp.toggle", "Turn MCP Server On or Off", Access::Execute).ui_only(),
            |s, _| {
                if s.mcp.is_on() {
                    s.mcp.stop();
                    s.status = "MCP server off".into();
                } else {
                    s.mcp.start(&s.settings.mcp);
                    s.status = match &s.mcp.error {
                        Some(e) => e.clone(),
                        None => format!("MCP server listening on port {}", s.settings.mcp.port),
                    };
                }
                Ok(json!(s.mcp.is_on()))
            },
        ),
        (
            CommandSpec::new("mcp.set_connect_from", "MCP: Connect From", Access::Execute)
                .ui_only()
                .params(json!({ "type": "object", "required": ["value"], "properties": {
                    "value": { "enum": ["this_computer", "another_device"] }
                }})),
            |s, inv| {
                s.settings.mcp.connect_from = match inv.str_arg("value")? {
                    "this_computer" => ConnectFrom::ThisComputer,
                    "another_device" => ConnectFrom::AnotherDevice,
                    _ => return Err(CommandError::InvalidArgs("`value` must be this_computer or another_device".into())),
                };
                save_and_restart(s)
            },
        ),
        (
            CommandSpec::new("mcp.set_port", "MCP: Set Port", Access::Execute)
                .ui_only()
                .params(json!({ "type": "object", "required": ["port"], "properties": {
                    "port": { "type": "integer", "minimum": 1024, "maximum": 65535 }
                }})),
            |s, inv| {
                let port = inv
                    .args
                    .get("port")
                    .and_then(Value::as_u64)
                    .filter(|p| (1024..=65535).contains(p))
                    .ok_or_else(|| CommandError::InvalidArgs("`port` must be 1024 to 65535".into()))?;
                s.settings.mcp.port = port as u16;
                save_and_restart(s)
            },
        ),
        (
            CommandSpec::new("mcp.new_token", "MCP: New Access Token", Access::Execute).ui_only(),
            |s, _| {
                s.settings.mcp.token = perox_mcp::generate_token();
                s.status = "New MCP token: copy the Claude Code command again".into();
                save_and_restart(s)
            },
        ),
        (
            CommandSpec::new("mcp.set_start_on_launch", "MCP: Turn On When PeroxIDE Starts", Access::Execute)
                .ui_only()
                .params(json!({ "type": "object", "required": ["value"], "properties": {
                    "value": { "type": "boolean" }
                }})),
            |s, inv| {
                s.settings.mcp.start_on_launch = inv
                    .bool_arg("value")?
                    .ok_or_else(|| CommandError::InvalidArgs("`value` is required".into()))?;
                s.settings.save();
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("mcp.set_access", "MCP: What Agents May Do", Access::Execute)
                .ui_only()
                .params(json!({ "type": "object", "required": ["value"], "properties": {
                    "value": { "enum": ["read", "edit", "execute"] }
                }})),
            |s, inv| {
                s.settings.mcp.max_access = match inv.str_arg("value")? {
                    "read" => Access::Read,
                    "edit" => Access::Edit,
                    "execute" => Access::Execute,
                    _ => return Err(CommandError::InvalidArgs("`value` must be read, edit or execute".into())),
                };
                save_and_restart(s)
            },
        ),
    ];
    for (spec, handler) in commands {
        bus.register(spec, handler)
            .expect("app command ids are valid and unique");
    }
}

fn save_and_restart(s: &mut AppState) -> Result<Value, CommandError> {
    s.settings.save();
    let was_on = s.mcp.is_on();
    s.mcp.restart_if_on(&s.settings.mcp);
    if was_on && !s.mcp.is_on() {
        let why = s
            .mcp
            .error
            .clone()
            .unwrap_or_else(|| "the MCP server stopped".into());
        s.status = why.clone();
        return Err(CommandError::Failed(why));
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use perox_core::Origin;

    fn setup() -> (CommandBus<AppState>, AppState) {
        let mut bus = CommandBus::new();
        register(&mut bus);
        let ctx = eframe::egui::Context::default();
        let (mcp, _rx) = McpController::new(Vec::new(), ctx.clone());
        (bus, AppState::new(Settings::default(), mcp, ctx))
    }

    #[test]
    fn agents_see_only_the_safe_commands() {
        let (bus, _) = setup();
        let ids: Vec<_> = bus.agent_specs(Access::Execute).map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                "activity.recent",
                "app.info",
                "status.show_message",
                "view.screenshot"
            ]
        );
    }

    #[test]
    fn status_message_and_activity() {
        let (bus, mut s) = setup();
        let agent = Origin::Agent {
            max_access: Access::Edit,
        };
        bus.execute(
            &mut s,
            "status.show_message",
            json!({"text": "hello\nignored"}),
            agent,
        )
        .unwrap();
        assert_eq!(s.status, "hello");
        s.activity.push(true, "one".into());
        s.activity.push(false, "two".into());
        let recent = bus
            .execute(&mut s, "activity.recent", json!({"limit": 1}), agent)
            .unwrap();
        assert_eq!(recent[0]["text"], "two");
        assert_eq!(recent[0]["by"], "user");
    }

    #[test]
    fn mcp_settings_validate() {
        let (bus, mut s) = setup();
        assert!(
            bus.execute(&mut s, "mcp.set_port", json!({"port": 80}), Origin::User)
                .is_err()
        );
        assert!(
            bus.execute(
                &mut s,
                "mcp.set_access",
                json!({"value": "root"}),
                Origin::User
            )
            .is_err()
        );
    }
}
