use std::{path::Path, str::FromStr, time::Duration};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

pub const DB_FILE: &str = "onnextlabs.db";

/// Opens (creating the directory and file if missing) the SQLite database in `dir`
/// and applies any pending migrations from `migrations/`.
pub async fn open(dir: &Path) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating db dir {}: {e}", dir.display()))?;
    let path = dir.join(DB_FILE);

    let opts = SqliteConnectOptions::from_str("sqlite://")?
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .map_err(|e| format!("opening {}: {e}", path.display()))?;

    sqlx::migrate!().run(&pool).await?;
    tracing::info!("database ready at {}", path.display());
    Ok(pool)
}

/// A fresh in-memory database with every migration applied (one connection, so it lives as
/// long as the pool).
#[cfg(test)]
pub async fn test_db() -> SqlitePool {
    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    sqlx::migrate!().run(&db).await.unwrap();
    db
}

// ---- batches ----
//
// A batch is a list of statements decided up front, run in one transaction: either every
// statement applies or none does. Nothing reads a result *between* statements; a later
// statement that needs an earlier one's outcome looks it up in SQL (a subquery). Results are
// only inspected after the commit. That keeps each transaction to one round trip, so the
// database can sit behind a stateless HTTP service.

/// A SQLite value, as bound to a statement or read from a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Integer(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Integer(v.into())
    }
}
impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Value::Integer(v as i64)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Integer(v.into())
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Real(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_owned())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}
impl From<&String> for Value {
    fn from(v: &String) -> Self {
        Value::Text(v.clone())
    }
}
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Blob(v)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn into_string(self) -> Option<String> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Builds a `Vec<Value>` of statement arguments: `params![a, &b, c]`.
#[macro_export]
macro_rules! params {
    ($($v:expr),* $(,)?) => { vec![$($crate::db::Value::from($v)),*] };
}

#[derive(Default, Debug)]
pub struct Batch {
    stmts: Vec<(&'static str, Vec<Value>)>,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a statement; returns its index in the results. SQL is always a literal; values
    /// go in `args`, never into the text.
    pub fn push(&mut self, sql: &'static str, args: Vec<Value>) -> usize {
        self.stmts.push((sql, args));
        self.stmts.len() - 1
    }
}

/// What one statement of a batch did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct StmtResult {
    /// Rows changed by an INSERT/UPDATE/DELETE (not counting foreign key cascades).
    pub rows_affected: u64,
    pub last_insert_id: i64,
    /// Rows from a SELECT or a RETURNING clause.
    pub rows: Vec<Vec<Value>>,
}

impl StmtResult {
    /// The first column of the first row, if there is one.
    pub fn first(&self) -> Option<&Value> {
        self.rows.first().and_then(|r| r.first())
    }

    /// Takes the first column of the first row, if there is one.
    pub fn into_first(self) -> Option<Value> {
        self.rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next())
    }
}

/// Runs the batch in one transaction; any failing statement rolls the whole batch back.
pub async fn run(db: &SqlitePool, batch: Batch) -> sqlx::Result<Vec<StmtResult>> {
    use futures_util::TryStreamExt;
    use sqlx::{Either, Row, TypeInfo, ValueRef};

    let mut tx = db.begin().await?;
    let mut results = Vec::with_capacity(batch.stmts.len());
    for (sql, args) in &batch.stmts {
        let mut q = sqlx::query(*sql);
        for a in args {
            q = match a {
                Value::Null => q.bind(None::<i64>),
                Value::Integer(i) => q.bind(*i),
                Value::Real(f) => q.bind(*f),
                Value::Text(s) => q.bind(s.as_str()),
                Value::Blob(b) => q.bind(b.as_slice()),
            };
        }
        let mut res = StmtResult::default();
        // Deprecated for running several statements in one string; here it's always one. It's
        // the only call that yields both the rows and the change count.
        #[allow(deprecated)]
        let mut stream = q.fetch_many(&mut *tx);
        while let Some(item) = stream.try_next().await? {
            match item {
                Either::Left(done) => {
                    res.rows_affected += done.rows_affected();
                    res.last_insert_id = done.last_insert_rowid();
                }
                Either::Right(row) => {
                    let mut values = Vec::with_capacity(row.len());
                    for i in 0..row.len() {
                        let raw = row.try_get_raw(i)?;
                        let value = if raw.is_null() {
                            Value::Null
                        } else {
                            match raw.type_info().name() {
                                "INTEGER" => Value::Integer(row.try_get(i)?),
                                "REAL" => Value::Real(row.try_get(i)?),
                                "BLOB" => Value::Blob(row.try_get(i)?),
                                _ => Value::Text(row.try_get(i)?),
                            }
                        };
                        values.push(value);
                    }
                    res.rows.push(values);
                }
            }
        }
        drop(stream);
        results.push(res);
    }
    tx.commit().await?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_db() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, n REAL, b BLOB)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn batch_returns_rows_counts_and_typed_values() {
        let db = mem_db().await;
        let mut b = Batch::new();
        let ins = b.push(
            "INSERT INTO t (name, n, b) VALUES (?, ?, ?) RETURNING id",
            params!["a", 1.5, vec![1u8, 2]],
        );
        let upd = b.push("UPDATE t SET n = n + 1 WHERE name = ?", params!["a"]);
        let sel = b.push("SELECT id, name, n, b, NULL FROM t", params![]);
        let r = run(&db, b).await.unwrap();
        assert_eq!(r[ins].first(), Some(&Value::Integer(1)));
        assert_eq!(r[ins].last_insert_id, 1);
        assert_eq!(r[upd].rows_affected, 1);
        assert_eq!(
            r[sel].rows,
            vec![vec![
                Value::Integer(1),
                Value::Text("a".into()),
                Value::Real(2.5),
                Value::Blob(vec![1, 2]),
                Value::Null
            ]]
        );
    }

    #[tokio::test]
    async fn failing_statement_rolls_back_the_whole_batch() {
        let db = mem_db().await;
        let mut b = Batch::new();
        b.push("INSERT INTO t (name) VALUES (?)", params!["a"]);
        b.push("INSERT INTO t (name) VALUES (?)", params!["a"]);
        let err = run(&db, b).await.unwrap_err();
        assert!(matches!(err, sqlx::Error::Database(ref d) if d.is_unique_violation()));
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM t")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }
}
