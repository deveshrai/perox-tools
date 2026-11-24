//! Native message boxes: the operating system's own alert, truly modal, on every
//! desktop platform (via [`rfd`]). They block until answered, so call them from a
//! handler and use the answer on the next line.
//!
//! ```ignore
//! perox_ui::msgbox::info(&ui, "Saved", "Your changes were saved.");
//! if perox_ui::msgbox::confirm(&ui, "Delete", "Delete this file?") { /* … */ }
//! match perox_ui::msgbox::ask_cancel(&ui, "Quit", "Save changes first?") {
//!     Some(true) => { /* save, then quit */ }
//!     Some(false) => { /* quit */ }
//!     None => { /* stay */ }
//! }
//! ```

use slint::ComponentHandle;

/// How serious the message is; sets the icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Info,
    Warning,
    Error,
}

/// Which buttons the box has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Buttons {
    #[default]
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    /// Your own labels (one to three), e.g. `["Save", "Don't Save", "Cancel"]`.
    Custom(Vec<String>),
}

/// The button that closed the box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Ok,
    Cancel,
    Yes,
    No,
    /// The label of a [`Buttons::Custom`] button.
    Custom(String),
}

/// A message box, for when the one-line functions aren't enough.
#[derive(Debug, Clone, Default)]
pub struct MessageBox {
    pub title: String,
    pub text: String,
    pub level: Level,
    pub buttons: Buttons,
}

impl MessageBox {
    pub fn new(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    pub fn buttons(mut self, buttons: Buttons) -> Self {
        self.buttons = buttons;
        self
    }

    fn dialog(&self) -> rfd::MessageDialog {
        let level = match self.level {
            Level::Info => rfd::MessageLevel::Info,
            Level::Warning => rfd::MessageLevel::Warning,
            Level::Error => rfd::MessageLevel::Error,
        };
        let buttons = match &self.buttons {
            Buttons::Ok => rfd::MessageButtons::Ok,
            Buttons::OkCancel => rfd::MessageButtons::OkCancel,
            Buttons::YesNo => rfd::MessageButtons::YesNo,
            Buttons::YesNoCancel => rfd::MessageButtons::YesNoCancel,
            Buttons::Custom(labels) => match labels.as_slice() {
                [a] => rfd::MessageButtons::OkCustom(a.clone()),
                [a, b] => rfd::MessageButtons::OkCancelCustom(a.clone(), b.clone()),
                [a, b, c, ..] => {
                    rfd::MessageButtons::YesNoCancelCustom(a.clone(), b.clone(), c.clone())
                }
                [] => rfd::MessageButtons::Ok,
            },
        };
        rfd::MessageDialog::new()
            .set_title(&self.title)
            .set_description(&self.text)
            .set_level(level)
            .set_buttons(buttons)
    }

    /// Shows the box over `parent` (or on its own) and waits for an answer.
    pub fn show(&self, parent: Option<&slint::Window>) -> Answer {
        let mut dialog = self.dialog();
        if let Some(w) = parent {
            dialog = dialog.set_parent(&w.window_handle());
        }
        answer(dialog.show(), &self.buttons)
    }
}

fn answer(r: rfd::MessageDialogResult, buttons: &Buttons) -> Answer {
    match r {
        rfd::MessageDialogResult::Ok => match buttons {
            Buttons::Custom(l) if !l.is_empty() => Answer::Custom(l[0].clone()),
            _ => Answer::Ok,
        },
        rfd::MessageDialogResult::Cancel => match buttons {
            Buttons::Custom(l) if l.len() >= 2 => Answer::Custom(l[l.len() - 1].clone()),
            _ => Answer::Cancel,
        },
        rfd::MessageDialogResult::Yes => Answer::Yes,
        rfd::MessageDialogResult::No => Answer::No,
        rfd::MessageDialogResult::Custom(label) => Answer::Custom(label),
    }
}

fn show(parent: &impl ComponentHandle, b: MessageBox) -> Answer {
    b.show(Some(parent.window()))
}

/// An information message with an OK button.
pub fn info(parent: &impl ComponentHandle, title: &str, text: &str) {
    show(parent, MessageBox::new(title, text));
}

/// A warning with an OK button.
pub fn warning(parent: &impl ComponentHandle, title: &str, text: &str) {
    show(parent, MessageBox::new(title, text).level(Level::Warning));
}

/// An error with an OK button.
pub fn error(parent: &impl ComponentHandle, title: &str, text: &str) {
    show(parent, MessageBox::new(title, text).level(Level::Error));
}

/// OK / Cancel: true for OK.
pub fn confirm(parent: &impl ComponentHandle, title: &str, text: &str) -> bool {
    show(
        parent,
        MessageBox::new(title, text)
            .level(Level::Warning)
            .buttons(Buttons::OkCancel),
    ) == Answer::Ok
}

/// Yes / No: true for Yes.
pub fn ask(parent: &impl ComponentHandle, title: &str, text: &str) -> bool {
    show(parent, MessageBox::new(title, text).buttons(Buttons::YesNo)) == Answer::Yes
}

/// Yes / No / Cancel: `Some(true)` for Yes, `Some(false)` for No, `None` for Cancel.
pub fn ask_cancel(parent: &impl ComponentHandle, title: &str, text: &str) -> Option<bool> {
    match show(
        parent,
        MessageBox::new(title, text).buttons(Buttons::YesNoCancel),
    ) {
        Answer::Yes => Some(true),
        Answer::No => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_map_back_to_buttons() {
        let custom = Buttons::Custom(vec!["Save".into(), "Discard".into()]);
        assert_eq!(
            answer(rfd::MessageDialogResult::Ok, &custom),
            Answer::Custom("Save".into())
        );
        assert_eq!(
            answer(rfd::MessageDialogResult::Cancel, &custom),
            Answer::Custom("Discard".into())
        );
        assert_eq!(
            answer(rfd::MessageDialogResult::Ok, &Buttons::OkCancel),
            Answer::Ok
        );
        assert_eq!(
            answer(rfd::MessageDialogResult::Custom("Later".into()), &custom),
            Answer::Custom("Later".into())
        );
        let b = MessageBox::new("T", "x")
            .level(Level::Error)
            .buttons(Buttons::YesNo);
        assert_eq!((b.level, b.buttons), (Level::Error, Buttons::YesNo));
    }
}
