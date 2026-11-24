//! Data-aware widgets: a list or table bound to a SQL query, in plain Slint.
//!
//! ```slint
//! in-out property <[[StandardListViewItem]]> people_rows <=> people.rows;
//! out property <string> people_query: "SELECT name, email FROM people";
//! people := StandardTableView { columns: [{ title: "name" }, { title: "email" }]; }
//! ```
//!
//! The build sees `people_query` next to the exposed `people_rows` and generates
//! `MainWindow::load_people(&db)`, which runs the query with `perox_ui::data::Database`
//! and fills the table. A `StandardListView` (`<id>_model`) and a `ComboBox`
//! (`<id>_model`) work the same way, with the query's first column.

use crate::{Component, Element};

/// The property a widget's data goes into, the Slint type to expose it as, and the
/// `perox_ui::data::Table` method that makes its model.
pub fn target(kind: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match kind {
        "StandardTableView" => ("rows", "[[StandardListViewItem]]", "rows_model"),
        "StandardListView" => ("model", "[StandardListViewItem]", "list_model"),
        "ComboBox" => ("model", "[string]", "strings_model"),
        _ => return None,
    })
}

/// `people` → `people_query`.
pub fn query_property(id: &str) -> String {
    format!("{}_query", id.replace('-', "_"))
}

/// `people` (a table) → `people_rows`.
pub fn data_property(id: &str, kind: &str) -> Option<String> {
    target(kind).map(|(p, _, _)| format!("{}_{p}", id.replace('-', "_")))
}

/// A widget bound to a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub id: String,
    pub kind: String,
    /// Where the query is (`people_query`) and its SQL as written (a string literal's
    /// value), if it is one.
    pub query_property: String,
    pub sql: Option<String>,
    /// The exposed data property (`people_rows`).
    pub data_property: String,
}

fn elements<'a>(el: &'a Element, out: &mut Vec<&'a Element>) {
    out.push(el);
    for c in &el.children {
        elements(c, out);
    }
}

/// The component's bound widgets: an element with an id, its `<id>_query` string
/// property and its data exposed as `<id>_rows` / `<id>_model`.
pub fn bindings(c: &Component, src: Option<&str>) -> Vec<Binding> {
    let mut all = Vec::new();
    elements(&c.root, &mut all);
    let norm = |s: &str| s.replace('-', "_");
    all.into_iter()
        .filter_map(|el| {
            let id = el.id.as_deref()?;
            let data = data_property(id, &el.kind)?;
            let query = query_property(id);
            let q = c.properties.iter().find(|p| norm(&p.name) == query)?;
            c.properties.iter().find(|p| norm(&p.name) == data)?;
            let sql = src
                .and_then(|s| s.get(q.span.clone()))
                .and_then(|t| {
                    t.split_once(':')
                        .map(|(_, v)| v.trim().trim_end_matches(';').trim().to_owned())
                })
                .and_then(|v| crate::Value::parse(&v).as_str().map(str::to_owned));
            Some(Binding {
                id: id.to_owned(),
                kind: el.kind.clone(),
                query_property: q.name.clone(),
                sql,
                data_property: data,
            })
        })
        .collect()
}

/// `impl <Name> { pub fn load_<id>(&self, db) … }` for each bound widget, or `None`.
pub fn loaders(c: &Component) -> Option<String> {
    let b = bindings(c, None);
    if b.is_empty() {
        return None;
    }
    let mut body = String::new();
    for x in &b {
        let (_, _, model) = target(&x.kind)?;
        let id = x.id.replace('-', "_");
        body.push_str(&format!(
            "    /// Runs `{q}` on `db` and shows the result in `{id}`. Returns the number of rows.\n    pub fn load_{id}(&self, db: &perox_ui::data::Database) -> Result<usize, perox_ui::data::Error> {{\n        let table = db.query(&self.get_{q}())?;\n        self.set_{d}(table.{model}());\n        Ok(table.len())\n    }}\n",
            q = crate::codegen::rust_name(&x.query_property),
            d = crate::codegen::rust_name(&x.data_property),
        ));
    }
    Some(format!("impl {} {{\n{body}}}\n", c.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "import { StandardTableView, StandardListView } from \"std-widgets.slint\";\nexport component W inherits Window {\n    in-out property <[[StandardListViewItem]]> people_rows <=> people.rows;\n    out property <string> people_query: \"SELECT name FROM people\";\n    in-out property <[StandardListViewItem]> cities_model <=> cities.model;\n    VerticalLayout {\n        people := StandardTableView { }\n        cities := StandardListView { }\n        other := StandardTableView { }\n    }\n}\n";

    #[test]
    fn finds_bound_widgets_and_generates_loaders() {
        let doc = crate::parse(SRC);
        let c = &doc.components[0];
        let b = bindings(c, Some(SRC));
        assert_eq!(b.len(), 1, "cities has no query, other no data property");
        assert_eq!(b[0].id, "people");
        assert_eq!(b[0].sql.as_deref(), Some("SELECT name FROM people"));
        let code = loaders(c).unwrap();
        assert!(
            code.contains("pub fn load_people(&self, db: &perox_ui::data::Database)"),
            "{code}"
        );
        assert!(
            code.contains("db.query(&self.get_people_query())?"),
            "{code}"
        );
        assert!(
            code.contains("self.set_people_rows(table.rows_model());"),
            "{code}"
        );
    }
}
