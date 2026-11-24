# 0012. Debugging through the Debug Adapter Protocol, with lldb-dap

- **Status:** Accepted
- **Date:** 2026-11-24

## Context
Phase 3 needs a debugger: breakpoints in the editor, stepping, the call stack, variables,
and an expression console, on macOS, Linux and Windows, scriptable over MCP. Writing a
debugger is out of the question; we drive an existing one. The Debug Adapter Protocol
(DAP) is the standard way to do that: JSON messages over a child's stdin/stdout, the same
framing as LSP.

Candidate adapters:
- **lldb-dap** (LLVM, Apache-2.0 with LLVM exception). Ships with Xcode on macOS
  (`xcrun -f lldb-dap`), with LLVM packages on Linux (`lldb-dap`, `lldb-dap-18`, …) and
  with the LLVM installer on Windows. Rust's toolchain ships LLDB formatters
  (`lib/rustlib/etc/lldb_lookup.py`, `lldb_commands`) that make `Vec`, `String`,
  `Option` and friends readable.
- **CodeLLDB** (MIT): lldb plus Rust-friendly extras, but bundled as a VS Code extension;
  using it means shipping a second copy of LLDB.
- **cppvsdbg / vsdbg** (Microsoft): the best Windows/MSVC experience, but its licence only
  allows use from Visual Studio products. Not an option.
- **gdb's DAP mode** (`gdb -i dap`, GPL): Linux only in practice.

## Decision
We will:
1. **Speak DAP** from a new crate, `perox-dap`: transport, a session state machine that
   is pure and unit-testable, and adapter discovery.
2. **Use lldb-dap on every desktop OS by default**, found in this order: the
   `debugger.adapter` setting, `PEROXIDE_DAP`, `xcrun -f lldb-dap` (macOS), `lldb-dap`
   or the newest `lldb-dap-N` on `PATH`, `/usr/lib/llvm-N/bin`, then
   `C:\Program Files\LLVM\bin` (Windows). Any DAP adapter that understands lldb-dap's
   `launch` arguments (CodeLLDB does) can be set instead.
3. **Load the toolchain's Rust formatters** into every session (`initCommands`), map
   `/rustc/<commit>` to the toolchain's `rust-src` when it's installed, and break on
   `rust_panic` by default so a panic stops where it happens.
4. **Build before launching**: `cargo build` (or `cargo test --no-run` for a test),
   taking the executable from cargo's JSON artifacts.

## Consequences
- No debugger ships inside PeroxIDE. The Toolchain Set may bundle lldb-dap later; until
  then, macOS needs Xcode's command line tools, Linux an `lldb` package, and Windows the
  LLVM installer. The IDE says so when it can't find one.
- Windows debugging of MSVC binaries goes through LLDB's PDB support, which is
  workable but rougher than on macOS and Linux.
- The session model doesn't care which adapter is on the other end, so adding gdb's DAP
  mode or CodeLLDB later is a setting, not a rewrite.

## Alternatives considered
See Context: CodeLLDB as the default (a second LLDB to ship), vsdbg (licence), gdb DAP
(Linux only), and driving `lldb` through its command line (fragile, not a protocol).
