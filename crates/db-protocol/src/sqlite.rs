//! Running statements and batches against a SQLite pool.

use futures_util::TryStreamExt;
use sqlx::sqlite::{SqliteConnection, SqlitePool, SqliteRow};
use sqlx::{AssertSqlSafe, Column, Either, Row, TypeInfo, ValueRef};

use crate::{Batch, Error, ErrorKind, Statement, StmtResult, Value};

/// Runs one statement on its own (SQLite makes a single statement atomic by itself).
pub async fn run_statement(db: &SqlitePool, stmt: &Statement) -> Result<StmtResult, Error> {
    let mut conn = db.acquire().await.map_err(from_sqlx)?;
    execute(&mut conn, stmt).await.map_err(from_sqlx)
}

/// Runs the batch in one transaction: every statement applies or none does.
///
/// `BEGIN IMMEDIATE` takes the write lock up front, so batches run one at a time and a
/// statement can rely on nothing else writing while its batch runs (e.g. "the newest row is
/// the one this batch inserted"). Reads outside batches carry on alongside (WAL).
pub async fn run_batch(db: &SqlitePool, batch: &Batch) -> Result<Vec<StmtResult>, Error> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.map_err(from_sqlx)?;
    let mut results = Vec::with_capacity(batch.statements.len());
    for (i, stmt) in batch.statements.iter().enumerate() {
        let res = execute(&mut tx, stmt).await.map_err(|e| Error {
            statement: Some(i),
            ..from_sqlx(e)
        })?;
        results.push(res);
    }
    tx.commit().await.map_err(from_sqlx)?;
    Ok(results)
}

async fn execute(conn: &mut SqliteConnection, stmt: &Statement) -> sqlx::Result<StmtResult> {
    // The SQL comes from the caller by design: this is a statement runner, and the caller is
    // trusted (in code, `Batch::push` only takes literals).
    let mut q = sqlx::query(AssertSqlSafe(stmt.sql.to_string()));
    for a in &stmt.args {
        q = match a {
            Value::Null => q.bind(None::<i64>),
            Value::Integer(i) => q.bind(*i),
            Value::Real(f) => q.bind(*f),
            Value::Text(s) => q.bind(s.clone()),
            Value::Blob(b) => q.bind(b.clone()),
        };
    }
    let mut res = StmtResult::default();
    // Deprecated for running several statements in one string; here it's always one. It's
    // the only call that yields both the rows and the change count.
    #[allow(deprecated)]
    let mut stream = q.fetch_many(&mut *conn);
    while let Some(item) = stream.try_next().await? {
        match item {
            Either::Left(done) => {
                res.rows_affected += done.rows_affected();
                res.last_insert_id = done.last_insert_rowid();
            }
            Either::Right(row) => {
                if res.columns.is_empty() {
                    res.columns = row.columns().iter().map(|c| c.name().to_owned()).collect();
                }
                res.rows.push(values(&row)?);
            }
        }
    }
    Ok(res)
}

fn values(row: &SqliteRow) -> sqlx::Result<Vec<Value>> {
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let raw = row.try_get_raw(i)?;
        out.push(if raw.is_null() {
            Value::Null
        } else {
            // The value's own storage class, not the column's declared type.
            match raw.type_info().name() {
                "INTEGER" => Value::Integer(row.try_get(i)?),
                "REAL" => Value::Real(row.try_get(i)?),
                "BLOB" => Value::Blob(row.try_get(i)?),
                _ => Value::Text(row.try_get(i)?),
            }
        });
    }
    Ok(out)
}

/// Database errors keep their kind and message; anything else is the service failing.
pub(crate) fn from_sqlx(e: sqlx::Error) -> Error {
    let kind = match &e {
        sqlx::Error::Database(d) => match d.kind() {
            sqlx::error::ErrorKind::UniqueViolation => ErrorKind::UniqueViolation,
            sqlx::error::ErrorKind::ForeignKeyViolation => ErrorKind::ForeignKeyViolation,
            sqlx::error::ErrorKind::NotNullViolation => ErrorKind::NotNullViolation,
            sqlx::error::ErrorKind::CheckViolation => ErrorKind::CheckViolation,
            _ => ErrorKind::Sql,
        },
        // Wrong argument count and the like.
        sqlx::Error::Encode(_) | sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => {
            ErrorKind::Sql
        }
        _ => ErrorKind::Internal,
    };
    let message = match &e {
        sqlx::Error::Database(d) => d.message().to_owned(),
        e => e.to_string(),
    };
    Error {
        kind,
        message,
        statement: None,
    }
}

// While the site still uses sqlx directly, it handles batch failures as sqlx errors (e.g.
// `is_unique_violation()`). Being a sqlx `DatabaseError` lets `?` carry a batch error into
// that code unchanged (sqlx supplies the `From`).
impl sqlx::error::DatabaseError for Error {
    fn message(&self) -> &str {
        &self.message
    }

    fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> sqlx::error::ErrorKind {
        match self.kind {
            ErrorKind::UniqueViolation => sqlx::error::ErrorKind::UniqueViolation,
            ErrorKind::ForeignKeyViolation => sqlx::error::ErrorKind::ForeignKeyViolation,
            ErrorKind::NotNullViolation => sqlx::error::ErrorKind::NotNullViolation,
            ErrorKind::CheckViolation => sqlx::error::ErrorKind::CheckViolation,
            _ => sqlx::error::ErrorKind::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params;
    use sqlx::sqlite::SqlitePoolOptions;

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
        let sel = b.push("SELECT id, name, n, b, NULL AS empty FROM t", params![]);
        let r = run_batch(&db, &b).await.unwrap();
        assert_eq!(r[ins].first(), Some(&Value::Integer(1)));
        assert_eq!(r[ins].last_insert_id, 1);
        assert_eq!(r[upd].rows_affected, 1);
        assert_eq!(r[sel].columns, ["id", "name", "n", "b", "empty"]);
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
        let err = run_batch(&db, &b).await.unwrap_err();
        assert_eq!(
            (err.kind, err.statement),
            (ErrorKind::UniqueViolation, Some(1))
        );
        let n = run_statement(
            &db,
            &Statement {
                sql: "SELECT count(*) FROM t".into(),
                args: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(n.first(), Some(&Value::Integer(0)));
    }

    #[tokio::test]
    async fn bad_sql_is_a_sql_error() {
        let db = mem_db().await;
        for sql in ["SELEKT 1", "SELECT * FROM missing"] {
            let err = run_statement(
                &db,
                &Statement {
                    sql: sql.into(),
                    args: vec![],
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Sql, "{sql}: {err}");
        }
    }
}
