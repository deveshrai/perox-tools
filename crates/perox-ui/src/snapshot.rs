//! A picture of the running app, for documentation and for agents: run the app with
//! `PEROX_SNAPSHOT=<file.png>` (PeroxIDE's `build.snapshot` does) and the first window
//! saves itself there once it has settled (`PEROX_SNAPSHOT_DELAY` milliseconds, default
//! 1500), then the app quits (unless `PEROX_SNAPSHOT_QUIT=0`).
//!
//! The generated `connect` calls [`arm`]; without the variable it does nothing.

use std::sync::atomic::{AtomicBool, Ordering};

static ARMED: AtomicBool = AtomicBool::new(false);

/// Schedules the snapshot of `ui`'s window, if one was asked for (once per run).
pub fn arm<C: slint::ComponentHandle + 'static>(ui: &C) {
    let Some(path) = std::env::var_os("PEROX_SNAPSHOT") else {
        return;
    };
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let delay = std::env::var("PEROX_SNAPSHOT_DELAY")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(1500);
    let quit = std::env::var("PEROX_SNAPSHOT_QUIT").map_or(true, |q| q != "0");
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(delay), move || {
        let Some(ui) = weak.upgrade() else { return };
        match save(ui.window(), std::path::Path::new(&path)) {
            Ok(()) => eprintln!("snapshot: {}", std::path::Path::new(&path).display()),
            Err(e) => eprintln!("snapshot failed: {e}"),
        }
        if quit {
            let _ = slint::quit_event_loop();
        }
    });
}

/// Saves what `window` shows now as a PNG.
pub fn save(window: &slint::Window, path: &std::path::Path) -> Result<(), String> {
    let shot = window.take_snapshot().map_err(|e| e.to_string())?;
    let (w, h) = (shot.width(), shot.height());
    let img = image::RgbaImage::from_raw(w, h, shot.as_bytes().to_vec())
        .ok_or("the snapshot has the wrong size")?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    img.save(path).map_err(|e| e.to_string())
}
