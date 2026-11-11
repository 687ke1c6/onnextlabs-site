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
