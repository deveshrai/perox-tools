//! The runtime PeroxIDE apps use on top of [Slint](https://slint.dev) (ADR 0011).
//!
//! - [`include_ui!`]: the Slint components compiled by `build.rs`, plus a
//!   `<Name>Handlers` trait and `connect` for each (see `perox_slint::codegen`).
//! - [`msgbox`]: native message boxes (information, warning, error, OK/Cancel, Yes/No).
//! - [`files`]: the system's Open and Save dialogs.
//! - [`dialog`]: show a `.slint` `Dialog` over a window and get the button that closed it.
//! - [`canvas`]: drawing in Rust (tiny-skia) for the designer's Canvas.
//! - [`snapshot`]: a picture of the running window (`PEROX_SNAPSHOT=<file.png>`).
//! - [`data`]: tables as list and table models; SQLite queries (feature `sqlite`).
//!
//! ```ignore
//! perox_ui::include_ui!();
//!
//! struct App;
//! impl MainWindowHandlers for App {
//!     fn save_clicked(&mut self, ui: &MainWindow) {
//!         if perox_ui::msgbox::confirm(ui, "Save", "Overwrite the file?") {
//!             // …
//!         }
//!     }
//! }
//!
//! fn main() -> Result<(), slint::PlatformError> {
//!     let ui = MainWindow::new()?;
//!     ui.connect(App);
//!     ui.run()
//! }
//! ```

pub mod canvas;
pub mod data;
pub mod dialog;
pub mod files;
pub mod msgbox;
pub mod snapshot;

pub use dialog::DialogResult;
pub use slint;

/// Brings in the UI compiled by `perox_slint::build::compile` in `build.rs`: every
/// exported component (`MainWindow`, …) and its `MainWindowHandlers` trait.
#[macro_export]
macro_rules! include_ui {
    () => {
        /// Generated from the `.slint` files; don't edit.
        #[allow(missing_debug_implementations, clippy::all)]
        mod generated_ui {
            slint::include_modules!();
            include!(concat!(env!("OUT_DIR"), "/perox_handlers.rs"));
        }
        pub use generated_ui::*;
    };
}
