//! Open and Save dialogs: the system's own, over a window.
//!
//! ```ignore
//! let js = [("JavaScript", &["js", "mjs"][..])];
//! if let Some(path) = perox_ui::files::open(&ui, "Open", &js) { … }
//! if let Some(path) = perox_ui::files::save(&ui, "Save As", "untitled.js", &js) { … }
//! ```

use std::path::PathBuf;

use slint::ComponentHandle;

/// File types to offer: a name and its extensions, without dots.
pub type Filters<'a> = [(&'a str, &'a [&'a str])];

fn dialog(parent: &impl ComponentHandle, title: &str, filters: &Filters) -> rfd::FileDialog {
    let mut d = rfd::FileDialog::new().set_title(title);
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    d.set_parent(&parent.window().window_handle())
}

/// Asks for a file to open; `None` if cancelled.
pub fn open(parent: &impl ComponentHandle, title: &str, filters: &Filters) -> Option<PathBuf> {
    dialog(parent, title, filters).pick_file()
}

/// Asks where to save, suggesting `name`; `None` if cancelled.
pub fn save(
    parent: &impl ComponentHandle,
    title: &str,
    name: &str,
    filters: &Filters,
) -> Option<PathBuf> {
    dialog(parent, title, filters)
        .set_file_name(name)
        .save_file()
}
