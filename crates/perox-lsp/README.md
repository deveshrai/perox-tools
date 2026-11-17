# perox-lsp

Language-server client. `client` speaks JSON-RPC over a child process's stdio and never blocks
the caller; `rust_analyzer` starts rust-analyzer with the capabilities the editor uses
(utf-32 positions, snippets, lazy auto-import edits, semantic tokens, live diagnostics).
`uri` converts paths. Converting answers into editor state is the app's job
(`apps/peroxide/src/lsp.rs`).

`cargo test -p perox-lsp -- --ignored` runs a live test against the real rust-analyzer.
License: GPL-3.0-or-later.
