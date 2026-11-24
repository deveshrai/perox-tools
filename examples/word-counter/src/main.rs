//! The code-behind for `ui/main_window.slint`: implement the handlers you need.

perox_ui::include_ui!();

use slint::ComponentHandle;

#[derive(Default)]
struct App;

impl App {
    fn update(&self, ui: &MainWindow) {
        let words = count_words(&ui.get_text(), ui.get_ignore_numbers());
        let plural = if words == 1 { "" } else { "s" };
        ui.set_result(format!("{words} word{plural}").into());
    }
}

impl MainWindowHandlers for App {
    fn input_edited(&mut self, ui: &MainWindow, _text: slint::SharedString) {
        self.update(ui);
    }

    fn ignore_numbers_toggled(&mut self, ui: &MainWindow) {
        self.update(ui);
    }

    fn clear_clicked(&mut self, ui: &MainWindow) {
        if ui.get_text().is_empty() || perox_ui::msgbox::confirm(ui, "Clear", "Clear the text?") {
            ui.set_text("".into());
            self.update(ui);
        }
    }

    fn about_clicked(&mut self, ui: &MainWindow) {
        if let Ok(about) = AboutDialog::new() {
            let _ = about.exec(Some(ui.window()), |_| {});
        }
    }
}

fn count_words(text: &str, skip_numbers: bool) -> usize {
    text.split_whitespace()
        .filter(|w| !skip_numbers || !w.chars().all(|c| c.is_ascii_digit()))
        .count()
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.connect(App);
    ui.run()
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
    fn handlers_update_the_window() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = MainWindow::new().unwrap();
        ui.connect(App);
        ui.set_text("a b 3".into());
        ui.invoke_input_edited("a b 3".into());
        assert_eq!(ui.get_result(), "3 words");
        ui.set_ignore_numbers(true);
        ui.invoke_ignore_numbers_toggled();
        assert_eq!(ui.get_result(), "2 words");
    }
}
