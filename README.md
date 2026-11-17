# PeroxIDE

**A complete RAD IDE for building GUI apps in Rust.** A drag-and-drop form
designer with real layouts, a full code editor, a debugger and project
management, all in one install with a pinned, tested Rust toolchain.
It also has a built-in MCP server, so AI agents can drive the IDE.

Targets: **milestone 1 is desktop: macOS, Linux and Windows.** Android comes later.
At that point the IDE itself will run on Android tablets and build installable APKs on
the device.

> Status: **Phase 0 — foundation.** There's a window, menus and the command bus.
> See [docs/roadmap.md](docs/roadmap.md).

## Build and run

```sh
cargo run -p peroxide        # the IDE
cargo test --workspace       # tests
```

The toolchain is pinned in `rust-toolchain.toml`. Each platform also needs:

| OS | Install |
|---|---|
| macOS | Xcode Command Line Tools: `xcode-select --install` |
| Linux (Debian/Ubuntu) | `sudo apt install build-essential libxkbcommon-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libwayland-dev` |
| Windows | Visual Studio Build Tools with "Desktop development with C++" |

## Find your way around

| Read | For |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | How the pieces fit together; one page. |
| [docs/adr/](docs/adr/) | Why things are the way they are: one page per decision. |
| [docs/roadmap.md](docs/roadmap.md) | What's being built, in which order, and what "done" means for each phase. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to work on the code, plus the CLA. |

## Repository layout

```
apps/peroxide/      the IDE executable (GPL-3.0-or-later)
crates/perox-core/  the command bus (GPL-3.0-or-later)
docs/               ADRs, roadmap, spikes
```

## License

- **The IDE** (everything not listed below): [GPL-3.0-or-later](LICENSE).
- **Everything that ends up inside *your* apps**: the `perox-ui` runtime, the `.pxui`
  format and the component SDK are **MIT OR Apache-2.0**
  ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)). Apps you build
  with PeroxIDE are yours, under any license you like.

The reasoning is in [ADR 0004](docs/adr/0004-licensing.md). Contributions need the
[CLA](CLA.md).
