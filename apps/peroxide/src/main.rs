//! PeroxIDE application shell.

mod app;
mod commands;
mod keymap;
mod layout;
mod mcp_host;
mod mcp_page;
mod palette;
mod settings;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("PeroxIDE")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PeroxIDE",
        options,
        Box::new(|cc| Ok(Box::new(app::PeroxApp::new(&cc.egui_ctx)))),
    )
}
