//! The database, wherever it lives.
//!
//! [`Db`] runs statements either on a local SQLite file (the default) or through the
//! database service (`crates/db-server`) when `DB_URL` is set. Both go through
//! `db-protocol`, so results and errors are the same either way.
//!
//! The query API mirrors sqlx's: `db::query_as::<T>(sql).bind(x).fetch_all(&db)`. Rows
//! decode with serde (`#[derive(Deserialize)]`; see `db_protocol::from_row`).
//!
//! There are no transactions spanning calls. Anything that must be atomic is one [`Batch`]
//! decided up front: later statements find what earlier ones did by subquery, and results
//! are only looked at after the commit. That keeps each transaction to one round trip.

use std::{marker::PhantomData, path::Path, str::FromStr, sync::Arc, time::Duration};

use db_protocol::{BatchResult, ErrorBody, ErrorKind, Statement};
use serde::{de::DeserializeOwned, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

pub use db_protocol::{Batch, Error, StmtResult, Value};

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub const DB_FILE: &str = "onnextlabs.db";

/// A handle to the database; cheap to clone.
#[derive(Clone)]
pub struct Db(Arc<Backend>);

enum Backend {
    Local(SqlitePool),
    Remote(Remote),
}

impl Db {
    /// Opens (creating the directory and file if missing) the SQLite database in `dir` and
    /// applies any pending migrations from `migrations/`.
    pub async fn open_local(dir: &Path) -> Result<Db, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("creating db dir {}: {e}", dir.display()))?;
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
        sqlx::migrate!("../../migrations").run(&pool).await?;
        tracing::info!("database ready at {}", path.display());
        Ok(Db::from_pool(pool))
    }

    /// The database service at `url` (e.g. `http://db.railway.internal:3100`). The service
    /// runs the migrations; this only checks it answers, and warns if it doesn't.
    pub async fn remote(url: &str, token: &str) -> Result<Db, Box<dyn std::error::Error>> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        let remote = Remote {
            http,
            url: url.trim_end_matches('/').to_owned(),
            auth: format!("Bearer {token}"),
        };
        match remote
            .http
            .get(format!("{}/health", remote.url))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                tracing::info!("database service at {}", remote.url)
            }
            Ok(r) => tracing::warn!("database service at {} answered {}", remote.url, r.status()),
            Err(e) => tracing::warn!("database service at {} unreachable: {e}", remote.url),
        }
        Ok(Db(Arc::new(Backend::Remote(remote))))
    }

    pub fn from_pool(pool: SqlitePool) -> Db {
        Db(Arc::new(Backend::Local(pool)))
    }

    pub async fn close(&self) {
        if let Backend::Local(pool) = &*self.0 {
            pool.close().await;
        }
    }

    async fn statement(&self, stmt: &Statement) -> Result<StmtResult> {
        match &*self.0 {
            Backend::Local(pool) => db_protocol::run_statement(pool, stmt).await,
            Backend::Remote(r) => r.post("/v1/query", stmt).await,
        }
    }

    async fn batch(&self, batch: &Batch) -> Result<Vec<StmtResult>> {
        match &*self.0 {
            Backend::Local(pool) => db_protocol::run_batch(pool, batch).await,
            Backend::Remote(r) => r
                .post::<_, BatchResult>("/v1/batch", batch)
                .await
                .map(|b| b.results),
        }
    }
}

/// Runs the batch in one transaction; any failing statement rolls the whole batch back.
pub async fn run(db: &Db, batch: Batch) -> Result<Vec<StmtResult>> {
    db.batch(&batch).await
}

struct Remote {
    http: reqwest::Client,
    url: String,
    auth: String,
}

impl Remote {
    async fn post<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp> {
        let unreachable =
            |e: reqwest::Error| Error::new(ErrorKind::Internal, format!("database service: {e}"));
        let body = serde_json::to_vec(body)
            .map_err(|e| Error::new(ErrorKind::BadRequest, e.to_string()))?;
        let res = self
            .http
            .post(format!("{}{path}", self.url))
            .header(reqwest::header::AUTHORIZATION, &self.auth)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(unreachable)?;
        let status = res.status();
        let bytes = res.bytes().await.map_err(unreachable)?;
        if status.is_success() {
            serde_json::from_slice(&bytes)
                .map_err(|e| Error::new(ErrorKind::Decode, format!("database service answer: {e}")))
        } else {
            Err(match serde_json::from_slice::<ErrorBody>(&bytes) {
                Ok(b) => b.error,
                Err(_) => Error::new(
                    ErrorKind::Internal,
                    format!("database service answered {status}"),
                ),
            })
        }
    }
}

// ---- queries ----

/// SQL text. In code it's a literal; [`Trusted`] marks text built at runtime from trusted
/// parts (e.g. table names read from the schema), never from user input.
pub struct Sql(std::borrow::Cow<'static, str>);

/// SQL built at runtime from trusted parts. Values still go in `bind`.
pub struct Trusted(pub String);

impl From<&'static str> for Sql {
    fn from(s: &'static str) -> Self {
        Sql(s.into())
    }
}

impl From<Trusted> for Sql {
    fn from(t: Trusted) -> Self {
        Sql(t.0.into())
    }
}

fn statement(sql: Sql) -> Statement {
    Statement {
        sql: sql.0,
        args: Vec::new(),
    }
}

/// A statement whose rows (if any) aren't wanted.
pub fn query(sql: impl Into<Sql>) -> Query {
    Query(statement(sql.into()))
}

/// A statement whose rows decode as `T`.
pub fn query_as<T: DeserializeOwned>(sql: impl Into<Sql>) -> QueryAs<T> {
    QueryAs(statement(sql.into()), PhantomData)
}

/// A statement whose rows' first column decodes as `T`.
pub fn query_scalar<T: DeserializeOwned>(sql: impl Into<Sql>) -> QueryScalar<T> {
    QueryScalar(statement(sql.into()), PhantomData)
}

#[must_use]
pub struct Query(Statement);

#[must_use]
pub struct QueryAs<T>(Statement, PhantomData<fn() -> T>);

#[must_use]
pub struct QueryScalar<T>(Statement, PhantomData<fn() -> T>);

impl Query {
    pub fn bind(mut self, v: impl Into<Value>) -> Self {
        self.0.args.push(v.into());
        self
    }

    pub async fn execute(self, db: &Db) -> Result<StmtResult> {
        db.statement(&self.0).await
    }
}

fn no_row() -> Error {
    Error::new(ErrorKind::Decode, "expected a row, got none")
}

impl<T: DeserializeOwned> QueryAs<T> {
    pub fn bind(mut self, v: impl Into<Value>) -> Self {
        self.0.args.push(v.into());
        self
    }

    pub async fn fetch_all(self, db: &Db) -> Result<Vec<T>> {
        db.statement(&self.0).await?.decode()
    }

    pub async fn fetch_optional(self, db: &Db) -> Result<Option<T>> {
        let res = db.statement(&self.0).await?;
        let columns = res.columns;
        res.rows
            .into_iter()
            .next()
            .map(|r| db_protocol::from_row(&columns, r))
            .transpose()
    }

    pub async fn fetch_one(self, db: &Db) -> Result<T> {
        self.fetch_optional(db).await?.ok_or_else(no_row)
    }
}

impl<T: DeserializeOwned> QueryScalar<T> {
    pub fn bind(mut self, v: impl Into<Value>) -> Self {
        self.0.args.push(v.into());
        self
    }

    pub async fn fetch_all(self, db: &Db) -> Result<Vec<T>> {
        db.statement(&self.0)
            .await?
            .rows
            .into_iter()
            .map(|r| first_column(r).and_then(db_protocol::from_value))
            .collect()
    }

    pub async fn fetch_optional(self, db: &Db) -> Result<Option<T>> {
        db.statement(&self.0)
            .await?
            .rows
            .into_iter()
            .next()
            .map(|r| first_column(r).and_then(db_protocol::from_value))
            .transpose()
    }

    pub async fn fetch_one(self, db: &Db) -> Result<T> {
        self.fetch_optional(db).await?.ok_or_else(no_row)
    }
}

fn first_column(row: Vec<Value>) -> Result<Value> {
    row.into_iter()
        .next()
        .ok_or_else(|| Error::new(ErrorKind::Decode, "row has no columns"))
}

/// A fresh in-memory database with every migration applied (one connection, so it lives as
/// long as the pool).
#[cfg(test)]
pub async fn test_db() -> Db {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    Db::from_pool(pool)
}
