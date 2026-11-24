//! Binding lists and tables to SQL queries ([`perox_slint::data`]) and showing the
//! results while you design.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use perox_slint::edit::{self, EditError};

use crate::free;

/// The crate folder (with `Cargo.toml`) at or above `dir`.
pub fn crate_root(dir: &Path) -> Option<PathBuf> {
    let mut at = Some(dir);
    while let Some(d) = at {
        if d.join("Cargo.toml").is_file() {
            return Some(d.to_owned());
        }
        at = d.parent();
    }
    None
}

/// Binds the widget at `path` to `sql`: exposes its data (`people_rows`), declares or
/// updates `people_query`, and for a table writes `columns` from the query's. Returns the
/// new source.
pub fn bind(
    src: &str,
    comp: &str,
    path: &[usize],
    sql: &str,
    columns: Option<&[String]>,
) -> Result<String, EditError> {
    let doc = perox_slint::parse(src);
    let c = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.to_owned()))?;
    let el = c.at(path).ok_or(EditError::NoSuchElement)?;
    let (prop, _, _) = perox_slint::data::target(&el.kind).ok_or(EditError::NoSuchElement)?;
    // Exposing names the element if needed, and gives `<id>_rows`.
    let mut work = crate::expose_property(src, comp, path, prop)?;
    let doc = perox_slint::parse(&work);
    let c = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.to_owned()))?;
    let el = c.at(path).ok_or(EditError::NoSuchElement)?;
    let id = el.id.clone().ok_or(EditError::NoSuchElement)?;
    let kind = el.kind.clone();
    let qname = perox_slint::data::query_property(&id);
    let literal = perox_slint::syntax::quote(sql);
    let existing = c
        .properties
        .iter()
        .find(|p| p.name.replace('-', "_") == qname)
        .map(|p| p.span.clone());
    match existing {
        Some(span) => {
            // `… people_query: "old";` → the new literal.
            let text = &work[span.clone()];
            let colon = text.rfind(':').ok_or(EditError::NoSuchElement)?;
            let start = span.start + colon + 1;
            let end = span.start + text.trim_end().trim_end_matches(';').len();
            work.replace_range(start..end, &format!(" {literal}"));
        }
        None => {
            let s = work.clone();
            free::step(
                &mut work,
                edit::add_property(&s, comp, &qname, "string", Some(&literal)),
            )?;
        }
    }
    if kind == "StandardTableView"
        && let Some(cols) = columns
        && !cols.is_empty()
    {
        let list = cols
            .iter()
            .map(|c| format!("{{ title: {} }}", perox_slint::syntax::quote(c)))
            .collect::<Vec<_>>()
            .join(", ");
        let s = work.clone();
        free::step(
            &mut work,
            edit::set_property(&s, comp, path, "columns", &format!("[{list}]")),
        )?;
    }
    Ok(work)
}

/// Adds `features = ["sqlite"]` to the `perox-ui` dependency in a `Cargo.toml`'s text,
/// for the generated `load_*` methods. `None` if it's there already or there's no
/// `perox-ui` line to change.
pub fn with_sqlite_feature(toml: &str) -> Option<String> {
    let line_start = toml
        .lines()
        .scan(0usize, |at, l| {
            let start = *at;
            *at += l.len() + 1;
            Some((start, l))
        })
        .find(|(_, l)| l.trim_start().starts_with("perox-ui") && l.contains('='))?;
    let (start, line) = line_start;
    if line.contains("\"sqlite\"") {
        return None;
    }
    let new_line = if let Some(i) = line.find("features") {
        // features = ["a"] → features = ["a", "sqlite"]
        let open = line[i..].find('[')? + i;
        let close = line[open..].find(']')? + open;
        let inner = line[open + 1..close].trim();
        let sep = if inner.is_empty() { "" } else { ", " };
        format!(
            "{}[{inner}{sep}\"sqlite\"]{}",
            &line[..open],
            &line[close + 1..]
        )
    } else if let Some(close) = line.rfind('}') {
        let before = line[..close].trim_end();
        format!("{before}, features = [\"sqlite\"] }}{}", &line[close + 1..])
    } else {
        // perox-ui = "0.5" → a table.
        let (name, version) = line.split_once('=')?;
        format!(
            "{}= {{ version = {}, features = [\"sqlite\"] }}",
            name,
            version.trim()
        )
    };
    let mut out = toml.to_owned();
    out.replace_range(start..start + line.len(), &new_line);
    Some(out)
}

/// The design database for a form in `form_dir` (`data.database`), if one is set.
pub fn design_database(form_dir: &Path) -> Option<PathBuf> {
    let db = perox_slint::project::settings_for(form_dir).database?;
    Some(crate_root(form_dir)?.join(db))
}

/// The rows to show while designing `comp` (source `src`): each bound widget's query run
/// on the design database `db`.
pub fn design_rows(
    cache: &mut Cache,
    db: &Path,
    src: &str,
    comp: &str,
) -> Vec<(String, String, Vec<Vec<String>>)> {
    let doc = perox_slint::parse(src);
    let Some(c) = doc.component(comp) else {
        return Vec::new();
    };
    perox_slint::data::bindings(c, Some(src))
        .into_iter()
        .filter_map(|b| {
            let t = cache.run(db, b.sql.as_deref()?).ok()?;
            Some((b.data_property, b.kind, t.rows))
        })
        .collect()
}

/// Query results kept while designing, keyed by database file (and its modified time)
/// and SQL.
#[derive(Debug, Default)]
pub struct Cache {
    results: HashMap<(PathBuf, Option<SystemTime>, String), Result<perox_ui::data::Table, String>>,
}

impl Cache {
    /// Runs `sql` on the SQLite file `db`, or gives the result from last time if neither
    /// changed.
    pub fn run(&mut self, db: &Path, sql: &str) -> Result<perox_ui::data::Table, String> {
        let mtime = std::fs::metadata(db).and_then(|m| m.modified()).ok();
        let key = (db.to_owned(), mtime, sql.to_owned());
        if let Some(r) = self.results.get(&key) {
            return r.clone();
        }
        let r = if db.is_file() {
            perox_ui::data::Database::open(&format!("sqlite:{}", db.display()))
                .and_then(|d| d.query(sql))
                .map_err(|e| e.to_string())
        } else {
            Err(format!("No database file at {}", db.display()))
        };
        if self.results.len() > 64 {
            self.results.clear();
        }
        self.results.insert(key, r.clone());
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "import { StandardTableView } from \"std-widgets.slint\";\nexport component W inherits Window {\n    VerticalLayout {\n        StandardTableView { }\n    }\n}\n";

    #[test]
    fn binding_a_table_declares_its_query_and_columns() {
        let cols = vec!["name".to_owned(), "email".to_owned()];
        let out = bind(
            SRC,
            "W",
            &[0, 0],
            "SELECT name, email FROM people",
            Some(&cols),
        )
        .unwrap();
        let doc = perox_slint::parse(&out);
        assert!(doc.errors.is_empty(), "{out}");
        let b = perox_slint::data::bindings(&doc.components[0], Some(&out));
        assert_eq!(b.len(), 1, "{out}");
        assert_eq!(b[0].sql.as_deref(), Some("SELECT name, email FROM people"));
        assert!(
            out.contains("columns: [{ title: \"name\" }, { title: \"email\" }]"),
            "{out}"
        );
        // Again with another query: updated in place, not declared twice.
        let again = bind(&out, "W", &[0, 0], "SELECT name FROM people", None).unwrap();
        assert_eq!(again.matches("_query").count(), 1, "{again}");
        let doc = perox_slint::parse(&again);
        assert_eq!(
            perox_slint::data::bindings(&doc.components[0], Some(&again))[0]
                .sql
                .as_deref(),
            Some("SELECT name FROM people")
        );
    }

    #[test]
    fn the_sqlite_feature_is_added_to_perox_ui() {
        let t = "[dependencies]\nperox-ui = { path = \"../perox-ui\" }\nslint = \"1\"\n";
        assert_eq!(
            with_sqlite_feature(t).unwrap(),
            "[dependencies]\nperox-ui = { path = \"../perox-ui\", features = [\"sqlite\"] }\nslint = \"1\"\n"
        );
        let t2 = "perox-ui = { path = \"x\", features = [\"a\"] }\n";
        assert_eq!(
            with_sqlite_feature(t2).unwrap(),
            "perox-ui = { path = \"x\", features = [\"a\", \"sqlite\"] }\n"
        );
        assert_eq!(with_sqlite_feature(&with_sqlite_feature(t).unwrap()), None);
        assert_eq!(
            with_sqlite_feature("perox-ui = \"0.5\"\n").unwrap(),
            "perox-ui = { version = \"0.5\", features = [\"sqlite\"] }\n"
        );
    }

    #[test]
    fn queries_run_on_a_sqlite_file() {
        let dir = std::env::temp_dir().join(format!("perox-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        let d = perox_ui::data::Database::open(&format!("sqlite:{}", db.display())).unwrap();
        d.execute("CREATE TABLE t (a TEXT); INSERT INTO t VALUES ('x'), ('y')")
            .unwrap();
        drop(d);
        let mut cache = Cache::default();
        let t = cache.run(&db, "SELECT a FROM t").unwrap();
        assert_eq!(t.rows, vec![vec!["x"], vec!["y"]]);
        assert!(cache.run(&db, "SELECT nope FROM t").is_err());
        assert!(cache.run(&dir.join("missing.db"), "SELECT 1").is_err());
    }
}
