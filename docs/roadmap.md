# Roadmap

**Milestone 1 = desktop: macOS, Linux, Windows.** Every phase must work on all three
(CI builds and tests on each). Day-to-day development happens on macOS.
Android is milestone 2 ([ADR 0007](adr/0007-android.md)).

Each phase ends with a usable build and a short demo note in `docs/`.

## Milestone 1: desktop

### Phase 0: foundation ✅
- [x] Workspace, pinned toolchain, CI on macOS/Linux/Windows
- [x] Licenses, CLA draft, README, ARCHITECTURE, ADRs 0001–0008
- [x] Command bus with access levels (`perox-core`), app shell with menus wired to it
- [x] Toolchain Set 2026.11 pinned, set check on every OS ([ADR 0009](adr/0009-toolchain-set-2026-11.md))
- [x] Docking layout (egui_tiles), command palette, keybindings, settings storage
- [x] MCP server: Streamable HTTP, token, sessions, four tools over the bus,
      Tools › MCP Server toggle, settings page ([docs/mcp.md](mcp.md))

**Done when:** Claude Code connects to a running PeroxIDE and calls `app.info`.
✅ Done 2026-11-17: a headless Claude Code session listed the commands, ran `app.info` and
posted a status message. The IDE's Activity panel showed each call.

**Carried into Phase 1:** save and restore the layout; user-defined keybindings; the `pending`
tool for long calls; the `peroxide mcp` stdio shim.

### Phase 1: a real Rust IDE ← *next*
Project model (Cargo workspaces, `[package.metadata.peroxide]`), file tree, custom code
editor (rope, tree-sitter, multi-cursor, folding), rust-analyzer (completion, go to
definition, references, rename, diagnostics), cargo build/run/test with a problems panel,
integrated terminal, git basics.

**Done when:** you can comfortably write, build and run a multi-file Rust project in PeroxIDE.

### Phase 2: forms
`.pxui` parser, formatter and interpreter; `perox-ui` runtime with taffy layouts and core
widgets; designer canvas, palette, object tree, property inspector; code generation and
handler hookup; live preview.

**Done when:** you can drag out a window with laid-out widgets, wire a button to Rust code,
hand-edit the `.pxui`, and run it.

### Phase 3: pro tooling
Debugger (lldb-dap on macOS/Linux; on Windows, lldb-dap or cppvsdbg, decided by ADR), test
explorer, full git, clippy and rustfmt integration, all major features exposed over MCP,
including designer operations and form screenshots.

### Phase 4: RAD depth
Third-party components and the preview host, non-visual components, menus, actions and
toolbars, resources, themes, i18n (Fluent), model/view and data-aware controls (sqlx), form
inheritance, tab order.

### Phase 5: ship
Toolchain Sets per OS ([ADR 0005](adr/0005-toolchain-sets.md)), installers
(signed and notarized DMG; MSI; AppImage and deb), packaging user apps for all three OSes,
templates, examples, user docs.

## Milestone 2: Android
Android spike ([spikes/0001](spikes/0001-android-toolchain.md)), IDE on Android, on-device
APK builds and installation, MCP for Termux and Wi-Fi, packaging user apps as APKs.
