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

    sqlx::migrate!("../../migrations").run(&pool).await?;
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
    sqlx::migrate!("../../migrations").run(&db).await.unwrap();
    db
}

// ---- batches ----
//
// A batch is a list of statements decided up front, run in one transaction: either every
// statement applies or none does. Nothing reads a result *between* statements; a later
// statement that needs an earlier one's outcome looks it up in SQL (a subquery). Results are
// only inspected after the commit. That keeps each transaction to one round trip, so the
// database can sit behind a stateless HTTP service (crates/db-server).

pub use db_protocol::{Batch, Error, StmtResult, Value};

/// Runs the batch in one transaction; any failing statement rolls the whole batch back.
pub async fn run(db: &SqlitePool, batch: Batch) -> Result<Vec<StmtResult>, Error> {
    db_protocol::run_batch(db, &batch).await
}
