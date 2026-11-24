# Architecture

One page. The decisions behind each part are in [docs/adr](docs/adr/).

## The big picture

```
            ┌──────────── PeroxIDE process (egui) ────────────┐
 Menus ─┐   │                                                 │
 Keys  ─┼──▶│  Command bus (perox-core)  ──▶  IDE state       │
 Palette┘   │      ▲                          projects, docs, │
            │      │                          undo stacks     │
 MCP ──HTTP─┼──────┘                                          │
 (agents)   │                                                 │
            └───┬──────────────┬───────────────┬──────────────┘
                │ LSP          │ DAP           │ IPC (paint output, input)
         rust-analyzer      lldb-dap      preview host (your project's
                                          components, rendering forms)
                │
          cargo / rustc  ← the bundled Toolchain Set
```

## Rules that hold everywhere

1. **Every action is a command.** Menus, shortcuts, the command palette and MCP
   all call `CommandBus::execute(id, args, origin)`. Nothing changes IDE state any
   other way. This is what lets an agent do anything a person can, and what
   gives one place for undo, logging and permissions. [ADR 0002](docs/adr/0002-command-bus.md)
2. **Forms are `.slint` files; code-behind is plain Rust.** The designer and people
   editing by hand work on the same text. Generated code is never edited.
   [ADR 0011](docs/adr/0011-slint-for-user-apps.md)
3. **Anything that can crash or hang runs in another process**: rust-analyzer, the
   debugger, cargo, and the designer's preview host, which runs third-party components.
   The IDE stays up. [ADR 0008](docs/adr/0008-preview-host.md)
4. **Nothing updates itself.** The toolchain, tools and vendored crates are one
   tested Toolchain Set per IDE release. [ADR 0005](docs/adr/0005-toolchain-sets.md)
5. **The licensing boundary is a dependency boundary.** Crates that end up inside user
   apps (`perox-ui` and friends) are MIT/Apache and must never depend on a GPL-only
   crate; Slint, under its own licenses, is the one exception.
   [ADR 0004](docs/adr/0004-licensing.md), [ADR 0011](docs/adr/0011-slint-for-user-apps.md)

## Crates

✅ exists · ⏳ planned

| Crate | What it does | License |
|---|---|---|
| ✅ `apps/peroxide` | The executable: docking (egui_tiles), menus, palette, settings, MCP host; wires everything to the bus | GPL |
| ✅ `perox-core` | Command bus, access levels. Later: project model, documents, undo | GPL |
| ✅ `perox-editor` | Code editor: rope, multi-cursor, tree-sitter highlighting, completion UI, snippets | GPL |
| ✅ `perox-lsp` | rust-analyzer client | GPL |
| ✅ `perox-dap` | Debug Adapter Protocol client and a pure session state machine; finds lldb-dap and the Rust formatters ([ADR 0012](docs/adr/0012-debugger.md)) | GPL |
| ✅ `perox-build` | cargo runs, JSON diagnostics and artifacts, test discovery and libtest result parsing, finding the toolchain | GPL |
| ✅ `perox-project` | Cargo projects and the file tree | GPL |
| ✅ `perox-terminal` | Integrated terminal (alacritty_terminal + egui) | GPL |
| ✅ `perox-icons` | Lucide SVG icons, tinted to the theme ([ADR 0010](docs/adr/0010-icons.md)) | MIT/Apache + ISC |
| ✅ `perox-slint` | `.slint` element tree with exact spans (Slint's own parser), text-preserving designer edits, handlers codegen + `build.rs` helper | MIT/Apache |
| ✅ `perox-ui` | **Runtime inside user apps**, on top of Slint: `include_ui!`, native message boxes, dialogs | MIT/Apache |
| ✅ `perox-designer` | Form designer: canvas rendered by `slint-interpreter`, palette, object tree, inspector. Gestures become `.slint` text edits | GPL |
| ⏳ `perox-preview` | Out-of-process form renderer | GPL |
| ✅ `perox-mcp` | MCP server: Streamable HTTP, token, sessions, four tools over the bus. Later: `peroxide mcp` stdio shim | GPL |
| ⏳ `perox-package` | .app/DMG bundles, signing, notarization; later APK/MSI/AppImage | GPL |
| ⏳ `perox-android` | Android host: activity glue, running the toolchain on-device | GPL |

## Where to start reading code

1. `crates/perox-core/src/command.rs`: the command bus.
2. `apps/peroxide/src/commands.rs`: the app's commands and the state they change.
3. `apps/peroxide/src/app.rs`: one frame. Agent calls are drained, shortcuts and menus collect
   command ids, and everything runs through `PeroxApp::run`, which logs it to Activity.
4. `crates/perox-mcp` and `apps/peroxide/src/mcp_host.rs`: how agent calls reach the bus
   ([docs/mcp.md](docs/mcp.md#how-it-works)).
