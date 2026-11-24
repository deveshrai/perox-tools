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

/// What a new form is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormKind {
    Window,
    Dialog,
}

/// `settings_dialog` / `settings-dialog` → `SettingsDialog`.
pub fn type_name(stem: &str) -> String {
    let mut out: String = stem
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "Form");
    }
    out
}

/// `SettingsDialog` → `Settings Dialog`.
fn words(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// The source of a new window or dialog called `name`.
pub fn form_source(name: &str, kind: FormKind) -> String {
    let title = words(name);
    match kind {
        FormKind::Window => format!(
            r#"import {{ VerticalBox }} from "std-widgets.slint";

// A window. Show it from Rust: `let w = {name}::new()?; w.show()?;`
export component {name} inherits Window {{
    title: "{title}";
    preferred-width: 400px;
    preferred-height: 300px;

    VerticalBox {{
        Text {{ text: "{title}"; }}
    }}
}}
"#
        ),
        FormKind::Dialog => format!(
            r#"import {{ StandardButton, VerticalBox }} from "std-widgets.slint";

// A dialog. Show it from Rust:
// `{name}::new()?.exec(Some(ui.window()), |result| {{ if result.accepted() {{ … }} }})?;`
export component {name} inherits Dialog {{
    title: "{title}";

    VerticalBox {{
        Text {{ text: "Put the dialog's content here."; }}
    }}
    StandardButton {{ kind: ok; }}
    StandardButton {{ kind: cancel; }}
}}
"#
        ),
    }
}

/// The project's main `.slint` file for `ui_dir` (from its `slint-project.json`).
fn main_slint(ui_dir: &Path) -> Option<std::path::PathBuf> {
    let json = std::fs::read_to_string(ui_dir.join("slint-project.json")).ok()?;
    let at = json.find("\"main\"")?;
    let rest = json[at + 6..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    Some(ui_dir.join(&rest[..rest.find('"')?]))
}

/// Writes a new form at `path` (a `.slint` file that doesn't exist yet) and exports it
/// from the project's main `.slint` file, so Rust can use it. Returns the type name.
pub fn add_form(path: &Path, kind: FormKind) -> Result<String, String> {
    add_form_with(path, |name| form_source(name, kind))
}

/// Adds a form that inherits `base` (a component exported from `base_file`): it starts
/// with everything `base` has, and adds to it.
pub fn add_inherited_form(path: &Path, base: &str, base_file: &Path) -> Result<String, String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let rel = relative_path(base_file, dir);
    let base_src = std::fs::read_to_string(base_file).map_err(|e| e.to_string())?;
    let doc = perox_slint::parse(&base_src);
    let forwards = doc.component(base).map(base_forwards).unwrap_or_default();
    add_form_with(path, |name| {
        format!(
            "import {{ {base} }} from \"{rel}\";\n\n// {name}: a {base} with more on it. What {base} has is edited in {file}.\nexport component {name} inherits {base} {{\n{forwards}}}\n",
            file = base_file
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        )
    })
}

/// Slint gives Rust only a component's own properties and callbacks, not inherited ones.
/// So an inheriting form passes its base's on under `base_` names:
/// `in-out property <string> base_name <=> self.name;` and
/// `callback base_save_clicked(); save_clicked => { root.base_save_clicked(); }`.
fn base_forwards(base: &perox_slint::Component) -> String {
    let mut out = String::new();
    let props: Vec<_> = base
        .properties
        .iter()
        .filter(|p| matches!(p.visibility.as_str(), "in" | "in-out" | "out") && p.ty.is_some())
        .collect();
    if props.is_empty() && base.callbacks.is_empty() {
        return out;
    }
    out.push_str(
        "    // The base form's properties and events for Rust (Slint doesn't pass them on).\n",
    );
    for p in props {
        let vis = if p.visibility == "out" {
            "out"
        } else {
            "in-out"
        };
        out.push_str(&format!(
            "    {vis} property <{}> base_{} <=> self.{};\n",
            p.ty.as_deref().unwrap_or_default(),
            p.name.replace('-', "_"),
            p.name
        ));
    }
    for cb in &base.callbacks {
        let names: Vec<String> = (0..cb.params.len())
            .map(|i| match &cb.params[i].0 {
                Some(n) => n.clone(),
                None => format!("arg{i}"),
            })
            .collect();
        let decl: Vec<String> = cb
            .params
            .iter()
            .zip(&names)
            .map(|((_, t), n)| format!("{n}: {t}"))
            .collect();
        let ret = cb
            .ret
            .as_ref()
            .map(|r| format!(" -> {r}"))
            .unwrap_or_default();
        let fwd = cb.name.replace('-', "_");
        let call = format!("root.base_{fwd}({})", names.join(", "));
        let body = if cb.ret.is_some() {
            format!("return {call};")
        } else {
            format!("{call};")
        };
        let params = if names.is_empty() {
            String::new()
        } else {
            format!("({})", names.join(", "))
        };
        out.push_str(&format!(
            "    callback base_{fwd}({}){ret};\n    {}{params} => {{ {body} }}\n",
            decl.join(", "),
            cb.name
        ));
    }
    out
}

/// Components exported from the `.slint` files under `dir` (a project's `ui/`): what a
/// new form can inherit. Sorted by name.
pub fn exported_forms(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    fn walk(d: &Path, out: &mut Vec<(String, std::path::PathBuf)>, depth: usize) {
        if depth > 4 {
            return;
        }
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if e.file_name() != "translations" {
                    walk(&p, out, depth + 1);
                }
            } else if p.extension().is_some_and(|x| x == "slint")
                && let Ok(src) = std::fs::read_to_string(&p)
            {
                for c in perox_slint::parse(&src).components {
                    if c.exported {
                        out.push((c.name, p.clone()));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out, 0);
    out.sort();
    out
}

/// `to` relative to the folder `from`, with `/` and `..` as needed.
fn relative_path(to: &Path, from: &Path) -> String {
    let to: Vec<_> = to.components().collect();
    let from: Vec<_> = from.components().collect();
    let common = to.iter().zip(&from).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_owned(), from.len() - common).collect();
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn add_form_with(path: &Path, source: impl FnOnce(&str) -> String) -> Result<String, String> {
    if path.exists() {
        return Err(format!("{} already exists.", path.display()));
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = type_name(&stem);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, source(&name)).map_err(|e| e.to_string())?;
    // Export it from the main file (ui/app.slint), next to it or above it.
    let mut dir = path.parent();
    while let Some(d) = dir {
        if let Some(main) = main_slint(d).filter(|m| m.exists() && m != path) {
            let rel = pathdiff(path, main.parent().unwrap_or(d));
            let mut text = std::fs::read_to_string(&main).map_err(|e| e.to_string())?;
            if !text.ends_with('\n') && !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("export {{ {name} }} from \"{rel}\";\n"));
            std::fs::write(&main, text).map_err(|e| e.to_string())?;
            break;
        }
        if d.join("Cargo.toml").exists() {
            break;
        }
        dir = d.parent();
    }
    Ok(name)
}

/// `path` relative to `base` with forward slashes (both absolute, `path` under `base`
/// or beside it).
fn pathdiff(path: &Path, base: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(r) => r.to_string_lossy().replace('\\', "/"),
        Err(_) => path.to_string_lossy().replace('\\', "/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_forms_are_exported_from_the_main_file() {
        let dir = std::env::temp_dir().join(format!("perox-forms-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = dir.join("app");
        let sdk = Path::new("/nowhere");
        create_gui_app(&app, sdk).unwrap();
        let name = add_form(&app.join("ui/settings_dialog.slint"), FormKind::Dialog).unwrap();
        assert_eq!(name, "SettingsDialog");
        let main = std::fs::read_to_string(app.join("ui/app.slint")).unwrap();
        assert!(
            main.ends_with("export { SettingsDialog } from \"settings_dialog.slint\";\n"),
            "{main}"
        );
        let src = std::fs::read_to_string(app.join("ui/settings_dialog.slint")).unwrap();
        let doc = perox_slint::parse(&src);
        assert!(doc.errors.is_empty());
        assert_eq!(doc.components[0].root.kind, "Dialog");
        assert!(add_form(&app.join("ui/settings_dialog.slint"), FormKind::Dialog).is_err());
        assert_eq!(type_name("tool-window"), "ToolWindow");
        // Inherited: imports its base relative to itself, and is exported too.
        let name = add_inherited_form(
            &app.join("ui/forms/big_window.slint"),
            "MainWindow",
            &app.join("ui/main_window.slint"),
        )
        .unwrap();
        assert_eq!(name, "BigWindow");
        let src = std::fs::read_to_string(app.join("ui/forms/big_window.slint")).unwrap();
        assert!(
            src.starts_with("import { MainWindow } from \"../main_window.slint\";"),
            "{src}"
        );
        assert!(
            src.contains("export component BigWindow inherits MainWindow {"),
            "{src}"
        );
        assert!(src.contains("callback base_greet_clicked();\n    greet_clicked => { root.base_greet_clicked(); }"), "{src}");
        assert!(
            src.contains("in-out property <string> base_name <=> self.name;"),
            "{src}"
        );
        assert!(perox_slint::parse(&src).errors.is_empty(), "{src}");
        let main = std::fs::read_to_string(app.join("ui/app.slint")).unwrap();
        assert!(
            main.ends_with("export { BigWindow } from \"forms/big_window.slint\";\n"),
            "{main}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(type_name("2nd"), "Form2nd");
    }

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
