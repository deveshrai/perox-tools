//! Settings saved between runs, as JSON in the OS config folder
//! (`PEROXIDE_CONFIG_DIR` overrides the folder, for tests and portable installs).

use std::path::PathBuf;

use perox_core::Access;
use serde::{Deserialize, Serialize};

/// Default MCP port: SolidCraft uses 27880, so the two can run side by side.
pub const DEFAULT_MCP_PORT: u16 = 27890;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mcp: McpSettings,
    /// Most recent first.
    pub recent_projects: Vec<PathBuf>,
    /// Run rustfmt (through rust-analyzer) when saving Rust files.
    pub format_on_save: bool,
    /// rust-analyzer runs plain `cargo check` on save instead of clippy (off: clippy).
    pub plain_check_on_save: bool,
    /// Open files per project root, restored when the project opens again.
    pub sessions: std::collections::BTreeMap<PathBuf, Session>,
    /// The debug adapter to use instead of the lldb-dap PeroxIDE finds (ADR 0012).
    pub debugger_adapter: Option<PathBuf>,
    /// Where these settings are saved. `None` (defaults, tests) never touches disk.
    #[serde(skip)]
    pub file: Option<PathBuf>,
}

/// A project's open files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub open: Vec<PathBuf>,
    pub active: Option<PathBuf>,
    /// Breakpoints by file, 1-based lines.
    pub breakpoints: std::collections::BTreeMap<PathBuf, Vec<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectFrom {
    /// `127.0.0.1` only.
    ThisComputer,
    /// The local network too.
    AnotherDevice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub start_on_launch: bool,
    pub connect_from: ConnectFrom,
    pub port: u16,
    pub token: String,
    pub max_access: Access,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            start_on_launch: false,
            connect_from: ConnectFrom::ThisComputer,
            port: DEFAULT_MCP_PORT,
            token: String::new(),
            max_access: Access::Execute,
        }
    }
}

fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("PEROXIDE_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    directories::ProjectDirs::from("dev", "PeroxIDE", "PeroxIDE").map(|d| d.config_dir().to_owned())
}

fn path() -> Option<PathBuf> {
    dir().map(|d| d.join("settings.json"))
}

impl Settings {
    /// Loads settings; missing or unreadable files give defaults. Makes sure a token exists.
    pub fn load() -> Self {
        let file = path();
        let mut s: Settings = file
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        s.file = file;
        if s.mcp.token.len() < 32 {
            s.mcp.token = perox_mcp::generate_token();
            s.save();
        }
        s
    }

    /// Puts a project at the top of the recent list and saves.
    pub fn remember_project(&mut self, root: &std::path::Path) {
        self.recent_projects.retain(|p| p != root);
        self.recent_projects.insert(0, root.to_owned());
        self.recent_projects.truncate(10);
        self.save();
    }

    /// Saves settings. The file holds the MCP token, so on Unix only the owner can read it.
    pub fn save(&self) {
        let Some(path) = &self.file else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(json) = serde_json::to_vec_pretty(self) else {
            return;
        };
        let _ = write_private(path, &json);
    }
}

/// Writes a file that only the owner can read (Unix). The mode is set at creation,
/// so the token is never briefly world-readable.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        // Also tighten a file created by an older version.
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    options.open(path)?.write_all(bytes)
}
