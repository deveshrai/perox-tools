# perox-slint

The `.slint` files PeroxIDE's designer edits
([ADR 0011](../../docs/adr/0011-slint-for-user-apps.md)). License: MIT OR Apache-2.0 (it
runs in your `build.rs`).

| Module | What |
|---|---|
| `syntax` | A document's imports, components and element trees, with exact byte spans, read with Slint's own lossless parser |
| `edit` | Designer operations (set a property, add/move/remove an element, wire a callback, import a widget) as minimal text edits, so comments and hand formatting survive |
| `codegen` | For each exported component: a `<Name>Handlers` trait (one method per `callback`) and `connect`; `exec` for dialogs. `build::compile` (feature `build`) runs `slint-build` too |

## In an app

```toml
[dependencies]
perox-ui = "…"
slint = "=1.19.0"
[build-dependencies]
perox-slint = { version = "…", features = ["build"] }
```

```rust
// build.rs: compiles ui/ (its slint-project.json names the main file)
fn main() { perox_slint::build::compile("ui"); }
```

```slint
// ui/main_window.slint
export component MainWindow inherits Window {
    callback save_clicked();
    Button { text: "Save"; clicked => { root.save_clicked(); } }
}
```

```rust
// src/main.rs
perox_ui::include_ui!();

struct App;
impl MainWindowHandlers for App {
    fn save_clicked(&mut self, ui: &MainWindow) { /* … */ }
}
```

Callback parameters map to Rust types (`string` → `SharedString`, `int` → `i32`,
`float`/`length` → `f32`, `bool`, `color`, `brush`, `image`). Callbacks with other types are
left for you to wire with `on_<name>`.
