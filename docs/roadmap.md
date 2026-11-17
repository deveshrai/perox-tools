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

### Phase 1: a real Rust IDE ✅
- [x] Projects: open a Cargo package or workspace, Explorer tree, recent projects
- [x] Editor: rope buffer, multi-cursor, undo/redo, auto-pairs, smart indent, ⌘D, comments
- [x] Syntax highlighting: tree-sitter, plus rust-analyzer's semantic colours on top
- [x] Completion: fast popup, fuzzy ranking with recency, snippets with Tab stops,
      auto-imports, docs panel, signature help. Measured: median 27 ms, p90 36 ms
- [x] Hover, go to definition (F12 / ⌘-click), live diagnostics, Problems panel
- [x] cargo check/build/run/test/clippy, Output panel, Stop
- [x] Agents: project, file, editor, build and problem commands; screenshots
- [x] Find and replace (case, whole word, regex, replace all, matches to cursors), go to line, quick open (⌘P)
- [x] Rename (all files), find references, quick fixes and refactorings (⌘.), format, format on save
- [x] Icons: Lucide SVGs for every toolbar button and menu item ([ADR 0010](adr/0010-icons.md))
- [x] Everything the IDE starts uses the pinned toolchain; quitting asks to save
- [x] Inlay hints (types, parameter names); decorations follow edits without flicker
- [x] Reload files changed on disk (never over unsaved edits); Explorer refreshes; open files restored per project
- [x] Integrated terminal (⌃`), shell in the project with the pinned toolchain; agents: terminal.run / terminal.read
- [x] Git basics: branch, file states in the Explorer, change bars in the gutter, Source Control panel
      with commit; agents: git.status / git.diff / git.commit. Uses the `git` CLI for now;
      moving to pure-Rust `gix` (no git install needed) is planned for the installers (Phase 5)
- [x] Completion benchmark: 25 everyday cases (methods, fields, associated functions, macros,
      auto-imports) against real rust-analyzer: **25/25 in the top 3**, median 7 ms.
      Run: `cargo test -p peroxide completion_benchmark -- --ignored --nocapture`

**Done when:** you can comfortably write, build and run a multi-file Rust project in PeroxIDE.
✅ Done 2026-11-17.

### Phase 2: forms ✅ (core, 2026-11-17)
`.pxui` parser, formatter and interpreter; `perox-ui` runtime with taffy layouts and core
widgets; designer canvas, palette, object tree, property inspector; code generation and
handler hookup; live preview.

**Done when:** you can drag out a window with laid-out widgets, wire a button to Rust code,
hand-edit the `.pxui`, and run it.

| Part | State |
|---|---|
| `.pxui` parser with spans, text-preserving edits | ✅ |
| `perox-ui` runtime: 18 widgets, flex/grid layouts, events | ✅ |
| Code generation, `build.rs` helper, `include_form!` | ✅ |
| Designer: palette drag-drop, move, resize, object tree, inspector, events | ✅ |
| Go to handler (adds the method to the code-behind) | ✅ |
| `designer.*` MCP commands for agents | ✅ |
| Free placement (`Panel`, `layout: free`): snap grid, move, resize handles, nudge, Break layout / Lay out | ✅ |
| File → New GUI App (template with form, code-behind, `build.rs`) | ✅ |
| `.pxui` formatter, multi-select, copy/paste, align/distribute, inspector shows defaults, live preview window | follow-ups |

### Phase 3: pro tooling ← *next*
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
