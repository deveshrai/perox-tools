//! The palette, three ways (like VS Code):
//! - ⌘P: type a file name to open it (fuzzy);
//! - `>` (⇧⌘P): commands;
//! - `:` (⌃G): go to a line in the current file.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Key, Modifiers};
use perox_core::{CommandBus, CommandSpec};
use perox_editor::completion::fuzzy_score;
use serde_json::{Value, json};

use crate::commands::AppState;
use crate::keymap;

#[derive(Debug, Default)]
pub struct Palette {
    query: String,
    selected: usize,
    was_open: bool,
}

/// What opening the palette starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Files,
    Commands,
    Line,
}

enum Row<'a> {
    Command(&'a CommandSpec),
    File(&'a Path),
}

impl Palette {
    pub fn open(&mut self, mode: Mode) {
        self.query = match mode {
            Mode::Files => String::new(),
            Mode::Commands => ">".into(),
            Mode::Line => ":".into(),
        };
        self.selected = 0;
        self.was_open = false;
    }

    /// Shows the palette while `*open`. Returns a command (and its arguments) to run.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        bus: &CommandBus<AppState>,
        files: &[PathBuf],
        line_count: Option<usize>,
        open: &mut bool,
    ) -> Option<(&'static str, Value)> {
        if !*open {
            self.was_open = false;
            return None;
        }
        let just_opened = !self.was_open;
        self.was_open = true;

        let rows: Vec<Row> = if let Some(q) = self.query.strip_prefix('>') {
            ranked_commands(
                bus.specs().filter(|s| {
                    !s.id.starts_with("view.command_palette") && s.id != "view.quick_open"
                }),
                q,
            )
            .into_iter()
            .map(Row::Command)
            .collect()
        } else if self.query.starts_with(':') {
            Vec::new()
        } else {
            ranked_files(files, &self.query)
                .into_iter()
                .take(200)
                .map(Row::File)
                .collect()
        };
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down {
            self.selected = (self.selected + 1).min(rows.len().saturating_sub(1));
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let choose = |row: &Row| -> (&'static str, Value) {
            match row {
                Row::Command(spec) => (spec.id, json!({})),
                Row::File(p) => ("file.open", json!({ "path": p })),
            }
        };
        let mut chosen = None;
        if enter {
            if let Some(l) = self.query.strip_prefix(':') {
                let mut parts = l.trim().split([':', ',']);
                if let Some(line) = parts.next().and_then(|n| n.trim().parse::<usize>().ok()) {
                    let col = parts
                        .next()
                        .and_then(|n| n.trim().parse::<usize>().ok())
                        .unwrap_or(1);
                    chosen = Some(("editor.goto_line", json!({ "line": line, "column": col })));
                }
            } else {
                chosen = rows.get(self.selected).map(choose);
            }
        }

        let response = egui::Window::new("command_palette")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, [0.0, 48.0])
            .default_width(560.0)
            .min_width(560.0)
            .show(ctx, |ui| {
                let hint = "Type a file name, > for commands, : for a line";
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                );
                if just_opened || !edit.has_focus() {
                    edit.request_focus();
                    if just_opened && let Some(mut state) = egui::TextEdit::load_state(ctx, edit.id)
                    {
                        let end = egui::text::CCursor::new(self.query.chars().count());
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(end)));
                        state.store(ctx, edit.id);
                    }
                }
                if edit.changed() {
                    self.selected = 0;
                }
                ui.separator();
                if self.query.starts_with(':') {
                    match line_count {
                        Some(n) => ui.weak(format!(
                            "Go to line (1–{n}). Type line, or line:column, and press Enter."
                        )),
                        None => ui.weak("Open a file first."),
                    };
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        if rows.is_empty() {
                            ui.weak(if self.query.starts_with('>') {
                                "No matching commands"
                            } else {
                                "No matching files"
                            });
                        }
                        for (i, row) in rows.iter().enumerate() {
                            let resp = match row {
                                Row::Command(spec) => {
                                    let shortcut = keymap::shortcut_for(spec.id)
                                        .map(|s| ctx.format_shortcut(&s))
                                        .unwrap_or_default();
                                    let icon = crate::icons::for_command(spec.id).image(ui);
                                    ui.add(
                                        egui::Button::selectable(
                                            i == self.selected,
                                            egui::Atoms::new((icon, spec.title)),
                                        )
                                        .shortcut_text(shortcut)
                                        .min_size(egui::vec2(ui.available_width(), 0.0)),
                                    )
                                    .on_hover_text(spec.id)
                                }
                                Row::File(p) => {
                                    let name = p.file_name().map_or_else(String::new, |n| {
                                        n.to_string_lossy().into_owned()
                                    });
                                    let dir = p
                                        .parent()
                                        .map(|d| d.display().to_string())
                                        .unwrap_or_default();
                                    let icon = crate::icons::for_file(p).image(ui);
                                    ui.add(
                                        egui::Button::selectable(
                                            i == self.selected,
                                            egui::Atoms::new((icon, name)),
                                        )
                                        .shortcut_text(dir)
                                        .min_size(egui::vec2(ui.available_width(), 0.0)),
                                    )
                                }
                            };
                            if i == self.selected && (up || down) {
                                resp.scroll_to_me(None);
                            }
                            if resp.clicked() {
                                chosen = Some(choose(row));
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

/// Commands matching `query`, best first.
fn ranked_commands<'a>(
    specs: impl Iterator<Item = &'a CommandSpec>,
    query: &str,
) -> Vec<&'a CommandSpec> {
    let q = query.trim();
    let mut scored: Vec<(i32, &CommandSpec)> = specs
        .filter_map(|s| {
            let by_title = fuzzy_score(q, s.title);
            let by_id = fuzzy_score(q, s.id).map(|x| x - 5);
            by_title.max(by_id).map(|sc| (sc, s))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(b.1.title)));
    scored.into_iter().map(|(_, s)| s).collect()
}

/// Files matching `query`: a match in the file name beats one spread over the path.
fn ranked_files<'a>(files: &'a [PathBuf], query: &str) -> Vec<&'a Path> {
    let q = query.trim();
    let mut scored: Vec<(i32, &Path)> = files
        .iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy();
            let full = p.to_string_lossy();
            let by_name = fuzzy_score(q, &name).map(|s| s + 30);
            let by_path = fuzzy_score(q, &full);
            by_name.max(by_path).map(|s| (s, p.as_path()))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.as_os_str().len().cmp(&b.1.as_os_str().len()))
    });
    scored.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use perox_core::Access;

    #[test]
    fn ranks_commands() {
        let specs = [
            CommandSpec::new("app.settings", "Settings…", Access::Read),
            CommandSpec::new("mcp.set_port", "MCP: Set Port", Access::Read),
            CommandSpec::new("app.quit", "Quit PeroxIDE", Access::Read),
        ];
        let ids = |q| {
            ranked_commands(specs.iter(), q)
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("set")[0], "app.settings");
        assert_eq!(ids("qpx"), ["app.quit"]);
        assert!(ids("zzz").is_empty());
    }

    #[test]
    fn file_name_matches_beat_path_matches() {
        let files = vec![
            PathBuf::from("src/main/window.rs"),
            PathBuf::from("src/ui/main.rs"),
            PathBuf::from("Cargo.toml"),
        ];
        let r = ranked_files(&files, "main");
        assert_eq!(r[0], Path::new("src/ui/main.rs"));
        assert_eq!(ranked_files(&files, "ct")[0], Path::new("Cargo.toml"));
    }
}
