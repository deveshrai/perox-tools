//! New projects. The GUI app template is a window with a form, its code-behind and a
//! `build.rs` that compiles the form.

use std::path::Path;

/// A Cargo package name from a folder name: `My App` → `my-app`.
pub fn package_name(dir: &Path) -> String {
    let raw = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".into());
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    match out.chars().next() {
        None => "app".into(),
        Some(c) if c.is_ascii_digit() => format!("app-{out}"),
        _ => out,
    }
}

/// `my-app` → `My App` (the window title).
fn title(name: &str) -> String {
    name.split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Cargo.toml's path to a runtime crate, with forward slashes (TOML strings on
/// Windows need them or escaping).
fn toml_path(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Writes a GUI app into `dir`, which must not exist or be empty. Returns the form file.
pub fn create_gui_app(dir: &Path, sdk: &Path) -> Result<std::path::PathBuf, String> {
    if dir.exists() && dir.read_dir().map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} isn't empty.", dir.display()));
    }
    let name = package_name(dir);
    let ui = toml_path(&sdk.join("perox-ui"));
    let forms = toml_path(&sdk.join("perox-forms"));
    let title = title(&name);
    let files: Vec<(&str, String)> = vec![
        (
            "Cargo.toml",
            format!(
                r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
perox-ui = {{ path = "{ui}" }}

[build-dependencies]
perox-forms = {{ path = "{forms}" }}
"#
            ),
        ),
        (
            "rust-toolchain.toml",
            format!(
                "[toolchain]\nchannel = \"{}\"\n",
                perox_build::tools::RUST_VERSION
            ),
        ),
        (".gitignore", "/target\n".into()),
        (
            "build.rs",
            "fn main() {\n    // Turns every src/ui/*.pxui into typed Rust (see `perox_ui::include_form!`).\n    perox_forms::codegen::build::compile_dir(\"src/ui\");\n}\n".into(),
        ),
        (
            "src/ui/main_window.pxui",
            format!(
                r#"// The main window. Design it in PeroxIDE (Design tab) or edit it here.
Window main_window {{
    title: "{title}"
    size: 480 x 300

    VBox {{
        Label {{ text: "What's your name?" }}
        HBox {{
            TextEdit name {{ placeholder: "Name"; on submitted: greet_clicked }}
            Button greet {{ text: "Greet"; on clicked: greet_clicked }}
        }}
        Label greeting {{ text: ""; heading: true }}
        Spacer {{}}
    }}
}}
"#
            ),
        ),
        (
            "src/main.rs",
            r#"//! The code-behind for `ui/main_window.pxui`. Each `on <event>: <method>` in the form
//! is a method of `MainWindowHandlers`; implement the ones you need.

mod main_window {
    perox_ui::include_form!("main_window");
}

use main_window::{MainWindow, MainWindowHandlers};

#[derive(Default)]
struct App;

impl MainWindowHandlers for App {
    fn greet_clicked(&mut self, ui: &mut MainWindow<'_>) {
        let name = ui.name().text();
        let name = if name.trim().is_empty() { "world" } else { name.trim() };
        ui.greeting().set_text(format!("Hello, {name}!"));
    }
}

fn main() -> perox_ui::eframe::Result {
    MainWindow::run(App)
}
"#
            .into(),
        ),
    ];
    for (rel, text) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(dir.join("src/ui/main_window.pxui"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(package_name(Path::new("/x/My Cool App")), "my-cool-app");
        assert_eq!(package_name(Path::new("/x/2048")), "app-2048");
        assert_eq!(title("my-cool-app"), "My Cool App");
    }

    /// The template builds and its handler works (runs `cargo check` against this tree).
    #[test]
    #[ignore]
    fn gui_template_builds() {
        let dir = std::env::temp_dir().join(format!("perox-template-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sdk = perox_build::tools::sdk_dir().expect("sdk");
        let form = create_gui_app(&dir, &sdk).unwrap();
        assert!(form.exists());
        let status = std::process::Command::new(perox_build::tools::cargo().unwrap())
            .arg("build")
            .current_dir(&dir)
            .envs(perox_build::tools::env())
            .env(
                "CARGO_TARGET_DIR",
                std::env::temp_dir().join("perox-template-target"),
            )
            .status()
            .unwrap();
        assert!(status.success());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
