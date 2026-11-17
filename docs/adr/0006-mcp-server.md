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
- **Four tools, not one per command:** `list_commands`, `describe_command`, `execute`, `batch`.
  They reach every command in `CommandBus::agent_specs()` ([0002](0002-command-bus.md)).
  Hundreds of per-command tools would flood the agent's context, and MCP tool names can't
  contain the dots our ids use. SolidCraft uses the same shape.
- Long calls (build, package) will get a `pending` tool: a long call returns "still running,
  call `pending`", is never re-run, and never runs twice. Until then a call waits up to 2 minutes.
- Security: off by default; a 64-character bearer token compared in constant time;
  `127.0.0.1` unless the user picks **Another device**; file access limited to the open
  project; agent edits go on the normal undo stack.

## Consequences
- Every new command is automatically usable by agents. Description quality matters.
- User guide: [docs/mcp.md](../mcp.md).
- LAN mode is plain HTTP. We say so in the UI and docs, same as SolidCraft.
