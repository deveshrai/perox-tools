//! Data for list and table widgets.
//!
//! A [`Table`] (column names and rows of text) turns into the models Slint's widgets
//! take: [`Table::rows_model`] and [`Table::columns_model`] for a `StandardTableView`,
//! [`Table::list_model`] for a `StandardListView`, [`Table::strings_model`] for a
//! `ComboBox`.
//!
//! With the `sqlite` feature, [`Database`] runs SQL on a SQLite file (through sqlx) and
//! gives back a `Table`. A table bound to a query in the designer gets a generated
//! `load_<id>(&db)` method that runs its query and fills it:
//!
//! ```ignore
//! let db = perox_ui::data::Database::open("sqlite:people.db")?;
//! ui.load_people(&db)?;
//! ```

use std::rc::Rc;

use slint::{ModelRc, SharedString, StandardListViewItem, TableColumn, VecModel};

/// Rows of text with column names: a query result, a CSV, anything tabular.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(columns: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Self { columns, rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// `rows` for a `StandardTableView`.
    pub fn rows_model(&self) -> ModelRc<ModelRc<StandardListViewItem>> {
        let rows: Vec<ModelRc<StandardListViewItem>> = self
            .rows
            .iter()
            .map(|r| {
                let cells: Vec<StandardListViewItem> = r
                    .iter()
                    .map(|c| StandardListViewItem::from(c.as_str()))
                    .collect();
                ModelRc::from(Rc::new(VecModel::from(cells)))
            })
            .collect();
        ModelRc::from(Rc::new(VecModel::from(rows)))
    }

    /// `columns` for a `StandardTableView`: one per column, titled with its name.
    pub fn columns_model(&self) -> ModelRc<TableColumn> {
        let cols: Vec<TableColumn> = self
            .columns
            .iter()
            .map(|c| {
                let mut col = TableColumn::default();
                col.title = c.as_str().into();
                col
            })
            .collect();
        ModelRc::from(Rc::new(VecModel::from(cols)))
    }

    /// `model` for a `StandardListView`: the first column.
    pub fn list_model(&self) -> ModelRc<StandardListViewItem> {
        let items: Vec<StandardListViewItem> = self
            .rows
            .iter()
            .map(|r| StandardListViewItem::from(r.first().map_or("", String::as_str)))
            .collect();
        ModelRc::from(Rc::new(VecModel::from(items)))
    }

    /// `model` for a `ComboBox`: the first column.
    pub fn strings_model(&self) -> ModelRc<SharedString> {
        let items: Vec<SharedString> = self
            .rows
            .iter()
            .map(|r| SharedString::from(r.first().map_or("", String::as_str)))
            .collect();
        ModelRc::from(Rc::new(VecModel::from(items)))
    }
}

#[cfg(feature = "sqlite")]
pub use db::{Database, Error};

#[cfg(feature = "sqlite")]
mod db {
    use super::Table;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
    use sqlx::{AssertSqlSafe, Column, Executor, Row, SqlSafeStr, Statement, TypeInfo, ValueRef};
    use std::str::FromStr;

    pub use sqlx::Error;

    /// A SQLite database, queried from the UI thread: each call waits for its result.
    /// Keep queries quick there, or run them on a thread and hand the `Table` back with
    /// `slint::invoke_from_event_loop`.
    #[derive(Debug)]
    pub struct Database {
        rt: tokio::runtime::Runtime,
        pool: SqlitePool,
    }

    impl Database {
        /// Opens `url` (`sqlite:data.db`, `sqlite::memory:`), creating the file if it
        /// doesn't exist.
        pub fn open(url: &str) -> Result<Self, Error> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(Error::Io)?;
            let options = SqliteConnectOptions::from_str(url)?.create_if_missing(true);
            let pool = rt.block_on(
                SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect_with(options),
            )?;
            Ok(Self { rt, pool })
        }

        /// Runs a query and returns its rows as text (NULL is empty, a blob is
        /// `<n bytes>`).
        pub fn query(&self, sql: &str) -> Result<Table, Error> {
            self.rt.block_on(async {
                // Queries here are written by the app's author (or bound in the designer),
                // not built from user input.
                let rows = sqlx::query(AssertSqlSafe(sql.to_owned()))
                    .fetch_all(&self.pool)
                    .await?;
                let columns: Vec<String> = match rows.first() {
                    Some(r) => r.columns().iter().map(|c| c.name().to_owned()).collect(),
                    None => {
                        // No rows: ask the prepared statement for its columns.
                        let st = (&self.pool)
                            .prepare(AssertSqlSafe(sql.to_owned()).into_sql_str())
                            .await?;
                        st.columns().iter().map(|c| c.name().to_owned()).collect()
                    }
                };
                let mut out = Vec::with_capacity(rows.len());
                for r in &rows {
                    let mut cells = Vec::with_capacity(columns.len());
                    for i in 0..r.columns().len() {
                        cells.push(cell(r, i)?);
                    }
                    out.push(cells);
                }
                Ok(Table::new(columns, out))
            })
        }

        /// Runs a statement that changes data (`INSERT`, `CREATE TABLE`…); returns the
        /// rows it changed.
        pub fn execute(&self, sql: &str) -> Result<u64, Error> {
            self.rt.block_on(async {
                let r = sqlx::query(AssertSqlSafe(sql.to_owned()))
                    .execute(&self.pool)
                    .await?;
                Ok(r.rows_affected())
            })
        }
    }

    fn cell(r: &sqlx::sqlite::SqliteRow, i: usize) -> Result<String, Error> {
        let raw = r.try_get_raw(i)?;
        if raw.is_null() {
            return Ok(String::new());
        }
        let ty = raw.type_info().name().to_ascii_uppercase();
        Ok(match ty.as_str() {
            "INTEGER" | "BOOLEAN" => r.try_get::<i64, _>(i)?.to_string(),
            "REAL" => r.try_get::<f64, _>(i)?.to_string(),
            "BLOB" => format!("<{} bytes>", r.try_get::<Vec<u8>, _>(i)?.len()),
            _ => r.try_get::<String, _>(i)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn tables_become_models() {
        let t = Table::new(
            vec!["name".into(), "age".into()],
            vec![
                vec!["Ada".into(), "36".into()],
                vec!["Alan".into(), "41".into()],
            ],
        );
        let rows = t.rows_model();
        assert_eq!(rows.row_count(), 2);
        assert_eq!(rows.row_data(1).unwrap().row_data(0).unwrap().text, "Alan");
        assert_eq!(t.columns_model().row_data(1).unwrap().title, "age");
        assert_eq!(t.list_model().row_data(0).unwrap().text, "Ada");
        assert_eq!(t.strings_model().row_data(1).unwrap(), "Alan");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_queries_come_back_as_text() {
        let db = Database::open("sqlite::memory:").unwrap();
        db.execute("CREATE TABLE people (name TEXT, age INTEGER, score REAL, photo BLOB)")
            .unwrap();
        assert_eq!(
            db.execute(
                "INSERT INTO people VALUES ('Ada', 36, 9.5, x'0102'), ('Alan', NULL, 7, NULL)"
            )
            .unwrap(),
            2
        );
        let t = db
            .query("SELECT name, age, score, photo FROM people ORDER BY name")
            .unwrap();
        assert_eq!(t.columns, vec!["name", "age", "score", "photo"]);
        assert_eq!(t.rows[0], vec!["Ada", "36", "9.5", "<2 bytes>"]);
        assert_eq!(t.rows[1], vec!["Alan", "", "7", ""]);
        let empty = db.query("SELECT name FROM people WHERE age > 100").unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.columns, vec!["name"]);
        assert!(db.query("SELECT nope FROM nowhere").is_err());
    }
}
