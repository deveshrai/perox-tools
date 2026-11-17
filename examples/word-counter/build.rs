fn main() {
    // Turns every src/ui/*.pxui into typed Rust (see `perox_ui::include_form!`).
    perox_forms::codegen::build::compile_dir("src/ui");
}
