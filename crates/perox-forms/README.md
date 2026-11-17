# perox-forms

The `.pxui` form language ([ADR 0003](../../docs/adr/0003-form-format.md)).
License: MIT OR Apache-2.0 (it runs in user builds).

```text
// main_window.pxui
Window main_window {
    title: "Word Counter"
    size: 640 x 480

    VBox {
        spacing: 8
        Button count {
            text: "Count"
            on clicked: count_clicked
        }
        Label result { text: "" }
    }
}
```

| Module | What |
|---|---|
| `syntax` | Tokens and the parser: elements, properties, handlers, each with exact byte spans |
| `edit` | Designer operations (set a property, add/move/remove a widget) as minimal text edits, so comments and hand formatting survive |
| `codegen` | Rust code for a form: a struct with typed fields per named widget, and a handlers trait |

## Using a form in an app

```toml
[dependencies]
perox-ui = "…"
[build-dependencies]
perox-forms = "…"
```

```rust
// build.rs
fn main() { perox_forms::codegen::build::compile_dir("src/ui"); }

// main.rs
mod main_window { perox_ui::include_form!("main_window"); }
use main_window::{MainWindow, MainWindowHandlers};

struct App;
impl MainWindowHandlers for App {
    fn count_clicked(&mut self, ui: &mut MainWindow<'_>) {
        ui.result().set_text("clicked");
    }
}
fn main() -> perox_ui::eframe::Result { MainWindow::run(App) }
```

Each named widget gets an accessor (`ui.result()`), and each handler named in the form
gets a trait method with an empty default. A complete app is in
[examples/word-counter](../../examples/word-counter).
