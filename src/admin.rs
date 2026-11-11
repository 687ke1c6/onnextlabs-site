//! Admin-only API. Every handler takes `Admin`, which rejects anyone without the role.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde_json::{json, Value};

use crate::{auth::Admin, AppState};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/api/admin/overview", get(overview))
}

/// GET /api/admin/overview
async fn overview(
    _admin: Admin,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let (subscribers, users): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM subscribers WHERE unsubscribed_at IS NULL),
                (SELECT count(*) FROM users)",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "admin overview failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(json!({ "subscribers": subscribers, "users": users })))
}
