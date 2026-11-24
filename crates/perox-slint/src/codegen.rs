//! Rust code-behind glue for `.slint` components: for each component with callbacks, a
//! `<Name>Handlers` trait (one method per callback, empty defaults) and a `connect`
//! method that routes every callback to it. Slint itself generates the component struct.
//!
//! ```text
//! export component MainWindow inherits Window {
//!     callback ok_clicked();
//!     callback name_edited(text: string);
//! }
//! ```
//! becomes
//! ```text
//! pub trait MainWindowHandlers: 'static {
//!     fn ok_clicked(&mut self, ui: &MainWindow) {}
//!     fn name_edited(&mut self, ui: &MainWindow, text: slint::SharedString) {}
//! }
//! impl MainWindow { pub fn connect<H: MainWindowHandlers>(&self, h: H) -> Rc<RefCell<H>> }
//! ```

use crate::syntax::{Callback, Component, Document, normalize};

/// The Rust type Slint uses for a `.slint` type, if the glue supports it.
pub fn rust_type(slint: &str) -> Option<&'static str> {
    Some(match slint.trim() {
        "string" => "slint::SharedString",
        "int" => "i32",
        "float" | "length" | "physical-length" | "physical_length" | "relative-font-size"
        | "relative_font_size" | "angle" | "percent" => "f32",
        "duration" => "i64",
        "bool" => "bool",
        "color" => "slint::Color",
        "brush" => "slint::Brush",
        "image" => "slint::Image",
        _ => return None,
    })
}

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "gen", "try", "box",
];

/// A `.slint` name as a Rust identifier (`ok-clicked` → `ok_clicked`, `type` → `type_`).
pub fn rust_name(name: &str) -> String {
    let n = normalize(name);
    if KEYWORDS.contains(&n.as_str()) {
        format!("{n}_")
    } else {
        n
    }
}

struct Sig {
    method: String,
    params: Vec<(String, &'static str)>,
    ret: Option<&'static str>,
}

fn signature(cb: &Callback) -> Result<Sig, String> {
    let mut params = Vec::new();
    for (i, (name, ty)) in cb.params.iter().enumerate() {
        let rust = rust_type(ty).ok_or_else(|| ty.clone())?;
        let name = name
            .as_deref()
            .map(rust_name)
            .unwrap_or_else(|| format!("arg{i}"));
        params.push((name, rust));
    }
    let ret = match &cb.ret {
        Some(t) => Some(rust_type(t).ok_or_else(|| t.clone())?),
        None => None,
    };
    Ok(Sig {
        method: rust_name(&cb.name),
        params,
        ret,
    })
}

/// Handlers trait and `connect` for one component; `None` if it has no callbacks.
pub fn component_handlers(c: &Component) -> Option<String> {
    if c.callbacks.is_empty() {
        return None;
    }
    let name = &c.name;
    let mut trait_body = String::new();
    let mut wiring = String::new();
    for cb in &c.callbacks {
        let sig = match signature(cb) {
            Ok(s) => s,
            Err(ty) => {
                trait_body.push_str(&format!(
                    "    // `{}` isn't routed: PeroxIDE can't map the type `{ty}` yet; use `on_{}` directly.\n",
                    cb.name,
                    rust_name(&cb.name)
                ));
                continue;
            }
        };
        let args: String = sig
            .params
            .iter()
            .map(|(n, t)| format!(", {n}: {t}"))
            .collect();
        let ret = sig.ret.map(|r| format!(" -> {r}")).unwrap_or_default();
        let default = if sig.ret.is_some() {
            "Default::default()"
        } else {
            ""
        };
        trait_body.push_str(&format!(
            "    fn {m}(&mut self, ui: &{name}{args}){ret} {{ {default} }}\n",
            m = sig.method
        ));
        let names: Vec<&str> = sig.params.iter().map(|(n, _)| n.as_str()).collect();
        let pass: String = names.iter().map(|n| format!(", {n}")).collect();
        wiring.push_str(&format!(
            "        {{\n            let h = shared.clone();\n            let ui = slint::ComponentHandle::as_weak(self);\n            self.on_{m}(move |{closure}| {{\n                // Busy (a handler is running, e.g. showing a message box): skip.\n                let (Some(ui), Ok(mut h)) = (ui.upgrade(), h.try_borrow_mut()) else {{\n                    return Default::default();\n                }};\n                h.{m}(&ui{pass})\n            }});\n        }}\n",
            m = sig.method,
            closure = names.join(", "),
        ));
    }
    Some(format!(
        "/// Handlers for `{name}`'s callbacks. Every method has an empty default; implement the\n/// ones you need and pass your type to [`{name}::connect`].\n#[allow(unused_variables)]\npub trait {name}Handlers: 'static {{\n{trait_body}}}\n\nimpl {name} {{\n    /// Routes every callback of this window to `handlers`, and returns them, shared.\n    #[allow(clippy::let_unit_value, clippy::unused_unit)]\n    pub fn connect<H: {name}Handlers>(&self, handlers: H) -> std::rc::Rc<std::cell::RefCell<H>> {{\n        let shared = std::rc::Rc::new(std::cell::RefCell::new(handlers));\n{wiring}        shared\n    }}\n}}\n"
    ))
}

/// Handlers for the named components of `doc` (each must be one Slint generates Rust for,
/// that is, exported from the main file).
pub fn handlers(doc: &Document, components: &[&str]) -> String {
    let mut out =
        String::from("// Generated by perox-slint from the .slint files. Don't edit.\n\n");
    for name in components {
        if let Some(code) = doc.component(name).and_then(component_handlers) {
            out.push_str(&code);
            out.push('\n');
        }
    }
    out
}

/// Exported components reachable from a main `.slint` file: its own exported components
/// plus anything it re-exports (`export { A } from "a.slint";`, `export * from …`).
/// `read` loads a file relative to the main file's directory. Returns `(exported name,
/// component, document)`.
pub fn exported_components(
    main: &std::path::Path,
    read: &mut dyn FnMut(&std::path::Path) -> Option<String>,
) -> Vec<(String, Document, String)> {
    let mut out = Vec::new();
    collect(main, None, read, &mut out, 0);
    out
}

/// `names`: which (name, alias) to take from this file; `None` = all exported.
fn collect(
    file: &std::path::Path,
    names: Option<&[(String, String)]>,
    read: &mut dyn FnMut(&std::path::Path) -> Option<String>,
    out: &mut Vec<(String, Document, String)>,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    let Some(src) = read(file) else { return };
    let doc = crate::parse(&src);
    for c in doc.components.iter().filter(|c| c.exported) {
        let alias = match names {
            None => Some(c.name.clone()),
            Some(list) => list
                .iter()
                .find(|(n, _)| *n == c.name)
                .map(|(_, a)| a.clone()),
        };
        if let Some(alias) = alias {
            out.push((alias, doc.clone(), c.name.clone()));
        }
    }
    let dir = file.parent().unwrap_or(std::path::Path::new("."));
    for re in &doc.re_exports {
        let wanted: Option<Vec<(String, String)>> = match (names, re.all) {
            (None, true) => None,
            (None, false) => Some(re.names.clone()),
            (Some(list), true) => Some(list.to_vec()),
            (Some(list), false) => Some(
                re.names
                    .iter()
                    .filter_map(|(n, a)| {
                        list.iter()
                            .find(|(ln, _)| ln == a)
                            .map(|(_, la)| (n.clone(), la.clone()))
                    })
                    .collect(),
            ),
        };
        collect(&dir.join(&re.from), wanted.as_deref(), read, out, depth + 1);
    }
}

/// Handlers for every component a main `.slint` file exports, ready to `include!`.
pub fn handlers_for_main(
    main: &std::path::Path,
    read: &mut dyn FnMut(&std::path::Path) -> Option<String>,
) -> String {
    let mut out =
        String::from("// Generated by perox-slint from the .slint files. Don't edit.\n\n");
    for (alias, doc, name) in exported_components(main, read) {
        if let Some(c) = doc.component(&name) {
            let mut c = c.clone();
            c.name = alias; // Slint names the Rust struct after the exported name.
            if let Some(code) = component_handlers(&c) {
                out.push_str(&code);
                out.push('\n');
            }
        }
    }
    out
}

/// For `build.rs`.
#[cfg(feature = "build")]
pub mod build {
    use std::path::{Path, PathBuf};

    /// The main `.slint` file for `path`: the file itself, or for a directory, the `main`
    /// of its `slint-project.json` (else `app.slint`, else its only `.slint` file).
    pub fn main_file(path: &Path) -> PathBuf {
        if path.is_file() {
            if path.file_name().is_some_and(|n| n == "slint-project.json") {
                return main_file(path.parent().unwrap_or(Path::new(".")));
            }
            return path.to_owned();
        }
        if let Ok(json) = std::fs::read_to_string(path.join("slint-project.json"))
            && let Some(main) = json_string_field(&json, "main")
        {
            return path.join(main);
        }
        if path.join("app.slint").exists() {
            return path.join("app.slint");
        }
        let mut slint: Vec<PathBuf> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "slint"))
            .collect();
        slint.sort();
        slint
            .into_iter()
            .next()
            .unwrap_or_else(|| path.join("app.slint"))
    }

    fn json_string_field(json: &str, key: &str) -> Option<String> {
        let at = json.find(&format!("\"{key}\""))?;
        let rest = json[at + key.len() + 2..]
            .trim_start()
            .strip_prefix(':')?
            .trim_start();
        let rest = rest.strip_prefix('"')?;
        Some(rest[..rest.find('"')?].to_owned())
    }

    /// Compiles the UI with `slint-build` and writes the handlers glue to
    /// `$OUT_DIR/perox_handlers.rs`. `path` is a `.slint` file, or a directory with a
    /// `slint-project.json`. Use with `perox_ui::include_ui!()`.
    pub fn compile(path: impl AsRef<Path>) {
        let path = path.as_ref();
        if let Err(e) = slint_build::compile(path) {
            panic!("Slint compile error in {}: {e}", path.display());
        }
        let main = main_file(path);
        let code = super::handlers_for_main(&main, &mut |p| std::fs::read_to_string(p).ok());
        let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR: run from build.rs"));
        std::fs::write(out.join("perox_handlers.rs"), code).expect("write perox_handlers.rs");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_trait_and_connect() {
        let doc = crate::parse(
            "export component MainWindow inherits Window {\n    callback ok-clicked();\n    callback name_edited(text: string);\n    callback compute(int, float) -> int;\n    callback pick(MyStruct);\n}\n",
        );
        let code = handlers(&doc, &["MainWindow"]);
        assert!(
            code.contains("pub trait MainWindowHandlers: 'static {"),
            "{code}"
        );
        assert!(
            code.contains("fn ok_clicked(&mut self, ui: &MainWindow) {  }"),
            "{code}"
        );
        assert!(
            code.contains("fn name_edited(&mut self, ui: &MainWindow, text: slint::SharedString)"),
            "{code}"
        );
        assert!(code.contains("fn compute(&mut self, ui: &MainWindow, arg0: i32, arg1: f32) -> i32 { Default::default() }"), "{code}");
        assert!(code.contains("`pick` isn't routed"), "{code}");
        assert!(code.contains("self.on_ok_clicked(move || {"), "{code}");
        assert!(
            code.contains("self.on_compute(move |arg0, arg1| {"),
            "{code}"
        );
        assert!(code.contains("h.name_edited(&ui, text)"), "{code}");
        assert_eq!(
            component_handlers(&crate::parse("export component A { }").components[0]),
            None
        );
    }

    #[test]
    fn follows_re_exports() {
        use std::collections::HashMap;
        use std::path::{Path, PathBuf};
        let files: HashMap<PathBuf, &str> = [
            ("ui/app.slint", "export { MainWindow } from \"main_window.slint\";\nexport { About as AboutDialog } from \"about.slint\";\nexport component Local inherits Window { callback x(); }\n"),
            ("ui/main_window.slint", "export component MainWindow inherits Window { callback go(); }\nexport component Hidden { callback no(); }\n"),
            ("ui/about.slint", "export component About inherits Dialog { callback closed(); }\n"),
        ]
        .into_iter()
        .map(|(p, s)| (PathBuf::from(p), s))
        .collect();
        let mut read = |p: &Path| files.get(p).map(|s| (*s).to_owned());
        let names: Vec<String> = exported_components(Path::new("ui/app.slint"), &mut read)
            .into_iter()
            .map(|(a, _, _)| a)
            .collect();
        assert_eq!(names, ["Local", "MainWindow", "AboutDialog"]);
        let code = handlers_for_main(Path::new("ui/app.slint"), &mut read);
        assert!(code.contains("pub trait AboutDialogHandlers"), "{code}");
        assert!(code.contains("impl AboutDialog {"), "{code}");
        assert!(!code.contains("Hidden"), "{code}");
    }
}
