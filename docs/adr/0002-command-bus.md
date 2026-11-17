# 0002. Every action is a command on one bus

- **Status:** Accepted
- **Date:** 2026-11-17

## Context
Menus, keyboard shortcuts, the command palette, undo and an AI agent over MCP all need
to trigger the same actions. If each wires into state on its own, the MCP server ends
up as a bolted-on second API that always lags behind the UI.

## Decision
We will route **every** state-changing action through `perox_core::CommandBus`. A command
is a `CommandSpec` (stable id, title, description, JSON Schema params, `Access` level,
agent visibility) plus a handler. Callers pass an `Origin`: `User`, or `Agent { max_access }`.
- `Access` is `Read < Edit < Execute`. An agent session is granted a maximum level.
- `.ui_only()` commands (quit, open a dialog) are invisible to agents. To an agent they
  are indistinguishable from commands that don't exist.
- The MCP server publishes `agent_specs()` as its tool list.

## Consequences
- MCP coverage comes for free: a new feature is agent-usable as soon as it's a command.
- One place for undo grouping, logging and permission checks.
- UI code must not mutate state directly. Reviewers check this.
- Argument checking is currently by hand (`Invocation::str_arg`, …). Full JSON Schema
  validation can come later, inside `execute`.

## Alternatives considered
- Separate MCP API layer: duplicated logic, and it drifts.
- Trait objects per command type: more boilerplate for the same result.
