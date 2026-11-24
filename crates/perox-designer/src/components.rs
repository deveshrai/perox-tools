//! Components to drop besides Slint's own: the project's reusable components (exported
//! from its other `.slint` files, not windows) and the components of its libraries
//! (`library-paths` in `slint-project.json`, imported as `@name`).
//!
//! They're `.slint` code, so the designer's preview runs them as they are. (Components
//! with Rust logic behind them would need the out-of-process preview host; ADR 0008.)

use std::path::{Path, PathBuf};

/// A component the palette offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub name: String,
    /// What to import it from, as written in the form: `widgets/card.slint` (relative to
    /// the form) or `@widgets`. `None`: it's in the form's own file.
    pub from: Option<String>,
    /// The palette group: `Project` or the library's name.
    pub group: String,
    /// It takes children (`@children`): the kind of element holding the slot.
    pub slot: Option<String>,
}

fn slot_of(c: &perox_slint::Component) -> Option<String> {
    c.children_slot().map(|(_, el)| el.kind.clone())
}

fn is_reusable(c: &perox_slint::Component) -> bool {
    !matches!(c.root.kind.as_str(), "Window" | "Dialog" | "PopupWindow")
}

/// `to` relative to the folder `from`, with `/` and `..`.
fn relative(to: &Path, from: &Path) -> String {
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

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') || name == "translations" {
            continue;
        }
        if p.is_dir() {
            slint_files(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x == "slint") {
            out.push(p);
        }
    }
}

/// What the palette can offer for the form `file` (its source `src`, the component
/// being designed `current`), in a project whose `ui/` folder is `ui`.
pub fn offers(file: &Path, src: &str, current: &str, ui: &Path) -> Vec<Offer> {
    let mut out: Vec<Offer> = Vec::new();
    let here = file.parent().unwrap_or(Path::new("."));
    let mut files = Vec::new();
    slint_files(ui, &mut files, 0);
    files.sort();
    let me = file.canonicalize().unwrap_or_else(|_| file.to_owned());
    // Every component: (file, component, is this file), to tell windows from parts,
    // following `inherits` (a form inheriting a window is a window).
    let mut all: Vec<(Option<PathBuf>, perox_slint::Component)> = perox_slint::parse(src)
        .components
        .into_iter()
        .map(|c| (None, c))
        .collect();
    for f in files {
        if f.canonicalize().unwrap_or_else(|_| f.clone()) == me {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&f) {
            for c in perox_slint::parse(&text).components {
                all.push((Some(f.clone()), c));
            }
        }
    }
    let mut windows: Vec<String> = Vec::new();
    loop {
        let before = windows.len();
        for (_, c) in &all {
            if !windows.contains(&c.name) && (!is_reusable(c) || windows.contains(&c.root.kind)) {
                windows.push(c.name.clone());
            }
        }
        if windows.len() == before {
            break;
        }
    }
    for (f, c) in all {
        if windows.contains(&c.name) || c.name == current || out.iter().any(|o| o.name == c.name) {
            continue;
        }
        match f {
            // In this file: no import needed.
            None => out.push(Offer {
                slot: slot_of(&c),
                name: c.name,
                from: None,
                group: "Project".into(),
            }),
            Some(f) if c.exported => out.push(Offer {
                slot: slot_of(&c),
                name: c.name,
                from: Some(relative(&f, here)),
                group: "Project".into(),
            }),
            Some(_) => {}
        }
    }
    // Libraries.
    if let Some(pf) = perox_slint::project::find(here)
        && let Ok(json) = std::fs::read_to_string(&pf)
    {
        let json_dir = pf.parent().unwrap_or(Path::new("."));
        for (lib, path) in perox_slint::project::library_paths(&json, json_dir) {
            let mut read = |p: &Path| std::fs::read_to_string(p).ok();
            if path.is_file() {
                for (alias, doc, name) in
                    perox_slint::codegen::exported_components(&path, &mut read)
                {
                    if let Some(c) = doc.component(&name).filter(|c| is_reusable(c)) {
                        out.push(Offer {
                            slot: slot_of(c),
                            name: alias,
                            from: Some(format!("@{lib}")),
                            group: lib.clone(),
                        });
                    }
                }
            } else if path.is_dir() {
                let mut files = Vec::new();
                slint_files(&path, &mut files, 0);
                files.sort();
                for f in files {
                    let Ok(text) = std::fs::read_to_string(&f) else {
                        continue;
                    };
                    for c in perox_slint::parse(&text).components {
                        if c.exported && is_reusable(&c) {
                            out.push(Offer {
                                slot: slot_of(&c),
                                name: c.name,
                                from: Some(format!("@{lib}/{}", relative(&f, &path))),
                                group: lib.clone(),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// The definition of component `kind` as the form at `file` (source `src`) sees it: in
/// the same file, or in the file (or library) it's imported from.
pub fn definition(file: &Path, src: &str, kind: &str) -> Option<perox_slint::Component> {
    let doc = perox_slint::parse(src);
    if let Some(c) = doc.component(kind) {
        return Some(c.clone());
    }
    let import = doc
        .imports
        .iter()
        .find(|i| i.names.iter().any(|n| n == kind))?;
    let here = file.parent().unwrap_or(Path::new("."));
    let target = match import.from.strip_prefix('@') {
        Some(lib) => {
            let (name, rest) = lib
                .split_once('/')
                .map_or((lib, None), |(n, r)| (n, Some(r)));
            let base = library_paths(here).get(name)?.clone();
            match rest {
                Some(r) => base.join(r),
                None => base,
            }
        }
        None if import.from.ends_with("std-widgets.slint") => return None,
        None => here.join(&import.from),
    };
    let mut read = |p: &Path| std::fs::read_to_string(p).ok();
    perox_slint::codegen::exported_components(&target, &mut read)
        .into_iter()
        .find(|(alias, _, _)| alias == kind)
        .and_then(|(_, d, name)| d.component(&name).cloned())
}

/// The library paths the preview needs for `@name` imports, for a form in `dir`.
pub fn library_paths(dir: &Path) -> std::collections::HashMap<String, PathBuf> {
    let Some(pf) = perox_slint::project::find(dir) else {
        return Default::default();
    };
    let json = std::fs::read_to_string(&pf).unwrap_or_default();
    perox_slint::project::library_paths(&json, pf.parent().unwrap_or(Path::new(".")))
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_project_components_and_libraries() {
        let root = std::env::temp_dir().join(format!("perox-comps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ui = root.join("ui");
        std::fs::create_dir_all(ui.join("parts")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let main_src =
            "component Badge inherits Rectangle { }\nexport component Main inherits Window { }\n";
        std::fs::write(ui.join("main.slint"), main_src).unwrap();
        std::fs::write(
            ui.join("parts/card.slint"),
            "export component Card inherits Rectangle { }\nexport component Other inherits Window { }\ncomponent Hidden { }\n",
        )
        .unwrap();
        // A form inheriting a window is a window, not a part.
        std::fs::write(
            ui.join("big.slint"),
            "import { Other } from \"parts/card.slint\";\nexport component Big inherits Other { }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("lib/widgets.slint"),
            "export component Chip inherits Rectangle { }\n",
        )
        .unwrap();
        std::fs::write(
            ui.join(perox_slint::project::FILE_NAME),
            perox_slint::project::add_library("{}", "widgets", "../lib/widgets.slint"),
        )
        .unwrap();
        let o = offers(&ui.join("main.slint"), main_src, "Main", &ui);
        assert!(o.iter().all(|o| o.slot.is_none()));
        let got: Vec<(&str, Option<&str>, &str)> = o
            .iter()
            .map(|o| (o.name.as_str(), o.from.as_deref(), o.group.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Badge", None, "Project"),
                ("Card", Some("parts/card.slint"), "Project"),
                ("Chip", Some("@widgets"), "widgets"),
            ]
        );
        let libs = library_paths(&ui);
        assert_eq!(libs.len(), 1);
    }
}

/// The structs a type names (`[DrinkData]`, `{ a: Layer }`) that another `.slint` file
/// of the project exports and `src` doesn't have yet: `(name, import from)`. Declaring a
/// property of such a type then imports them, as dropping a component does.
pub fn struct_imports(file: &Path, src: &str, ty: &str, ui: &Path) -> Vec<(String, String)> {
    let doc = perox_slint::parse(src);
    let here = file.parent().unwrap_or(Path::new("."));
    let mut files = Vec::new();
    slint_files(ui, &mut files, 0);
    let mut out: Vec<(String, String)> = Vec::new();
    for name in ty
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .filter(|w| w.chars().next().is_some_and(char::is_uppercase))
    {
        let known = doc
            .imports
            .iter()
            .any(|i| i.names.iter().any(|n| n == name))
            || src.contains(&format!("struct {name} "))
            || src.contains(&format!("struct {name}{{"))
            || out.iter().any(|(n, _)| n == name);
        if known {
            continue;
        }
        let decl = format!("export struct {name} ");
        if let Some(f) = files.iter().find(|f| {
            *f != file
                && std::fs::read_to_string(f).is_ok_and(|t| {
                    t.contains(&decl) || t.contains(&format!("export struct {name}{{"))
                })
        }) {
            out.push((name.to_owned(), relative(f, here)));
        }
    }
    out
}

#[cfg(test)]
mod struct_tests {
    use super::*;

    #[test]
    fn structs_from_other_files_are_found() {
        let dir = std::env::temp_dir().join(format!("perox-structs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        std::fs::write(
            dir.join("parts/tile.slint"),
            "export struct Layer { c: string }\nexport struct DrinkData { layers: [Layer] }\nexport component Tile {}\n",
        )
        .unwrap();
        let main = dir.join("main.slint");
        let src = "export component W inherits Window {}\n";
        std::fs::write(&main, src).unwrap();
        let found = struct_imports(&main, src, "[DrinkData]", &dir);
        assert_eq!(
            found,
            [("DrinkData".to_owned(), "parts/tile.slint".to_owned())]
        );
        let src2 = "import { DrinkData } from \"parts/tile.slint\";\nexport component W inherits Window {}\n";
        assert!(struct_imports(&main, src2, "[DrinkData]", &dir).is_empty());
        assert!(struct_imports(&main, src, "string", &dir).is_empty());
    }
}
