//! Default keyboard shortcuts. `Modifiers::COMMAND` is ⌘ on macOS and Ctrl elsewhere.
//! User-defined overrides come later, stored in settings.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

pub const BINDINGS: &[(KeyboardShortcut, &str)] = &[
    (
        KeyboardShortcut::new(CMD_SHIFT, Key::P),
        "view.command_palette",
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma),
        "app.settings",
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Q),
        "app.quit",
    ),
];

pub fn shortcut_for(id: &str) -> Option<KeyboardShortcut> {
    BINDINGS.iter().find(|(_, c)| *c == id).map(|(s, _)| *s)
}

/// Commands whose shortcuts were pressed this frame (consumed, so widgets don't also see them).
pub fn pressed(ctx: &egui::Context) -> Vec<&'static str> {
    ctx.input_mut(|i| {
        BINDINGS
            .iter()
            .filter(|(s, _)| i.consume_shortcut(s))
            .map(|(_, id)| *id)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_binding_is_unique() {
        let mut ids: Vec<_> = super::BINDINGS.iter().map(|(_, id)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), super::BINDINGS.len());
    }
}
