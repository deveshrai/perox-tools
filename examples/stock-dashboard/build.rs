fn main() {
    // Compiles ui/ (see ui/slint-project.json) and writes the MainWindowHandlers glue.
    perox_slint::build::compile("ui");
}
