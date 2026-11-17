# Contributing

## Before your first pull request

Sign the [CLA](CLA.md). PeroxIDE is open source and also sold commercially
([ADR 0004](docs/adr/0004-licensing.md)). The CLA is what makes that possible. Your code
stays open source either way.

## Working on the code

```sh
cargo run -p peroxide
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same three checks on macOS. A pull request needs all three passing.

## Conventions

- **New user-visible action?** Register a command (`CommandSpec` + handler) and call it from the
  menu. Don't change state directly in UI code. Give it an honest `Access` level, and use
  `.ui_only()` only if an agent truly can't use it.
- **Command ids** are `area.verb_noun`: `file.save`, `designer.add_widget`, `build.run`.
- **New crate?** Add a short README that says what it does, why it exists and its
  license. Add it to the crate table in [ARCHITECTURE.md](ARCHITECTURE.md).
- **Crates that ship inside user apps** (`perox-ui`, `perox-forms`, component SDK) are
  `MIT OR Apache-2.0` and must not depend on any GPL crate in this repo.
- **A decision someone might later ask "why?" about** gets an ADR: copy
  [docs/adr/0000-template.md](docs/adr/0000-template.md), keep it to one page.
- **Docs**: short sentences, task first, tables for settings and troubleshooting.
- `unsafe` is denied workspace-wide. If you need it (FFI, Android), allow it on the
  smallest scope you can, with a `// SAFETY:` comment.
