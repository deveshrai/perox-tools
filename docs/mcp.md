# Claude Code and PeroxIDE: the MCP server

PeroxIDE has a built-in MCP server. An AI agent such as [Claude Code](https://claude.com/claude-code)
can drive the IDE, and you watch every step in the **Activity** panel. The agent can run:

- on **this computer**, or
- on **another device on the same network**.

## Quick start

1. **Tools › MCP Server**, or open **Settings** (⌘, / Ctrl+,).
2. Tick **Turn on the MCP server**.
3. Under **Connect from**, choose **This computer** or **Another device on the network**.
4. Click **Copy** next to **Start Claude Code with it**, and paste into a terminal.
5. Ask: *"What version of PeroxIDE is running? Then say hello in its status bar."*

The command looks like this, with your address and token:

```sh
claude --mcp-config '{"mcpServers":{"peroxide":{"type":"http","url":"http://127.0.0.1:27890/mcp","headers":{"Authorization":"Bearer 3f9c…"}}}}'
```

## The settings page

| Setting | What it does |
|---|---|
| **Turn on the MCP server** | Starts it. When off, nothing listens. |
| **Connect from: This computer** | Listens on `127.0.0.1` only. Nothing on the network can reach it. |
| **Connect from: Another device** | Listens on the network too. The address shown is this computer's. |
| **Server address** | The URL a client connects to, with **Copy**. |
| **Start Claude Code with it** | One command that starts Claude Code connected to PeroxIDE for that session. |
| Status line | *Listening*, or *Connected: n sessions, last request … ago*. A red line counts requests refused for a wrong or missing token. |

**Advanced**:

| Setting | What it does |
|---|---|
| **Port** | 27890 unless you change it (1024 to 65535). **Use** applies it straight away. |
| **Agents may** | *Look only*, *Look and edit*, or *Everything, including build and run*. Commands above this level are refused. |
| **Access token** | Part of every command. **New token** makes a new one; clients holding the old one are refused. |
| **Turn on when PeroxIDE starts** | Off by default. |
| **Or add it to Claude Code for good** | `claude mcp add --transport http peroxide <url> --header "Authorization: Bearer <token>"` |

The status bar's right-hand corner shows the server's state at all times. Click it to open these settings.

## What the agent can do

Agents get four tools that reach every IDE command they're allowed to use:

| Tool | Does |
|---|---|
| `list_commands` | Lists the commands, optionally filtered by id prefix (`file.`, `designer.`) |
| `describe_command` | One command's description and argument schema |
| `execute` | Runs one command, exactly as if you chose it from a menu |
| `batch` | Runs several in order; stops at the first failure unless told otherwise |

Commands so far, by area: `project.*` (open, info, files), `file.*` (open, read, write,
save, close), `editor.*` (replace, type, completions, accept_completion, cursor, goto_line,
references, rename, code_actions, apply_code_action, format), `build.*`
(check, build, run, test, clippy, clippy_fix, fmt, fmt_check, stop, status, output), `problems.list`, `lsp.*`,
`terminal.run`, `terminal.read`, `test.*` (discover, list, run, results, go_to), `debug.*` (start, continue, step_over,
step_in, step_out, pause, stop, state, select_frame, variables, evaluate, console, watch,
toggle_breakpoint, breakpoints, clear_breakpoints),
`git.*` (status, diff, stage, unstage, discard, commit, branches, switch, log, show, fetch, pull, push), `designer.*` (tree,
add_widget, set_property, remove_widget, move_widget, set_handler, go_to_handler, expose,
select, open_handler, break_layout, lay_out, screenshot, actions, set_action, use_action, resources, add_resource, app_style, make_translatable, bind_query, tab_order, components, add_library), `i18n.*` (strings, add_language,
translate), `data.*` (database, query, execute), `form.new` (a new window or dialog),
`view.screenshot`, `app.info`, `activity.recent`. File access is limited to the open project.

Designer commands edit the open `.slint` document the same way the designer does. They
take elements by id or by path (`#0.1`; `#` is the component's root), an optional
`component` (default: the file's main window), and values as `.slint` expressions. So
text is `"\"Save\""`, and lengths, booleans and colours are `12px`, `true` and `#ff8800`.
Try `designer.tree`, then `designer.select` plus `view.screenshot` to see the result.
Lines and columns are 1-based. Each new IDE feature adds commands, and agents can use them
straight away
([ADR 0002](adr/0002-command-bus.md)). Commands that only make sense with a person present,
such as quitting or changing the MCP settings, are hidden from agents.

## Security

- **Off by default**, and stays off at the next start unless you tick **Turn on when PeroxIDE starts**.
- **A token on every request**: `Authorization: Bearer <token>`, 64 random hex characters,
  compared in constant time. The settings file holding it is readable only by you (macOS, Linux).
- **This computer** listens on `127.0.0.1` only. **Another device** listens on the network over
  plain HTTP. Use it on networks you trust.
- Requests from web pages (with a non-local `Origin` header) are refused, so a malicious site
  can't reach the server through your browser.
- **Agents may** limits what any session can do.

## Troubleshooting

| What you see | Do this |
|---|---|
| Claude Code says the server needs authentication, or *401* | The token changed (**New token**), or the command was cut short. Copy it again. |
| *Port 27890 is in use by another app* | **Advanced › Port**: choose another, **Use**, copy the command again. |
| *This computer isn't on a network* | Join a network, or choose **This computer**. |
| Another device can't connect | Same network? Guest and office networks often keep devices apart. Allow PeroxIDE through the firewall when asked. |
| On Windows, the session command fails in `cmd` | `cmd` doesn't understand single quotes. Use PowerShell 7, or the **add it for good** command. |
| *PeroxIDE did not answer within 2 minutes* | The IDE is busy or a dialog is open. Check the IDE and try again. |
| Claude Code doesn't list PeroxIDE's tools | Run `/mcp` in Claude Code, and check the status line in Settings. |

## How it works

```
Claude Code ── POST /mcp ──▶ perox-mcp (server threads) ──▶ channel ──▶ UI thread: CommandBus::execute
               Bearer token    sessions, JSON-RPC            Call        logs to Activity, replies
```

- `crates/perox-mcp`: Streamable HTTP (`POST /mcp`, JSON responses, no event stream; `GET`
  is 405, `DELETE` ends a session). Protocol revisions 2025-11-25, 2025-06-18 and 2025-03-26.
  Up to 16 sessions and 32 connections; 16 KiB headers, 4 MiB bodies, 30 s to send a request.
- `apps/peroxide/src/mcp_host.rs`: bridges calls to the UI thread and owns start and stop.
- `apps/peroxide/src/mcp_page.rs`: the settings page. Each control runs an `mcp.*` command.
