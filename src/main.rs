// Handlers return `Result<_, Response>` so errors are ready-made JSON responses. axum's
// `Response` is ~128 bytes, which trips this lint; boxing every error isn't worth it here.
#![allow(clippy::result_large_err)]

mod admin;
mod altcha;
mod api;
mod auth;
mod db;
mod documents;
mod orgs;
mod rate_limit;
mod rbac;
mod subscribe;
mod users;

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    http::HeaderName,
    http::StatusCode,
    routing::{any, get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use rate_limit::{Limit, RateLimiter};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::signal;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Port to listen on.
    #[arg(long, env = "PORT", default_value_t = 3000)]
    port: u16,

    /// Directory of built frontend assets.
    #[arg(long, env = "STATIC_DIR", default_value = "web/dist")]
    static_dir: PathBuf,

    /// Directory for everything the app stores: the SQLite database and per-organisation
    /// files under `orgs/<slug>/`. Created if missing. Mount persistent storage here.
    #[arg(long, env = "DATA_DIR", default_value = "data")]
    data_dir: PathBuf,

    /// Largest document upload allowed, in megabytes.
    #[arg(long, env = "MAX_UPLOAD_MB", default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=10_240))]
    max_upload_mb: u64,

    /// Header carrying the real client IP when behind a trusted reverse proxy
    /// (e.g. x-forwarded-for, x-real-ip). Unset = use the TCP peer address.
    #[arg(long, env = "CLIENT_IP_HEADER")]
    client_ip_header: Option<HeaderName>,

    /// Secret for signing ALTCHA challenges. Unset = random per process, so challenges
    /// issued before a restart stop verifying (users just retry).
    #[arg(long, env = "ALTCHA_HMAC_KEY", hide_env_values = true)]
    altcha_hmac_key: Option<String>,

    /// Public origin of the site, e.g. https://onnextlabs.xyz (http://localhost:5173 in dev).
    /// Used to build OAuth redirect URLs; https here also marks cookies Secure.
    #[arg(long, env = "PUBLIC_URL")]
    public_url: Option<String>,

    /// Google OAuth client ID. Sign-in with Google is enabled when this and the secret are set.
    #[arg(long, env = "GOOGLE_CLIENT_ID")]
    google_client_id: Option<String>,

    /// Google OAuth client secret.
    #[arg(long, env = "GOOGLE_CLIENT_SECRET", hide_env_values = true)]
    google_client_secret: Option<String>,

    /// Comma-separated emails that get admin access on sign-in (provider-verified emails only).
    /// Always admins while listed; other admins are set in the portal or with `grant-admin`.
    #[arg(long, env = "ADMIN_EMAILS", value_delimiter = ',')]
    admin_emails: Vec<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Admin tasks run against the database instead of starting the server.
#[derive(Subcommand)]
enum Command {
    /// Make a user an admin, by email. They must have signed in once.
    GrantAdmin { email: String },
    /// Clear a user's admin flag, by email. (ADMIN_EMAILS addresses stay admins.)
    RevokeAdmin { email: String },
    /// List users (and which are admins).
    ListUsers,
    /// Create an organisation. The slug (URL/folder name) is derived from the name if omitted.
    CreateOrg {
        name: String,
        #[arg(long)]
        slug: Option<String>,
    },
    /// Add a user (by email) to an organisation, or change their role there. They must have
    /// signed in once.
    AddMember {
        org: String,
        email: String,
        /// Role name, e.g. Admin or Member.
        #[arg(long, default_value = "Member")]
        role: String,
    },
    /// Remove a user (by email) from an organisation.
    RemoveMember { org: String, email: String },
    /// List organisations and their members.
    ListOrgs,
}

pub struct AppState {
    db: SqlitePool,
    auth: auth::Auth,
    altcha: altcha::Altcha,
    /// Root of all stored files (see `Args::data_dir`).
    data_dir: PathBuf,
    max_upload_bytes: u64,
    limiter: RateLimiter,
    client_ip_header: Option<HeaderName>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "onnextlabs_site=info,tower_http=info".into()),
        )
        .init();

    let args = Args::parse();
    let static_dir = args.static_dir;

    let db = db::open(&args.data_dir).await.unwrap_or_else(|e| {
        tracing::error!("database: {e}");
        std::process::exit(1);
    });

    if let Some(cmd) = args.command {
        let result = match cmd {
            Command::GrantAdmin { email } => users::set_admin(&db, &email, true).await,
            Command::RevokeAdmin { email } => users::set_admin(&db, &email, false).await,
            Command::ListUsers => users::list_users(&db).await,
            Command::CreateOrg { name, slug } => {
                users::create_org(&db, &name, slug.as_deref()).await
            }
            Command::AddMember { org, email, role } => {
                users::add_member(&db, &org, &email, &role).await
            }
            Command::RemoveMember { org, email } => users::remove_member(&db, &org, &email).await,
            Command::ListOrgs => users::list_orgs(&db).await,
        };
        db.close().await;
        if let Err(e) = result {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    let admin_emails: Vec<String> = args
        .admin_emails
        .iter()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    if admin_emails.is_empty() {
        tracing::warn!("ADMIN_EMAILS not set; only accounts flagged as admin have admin access");
    } else {
        tracing::info!("admin emails: {}", admin_emails.join(", "));
    }
    let auth = build_auth(
        args.public_url.as_deref(),
        args.google_client_id,
        args.google_client_secret,
        admin_emails,
    );

    let altcha_key = match args.altcha_hmac_key {
        Some(k) if !k.is_empty() => k.into_bytes(),
        _ => {
            tracing::warn!("ALTCHA_HMAC_KEY not set; using a random key for this process");
            rand::random::<[u8; 32]>().to_vec()
        }
    };

    let max_upload_bytes = args.max_upload_mb * 1024 * 1024;
    let data_dir = args.data_dir;
    if let Err(e) = documents::prepare(&data_dir) {
        tracing::error!("data dir {}: {e}", data_dir.display());
        std::process::exit(1);
    }
    tracing::info!(
        "org data stored in {} (max upload {} MB)",
        data_dir.join("orgs").display(),
        args.max_upload_mb
    );

    let state = Arc::new(AppState {
        db,
        auth,
        // ~50k SHA-256 hashes worst case: well under a second in a browser.
        altcha: altcha::Altcha::new(altcha_key, 50_000, Duration::from_secs(600)),
        data_dir,
        max_upload_bytes,
        // Per IP: 5 signups / 10 min. Globally: 60 / min across everyone.
        limiter: RateLimiter::new(
            Limit {
                max: 5,
                window: Duration::from_secs(600),
            },
            Limit {
                max: 60,
                window: Duration::from_secs(60),
            },
        ),
        client_ip_header: args.client_ip_header,
    });

    let sweeper = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            sweeper.limiter.sweep();
            auth::sweep_sessions(&sweeper.db).await;
        }
    });

    // Unknown paths fall back to index.html so client-side routing keeps working.
    let spa = ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html")));

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/altcha/challenge", get(subscribe::challenge))
        .route("/api/subscribe", post(subscribe::subscribe))
        .merge(auth::routes())
        .merge(admin::routes())
        .merge(orgs::routes())
        .merge(rbac::routes())
        .merge(documents::routes(max_upload_bytes))
        // Unknown API paths get a JSON 404 rather than the SPA's index.html.
        .route("/api/{*rest}", any(api_not_found))
        .fallback_service(spa)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");
    tracing::info!(
        "listening on http://{addr}, serving {}",
        static_dir.display()
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await
    .expect("server error");

    state.db.close().await;
}

/// Sign-in providers from config. Missing settings just disable sign-in (logged), so the
/// rest of the site still runs.
fn build_auth(
    public_url: Option<&str>,
    google_id: Option<String>,
    google_secret: Option<String>,
    admin_emails: Vec<String>,
) -> auth::Auth {
    let mut providers = Vec::new();
    match (public_url, google_id, google_secret) {
        (Some(url), Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
            match auth::Provider::new("google", "https://accounts.google.com", id, secret, url) {
                Ok(p) => {
                    tracing::info!("Google sign-in enabled; redirect URI {}", p.redirect_url());
                    providers.push(p);
                }
                Err(e) => tracing::error!("Google sign-in disabled: {e}"),
            }
        }
        (None, Some(_), _) => {
            tracing::warn!("Google sign-in disabled: PUBLIC_URL is not set")
        }
        _ => tracing::warn!(
            "Google sign-in disabled: GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET not set"
        ),
    }
    let secure = public_url.is_some_and(|u| u.starts_with("https://"));
    auth::Auth::new(providers, admin_emails, secure)
}

async fn api_not_found() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "Not found." })),
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
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
