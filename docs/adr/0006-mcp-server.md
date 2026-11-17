# 0006. Built-in MCP server, off by default

- **Status:** Accepted
- **Date:** 2026-11-17

## Context
AI agents such as Claude Code should be able to drive the IDE: edit code, build, run,
and design forms. SolidCraft's Android port has a proven, simple user experience:
[android-mcp.md](https://github.com/deveshrai/Solidcraft-Android-Port/blob/main/docs/android-mcp.md).

## Decision
We will copy that design.
- **Tools › MCP Server** turns it on or off. **Settings › MCP server** shows:

  | Setting | Purpose |
  |---|---|
  | **Connect from** | **This computer** (`127.0.0.1`) or **Another device** (LAN) |
  | **Start Claude Code with it** | copyable `claude --mcp-config '…'` command |
  | Advanced | port (default **27890**), token (Show / New token), permanent `claude mcp add …` command, start on launch |
  | Status line | listening / n sessions connected, last request; count of refused requests |

- Transport: **Streamable HTTP** (`POST /mcp`, JSON responses, `Mcp-Session-Id`), plus a
  `peroxide mcp` **stdio shim** that forwards to the running IDE.
- Tools = `CommandBus::agent_specs()` ([0002](0002-command-bus.md)), plus `pending` for
  long calls (build, package): a long call returns "still running, call `pending`", is never
  re-run, and never runs twice.
- Security: off by default; a 64-character bearer token compared in constant time;
  `127.0.0.1` unless the user picks **Another device**; file access limited to the open
  project; agent edits go on the normal undo stack.

## Consequences
- Every new command is automatically an MCP tool. Description quality matters.
- LAN mode is plain HTTP. We say so in the UI and docs, same as SolidCraft.
