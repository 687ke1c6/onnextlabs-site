use std::{env, net::SocketAddr, path::PathBuf};

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};
use tokio::signal;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "onnextlabs_site=info,tower_http=info".into()),
        )
        .init();

    let port: u16 = env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let static_dir = PathBuf::from(env::var("STATIC_DIR").unwrap_or_else(|_| "web/dist".into()));

    // Unknown paths fall back to index.html so client-side routing keeps working.
    let spa = ServeDir::new(&static_dir)
        .not_found_service(ServeFile::new(static_dir.join("index.html")));

    let app = Router::new()
        .route("/api/health", get(health))
        .fallback_service(spa)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await.expect("failed to bind");
    tracing::info!("listening on http://{addr}, serving {}", static_dir.display());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .expect("server error");
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
