//! New projects. The GUI app template is a Slint window and an About dialog, their
//! code-behind, and a `build.rs` that compiles them (ADR 0011).

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

/// Writes a GUI app into `dir`, which must not exist or be empty. Returns the main
/// window's `.slint` file.
pub fn create_gui_app(dir: &Path, sdk: &Path) -> Result<std::path::PathBuf, String> {
    if dir.exists() && dir.read_dir().map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} isn't empty.", dir.display()));
    }
    let name = package_name(dir);
    let ui = toml_path(&sdk.join("perox-ui"));
    let slint_tools = toml_path(&sdk.join("perox-slint"));
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
slint = "=1.19.0"

[build-dependencies]
perox-slint = {{ path = "{slint_tools}", features = ["build"] }}
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
            "fn main() {\n    // Compiles ui/ (see ui/slint-project.json) and writes the MainWindowHandlers glue.\n    perox_slint::build::compile(\"ui\");\n}\n".into(),
        ),
        (
            "ui/slint-project.json",
            "{ \"edition\": \"1.19\", \"main\": \"app.slint\" }\n".into(),
        ),
        (
            "ui/app.slint",
            "// Every window the app uses. Rust sees what this file exports.\nexport { MainWindow } from \"main_window.slint\";\nexport { AboutDialog } from \"about_dialog.slint\";\n".into(),
        ),
        (
            "ui/main_window.slint",
            format!(
                r#"import {{ Button, HorizontalBox, LineEdit, VerticalBox }} from "std-widgets.slint";

// The main window. Design it in PeroxIDE (Design tab) or edit it here.
export component MainWindow inherits Window {{
    title: "{title}";
    preferred-width: 480px;
    preferred-height: 240px;

    // What the Rust code reads and writes.
    in-out property <string> name <=> name_edit.text;
    in-out property <string> greeting: "";

    callback greet_clicked();
    callback about_clicked();

    VerticalBox {{
        alignment: start;
        Text {{ text: "What's your name?"; }}
        HorizontalBox {{
            padding: 0px;
            name_edit := LineEdit {{ placeholder-text: "Name"; accepted(text) => {{ root.greet_clicked(); }} }}
            greet := Button {{ text: "Greet"; primary: true; clicked => {{ root.greet_clicked(); }} }}
        }}
        Text {{ text: root.greeting; font-size: 20px; font-weight: 700; }}
        HorizontalBox {{
            padding: 0px;
            alignment: end;
            about := Button {{ text: "About"; clicked => {{ root.about_clicked(); }} }}
        }}
    }}
}}
"#
            ),
        ),
        (
            "ui/about_dialog.slint",
            format!(
                r#"import {{ AboutSlint, StandardButton, VerticalBox }} from "std-widgets.slint";

// Shown from the About button. `AboutSlint` credits Slint, as its royalty-free
// license asks; keep it (or buy a Slint license) when you ship.
export component AboutDialog inherits Dialog {{
    title: "About {title}";

    VerticalBox {{
        Text {{ text: "{title}"; font-size: 18px; font-weight: 700; }}
        Text {{ text: "Version 0.1.0"; }}
        AboutSlint {{ }}
    }}
    StandardButton {{ kind: ok; }}
}}
"#
            ),
        ),
        (
            "src/main.rs",
            r#"//! The code-behind for `ui/main_window.slint`. Each `callback` the window declares is
//! a method of `MainWindowHandlers`; implement the ones you need.

perox_ui::include_ui!();

use slint::ComponentHandle;

#[derive(Default)]
struct App;

impl MainWindowHandlers for App {
    fn greet_clicked(&mut self, ui: &MainWindow) {
        let name = ui.get_name();
        let name = if name.trim().is_empty() { "world" } else { name.trim() };
        ui.set_greeting(format!("Hello, {name}!").into());
    }

    fn about_clicked(&mut self, ui: &MainWindow) {
        if let Ok(about) = AboutDialog::new() {
            let _ = about.exec(Some(ui.window()), |_| {});
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.connect(App);
    ui.run()
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
    Ok(dir.join("ui/main_window.slint"))
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
