# 0007. Android: the IDE runs on-device

- **Status:** Proposed. **Not part of milestone 1 (desktop).** Needs the spike in
  [docs/spikes/0001-android-toolchain.md](../spikes/0001-android-toolchain.md) before acceptance.
- **Date:** 2026-11-17

## Context
The IDE should run on Android tablets and phones, build APKs on the device and install them,
and serve MCP to Claude Code in Termux or over Wi-Fi. Four facts constrain this:
- The Rust project ships no rustc or cargo that *run on* Android.
- Since Android 10, apps can't `exec` files from their own writable data folder (W^X).
- There's no Java toolchain on the device.
- Play Store policy restricts apps that run code they didn't ship with.

## Decision (proposed)
- Build rustc, cargo, rust-analyzer, clang and lld for `aarch64-linux-android` in our CI
  (Termux's recipes show it works). Bundle the NDK sysroot.
- Ship those tools in the APK as `lib*.so` (installed to `nativeLibraryDir`, which is
  executable). Run build scripts compiled on the device through `/system/bin/linker64`
  (the termux-exec technique). Proc macros load via `dlopen`; **the spike must confirm this**.
- User apps use `NativeActivity` (`hasCode="false"`). The small Java helper (clipboard, IME)
  ships **precompiled as `classes.dex`**. Manifest, resources and v2/v3 signing are done in
  Rust. Installation goes through the PackageInstaller session API.
- Builds run in a foreground service.
- Distribution: GitHub releases and F-Droid first; Play Store best-effort.

## Consequences
- The riskiest part of the whole project. If the spike fails, fall back to: the IDE on
  Android edits and designs, and builds on a paired desktop over the network.
