//! The command palette (⇧⌘P / Ctrl+Shift+P): type to filter every command, Enter runs it.

use eframe::egui::{self, Align2, Key, Modifiers};
use perox_core::{CommandBus, CommandSpec};

use crate::commands::AppState;
use crate::keymap;

#[derive(Debug, Default)]
pub struct Palette {
    query: String,
    selected: usize,
    was_open: bool,
}

impl Palette {
    /// Shows the palette while `*open`. Returns the command to run, if one was chosen.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        bus: &CommandBus<AppState>,
        open: &mut bool,
    ) -> Option<&'static str> {
        if !*open {
            self.was_open = false;
            return None;
        }
        let just_opened = !self.was_open;
        if just_opened {
            self.query.clear();
            self.selected = 0;
        }
        self.was_open = true;

        let matches = ranked(
            bus.specs().filter(|s| s.id != "view.command_palette"),
            &self.query,
        );
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down {
            self.selected = (self.selected + 1).min(matches.len().saturating_sub(1));
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut chosen = enter
            .then(|| matches.get(self.selected).map(|s| s.id))
            .flatten();

        let response = egui::Window::new("command_palette")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, [0.0, 48.0])
            .fixed_size([520.0, 0.0])
            .show(ctx, |ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Type a command")
                        .desired_width(f32::INFINITY),
                );
                if just_opened || !edit.has_focus() {
                    edit.request_focus();
                }
                if edit.changed() {
                    self.selected = 0;
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        if matches.is_empty() {
                            ui.weak("No matching commands");
                        }
                        for (i, spec) in matches.iter().enumerate() {
                            let shortcut = keymap::shortcut_for(spec.id)
                                .map(|s| ctx.format_shortcut(&s))
                                .unwrap_or_default();
                            let row = ui.add(
                                egui::Button::selectable(i == self.selected, spec.title)
                                    .shortcut_text(shortcut)
                                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                            );
                            let row = row.on_hover_text(spec.id);
                            if i == self.selected && (up || down) {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() {
                                chosen = Some(spec.id);
                            }
                        }
                    });
            });

        let clicked_away = response.is_some_and(|r| r.response.clicked_elsewhere());
        if escape || chosen.is_some() || clicked_away {
            *open = false;
        }
        chosen
    }
}

/// Commands matching `query`, best first: title prefix, then substring, then subsequence.
fn ranked<'a>(specs: impl Iterator<Item = &'a CommandSpec>, query: &str) -> Vec<&'a CommandSpec> {
    let q = query.trim().to_lowercase();
    let mut scored: Vec<(u8, &CommandSpec)> = specs
        .filter_map(|s| score(&q, s).map(|sc| (sc, s)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(b.1.title)));
    scored.into_iter().map(|(_, s)| s).collect()
}

fn score(q: &str, spec: &CommandSpec) -> Option<u8> {
    if q.is_empty() {
        return Some(0);
    }
    let title = spec.title.to_lowercase();
    if title.starts_with(q) {
        return Some(3);
    }
    if title.contains(q) || spec.id.contains(q) {
        return Some(2);
    }
    let hay = format!("{title} {}", spec.id);
    let mut chars = hay.chars();
    q.chars()
        .filter(|c| !c.is_whitespace())
        .all(|c| chars.any(|h| h == c))
        .then_some(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use perox_core::Access;

    #[test]
    fn ranks_prefix_then_substring_then_subsequence() {
        let specs = [
            CommandSpec::new("app.settings", "Settings…", Access::Read),
            CommandSpec::new("mcp.set_port", "MCP: Set Port", Access::Read),
            CommandSpec::new("app.quit", "Quit PeroxIDE", Access::Read),
        ];
        let ids = |q| {
            ranked(specs.iter(), q)
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("set"), ["app.settings", "mcp.set_port"]);
        assert_eq!(ids("qpx"), ["app.quit"]);
        assert_eq!(ids("zzz"), Vec::<&str>::new());
        assert_eq!(ids("").len(), 3);
    }
}
