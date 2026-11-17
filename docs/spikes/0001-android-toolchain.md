# Spike 0001: Rust toolchain on Android

**Milestone 2.** Not started. Run before accepting [ADR 0007](../adr/0007-android.md).
Time box: two weeks.

## Questions, each with a pass condition

| # | Question | Passes when |
|---|---|---|
| 1 | Can we build rustc and cargo for `aarch64-linux-android` and run them from an APK's `nativeLibraryDir`? | `rustc --version` runs from inside our test app on Android 14+ |
| 2 | Can cargo run a build script compiled on the device? | A crate with `build.rs` builds, using the `/system/bin/linker64` wrapper |
| 3 | Can rustc load proc macros built on the device? | A crate using `serde_derive` builds |
| 4 | Can we produce a signed APK without Java? | A hello-egui `NativeActivity` APK, built and signed on the device, installs and runs |
| 5 | Is it fast enough? | Note: clean and incremental build times for hello-egui on a mid-range tablet |

## Output
A short report in this folder: what worked, what didn't, numbers, and a go/no-go
recommendation for ADR 0007, including the fallback (build on a paired desktop).
