# 0008. Designer previews run out of process

- **Status:** Accepted (amended by [0011](0011-slint-for-user-apps.md))
- **Date:** 2026-11-17

## Context
The designer must show third-party widget crates exactly as they'll look at runtime.
Rust has no stable ABI, so loading arbitrary crates into the IDE as plugins is fragile, and
a buggy component would crash the IDE.

## Decision
We will build a **preview host**: a small binary compiled from the user's own project and
its dependencies (with a `designer` feature). It interprets the `.pxui` form, renders it
with egui, and sends egui's paint output (shapes and textures) to the IDE over IPC. The IDE
draws that on the design canvas and forwards input back. Built-in widgets may also render
in-process for speed.

## Consequences
- Crash isolation, and the preview is exactly what the app will show, with any crate.
- Adding a dependency rebuilds the preview host (incremental, but it takes seconds).
- We maintain a serialisation of egui paint output across process boundaries.

## Alternatives considered
- `dylib` plugins via `abi_stable`: fragile, and component authors pay the price.
- WASM components: egui output across the WASM boundary has the same IPC problem, plus a
  sandboxed and restricted runtime.
