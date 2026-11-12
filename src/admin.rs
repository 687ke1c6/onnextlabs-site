//! Admin-only API. Every handler takes `Admin`, which rejects anyone who isn't one.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::Response,
    routing::{get, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::{
    api::{bad_request, db_error, not_found},
    auth::Admin,
    AppState,
};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/admin/overview", get(overview))
        .route("/api/admin/users", get(users))
        .route("/api/admin/users/{id}/admin", put(set_admin))
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

#[derive(Serialize)]
struct Membership {
    slug: String,
    name: String,
    role: String,
}

/// An account as the Users page shows it.
#[derive(Serialize, sqlx::FromRow)]
struct AdminUser {
    id: i64,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
    created_at: String,
    last_login_at: String,
    /// The stored flag (set in the portal or with `grant-admin`).
    is_admin: bool,
    #[serde(skip)]
    email_verified: bool,
    /// Admin because of ADMIN_EMAILS: can't be removed here.
    #[sqlx(skip)]
    env_admin: bool,
    #[sqlx(skip)]
    orgs: Vec<Membership>,
}

macro_rules! select_user {
    ($tail:literal) => {
        concat!(
            "SELECT id, email, name, picture, created_at, last_login_at, is_admin, email_verified
             FROM users ",
            $tail
        )
    };
}

/// Fills in `env_admin` and each user's org memberships.
async fn complete(state: &AppState, mut users: Vec<AdminUser>) -> Result<Vec<AdminUser>, Response> {
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT m.user_id, o.slug, o.name, r.name
         FROM org_members m
         JOIN organisations o ON o.id = m.org_id
         JOIN roles r ON r.id = m.role_id
         ORDER BY o.name COLLATE NOCASE",
    )
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;
    let mut by_user: HashMap<i64, Vec<Membership>> = HashMap::new();
    for (user_id, slug, name, role) in rows {
        by_user
            .entry(user_id)
            .or_default()
            .push(Membership { slug, name, role });
    }
    for u in &mut users {
        u.env_admin = state
            .auth
            .is_env_admin(u.email.as_deref(), u.email_verified);
        u.orgs = by_user.remove(&u.id).unwrap_or_default();
    }
    Ok(users)
}

/// GET /api/admin/users — every account, with admin status and memberships.
async fn users(
    _admin: Admin,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AdminUser>>, Response> {
    let users = sqlx::query_as(select_user!(
        "ORDER BY coalesce(name, email) COLLATE NOCASE"
    ))
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;
    complete(&state, users).await.map(Json)
}

async fn fetch_user(db: &SqlitePool, id: i64) -> Result<AdminUser, Response> {
    sqlx::query_as(select_user!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("No such account."))
}

#[derive(Deserialize)]
struct AdminChange {
    admin: bool,
}

/// PUT /api/admin/users/{id}/admin — set or clear the account's admin flag. You can't
/// clear your own (so the last admin can't lock everyone out).
async fn set_admin(
    Admin(me): Admin,
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<i64>,
    Json(body): Json<AdminChange>,
) -> Result<Json<AdminUser>, Response> {
    if id == me.id && !body.admin {
        return Err(bad_request(
            "You can't remove your own admin access. Ask another admin.",
        ));
    }
    fetch_user(&state.db, id).await?;
    sqlx::query("UPDATE users SET is_admin = ? WHERE id = ?")
        .bind(body.admin)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::info!(
        user = id,
        admin = body.admin,
        by = me.id,
        "admin flag changed"
    );
    let user = fetch_user(&state.db, id).await?;
    let mut done = complete(&state, vec![user]).await?;
    Ok(Json(done.remove(0)))
}
