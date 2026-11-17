//! PeroxIDE application shell.

mod app;
mod commands;
mod docs;
mod ide;
mod keymap;
mod layout;
mod lsp;
mod mcp_host;
mod mcp_page;
mod palette;
mod settings;

fn main() -> eframe::Result {
    // `peroxide <folder or file>` opens that project.
    let open = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p));
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
        Box::new(move |cc| Ok(Box::new(app::PeroxApp::new(&cc.egui_ctx, open)))),
    )
}
