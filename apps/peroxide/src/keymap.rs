//! Default keyboard shortcuts. `Modifiers::COMMAND` is ⌘ on macOS and Ctrl elsewhere.
//! User-defined overrides come later, stored in settings.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

#[rustfmt::skip]
pub const BINDINGS: &[(KeyboardShortcut, &str)] = &[
    // Most specific first: egui ignores extra Shift/Alt when matching (⇧⌘P would also match ⌘P).
    (KeyboardShortcut::new(CMD_SHIFT, Key::P), "view.command_palette"),
    (KeyboardShortcut::new(CMD_SHIFT, Key::T), "build.test"),
    (KeyboardShortcut::new(CMD_SHIFT, Key::K), "build.check"),
    (KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::G), "view.source_control"),
    (KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::S), "file.save_all"),
    (KeyboardShortcut::new(Modifiers::SHIFT.plus(Modifiers::ALT), Key::F), "editor.format"),
    (KeyboardShortcut::new(Modifiers::SHIFT, Key::F5), "build.stop"),
    (KeyboardShortcut::new(Modifiers::SHIFT, Key::F12), "editor.references"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::P), "view.quick_open"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma), "app.settings"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::Q), "app.quit"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::S), "file.save"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::O), "project.open_dialog"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::W), "file.close"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::B), "build.build"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::R), "build.run"),
    (KeyboardShortcut::new(Modifiers::COMMAND, Key::Period), "editor.code_actions"),
    (KeyboardShortcut::new(Modifiers::CTRL, Key::G), "editor.goto_line_prompt"),
    (KeyboardShortcut::new(Modifiers::CTRL, Key::Backtick), "terminal.show"),
    (KeyboardShortcut::new(Modifiers::NONE, Key::F5), "build.run"),
    (KeyboardShortcut::new(Modifiers::NONE, Key::F2), "editor.rename_prompt"),
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
    use eframe::egui::{self, Event, Key, Modifiers, RawInput};

    fn press(key: Key, modifiers: Modifiers) -> Vec<&'static str> {
        let ctx = egui::Context::default();
        let input = RawInput {
            events: vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let mut got = Vec::new();
        let mut out = ctx.run_ui(input, |ui| got = super::pressed(ui.ctx()));
        out.textures_delta.clear();
        got
    }

    #[test]
    fn command_p_and_command_shift_p_are_different() {
        let cmd = Modifiers {
            mac_cmd: cfg!(target_os = "macos"),
            ctrl: !cfg!(target_os = "macos"),
            command: true,
            ..Default::default()
        };
        assert_eq!(press(Key::P, cmd), ["view.quick_open"]);
        assert_eq!(
            press(Key::P, cmd.plus(Modifiers::SHIFT)),
            ["view.command_palette"]
        );
    }

    #[test]
    fn each_shortcut_fires_exactly_its_command() {
        let cmd = Modifiers {
            mac_cmd: cfg!(target_os = "macos"),
            ctrl: !cfg!(target_os = "macos"),
            command: true,
            ..Default::default()
        };
        for (key, mods, want) in [
            (Key::S, cmd, "file.save"),
            (Key::S, cmd.plus(Modifiers::ALT), "file.save_all"),
            (Key::B, cmd, "build.build"),
            (Key::R, cmd, "build.run"),
            (Key::F5, Modifiers::NONE, "build.run"),
            (Key::F5, Modifiers::SHIFT, "build.stop"),
            (Key::F2, Modifiers::NONE, "editor.rename_prompt"),
            (Key::F12, Modifiers::SHIFT, "editor.references"),
            (Key::Period, cmd, "editor.code_actions"),
            (Key::T, cmd.plus(Modifiers::SHIFT), "build.test"),
        ] {
            assert_eq!(press(key, mods), [want], "{key:?} {mods:?}");
        }
    }

    #[test]
    fn no_shortcut_is_bound_twice() {
        let mut keys: Vec<_> = super::BINDINGS
            .iter()
            .map(|(s, _)| format!("{s:?}"))
            .collect();
        keys.sort_unstable();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }
}
