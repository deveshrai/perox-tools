//! The app's look, kept in `ui/slint-project.json` (Slint's project file) under
//! `metadata`, which Slint's compiler ignores:
//!
//! ```json
//! { "edition": "1.19", "main": "app.slint",
//!   "metadata": { "peroxide": { "style": "fluent", "color-scheme": "dark" } } }
//! ```
//!
//! No style means each OS's own (Cupertino on macOS, Material on Android, Fluent
//! elsewhere); no colour scheme means following the system's light or dark setting. The
//! build helper and the designer's preview both read it, so they agree.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const FILE_NAME: &str = "slint-project.json";

/// Styles an app can use (all built into Slint).
pub const STYLES: &[&str] = &["fluent", "cupertino", "material", "cosmic"];
pub const COLOR_SCHEMES: &[&str] = &["light", "dark"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiSettings {
    /// `fluent`, `cupertino`, `material`, `cosmic`; `None`: the OS's own.
    pub style: Option<String>,
    /// `light` or `dark`; `None`: follow the system.
    pub color_scheme: Option<String>,
    /// The SQLite file the designer runs bound queries on to show data while you design
    /// (relative to the crate root). The app opens its own database at run time.
    pub database: Option<String>,
}

impl UiSettings {
    /// The Slint style name to compile with for `os` (a `target_os` value), or `None` to
    /// leave Slint's default: `fluent-dark`, `cupertino`, …
    pub fn style_for(&self, os: &str) -> Option<String> {
        if self.style.is_none() && self.color_scheme.is_none() {
            return None;
        }
        let base = self
            .style
            .clone()
            .unwrap_or_else(|| native_style(os).to_owned());
        Some(match &self.color_scheme {
            Some(scheme) => format!("{base}-{scheme}"),
            None => base,
        })
    }
}

/// The style Slint picks by default on `os`.
pub fn native_style(os: &str) -> &'static str {
    match os {
        "macos" | "ios" => "cupertino",
        "android" => "material",
        _ => "fluent",
    }
}

/// Reads the settings from a project file's text. Anything missing or malformed reads as
/// the default.
pub fn read(json: &str) -> UiSettings {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let p = &v["metadata"]["peroxide"];
    let get = |k: &str, allowed: &[&str]| {
        p[k].as_str()
            .filter(|s| allowed.contains(s))
            .map(str::to_owned)
    };
    UiSettings {
        style: get("style", STYLES),
        color_scheme: get("color-scheme", COLOR_SCHEMES),
        database: p["database"].as_str().map(str::to_owned),
    }
}

/// Writes the settings into a project file's text, keeping everything else.
pub fn write(json: &str, s: &UiSettings) -> String {
    let mut v: Value = serde_json::from_str(json).unwrap_or_else(|_| Value::Object(Map::new()));
    if !v.is_object() {
        v = Value::Object(Map::new());
    }
    let root = v.as_object_mut().expect("an object");
    if !root.contains_key("edition") {
        root.insert("edition".into(), "1.19".into());
    }
    let meta = root
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()));
    if !meta.is_object() {
        *meta = Value::Object(Map::new());
    }
    let meta = meta.as_object_mut().expect("an object");
    let mut ours = Map::new();
    if let Some(style) = &s.style {
        ours.insert("style".into(), style.clone().into());
    }
    if let Some(scheme) = &s.color_scheme {
        ours.insert("color-scheme".into(), scheme.clone().into());
    }
    if let Some(db) = &s.database {
        ours.insert("database".into(), db.clone().into());
    }
    if ours.is_empty() {
        meta.remove("peroxide");
    } else {
        meta.insert("peroxide".into(), Value::Object(ours));
    }
    if meta.is_empty() {
        root.remove("metadata");
    }
    let mut out = serde_json::to_string_pretty(&v).unwrap_or_default();
    out.push('\n');
    out
}

/// The project file that governs a `.slint` file in `dir`: the nearest
/// `slint-project.json` in `dir` or above, stopping at the crate root (`Cargo.toml`).
pub fn find(dir: &Path) -> Option<PathBuf> {
    let mut at = Some(dir);
    while let Some(d) = at {
        let candidate = d.join(FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if d.join("Cargo.toml").is_file() {
            return None;
        }
        at = d.parent();
    }
    None
}

/// The settings for a `.slint` file in `dir` (defaults if there's no project file).
pub fn settings_for(dir: &Path) -> UiSettings {
    find(dir)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| read(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "{ \"edition\": \"1.19\", \"main\": \"app.slint\" }\n";

    #[test]
    fn defaults_and_round_trip() {
        assert_eq!(read(TEMPLATE), UiSettings::default());
        assert_eq!(read("not json"), UiSettings::default());
        let s = UiSettings {
            style: Some("fluent".into()),
            color_scheme: Some("dark".into()),
            database: Some("data/app.db".into()),
        };
        let out = write(TEMPLATE, &s);
        assert_eq!(read(&out), s);
        assert!(out.contains("\"main\": \"app.slint\""), "{out}");
        assert!(out.contains("\"peroxide\""), "{out}");
        // Back to the defaults: no metadata left behind.
        let back = write(&out, &UiSettings::default());
        assert!(!back.contains("metadata"), "{back}");
        assert!(back.contains("\"edition\": \"1.19\""));
        // Unknown values are ignored.
        let odd = "{\"metadata\":{\"peroxide\":{\"style\":\"qt-plus\",\"color-scheme\":\"blue\"}}}";
        assert_eq!(read(odd), UiSettings::default());
    }

    #[test]
    fn compile_style_per_os() {
        let none = UiSettings::default();
        assert_eq!(none.style_for("macos"), None);
        let dark = UiSettings {
            color_scheme: Some("dark".into()),
            ..Default::default()
        };
        assert_eq!(dark.style_for("macos").as_deref(), Some("cupertino-dark"));
        assert_eq!(dark.style_for("windows").as_deref(), Some("fluent-dark"));
        let material = UiSettings {
            style: Some("material".into()),
            ..Default::default()
        };
        assert_eq!(material.style_for("linux").as_deref(), Some("material"));
    }

    #[test]
    fn found_up_to_the_crate_root() {
        let root = std::env::temp_dir().join(format!("perox-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ui/forms")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        assert_eq!(find(&root.join("ui/forms")), None);
        std::fs::write(root.join("ui").join(FILE_NAME), TEMPLATE).unwrap();
        assert_eq!(
            find(&root.join("ui/forms")),
            Some(root.join("ui").join(FILE_NAME))
        );
    }
}
