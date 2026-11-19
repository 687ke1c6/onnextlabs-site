//! The site's SQLite database behind HTTP.
//!
//! - `GET  /health`    — no auth; `{"status": "ok"}` once the database answers.
//! - `POST /v1/query`  — one statement on its own.
//! - `POST /v1/batch`  — statements in one transaction (all or nothing).
//! - `GET  /v1/backup` — a consistent copy of the whole database, as one SQLite file.
//!
//! Every `/v1` request needs `Authorization: Bearer <DB_TOKEN>`. The request and response
//! shapes are in the `db-protocol` crate. Migrations (`/migrations`) run at startup, so a
//! deploy of this service is what changes the schema.

// Handlers return `Result<_, Response>`, axum's usual shape.
#![allow(clippy::result_large_err)]

use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use db_protocol::{Batch, BatchResult, Error, ErrorBody, ErrorKind, Statement};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use subtle::ConstantTimeEq;
use tokio::signal;

const DB_FILE: &str = "onnextlabs.db";

#[derive(Parser)]
#[command(about = "The site's SQLite database behind HTTP")]
struct Args {
    #[arg(long, env = "PORT", default_value_t = 3100)]
    port: u16,
    /// Directory holding the database file (created if missing). Mount a volume here.
    #[arg(long, env = "DATA_DIR", default_value = "data")]
    data_dir: PathBuf,
    /// Shared secret clients send as `Authorization: Bearer <token>`.
    #[arg(long, env = "DB_TOKEN", hide_env_values = true)]
    token: String,
    /// Largest request body accepted, in MiB (big statement uploads are one batch).
    #[arg(long, env = "MAX_BODY_MIB", default_value_t = 64)]
    max_body_mib: usize,
}

struct AppState {
    db: SqlitePool,
    /// SHA-256 of the token: comparing digests keeps the comparison constant-time whatever
    /// the length of what a client sends.
    token_hash: [u8; 32],
    /// Where backups are written before they're sent (the data volume, so there's room).
    scratch_dir: PathBuf,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "db_server=info".into()),
        )
        .init();

    let args = Args::parse();
    if args.token.len() < 16 {
        tracing::error!("DB_TOKEN must be at least 16 characters");
        std::process::exit(1);
    }

    let db = open(&args.data_dir).await.unwrap_or_else(|e| {
        tracing::error!("database: {e}");
        std::process::exit(1);
    });
    let app = app(
        db.clone(),
        &args.token,
        args.max_body_mib * 1024 * 1024,
        args.data_dir.clone(),
    );

    // [::] takes IPv4 too on Linux, and Railway's private network is IPv6.
    let addr = SocketAddr::from(([0u16; 8], args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .expect("server error");
    db.close().await;
}

/// Opens (creating the directory and file if missing) the database in `dir` and applies any
/// pending migrations.
async fn open(dir: &std::path::Path) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = dir.join(DB_FILE);
    let opts = SqliteConnectOptions::from_str("sqlite://")?
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(|e| format!("opening {}: {e}", path.display()))?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    tracing::info!("database ready at {}", path.display());
    Ok(pool)
}

fn app(db: SqlitePool, token: &str, max_body: usize, scratch_dir: PathBuf) -> Router {
    let state = Arc::new(AppState {
        db,
        token_hash: Sha256::digest(token.as_bytes()).into(),
        scratch_dir,
    });
    let v1 = Router::new()
        .route("/query", post(query))
        .route("/batch", post(batch))
        .route("/backup", get(backup))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));
    Router::new()
        .route("/health", get(health))
        .nest("/v1", v1)
        .layer(DefaultBodyLimit::max(max_body))
        .layer(middleware::from_fn(log_request))
        .with_state(state)
}

/// What a handler adds to its request's log line (never argument values: only SQL text,
/// which holds no user data, and counts).
#[derive(Clone)]
struct Detail(String);

/// One line per request: method, path, status, time, and the handler's [`Detail`]. Health
/// checks (polled constantly by the host) only at debug level.
async fn log_request(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let started = std::time::Instant::now();
    let res = next.run(req).await;
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = res.status().as_u16();
    let detail = res
        .extensions()
        .get::<Detail>()
        .map(|d| d.0.as_str())
        .unwrap_or("");
    if path == "/health" {
        tracing::debug!("{method} {path} {status} {ms:.1}ms");
    } else if res.status().is_server_error() {
        tracing::error!("{method} {path} {status} {ms:.1}ms {detail}");
    } else {
        tracing::info!("{method} {path} {status} {ms:.1}ms {detail}");
    }
    res
}

/// The SQL on one line, shortened: enough to tell statements apart in the log.
fn sql_summary(sql: &str) -> String {
    const MAX: usize = 120;
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(MAX) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

async fn require_token(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let given = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let given_hash: [u8; 32] = Sha256::digest(given.as_bytes()).into();
    if given.is_empty() || !bool::from(given_hash.ct_eq(&state.token_hash)) {
        return failure(
            ErrorKind::Unauthorized,
            "missing or wrong bearer token".into(),
        );
    }
    next.run(req).await
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => failure(ErrorKind::Internal, e.to_string()),
    }
}

async fn query(
    State(state): State<Arc<AppState>>,
    body: Result<Json<Statement>, JsonRejection>,
) -> Response {
    let Json(stmt) = match body {
        Ok(b) => b,
        Err(e) => return bad_json(e),
    };
    let res = db_protocol::run_statement(&state.db, &stmt).await;
    let detail = match &res {
        Ok(r) if !r.rows.is_empty() => {
            format!("{} rows | {}", r.rows.len(), sql_summary(&stmt.sql))
        }
        Ok(r) if r.rows_affected > 0 => {
            format!("{} changed | {}", r.rows_affected, sql_summary(&stmt.sql))
        }
        Ok(_) => sql_summary(&stmt.sql),
        Err(e) => format!("{:?}: {} | {}", e.kind, e.message, sql_summary(&stmt.sql)),
    };
    let mut res = match res {
        Ok(r) => Json(r).into_response(),
        Err(e) => error_response(e),
    };
    res.extensions_mut().insert(Detail(detail));
    res
}

async fn batch(
    State(state): State<Arc<AppState>>,
    body: Result<Json<Batch>, JsonRejection>,
) -> Response {
    let Json(batch) = match body {
        Ok(b) => b,
        Err(e) => return bad_json(e),
    };
    let res = db_protocol::run_batch(&state.db, &batch).await;
    let n = batch.statements.len();
    let first = batch
        .statements
        .first()
        .map(|s| sql_summary(&s.sql))
        .unwrap_or_default();
    let detail = match &res {
        Ok(_) => format!("{n} statements | first: {first}"),
        Err(e) => {
            let failed = e
                .statement
                .and_then(|i| batch.statements.get(i))
                .map(|s| sql_summary(&s.sql))
                .unwrap_or_default();
            format!(
                "{n} statements, rolled back: {:?} in #{}: {} | {failed}",
                e.kind,
                e.statement.map_or("?".into(), |i| i.to_string()),
                e.message
            )
        }
    };
    let mut res = match res {
        Ok(results) => Json(BatchResult { results }).into_response(),
        Err(e) => error_response(e),
    };
    res.extensions_mut().insert(Detail(detail));
    res
}

/// `VACUUM INTO` writes a consistent, compacted copy while writes carry on, without the
/// WAL files a plain file copy would need. The copy is unlinked once open, so it's gone
/// from disk when the download ends, however it ends.
async fn backup(State(state): State<Arc<AppState>>) -> Result<Response, Response> {
    let name = format!(
        ".backup-{}-{}.db",
        std::process::id(),
        BACKUP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let path = state.scratch_dir.join(name);
    let internal = |e: &dyn std::fmt::Display| failure(ErrorKind::Internal, e.to_string());
    sqlx::query("VACUUM INTO ?")
        .bind(path.to_string_lossy().into_owned())
        .execute(&state.db)
        .await
        .map_err(|e| internal(&e))?;
    let file = tokio::fs::File::open(&path).await;
    let _ = tokio::fs::remove_file(&path).await;
    let file = file.map_err(|e| internal(&e))?;
    let len = file.metadata().await.map_err(|e| internal(&e))?.len();
    tracing::info!(bytes = len, "backup taken");
    Ok((
        [
            (header::CONTENT_TYPE, "application/vnd.sqlite3".to_owned()),
            (header::CONTENT_LENGTH, len.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{DB_FILE}\""),
            ),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}

static BACKUP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn bad_json(e: JsonRejection) -> Response {
    failure(ErrorKind::BadRequest, e.body_text())
}

fn failure(kind: ErrorKind, message: String) -> Response {
    error_response(Error::new(kind, message))
}

fn error_response(error: Error) -> Response {
    let status = match error.kind {
        ErrorKind::UniqueViolation
        | ErrorKind::ForeignKeyViolation
        | ErrorKind::NotNullViolation
        | ErrorKind::CheckViolation => StatusCode::CONFLICT,
        ErrorKind::Sql | ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
        ErrorKind::Internal | ErrorKind::Decode => {
            tracing::error!("{error}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    (status, Json(ErrorBody { error })).into_response()
}

async fn shutdown() {
    let ctrl_c = async { signal::ctrl_c().await.expect("ctrl-c handler") };
    #[cfg(unix)]
    let term = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use serde_json::{json, Value as Json};
    use tower::ServiceExt;

    const TOKEN: &str = "test-token-0123456789";

    async fn test_app() -> Router {
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
        app(db, TOKEN, 1024 * 1024, std::env::temp_dir())
    }

    async fn call(app: &Router, path: &str, token: Option<&str>, body: &str) -> (StatusCode, Json) {
        let mut req = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn sql_is_summarised_on_one_line() {
        assert_eq!(
            sql_summary("SELECT a,\n       b\n  FROM t"),
            "SELECT a, b FROM t"
        );
        let long = sql_summary(&format!("SELECT {}", "x".repeat(200)));
        assert_eq!(long.chars().count(), 121);
        assert!(long.ends_with('…'));
    }

    #[tokio::test]
    async fn health_needs_no_token() {
        let app = test_app().await;
        let res = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn v1_needs_the_token() {
        let app = test_app().await;
        let body = r#"{"sql": "SELECT 1"}"#;
        for token in [None, Some("wrong"), Some("")] {
            let (status, j) = call(&app, "/v1/query", token, body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{token:?}");
            assert_eq!(j["error"]["kind"], "unauthorized");
        }
        let (status, _) = call(&app, "/v1/query", Some(TOKEN), body).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn query_returns_columns_and_rows() {
        let app = test_app().await;
        let (status, j) = call(
            &app,
            "/v1/query",
            Some(TOKEN),
            r#"{"sql": "SELECT ?1 AS n, ?2 AS s, 2.5 AS r, x'00ff' AS b", "args": [7, "hi"]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(j["columns"], json!(["n", "s", "r", "b"]));
        assert_eq!(j["rows"], json!([[7, "hi", 2.5, {"base64": "AP8="}]]));
    }

    #[tokio::test]
    async fn batches_are_atomic_and_report_the_failing_statement() {
        let app = test_app().await;
        let insert = |slug: &str| json!({"sql": "INSERT INTO organisations (slug, name) VALUES (?, 'X') RETURNING id", "args": [slug]});
        let ok = json!({"statements": [insert("a"), insert("b")]}).to_string();
        let (status, j) = call(&app, "/v1/batch", Some(TOKEN), &ok).await;
        assert_eq!(status, StatusCode::OK, "{j}");
        assert_eq!(j["results"][1]["rows"], json!([[2]]));

        let clash = json!({"statements": [insert("c"), insert("a")]}).to_string();
        let (status, j) = call(&app, "/v1/batch", Some(TOKEN), &clash).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(j["error"]["kind"], "unique_violation");
        assert_eq!(j["error"]["statement"], 1);

        let (_, j) = call(
            &app,
            "/v1/query",
            Some(TOKEN),
            r#"{"sql": "SELECT group_concat(slug) FROM organisations"}"#,
        )
        .await;
        assert_eq!(j["rows"], json!([["a,b"]]), "c was rolled back");
    }

    #[tokio::test]
    async fn bad_requests_say_why() {
        let app = test_app().await;
        let (status, j) = call(&app, "/v1/query", Some(TOKEN), r#"{"sql": "SELEKT"}"#).await;
        assert_eq!(
            (status, j["error"]["kind"].as_str()),
            (StatusCode::BAD_REQUEST, Some("sql"))
        );
        let (status, j) = call(&app, "/v1/query", Some(TOKEN), r#"{"nope": 1}"#).await;
        assert_eq!(
            (status, j["error"]["kind"].as_str()),
            (StatusCode::BAD_REQUEST, Some("bad_request"))
        );
    }

    #[tokio::test]
    async fn backup_is_a_whole_sqlite_file_and_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("db-server-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = open(&dir.join("data")).await.unwrap();
        let app = app(db, TOKEN, 1024 * 1024, dir.join("data"));
        let (status, _) = call(
            &app,
            "/v1/query",
            Some(TOKEN),
            r#"{"sql": "INSERT INTO organisations (slug, name) VALUES ('kept', 'K')"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let unauthorised = app
            .clone()
            .oneshot(Request::get("/v1/backup").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorised.status(), StatusCode::UNAUTHORIZED);

        let res = app
            .oneshot(
                Request::get("/v1/backup")
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        assert!(bytes.starts_with(b"SQLite format 3\0"));

        // The copy opens as a database with the row in it.
        let copy = dir.join("copy.db");
        std::fs::write(&copy, &bytes).unwrap();
        let db = SqlitePoolOptions::new()
            .connect_with(SqliteConnectOptions::new().filename(&copy))
            .await
            .unwrap();
        let slugs: Vec<String> = sqlx::query_scalar("SELECT slug FROM organisations")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(slugs, ["kept"]);
        db.close().await;

        let leftovers = std::fs::read_dir(dir.join("data"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".backup-"))
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
