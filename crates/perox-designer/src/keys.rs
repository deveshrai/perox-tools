//! Keyboard shortcuts as Slint writes them: `@keys(Control + Shift + O)`.
//!
//! Slint's modifier names are cross-platform: `Control` is ⌘ on macOS and Ctrl
//! elsewhere, `Meta` is ⌃ on macOS and the Windows key elsewhere.

use eframe::egui::{Key, Modifiers};

/// The text inside `@keys(…)` for a key press, or `None` for a key a shortcut can't use
/// (a lone modifier).
pub fn from_press(key: Key, m: Modifiers) -> Option<String> {
    let name = key_name(key)?;
    let mut parts: Vec<String> = Vec::new();
    if m.command {
        parts.push("Control".into());
    }
    // On macOS ⌃ is separate from ⌘; elsewhere `ctrl` is `command`.
    if m.ctrl && !m.command {
        parts.push("Meta".into());
    }
    if m.alt {
        parts.push("Alt".into());
    }
    if m.shift {
        parts.push("Shift".into());
    }
    parts.push(name);
    Some(parts.join(" + "))
}

fn key_name(key: Key) -> Option<String> {
    use Key::*;
    let named = match key {
        Enter => "Return",
        Escape => "Escape",
        Tab => "Tab",
        Backspace => "Backspace",
        Delete => "Delete",
        Insert => "Insert",
        Space => "Space",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        ArrowUp => "UpArrow",
        ArrowDown => "DownArrow",
        ArrowLeft => "LeftArrow",
        ArrowRight => "RightArrow",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.into());
    }
    let s = key.name();
    if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()) {
        return Some(s.to_ascii_uppercase());
    }
    if let Some(n) = s.strip_prefix('F').filter(|n| n.parse::<u8>().is_ok()) {
        return Some(format!("F{n}"));
    }
    // Digits and punctuation: as the character, quoted.
    let ch = match key {
        Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9 => {
            s.trim_start_matches("Num").to_owned()
        }
        _ => key.symbol_or_name().to_owned(),
    };
    (ch.chars().count() == 1).then(|| format!("\"{}\"", ch.replace('"', "\\\"")))
}

/// `@keys(Control + O)` → `Control + O`; anything else → `None`.
pub fn inner(expr: &str) -> Option<&str> {
    expr.trim()
        .strip_prefix("@keys(")?
        .strip_suffix(')')
        .map(str::trim)
}

/// `Control + O` → `@keys(Control + O)`.
pub fn expr(inner: &str) -> String {
    format!("@keys({})", inner.trim())
}

/// How a shortcut reads to a person on this OS: `⌘⇧O` on macOS, `Ctrl+Shift+O` elsewhere.
pub fn display(inner: &str, mac: bool) -> String {
    let parts: Vec<&str> = inner
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let key = |p: &str| p.trim_matches('"').to_owned();
    if mac {
        parts
            .iter()
            .map(|p| match *p {
                "Control" => "⌘".to_owned(),
                "Meta" => "⌃".to_owned(),
                "Alt" => "⌥".to_owned(),
                "Shift" => "⇧".to_owned(),
                k => key(k),
            })
            .collect()
    } else {
        parts
            .iter()
            .map(|p| match *p {
                "Control" => "Ctrl".to_owned(),
                "Meta" => "Win".to_owned(),
                k => key(k),
            })
            .collect::<Vec<_>>()
            .join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_become_slint_shortcuts() {
        let cmd = Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        };
        assert_eq!(from_press(Key::O, cmd).unwrap(), "Control + O");
        let cmd_shift = Modifiers { shift: true, ..cmd };
        assert_eq!(
            from_press(Key::S, cmd_shift).unwrap(),
            "Control + Shift + S"
        );
        // ⌃ on a Mac is Meta.
        let mac_ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(from_press(Key::Q, mac_ctrl).unwrap(), "Meta + Q");
        // Ctrl elsewhere sets both: Control.
        let win_ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        assert_eq!(from_press(Key::Q, win_ctrl).unwrap(), "Control + Q");
        assert_eq!(from_press(Key::F5, Modifiers::NONE).unwrap(), "F5");
        assert_eq!(from_press(Key::Enter, Modifiers::NONE).unwrap(), "Return");
        assert_eq!(from_press(Key::Num1, cmd).unwrap(), "Control + \"1\"");
        assert_eq!(from_press(Key::Plus, cmd).unwrap(), "Control + \"+\"");
    }

    #[test]
    fn round_trips_and_display() {
        assert_eq!(inner("@keys(Control + O)"), Some("Control + O"));
        assert_eq!(inner("\"x\""), None);
        assert_eq!(expr(" Control + O "), "@keys(Control + O)");
        assert_eq!(display("Control + Shift + O", true), "⌘⇧O");
        assert_eq!(display("Control + Shift + O", false), "Ctrl+Shift+O");
        assert_eq!(display("Control + \"1\"", false), "Ctrl+1");
    }
}
