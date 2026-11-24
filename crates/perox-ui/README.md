# perox-ui

The runtime PeroxIDE apps use on top of [Slint](https://slint.dev)
([ADR 0011](../../docs/adr/0011-slint-for-user-apps.md)).
License: MIT OR Apache-2.0 (it ships inside your app). Slint itself has its own licenses.

| Item | What |
|---|---|
| `include_ui!()` | The components `build.rs` compiled, plus `<Name>Handlers` and `connect` for each |
| `msgbox` | Native message boxes: `info`, `warning`, `error`, `confirm` (OK/Cancel), `ask` (Yes/No), `ask_cancel`, or a `MessageBox` builder with your own buttons |
| `dialog` | `exec` for components that `inherits Dialog`: shown over a parent, calls you back with the `StandardButton` that closed it |

```rust
perox_ui::include_ui!();

use slint::ComponentHandle;

struct App;

impl MainWindowHandlers for App {
    fn delete_clicked(&mut self, ui: &MainWindow) {
        if perox_ui::msgbox::confirm(ui, "Delete", "Delete this item?") {
            // …
        }
    }

    fn about_clicked(&mut self, ui: &MainWindow) {
        if let Ok(about) = AboutDialog::new() {
            let _ = about.exec(Some(ui.window()), |result| println!("{result:?}"));
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.connect(App);
    ui.run()
}
```

Message boxes are truly modal (the OS's own alert). Slint 1.19 has no OS-level window
modality yet, so a `Dialog` leaves its parent usable; use a message box for a blocking
question. A complete app is in [examples/word-counter](../../examples/word-counter).
