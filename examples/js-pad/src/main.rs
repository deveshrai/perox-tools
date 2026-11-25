//! JS Pad: a small JavaScript editor in the spirit of Notepad, with highlighting, a
//! console, and Run. The window is `ui/main_window.slint`; this is its code-behind.
//!
//! The editor is a `TextInput` whose own text is transparent, over the same text drawn
//! as coloured runs (`js`): the input does the typing, selecting and undo; the runs only
//! show it. Only the rows in view are drawn.

mod engine;
mod examples;
mod js;

perox_ui::include_ui!();

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use engine::{Engine, Level, Line};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

const JS: [(&str, &[&str]); 1] = [("JavaScript", &["js", "mjs", "cjs"])];
/// The gap between the text area's top and the first row, as in the window.
const TOP: f32 = 8.0;

#[derive(Default)]
struct App {
    path: Option<PathBuf>,
    /// The text as last saved or opened, to know whether it changed.
    saved: String,
    tokens: Vec<js::Token>,
    /// What the tokens were made from.
    tokenized: String,
    caret_row: usize,
    engine: Engine,
    console: Vec<Line>,
}

fn color(c: [u8; 3]) -> Color {
    Color::from_rgb_u8(c[0], c[1], c[2])
}

/// Row and column (both from 0) of byte `offset` in `text`.
pub fn row_col(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let row = before.matches('\n').count();
    let col = before.rsplit('\n').next().unwrap_or("").chars().count();
    (row, col)
}

/// What Return inserts at `offset`: a newline and the line's indentation, one level more
/// after an opening bracket.
pub fn newline_text(text: &str, offset: usize) -> String {
    let before = &text[..offset.min(text.len())];
    let line = before.rsplit('\n').next().unwrap_or("");
    let mut indent: String = line.chars().take_while(|c| *c == ' ').collect();
    if line.trim_end().ends_with(['{', '(', '[']) {
        indent.push_str("  ");
    }
    format!("\n{indent}")
}

/// Typing `}` on a line that's only indentation: the line, one level less, and the `}`.
/// `None` where `}` goes in as usual. Returns (where the line starts, its new text).
pub fn dedent_brace(text: &str, offset: usize) -> Option<(usize, String)> {
    let before = &text[..offset.min(text.len())];
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[start..];
    (line.len() >= 2 && line.chars().all(|c| c == ' '))
        .then(|| (start, format!("{}}}", &line[2..])))
}

impl App {
    fn file_name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(
            || "untitled.js".into(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// The runs and line numbers for the rows in view.
    fn redraw(&mut self, ui: &MainWindow) {
        let code = ui.get_code().to_string();
        if code != self.tokenized {
            self.tokens = js::tokens(&code);
            self.tokenized = code.clone();
            let lines = code.split('\n').count();
            ui.set_line_count(lines as i32);
            let widest = code
                .split('\n')
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0);
            ui.set_content_width(68.0 + widest as f32 * ui.get_char_w() + 120.0);
        }
        let line_h = ui.get_line_h().max(1.0);
        let first = ((ui.get_scroll_y() - TOP) / line_h).floor().max(0.0) as usize;
        let last = first + (ui.get_view_h() / line_h).ceil() as usize + 2;
        let runs: Vec<Run> = js::visible(&self.tokens, first, last)
            .into_iter()
            .map(|t| Run {
                row: t.row as i32,
                col: t.col as i32,
                text: t.text.into(),
                color: color(t.kind.color()),
            })
            .collect();
        ui.set_runs(ModelRc::new(VecModel::from(runs)));
        let count = ui.get_line_count().max(1) as usize;
        let numbers: Vec<Run> = (first..last.min(count))
            .map(|r| Run {
                row: r as i32,
                col: 0,
                text: (r + 1).to_string().into(),
                color: if r == self.caret_row {
                    Color::from_rgb_u8(0xc8, 0xcc, 0xd4)
                } else {
                    Color::from_rgb_u8(0x4b, 0x52, 0x63)
                },
            })
            .collect();
        ui.set_line_numbers(ModelRc::new(VecModel::from(numbers)));
    }

    fn show_console(&self, ui: &MainWindow) {
        let lines: Vec<ConsoleLine> = self
            .console
            .iter()
            .map(|l| ConsoleLine {
                text: l.text.as_str().into(),
                color: color(l.level.color()),
            })
            .collect();
        ui.set_console(ModelRc::new(VecModel::from(lines)));
    }

    fn note(&mut self, text: impl Into<String>) {
        self.console.push(Line {
            level: Level::Note,
            text: text.into(),
        });
    }

    fn load(&mut self, ui: &MainWindow, text: String, path: Option<PathBuf>) {
        self.saved = text.clone();
        self.path = path;
        ui.set_code(text.into());
        ui.set_file_name(self.file_name().into());
        ui.set_modified(false);
        self.caret_row = 0;
        self.redraw(ui);
    }

    /// Unsaved changes: asks before they're thrown away.
    fn can_discard(&self, ui: &MainWindow) -> bool {
        ui.get_code().as_str() == self.saved
            || perox_ui::msgbox::confirm(
                ui,
                "Discard changes?",
                &format!("{} has changes that aren't saved.", self.file_name()),
            )
    }

    fn write(&mut self, ui: &MainWindow, path: PathBuf) {
        let code = ui.get_code().to_string();
        match std::fs::write(&path, &code) {
            Ok(()) => {
                self.saved = code;
                self.path = Some(path);
                ui.set_file_name(self.file_name().into());
                ui.set_modified(false);
                ui.set_status("Saved".into());
            }
            Err(e) => perox_ui::msgbox::error(ui, "Couldn't save", &e.to_string()),
        }
    }

    /// Puts `insert` at byte `offset` of the text, and the caret after it.
    fn insert(&mut self, ui: &MainWindow, offset: i32, insert: &str) {
        let mut code = ui.get_code().to_string();
        let mut at = (offset.max(0) as usize).min(code.len());
        while !code.is_char_boundary(at) {
            at -= 1;
        }
        code.insert_str(at, insert);
        ui.set_code(code.into());
        ui.set_caret_after((at + insert.len()) as i32);
        self.code_edited(ui);
    }
}

impl MainWindowHandlers for App {
    fn code_edited(&mut self, ui: &MainWindow) {
        ui.set_modified(ui.get_code().as_str() != self.saved);
        self.redraw(ui);
    }

    fn caret_moved(&mut self, ui: &MainWindow, offset: i32) {
        let (row, col) = row_col(&ui.get_code(), offset.max(0) as usize);
        ui.set_position(format!("Ln {}, Col {}", row + 1, col + 1).into());
        if row != self.caret_row {
            self.caret_row = row;
            self.redraw(ui);
        }
    }

    fn indent(&mut self, ui: &MainWindow, offset: i32) {
        self.insert(ui, offset, "  ");
    }

    fn newline(&mut self, ui: &MainWindow, offset: i32) {
        let text = newline_text(&ui.get_code(), offset.max(0) as usize);
        self.insert(ui, offset, &text);
    }

    fn close_brace(&mut self, ui: &MainWindow, offset: i32) {
        let code = ui.get_code().to_string();
        let offset = (offset.max(0) as usize).min(code.len());
        match dedent_brace(&code, offset) {
            Some((start, line)) => {
                let mut code = code;
                code.replace_range(start..offset, &line);
                ui.set_caret_after((start + line.len()) as i32);
                ui.set_code(code.into());
                ui.set_handled(true);
                self.code_edited(ui);
            }
            None => ui.set_handled(false),
        }
    }

    fn scrolled(&mut self, ui: &MainWindow) {
        self.redraw(ui);
    }

    fn run_code(&mut self, ui: &MainWindow) {
        let code = ui.get_code().to_string();
        self.engine = Engine::new();
        self.note(format!("▶ {}", ui.get_file_name()));
        let started = Instant::now();
        let out = self.engine.run(&code);
        // console.error is output like any other; only an uncaught error stops the run.
        let failed = out.iter().any(|l| l.text.starts_with("Uncaught"));
        self.console.extend(out);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        self.note(format!(
            "{} in {ms:.1} ms",
            if failed { "Stopped" } else { "Done" }
        ));
        ui.set_status(
            if failed {
                "Error: see the console"
            } else {
                "Ran"
            }
            .into(),
        );
        ui.set_console_open(true);
        self.show_console(ui);
    }

    fn repl(&mut self, ui: &MainWindow, text: SharedString) {
        if text.trim().is_empty() {
            return;
        }
        let out = self.engine.eval(&text);
        self.console.extend(out);
        self.show_console(ui);
    }

    fn clear_console(&mut self, ui: &MainWindow) {
        self.console.clear();
        self.show_console(ui);
    }

    fn new_file(&mut self, ui: &MainWindow) {
        if self.can_discard(ui) {
            self.load(ui, String::new(), None);
            ui.set_status("New file".into());
        }
    }

    fn open_file(&mut self, ui: &MainWindow) {
        if !self.can_discard(ui) {
            return;
        }
        if let Some(path) = perox_ui::files::open(ui, "Open", &JS) {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    self.load(ui, text, Some(path));
                    ui.set_status("Opened".into());
                }
                Err(e) => perox_ui::msgbox::error(ui, "Couldn't open", &e.to_string()),
            }
        }
    }

    fn save_file(&mut self, ui: &MainWindow) {
        match self.path.clone() {
            Some(p) => self.write(ui, p),
            None => self.save_as(ui),
        }
    }

    fn save_as(&mut self, ui: &MainWindow) {
        if let Some(p) = perox_ui::files::save(ui, "Save As", &self.file_name(), &JS) {
            self.write(ui, p);
        }
    }

    fn example_picked(&mut self, ui: &MainWindow, name: SharedString) {
        if let Some(text) = examples::get(&name)
            && self.can_discard(ui)
        {
            self.load(ui, text.into(), None);
            ui.set_file_name(format!("{}.js", name.to_lowercase()).into());
            ui.set_status(format!("Example: {name}").into());
        }
    }

    fn bigger(&mut self, ui: &MainWindow) {
        ui.set_font_size((ui.get_font_size() + 1.0).min(28.0));
        self.tokenized.clear();
        self.redraw(ui);
    }

    fn smaller(&mut self, ui: &MainWindow) {
        ui.set_font_size((ui.get_font_size() - 1.0).max(10.0));
        self.tokenized.clear();
        self.redraw(ui);
    }

    fn toggle_console(&mut self, ui: &MainWindow) {
        ui.set_console_open(!ui.get_console_open());
    }

    fn about_clicked(&mut self, ui: &MainWindow) {
        if let Ok(about) = AboutDialog::new() {
            let _ = about.exec(Some(ui.window()), |_| {});
        }
    }
}

/// Start-up: a file to open, or `--example <name>`; `--run` runs it; `--eval <code>`
/// (repeatable) types into the console. For trying things out and for screenshots.
fn start(app: &mut App, ui: &MainWindow, args: &[String]) {
    let mut it = args.iter();
    let mut run = false;
    let mut evals = Vec::new();
    let mut names = vec![SharedString::from("Examples…")];
    names.extend(examples::NAMES.iter().map(|n| SharedString::from(*n)));
    ui.set_examples(ModelRc::new(VecModel::from(names)));
    app.load(ui, examples::get("Hello").unwrap_or_default().into(), None);
    ui.set_file_name("hello.js".into());
    while let Some(a) = it.next() {
        match a.as_str() {
            "--example" => {
                if let Some(n) = it.next()
                    && let Some(text) = examples::get(n)
                {
                    app.load(ui, text.into(), None);
                    ui.set_file_name(format!("{}.js", n.to_lowercase()).into());
                }
            }
            "--run" => run = true,
            "--eval" => evals.extend(it.next().cloned()),
            path => match std::fs::read_to_string(path) {
                Ok(text) => app.load(ui, text, Some(PathBuf::from(path))),
                Err(e) => ui.set_status(format!("Couldn't open {path}: {e}").into()),
            },
        }
    }
    if run {
        app.run_code(ui);
    }
    for e in evals {
        app.repl(ui, e.into());
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let app = ui.connect(App::default());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weak = ui.as_weak();
    let first = Rc::clone(&app);
    // Once the window has its size (so the rows in view are known).
    slint::Timer::single_shot(std::time::Duration::from_millis(20), move || {
        if let Some(ui) = weak.upgrade() {
            let mut app = first.borrow_mut();
            start(&mut app, &ui, &args);
            app.show_console(&ui);
        }
    });
    ui.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_columns() {
        assert_eq!(row_col("ab\ncdé\nf", 0), (0, 0));
        assert_eq!(row_col("ab\ncdé\nf", 3), (1, 0));
        assert_eq!(row_col("ab\ncdé\nf", 7), (1, 3));
        assert_eq!(row_col("ab", 99), (0, 2));
    }

    #[test]
    fn return_keeps_and_adds_indentation() {
        assert_eq!(newline_text("  let x = 1;", 12), "\n  ");
        assert_eq!(newline_text("function f() {", 14), "\n  ");
        assert_eq!(newline_text("  if (a) {", 10), "\n    ");
        assert_eq!(newline_text("x", 1), "\n");
    }

    #[test]
    fn a_closing_brace_steps_back() {
        assert_eq!(dedent_brace("if (a) {\n    ", 13), Some((9, "  }".into())));
        assert_eq!(dedent_brace("  x", 3), None);
        assert_eq!(dedent_brace("", 0), None);
    }

    #[test]
    fn every_example_runs_cleanly() {
        for name in examples::NAMES {
            let out = Engine::new().run(examples::get(name).unwrap());
            // (Promises logs a caught error on purpose.)
            assert!(
                out.iter().all(|l| !l.text.starts_with("Uncaught")),
                "{name}: {out:?}"
            );
        }
    }
}
