# 0009. Toolchain Set 2026.11: what's in, what's an add-on

- **Status:** Accepted
- **Date:** 2026-11-17

## Context
[ADR 0005](0005-toolchain-sets.md) says each release bundles one pinned set. This ADR
picks the first set and the rule for what's in it. Manifest: [toolchain-set/](../../toolchain-set/).

## Decision

**Toolchain:** Rust **1.100.0** (edition 2024, LLVM 23.1.5), rust-analyzer from the same
release, lldb-dap from **LLVM 23.1.5**, rust-lld. OS floors: macOS 12, glibc 2.35, Windows 10 1809.

**The basics** (exact versions in `check/Cargo.toml`):

| Area | Crates |
|---|---|
| UI | eframe/egui 0.37.0, egui_extras (tables, image/svg loaders), egui_plot, egui_tiles (docking), taffy (layout) |
| Desktop | rfd (file dialogs), arboard (clipboard), directories, open |
| Data formats | serde, serde_json, toml, ron, quick-xml, csv |
| Errors, logging | anyhow, thiserror, log, tracing, tracing-subscriber |
| Text, time | regex, unicode-segmentation, chrono |
| Async, network, DB | tokio, reqwest (rustls), sqlx (SQLite bundled, Postgres, MySQL), rustls |
| Images | image, resvg |
| Utilities | uuid, rand, sha2, base64, flate2, zip, rayon, crossbeam-channel, parking_lot |
| i18n | fluent-bundle, unic-langid |

**What gets in.** A crate is a basic only if *all* of these hold. Otherwise it's an add-on.
1. Pure Rust, or it vendors its C code: it needs **no system development packages** beyond
   what egui itself needs.
2. Builds and passes the set check on macOS, Linux and Windows, and doesn't rule out Android.
3. Permissive license (MIT, Apache, BSD, ISC, Zlib, Unicode, MPL file-level). No GPL/LGPL.
4. Widely used and maintained.
5. Something a typical GUI app needs.

**Add-ons** (installed separately, with their own version pins and set checks). Examples:
local AI (llama.cpp), audio (cpal/rodio: ALSA on Linux), media (GStreamer), web view (wry:
WebKitGTK on Linux), system tray and global hotkeys (tray-icon: GTK on Linux), PDF (pdfium),
serial ports, Bluetooth, scripting (rhai, mlua), 3D.

**Choices made while building the set:**
- **egui_tiles, not egui_dock**, for docking: egui_dock 0.21 still targets egui 0.36, and
  would pull in a second egui.
- **TLS: rustls with `ring` only.** If aws-lc-rs is also enabled, rustls has two providers
  and panics at runtime. ring also cross-compiles without CMake, which helps Android.
  reqwest uses `rustls-no-provider`, so **`perox-ui` installs the ring provider at app start**.
  The set check does the same, and caught this.
- **chrono** for time, because sqlx supports it. egui_extras' date picker (jiff) is not enabled;
  `perox-ui` provides its own.
- **resvg 0.45** and **parking_lot 0.12**: the versions egui uses, so there's one copy in each build.

## Consequences
- New projects build offline with these crates. Adding anything else from crates.io works
  when online.
- Some crates appear twice in the dependency tree (base64, sha2, objc2…) because the
  ecosystem is split between versions. That costs compile time only. Revisit with each set.
- Printing has no good cross-platform crate. That's a known gap, likely something `perox-ui`
  provides later.
