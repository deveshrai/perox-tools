# peroxide (app)

The IDE executable. License: GPL-3.0-or-later.

| File | What |
|---|---|
| `app.rs` | One frame: drain agent calls, menus, status bar, docking, palette, dialogs. All actions go through `PeroxApp::run`. |
| `commands.rs` | The app's commands and `AppState`, the state they change. |
| `layout.rs` | Docking panes (egui_tiles). |
| `palette.rs` | Command palette. |
| `keymap.rs` | Default shortcuts. |
| `settings.rs` | Settings saved as JSON (`PEROXIDE_CONFIG_DIR` overrides the folder). |
| `mcp_host.rs`, `mcp_page.rs` | MCP server lifecycle and bridge; its settings page. |

Rule: UI code never changes state directly. It collects command ids, and `run` executes them.
