//! Translations, the way Slint does them: text written as `@tr("Save")` is looked up in
//! gettext `.po` files that the build bundles into the app,
//! `ui/translations/<lang>/LC_MESSAGES/<crate>.po`. The app picks the system's language
//! at start (or `slint::select_bundled_translation("de")`).
//!
//! Each message's context is the component it's in (Slint's default), so the same
//! word can translate differently in two windows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where translations live, relative to the `ui/` folder.
pub const DIR: &str = "translations";

/// One translatable string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Msg {
    /// `msgctxt`: the component, or the explicit `@tr("ctx" => …)`.
    pub context: String,
    pub id: String,
    /// `"one" | "many" % n`: the plural form's text.
    pub plural: Option<String>,
}

/// Every `@tr(…)` in a `.slint` source, with its context, in order, without repeats.
pub fn extract(src: &str) -> Vec<Msg> {
    let doc = crate::parse(src);
    // Components by span, to find which one a `@tr` is in.
    let comps: Vec<(std::ops::Range<usize>, String)> = doc
        .components
        .iter()
        .map(|c| (c.root.span.clone(), c.name.clone()))
        .collect();
    let mut out: Vec<Msg> = Vec::new();
    let mut at = 0;
    while let Some(i) = src[at..].find("@tr(") {
        let start = at + i;
        at = start + 4;
        if in_comment(src, start) {
            continue;
        }
        let Some((first, rest)) = string_lit(&src[at..]) else {
            continue;
        };
        let rest_trim = rest.trim_start();
        let (context, id, rest) = if let Some(r) = rest_trim.strip_prefix("=>") {
            match string_lit(r) {
                Some((id, rest)) => (Some(first), id, rest),
                None => continue,
            }
        } else {
            (None, first, rest)
        };
        let plural = rest
            .trim_start()
            .strip_prefix('|')
            .and_then(string_lit)
            .map(|(p, _)| p);
        let context = context.unwrap_or_else(|| {
            comps
                .iter()
                .find(|(r, _)| r.contains(&start))
                .map(|(_, n)| n.clone())
                .unwrap_or_default()
        });
        let m = Msg {
            context,
            id,
            plural,
        };
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

fn in_comment(src: &str, at: usize) -> bool {
    let line = &src[src[..at].rfind('\n').map_or(0, |i| i + 1)..at];
    line.contains("//")
}

/// A string literal at the start of `s` (after spaces): its value and the text after it.
fn string_lit(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let body = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &body[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
    None
}

/// Is `value` (a property's source) a plain translatable string, `@tr("…")`? Its text.
pub fn tr_text(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix("@tr(")?.strip_suffix(')')?;
    let (text, rest) = string_lit(inner)?;
    rest.trim().is_empty().then_some(text)
}

/// `Save` → `@tr("Save")`.
pub fn tr(text: &str) -> String {
    format!("@tr({})", crate::syntax::quote(text))
}

/// A `.po` file: translations keyed by `(context, id)`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Po {
    /// The header's text (`Language: de\n…`), kept as is.
    pub header: String,
    pub entries: BTreeMap<(String, String), Entry>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub plural: Option<String>,
    /// `msgstr`, or `msgstr[0..]` for plurals. Empty: not translated yet.
    pub text: Vec<String>,
}

impl Po {
    pub fn get(&self, context: &str, id: &str) -> Option<&str> {
        self.entries
            .get(&(context.to_owned(), id.to_owned()))
            .and_then(|e| e.text.first())
            .map(String::as_str)
            .filter(|t| !t.is_empty())
    }

    pub fn set(&mut self, context: &str, id: &str, text: &str) {
        let e = self
            .entries
            .entry((context.to_owned(), id.to_owned()))
            .or_default();
        if e.text.is_empty() {
            e.text.push(String::new());
        }
        e.text[0] = text.to_owned();
    }
}

fn unquote_po(s: &str) -> String {
    string_lit(s).map(|(v, _)| v).unwrap_or_default()
}

fn quote_po(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Reads a `.po` file. Obsolete (`#~`) entries and comments are dropped.
pub fn parse_po(text: &str) -> Po {
    #[derive(Default)]
    struct Cur {
        ctx: String,
        id: Option<String>,
        plural: Option<String>,
        strs: Vec<String>,
        field: u8, // 0 ctx, 1 id, 2 plural, 3 str
    }
    let mut po = Po::default();
    let mut cur = Cur::default();
    let flush = |cur: &mut Cur, po: &mut Po| {
        if let Some(id) = cur.id.take() {
            if id.is_empty() && cur.ctx.is_empty() {
                po.header = cur.strs.first().cloned().unwrap_or_default();
            } else {
                po.entries.insert(
                    (std::mem::take(&mut cur.ctx), id),
                    Entry {
                        plural: cur.plural.take(),
                        text: std::mem::take(&mut cur.strs),
                    },
                );
            }
        }
        *cur = Cur::default();
    };
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            if l.is_empty() {
                flush(&mut cur, &mut po);
            }
            continue;
        }
        if let Some(r) = l.strip_prefix("msgctxt ") {
            flush(&mut cur, &mut po);
            cur.ctx = unquote_po(r);
            cur.field = 0;
        } else if let Some(r) = l.strip_prefix("msgid_plural ") {
            cur.plural = Some(unquote_po(r));
            cur.field = 2;
        } else if let Some(r) = l.strip_prefix("msgid ") {
            if cur.id.is_some() {
                flush(&mut cur, &mut po);
            }
            cur.id = Some(unquote_po(r));
            cur.field = 1;
        } else if let Some(r) = l.strip_prefix("msgstr") {
            let r = r.trim_start();
            let r = match r.strip_prefix('[') {
                Some(x) => x.split_once(']').map_or(x, |(_, rest)| rest).trim_start(),
                None => r,
            };
            cur.strs.push(unquote_po(r));
            cur.field = 3;
        } else if l.starts_with('"') {
            let more = unquote_po(l);
            match cur.field {
                0 => cur.ctx.push_str(&more),
                1 => {
                    if let Some(id) = cur.id.as_mut() {
                        id.push_str(&more);
                    }
                }
                2 => {
                    if let Some(p) = cur.plural.as_mut() {
                        p.push_str(&more);
                    }
                }
                _ => {
                    if let Some(s) = cur.strs.last_mut() {
                        s.push_str(&more);
                    }
                }
            }
        }
    }
    flush(&mut cur, &mut po);
    po
}

/// Writes a `.po` file for `lang` with exactly `msgs`, in that order, keeping the
/// translations `old` has for them.
pub fn write_po(lang: &str, msgs: &[Msg], old: &Po) -> String {
    let header = if old.header.is_empty() {
        format!(
            "Language: {lang}\nMIME-Version: 1.0\nContent-Type: text/plain; charset=UTF-8\nContent-Transfer-Encoding: 8bit\nPlural-Forms: {}\n",
            plural_forms(lang)
        )
    } else {
        old.header.clone()
    };
    let mut out = String::new();
    out.push_str("msgid \"\"\nmsgstr \"\"\n");
    for line in header.split_inclusive('\n') {
        out.push_str(&quote_po(line));
        out.push('\n');
    }
    for m in msgs {
        let prev = old.entries.get(&(m.context.clone(), m.id.clone()));
        out.push('\n');
        if !m.context.is_empty() {
            out.push_str(&format!("msgctxt {}\n", quote_po(&m.context)));
        }
        out.push_str(&format!("msgid {}\n", quote_po(&m.id)));
        match &m.plural {
            Some(p) => {
                out.push_str(&format!("msgid_plural {}\n", quote_po(p)));
                let n = plural_count(lang);
                for i in 0..n {
                    let t = prev
                        .and_then(|e| e.text.get(i))
                        .cloned()
                        .unwrap_or_default();
                    out.push_str(&format!("msgstr[{i}] {}\n", quote_po(&t)));
                }
            }
            None => {
                let t = prev
                    .and_then(|e| e.text.first())
                    .cloned()
                    .unwrap_or_default();
                out.push_str(&format!("msgstr {}\n", quote_po(&t)));
            }
        }
    }
    out
}

/// The gettext plural rule for common languages (two forms unless known otherwise).
pub fn plural_forms(lang: &str) -> &'static str {
    match base(lang) {
        "ja" | "zh" | "ko" | "vi" | "th" | "id" => "nplurals=1; plural=0;",
        "fr" | "pt" => "nplurals=2; plural=(n > 1);",
        "ru" | "uk" | "pl" | "cs" | "sk" | "hr" | "sr" => {
            "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);"
        }
        _ => "nplurals=2; plural=(n != 1);",
    }
}

fn plural_count(lang: &str) -> usize {
    plural_forms(lang)
        .split("nplurals=")
        .nth(1)
        .and_then(|r| r.split(';').next())
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(2)
}

fn base(lang: &str) -> &str {
    lang.split(['-', '_', '@']).next().unwrap_or(lang)
}

/// `ui/translations/<lang>/LC_MESSAGES/<domain>.po`.
pub fn po_path(ui_dir: &Path, lang: &str, domain: &str) -> PathBuf {
    ui_dir
        .join(DIR)
        .join(lang)
        .join("LC_MESSAGES")
        .join(format!("{domain}.po"))
}

/// The languages with a translation folder, sorted.
pub fn languages(ui_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(ui_dir.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("LC_MESSAGES").is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The crate name that is the gettext domain: `[package] name` in the nearest
/// `Cargo.toml` at or above `dir`.
pub fn domain_for(dir: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(d) = at {
        if let Ok(toml) = std::fs::read_to_string(d.join("Cargo.toml")) {
            let mut in_package = false;
            for line in toml.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    in_package = l == "[package]";
                } else if in_package
                    && let Some(v) = l.strip_prefix("name")
                    && let Some(v) = v.trim_start().strip_prefix('=')
                {
                    return Some(v.trim().trim_matches('"').to_owned());
                }
            }
            return None;
        }
        at = d.parent();
    }
    None
}

/// Every message in the `.slint` files of `ui_dir` (sorted by file, in source order).
pub fn extract_dir(ui_dir: &Path) -> Vec<Msg> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_slint(ui_dir, &mut files, 0);
    files.sort();
    let mut out: Vec<Msg> = Vec::new();
    for f in files {
        if let Ok(src) = std::fs::read_to_string(&f) {
            for m in extract(&src) {
                if !out.contains(&m) {
                    out.push(m);
                }
            }
        }
    }
    out
}

fn collect_slint(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') || name == DIR {
            continue;
        }
        if p.is_dir() {
            collect_slint(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x == "slint") {
            out.push(p);
        }
    }
}

/// Brings every language's `.po` up to date with the strings in `ui_dir` (new ones added
/// untranslated, gone ones dropped), and creates `lang`'s if given. Returns the files
/// written.
pub fn sync(ui_dir: &Path, domain: &str, add_lang: Option<&str>) -> std::io::Result<Vec<PathBuf>> {
    let msgs = extract_dir(ui_dir);
    let mut langs = languages(ui_dir);
    if let Some(l) = add_lang
        && !langs.iter().any(|x| x == l)
    {
        langs.push(l.to_owned());
    }
    let mut written = Vec::new();
    for lang in langs {
        let path = po_path(ui_dir, &lang, domain);
        let old = std::fs::read_to_string(&path)
            .map(|t| parse_po(&t))
            .unwrap_or_default();
        let text = write_po(&lang, &msgs, &old);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&path, text)?;
            written.push(path);
        }
    }
    Ok(written)
}

/// Is `lang` a plausible language tag (`de`, `pt_BR`, `zh-Hans`)?
pub fn is_language_tag(lang: &str) -> bool {
    let mut parts = lang.split(['-', '_']);
    parts
        .next()
        .is_some_and(|p| (2..=3).contains(&p.len()) && p.chars().all(|c| c.is_ascii_lowercase()))
        && parts.all(|p| (2..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"import { Button } from "std-widgets.slint";
export component MainWindow inherits Window {
    title: @tr("Notes");
    // @tr("not this")
    Button { text: @tr("Save"); }
    Button { text: @tr("Menu" => "Save"); }
    Text { text: @tr("{} file" | "{} files" % 3); }
    Text { text: @tr("Hello, {}", "x"); }
    Text { text: @tr("Save"); }
}
component Other {
    Text { text: @tr("Save"); }
}
"#;

    #[test]
    fn extracts_with_contexts() {
        let m = extract(SRC);
        let ids: Vec<(&str, &str)> = m
            .iter()
            .map(|m| (m.context.as_str(), m.id.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("MainWindow", "Notes"),
                ("MainWindow", "Save"),
                ("Menu", "Save"),
                ("MainWindow", "{} file"),
                ("MainWindow", "Hello, {}"),
                ("Other", "Save"),
            ]
        );
        assert_eq!(m[3].plural.as_deref(), Some("{} files"));
        assert_eq!(tr_text("@tr(\"Save\")").as_deref(), Some("Save"));
        assert_eq!(tr_text("@tr(\"Hi {}\", name)"), None);
        assert_eq!(tr("Say \"hi\""), "@tr(\"Say \\\"hi\\\"\")");
    }

    #[test]
    fn po_round_trip_keeps_translations() {
        let msgs = extract(SRC);
        let po = parse_po(&write_po("de", &msgs, &Po::default()));
        assert!(po.header.contains("Language: de"));
        assert_eq!(po.entries.len(), msgs.len());
        let mut po = po;
        po.set("MainWindow", "Save", "Speichern");
        po.set("Menu", "Save", "Sichern");
        let text = write_po("de", &msgs, &po);
        assert!(
            text.contains("msgctxt \"MainWindow\"\nmsgid \"Save\"\nmsgstr \"Speichern\""),
            "{text}"
        );
        assert!(
            text.contains("msgid_plural \"{} files\"\nmsgstr[0] \"\"\nmsgstr[1] \"\""),
            "{text}"
        );
        let again = parse_po(&text);
        assert_eq!(again.get("MainWindow", "Save"), Some("Speichern"));
        assert_eq!(again.get("Menu", "Save"), Some("Sichern"));
        assert_eq!(again.get("MainWindow", "Notes"), None);
        // A string that's gone from the form is dropped.
        let fewer: Vec<Msg> = msgs
            .iter()
            .filter(|m| m.context != "Menu")
            .cloned()
            .collect();
        let text = write_po("de", &fewer, &again);
        assert!(!text.contains("Sichern"));
        assert!(text.contains("Speichern"));
    }

    #[test]
    fn sync_writes_every_language() {
        let root = std::env::temp_dir().join(format!("perox-i18n-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ui = root.join("ui");
        std::fs::create_dir_all(&ui).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"notes\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(ui.join("main.slint"), SRC).unwrap();
        assert_eq!(domain_for(&ui).as_deref(), Some("notes"));
        let w = sync(&ui, "notes", Some("de")).unwrap();
        assert_eq!(w, vec![po_path(&ui, "de", "notes")]);
        assert_eq!(languages(&ui), vec!["de"]);
        // Nothing changed: nothing written.
        assert!(sync(&ui, "notes", None).unwrap().is_empty());
        assert!(is_language_tag("de") && is_language_tag("pt_BR") && is_language_tag("zh-Hans"));
        assert!(!is_language_tag("../x") && !is_language_tag("D"));
    }
}
