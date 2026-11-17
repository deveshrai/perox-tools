# perox-core

The command bus. Every IDE action is a `CommandSpec` plus a handler registered
on `CommandBus<S>`; menus, shortcuts, the command palette and the MCP server
all invoke commands by id. Agents only see commands that are `agent_visible`
and within their `Access` level. Why: [ADR 0002](../../docs/adr/0002-command-bus.md).

License: GPL-3.0-or-later (IDE side).
