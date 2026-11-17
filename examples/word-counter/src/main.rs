//! The code-behind for `ui/main_window.pxui`: implement the handlers you need.

mod main_window {
    perox_ui::include_form!("main_window");
}

use main_window::{MainWindow, MainWindowHandlers};

#[derive(Default)]
struct App;

impl MainWindowHandlers for App {
    fn input_changed(&mut self, ui: &mut MainWindow<'_>) {
        let text = ui.input().text();
        let skip_numbers = ui.ignore_numbers().checked();
        let words = count_words(&text, skip_numbers);
        let plural = if words == 1 { "" } else { "s" };
        ui.result().set_text(format!("{words} word{plural}"));
    }

    fn clear_clicked(&mut self, ui: &mut MainWindow<'_>) {
        ui.input().set_text("");
        self.input_changed(ui);
    }
}

fn count_words(text: &str, skip_numbers: bool) -> usize {
    text.split_whitespace()
        .filter(|w| !skip_numbers || !w.chars().all(|c| c.is_ascii_digit()))
        .count()
}

fn main() -> perox_ui::eframe::Result {
    MainWindow::run(App)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts() {
        assert_eq!(count_words("one two  three", false), 3);
        assert_eq!(count_words("call 555 now", true), 2);
    }

    #[test]
    fn handlers_update_the_form() {
        let mut form = perox_ui::Form::new(main_window::SOURCE);
        let mut ui = MainWindow { form: &mut form };
        ui.input().set_text("a b c");
        App.input_changed(&mut ui);
        assert_eq!(ui.result().text(), "3 words");
        App.clear_clicked(&mut ui);
        assert_eq!(ui.result().text(), "0 words");
    }
}
