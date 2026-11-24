//! PeroxIDE application shell.

mod app;
mod code_behind;
mod commands;
mod docs;
mod git;
mod icons;
mod ide;
mod keymap;
mod layout;
mod lsp;
mod mcp_host;
mod mcp_page;
mod palette;
mod settings;
mod templates;
mod testing;
mod watcher;

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
        // winit's default macOS menu quits the app directly on ⌘Q, skipping the
        // "save changes?" prompt. Without it, ⌘Q reaches PeroxIDE like any shortcut.
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_default_menu(false);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "PeroxIDE",
        options,
        Box::new(move |cc| Ok(Box::new(app::PeroxApp::new(&cc.egui_ctx, open)))),
    )
}
