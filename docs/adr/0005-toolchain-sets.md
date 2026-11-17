# 0005. Bundled, pinned Toolchain Sets; no self-update

- **Status:** Accepted
- **Date:** 2026-11-17

## Context
One install must contain everything, and nothing may change underneath the user.
Self-updating toolchains break builds unexpectedly.

## Decision
Each IDE release ships exactly one **Toolchain Set**, versioned like `2027.1`, built and
tested in CI from one manifest:
- rustc, cargo, std (host and all supported targets), rustfmt, clippy
- rust-analyzer, lldb-dap, lld
- a **vendored local registry** of approved crates (egui, `perox-ui`, bundled
  components and their dependencies), so new projects build offline
- a **pre-warmed build cache** for the project templates, so the first build is fast

Nothing in the set updates on its own. The only way to update is to install a new IDE
release. Projects can still add crates from crates.io when online; vendored crates
resolve first.

## Consequences
- Large installers, about 0.6–1 GB per platform.
- We must build, test and sign the set for every OS and architecture.
- Platform linkers can't be redistributed: macOS needs Xcode Command Line Tools,
  Windows needs MSVC Build Tools (or we evaluate the GNU target with bundled
  MinGW). The IDE detects what's missing and guides the user through installing it.

## Alternatives considered
- Installing through rustup on first run: smaller download, but not "everything in one
  install", and versions drift.
