# Architecture Decision Records

One page per decision that someone might later ask "why?" about. To add one, copy
[0000-template.md](0000-template.md) and take the next number. Never edit an accepted ADR's
decision. Supersede it with a new one.

| # | Decision | Status |
|---|---|---|
| [0001](0001-ui-toolkit.md) | egui for the IDE and for user apps | Accepted |
| [0002](0002-command-bus.md) | Every action is a command on one bus | Accepted |
| [0003](0003-form-format.md) | `.pxui` declarative forms + Rust code-behind | Accepted |
| [0004](0004-licensing.md) | GPL-3.0 IDE, MIT/Apache runtime, CLA | Accepted |
| [0005](0005-toolchain-sets.md) | Bundled, pinned Toolchain Sets; no self-update | Accepted |
| [0006](0006-mcp-server.md) | Built-in MCP server, off by default | Accepted |
| [0007](0007-android.md) | Android: the IDE runs on-device | Proposed (needs spike) |
| [0008](0008-preview-host.md) | Designer previews run out of process | Accepted |
