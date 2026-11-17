# perox-build

`tools` finds `cargo` and `rust-analyzer` (bundled Toolchain Set, then rustup's pinned
toolchain, then PATH). `job` runs a cargo command, turns its JSON into diagnostics and
Output-panel lines, and stops it with whatever it started. License: GPL-3.0-or-later.
