//! Dialogs designed in `.slint`: a component that `inherits Dialog`, with
//! `StandardButton { kind: ok; }` and friends. The generated code gives each one an
//! `exec` method:
//!
//! ```ignore
//! let about = AboutDialog::new()?;
//! about.exec(Some(ui.window()), |result| {
//!     if result == perox_ui::DialogResult::Ok { /* … */ }
//! })?;
//! ```
//!
//! `exec` shows the dialog centred over its parent and calls you back once, with the
//! button that closed it ([`DialogResult::Closed`] for the title bar's close button).
//! Slint 1.19 has no OS-level window modality, so the parent stays usable while the
//! dialog is open; for a blocking question use a [`crate::msgbox`].

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

/// The button that closed a dialog: one per Slint `StandardButtonKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialogResult {
    Ok,
    Cancel,
    Apply,
    Close,
    Reset,
    Help,
    Yes,
    No,
    Abort,
    Retry,
    Ignore,
    /// The window was closed from its title bar.
    Closed,
}

impl DialogResult {
    /// For a `StandardButton` `kind` (`ok`, `cancel`, …).
    pub fn from_kind(kind: &str) -> Option<Self> {
        Some(match kind {
            "ok" => Self::Ok,
            "cancel" => Self::Cancel,
            "apply" => Self::Apply,
            "close" => Self::Close,
            "reset" => Self::Reset,
            "help" => Self::Help,
            "yes" => Self::Yes,
            "no" => Self::No,
            "abort" => Self::Abort,
            "retry" => Self::Retry,
            "ignore" => Self::Ignore,
            _ => return None,
        })
    }

    /// Accepting results (OK, Yes, Apply): the dialog's changes should be kept.
    pub fn accepted(self) -> bool {
        matches!(self, Self::Ok | Self::Yes | Self::Apply)
    }
}

type Done = Box<dyn FnOnce(DialogResult)>;

/// Closes a dialog once with a result; used by the generated `exec`. Clones share the
/// same callback.
pub struct Finish<T: ComponentHandle + 'static> {
    dialog: slint::Weak<T>,
    done: Rc<RefCell<Option<Done>>>,
    /// Keeps the dialog alive while it's open, so the caller can drop its handle.
    keep: Rc<RefCell<Option<T>>>,
}

impl<T: ComponentHandle + 'static> Clone for Finish<T> {
    fn clone(&self) -> Self {
        Self {
            dialog: self.dialog.clone(),
            done: self.done.clone(),
            keep: self.keep.clone(),
        }
    }
}

impl<T: ComponentHandle + 'static> std::fmt::Debug for Finish<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Finish")
            .field("pending", &self.done.borrow().is_some())
            .finish()
    }
}

impl<T: ComponentHandle + 'static> Finish<T> {
    pub fn new(dialog: &T, done: impl FnOnce(DialogResult) + 'static) -> Self {
        Self {
            dialog: dialog.as_weak(),
            done: Rc::new(RefCell::new(Some(Box::new(done)))),
            keep: Rc::new(RefCell::new(Some(dialog.clone_strong()))),
        }
    }

    /// Hides the dialog and reports `result`, the first time only.
    pub fn finish(&self, result: DialogResult) {
        let Some(done) = self.done.borrow_mut().take() else {
            return;
        };
        if let Some(d) = self.dialog.upgrade() {
            let _ = d.hide();
        }
        let keep = self.keep.borrow_mut().take();
        done(result);
        drop(keep);
    }

    /// Routes the title bar's close button to [`DialogResult::Closed`].
    pub fn on_close_requested(&self, window: &slint::Window) {
        let f = self.clone();
        window.on_close_requested(move || {
            f.finish(DialogResult::Closed);
            slint::CloseRequestResponse::HideWindow
        });
    }
}

/// Shows `dialog`, centred over `parent` if there is one.
pub fn show_over(
    dialog: &slint::Window,
    parent: Option<&slint::Window>,
) -> Result<(), slint::PlatformError> {
    dialog.show()?;
    if let Some(p) = parent {
        let (pp, ps, ds) = (p.position(), p.size(), dialog.size());
        let x = pp.x + (ps.width as i32 - ds.width as i32) / 2;
        let y = pp.y + (ps.height as i32 - ds.height as i32) / 2;
        dialog.set_position(slint::PhysicalPosition::new(x, y.max(pp.y)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(DialogResult::from_kind("ok"), Some(DialogResult::Ok));
        assert_eq!(DialogResult::from_kind("retry"), Some(DialogResult::Retry));
        assert_eq!(DialogResult::from_kind("bogus"), None);
        assert!(DialogResult::Yes.accepted() && !DialogResult::Closed.accepted());
    }
}
