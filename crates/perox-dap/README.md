# perox-dap

PeroxIDE's debugger plumbing ([ADR 0012](../../docs/adr/0012-debugger.md)):

- `client`: a Debug Adapter Protocol client over a child process's stdin/stdout. Never
  blocks: messages arrive on a channel.
- `session`: the debug session as a state machine. Feed it the adapter's messages; it
  tracks threads, the call stack, variables, breakpoints and console output, and queues
  the requests to send next. It does no I/O, so it's unit-tested without a debugger.
- `adapter`: finding `lldb-dap` and the toolchain's Rust formatters.

GPL-3.0-or-later, like the rest of the IDE.
