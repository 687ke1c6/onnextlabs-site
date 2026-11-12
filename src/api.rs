//! Small helpers shared by the JSON API handlers.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// `{"error": msg}` with the given status.
pub fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

pub fn bad_request(msg: &str) -> Response {
    error(StatusCode::BAD_REQUEST, msg)
}

pub fn not_found(msg: &str) -> Response {
    error(StatusCode::NOT_FOUND, msg)
}

pub fn db_error(e: sqlx::Error) -> Response {
    tracing::error!(error = %e, "database error");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
}

pub fn io_error(e: std::io::Error) -> Response {
    tracing::error!(error = %e, "filesystem error");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
}
