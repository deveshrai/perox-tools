# Toolchain Set

Everything a PeroxIDE release bundles: the Rust toolchain, tools, and the **basics**:
crates that every new project can use offline, with versions that are known to work
together on macOS, Linux and Windows.

| File | What |
|---|---|
| [set.toml](set.toml) | Toolchain, tools, targets, OS floors, prerequisites |
| [check/Cargo.toml](check/Cargo.toml) | **The basics**, with exact (`=`) versions and features |
| [check/src/main.rs](check/src/main.rs) | Uses every basic crate; CI runs it on every OS |

Why the set exists and what goes in it: [ADR 0005](../docs/adr/0005-toolchain-sets.md),
[ADR 0009](../docs/adr/0009-toolchain-set-2026-11.md).

## Run the check

```sh
cargo run -p perox-set-check     # last line: "OK: Toolchain Set check passed"
```

## Change the set

Only as a whole, for a release:
1. Edit versions in `check/Cargo.toml` (and `set.toml` for tools).
2. `cargo tree -p perox-set-check -d`: no second copy of egui, rustls providers, or anything
   you just changed.
3. The check passes on all three OSes in CI.
4. Record what changed and why in a new ADR that supersedes the current set ADR.
