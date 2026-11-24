//! Images and fonts a form uses. They live next to the `.slint` file (`ui/images/`,
//! `ui/fonts/`) and are referenced relative to it, so Slint embeds them in the app at
//! build time and the designer's preview finds them too:
//!
//! ```slint
//! import "fonts/Inter.ttf";                 // makes the family available
//! Image { source: @image-url("images/logo.png"); }
//! ```

use std::path::{Component, Path, PathBuf};

/// File types Slint's `@image-url` loads.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "svg", "gif", "webp", "bmp"];
pub const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc"];

fn has_ext(p: &Path, exts: &[&str]) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub fn is_image(p: &Path) -> bool {
    has_ext(p, IMAGE_EXTENSIONS)
}

pub fn is_font(p: &Path) -> bool {
    has_ext(p, FONT_EXTENSIONS)
}

/// Images under `dir` (the `.slint` file's folder), as paths relative to it with `/`,
/// sorted. Skips `target` and hidden folders.
pub fn images(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(dir, dir, 0, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, depth + 1, out);
        } else if is_image(&p)
            && let Some(rel) = relative(root, &p)
        {
            out.push(rel);
        }
    }
}

/// `file` relative to `dir`, with `/`, if it's inside it.
pub fn relative(dir: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(dir).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(parts.join("/"))
}

/// Brings `file` into the form's folder `dir`: a file already inside it is used where it
/// is; anything else is copied to `dir/<sub>/` (`images`, `fonts`), with a number added
/// if a different file has that name. Returns the path relative to `dir`.
pub fn import_file(file: &Path, dir: &Path, sub: &str) -> std::io::Result<String> {
    let file = file.canonicalize()?;
    let dir_c = dir.canonicalize()?;
    if let Some(rel) = relative(&dir_c, &file) {
        return Ok(rel);
    }
    let target_dir = dir.join(sub);
    std::fs::create_dir_all(&target_dir)?;
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let ext = file
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let bytes = std::fs::read(&file)?;
    for n in 0.. {
        let name = if n == 0 {
            format!("{stem}{ext}")
        } else {
            format!("{stem}-{n}{ext}")
        };
        let target: PathBuf = target_dir.join(&name);
        match std::fs::read(&target) {
            // The same file is already there.
            Ok(existing) if existing == bytes => return Ok(format!("{sub}/{name}")),
            Ok(_) => continue,
            Err(_) => {
                std::fs::write(&target, &bytes)?;
                return Ok(format!("{sub}/{name}"));
            }
        }
    }
    unreachable!()
}

/// `images/logo.png` → `@image-url("images/logo.png")`.
pub fn image_url(rel: &str) -> String {
    format!("@image-url({})", perox_slint::syntax::quote(rel))
}

/// The path inside `@image-url("…")`, if that's what `expr` is.
pub fn image_path(expr: &str) -> Option<String> {
    let inner = expr
        .trim()
        .strip_prefix("@image-url(")?
        .strip_suffix(')')?
        .trim();
    // A second argument (nine-slice) is kept out of the path.
    let lit = inner.split(',').next()?.trim();
    perox_slint::Value::parse(lit).as_str().map(str::to_owned)
}

/// The family name in a font file (`Inter`), from its name table.
pub fn font_family(file: &Path) -> Option<String> {
    let data = std::fs::read(file).ok()?;
    let face = ttf_parser::Face::parse(&data, 0).ok()?;
    let names = face.names();
    let pick = |id: u16| {
        names
            .into_iter()
            .filter(|n| n.name_id == id && n.is_unicode())
            .find_map(|n| n.to_string())
    };
    // Typographic family first (16), else the legacy family (1).
    pick(16).or_else(|| pick(1))
}

/// Families of the fonts `doc` imports (`import "fonts/X.ttf";`), with their files,
/// resolved against the form's folder `dir`.
pub fn imported_fonts(doc: &perox_slint::Document, dir: &Path) -> Vec<(String, String)> {
    doc.imports
        .iter()
        .filter(|i| i.names.is_empty() && is_font(Path::new(&i.from)))
        .filter_map(|i| Some((font_family(&dir.join(&i.from))?, i.from.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("perox-res-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn images_are_found_and_imported_once() {
        let ui = temp("ui");
        std::fs::create_dir_all(ui.join("images/sub")).unwrap();
        std::fs::write(ui.join("images/a.png"), b"a").unwrap();
        std::fs::write(ui.join("images/sub/b.SVG"), b"b").unwrap();
        std::fs::write(ui.join("notes.txt"), b"x").unwrap();
        assert_eq!(images(&ui), vec!["images/a.png", "images/sub/b.SVG"]);

        // Inside the folder: used where it is.
        assert_eq!(
            import_file(&ui.join("images/a.png"), &ui, "images").unwrap(),
            "images/a.png"
        );

        // Outside: copied; the same file again reuses the copy; another file with the same
        // name gets a number.
        let outside = temp("outside");
        std::fs::write(outside.join("logo.png"), b"logo").unwrap();
        assert_eq!(
            import_file(&outside.join("logo.png"), &ui, "images").unwrap(),
            "images/logo.png"
        );
        assert_eq!(
            import_file(&outside.join("logo.png"), &ui, "images").unwrap(),
            "images/logo.png"
        );
        let other = temp("other");
        std::fs::write(other.join("logo.png"), b"different").unwrap();
        assert_eq!(
            import_file(&other.join("logo.png"), &ui, "images").unwrap(),
            "images/logo-1.png"
        );
        assert_eq!(
            std::fs::read(ui.join("images/logo-1.png")).unwrap(),
            b"different"
        );
    }

    #[test]
    fn image_urls_round_trip() {
        assert_eq!(image_url("images/a.png"), "@image-url(\"images/a.png\")");
        assert_eq!(
            image_path("@image-url(\"images/a.png\")").as_deref(),
            Some("images/a.png")
        );
        assert_eq!(
            image_path("@image-url(\"b.png\", nine-slice(1 2 3 4))").as_deref(),
            Some("b.png")
        );
        assert_eq!(image_path("root.icon"), None);
    }
}
