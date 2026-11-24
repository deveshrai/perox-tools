//! The runtime PeroxIDE apps use on top of [Slint](https://slint.dev) (ADR 0011).
//!
//! - [`include_ui!`]: the Slint components compiled by `build.rs`, plus a
//!   `<Name>Handlers` trait and `connect` for each (see `perox_slint::codegen`).
//! - [`msgbox`]: native message boxes (information, warning, error, OK/Cancel, Yes/No).
//! - [`dialog`]: show a `.slint` `Dialog` over a window and get the button that closed it.
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

pub mod dialog;
pub mod msgbox;

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
