//! The org's statement types (the values a row's `type` can take) and editing a row's type.
//!
//! Rows and type rules keep the type's *name*, so the rules engine and Google Sheets are
//! unchanged. Renaming a type renames it everywhere in one transaction; a type still used by a
//! row or rule can't be deleted.
//!
//! A type someone picks on a row is hand-set (`type_manual`): saving a statement never changes
//! it. Clearing it makes the row blank and not hand-set, so the next save can type it again.

use std::sync::Arc;

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::Response,
    routing::{get, patch},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    api::{bad_request, db_error, error, not_found},
    auth::CurrentUser,
    rbac::{self, Permission},
    AppState,
};

const MAX_TYPE_CHARS: usize = 100;
const MAX_TYPES: i64 = 500;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/orgs/{org}/statement-types",
            get(list_types).post(create_type),
        )
        .route(
            "/api/orgs/{org}/statement-types/{id}",
            patch(rename_type).delete(delete_type),
        )
        .route(
            "/api/orgs/{org}/statement-transactions/{id}",
            patch(set_row_type),
        )
}

#[derive(Serialize, sqlx::FromRow)]
pub struct TypeInfo {
    id: i64,
    name: String,
    /// Rows and rules using it.
    rows: i64,
    rules: i64,
}

pub async fn org_types(db: &SqlitePool, org_id: i64) -> sqlx::Result<Vec<TypeInfo>> {
    sqlx::query_as(
        "SELECT s.id, s.name,
                (SELECT count(*) FROM statement_transactions t
                 WHERE t.org_id = s.org_id AND t.type = s.name) AS rows,
                (SELECT count(*) FROM statement_type_rules r
                 WHERE r.org_id = s.org_id AND r.type = s.name) AS rules
         FROM statement_types s WHERE s.org_id = ? ORDER BY s.name COLLATE NOCASE",
    )
    .bind(org_id)
    .fetch_all(db)
    .await
}

/// The list's own spelling of `name` (matched ignoring case), if it's on the list.
pub async fn canonical_name(
    db: &SqlitePool,
    org_id: i64,
    name: &str,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT name FROM statement_types WHERE org_id = ? AND name = ?")
        .bind(org_id)
        .bind(name.trim())
        .fetch_optional(db)
        .await
}

fn clean_name(raw: &str) -> Result<String, Response> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_TYPE_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(bad_request("Give the type a name of up to 100 characters."));
    }
    Ok(name.to_owned())
}

fn taken() -> Response {
    error(
        StatusCode::CONFLICT,
        "There's already a type with that name.",
    )
}

/// GET /api/orgs/{org}/statement-types
async fn list_types(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<Vec<TypeInfo>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(org_types(&state.db, org.id).await.map_err(db_error)?))
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

/// POST /api/orgs/{org}/statement-types
async fn create_type(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Json(body): Json<NameBody>,
) -> Result<(StatusCode, Json<Vec<TypeInfo>>), Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let name = clean_name(&body.name)?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM statement_types WHERE org_id = ?")
        .bind(org.id)
        .fetch_one(&state.db)
        .await
        .map_err(db_error)?;
    if count >= MAX_TYPES {
        return Err(bad_request("At most 500 types."));
    }
    match sqlx::query("INSERT INTO statement_types (org_id, name) VALUES (?, ?)")
        .bind(org.id)
        .bind(&name)
        .execute(&state.db)
        .await
    {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(taken()),
        Err(e) => return Err(db_error(e)),
    }
    tracing::info!(org = %org.slug, %name, user = user.id, "statement type added");
    Ok((
        StatusCode::CREATED,
        Json(org_types(&state.db, org.id).await.map_err(db_error)?),
    ))
}

/// PATCH /api/orgs/{org}/statement-types/{id} — rename it on the list, every row and every rule.
async fn rename_type(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<NameBody>,
) -> Result<Json<Vec<TypeInfo>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let name = clean_name(&body.name)?;
    let renamed = rename(&state.db, org.id, id, &name).await;
    match renamed {
        Ok(Some(old)) => {
            tracing::info!(org = %org.slug, from = %old, to = %name, user = user.id, "statement type renamed")
        }
        Ok(None) => return Err(not_found("Type not found.")),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(taken()),
        Err(e) => return Err(db_error(e)),
    }
    Ok(Json(org_types(&state.db, org.id).await.map_err(db_error)?))
}

/// The old name, or None if there's no such type in the org.
async fn rename(db: &SqlitePool, org_id: i64, id: i64, name: &str) -> sqlx::Result<Option<String>> {
    let mut tx = db.begin().await?;
    let old: Option<String> =
        sqlx::query_scalar("SELECT name FROM statement_types WHERE id = ? AND org_id = ?")
            .bind(id)
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(old) = old else {
        return Ok(None);
    };
    sqlx::query("UPDATE statement_types SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for table in [
        "UPDATE statement_transactions SET type = ?1 WHERE org_id = ?2 AND type = ?3",
        "UPDATE statement_type_rules SET type = ?1 WHERE org_id = ?2 AND type = ?3",
    ] {
        sqlx::query(table)
            .bind(name)
            .bind(org_id)
            .bind(&old)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Some(old))
}

/// DELETE /api/orgs/{org}/statement-types/{id} — only if no row or rule uses it.
async fn delete_type(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<Json<Vec<TypeInfo>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let types = org_types(&state.db, org.id).await.map_err(db_error)?;
    let t = types
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| not_found("Type not found."))?;
    if t.rows > 0 || t.rules > 0 {
        let mut uses = Vec::new();
        if t.rows > 0 {
            uses.push(format!(
                "{} {}",
                t.rows,
                if t.rows == 1 { "row" } else { "rows" }
            ));
        }
        if t.rules > 0 {
            uses.push(format!(
                "{} {}",
                t.rules,
                if t.rules == 1 { "rule" } else { "rules" }
            ));
        }
        return Err(error(
            StatusCode::CONFLICT,
            &format!(
                "{} is used by {}. Change those first.",
                t.name,
                uses.join(" and ")
            ),
        ));
    }
    sqlx::query("DELETE FROM statement_types WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::info!(org = %org.slug, name = %t.name, user = user.id, "statement type deleted");
    Ok(Json(org_types(&state.db, org.id).await.map_err(db_error)?))
}

#[derive(Deserialize)]
struct RowTypeBody {
    /// A type on the list, or null / "" to clear it.
    #[serde(rename = "type")]
    category: Option<String>,
}

#[derive(Serialize)]
struct RowTypeResult {
    #[serde(rename = "type")]
    category: Option<String>,
    type_manual: bool,
}

/// PATCH /api/orgs/{org}/statement-transactions/{id} — set or clear a row's type by hand.
async fn set_row_type(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<RowTypeBody>,
) -> Result<Json<RowTypeResult>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let wanted = body
        .category
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let category = match wanted {
        None => None,
        Some(t) => Some(
            canonical_name(&state.db, org.id, t)
                .await
                .map_err(db_error)?
                .ok_or_else(|| {
                    bad_request("That type isn't on the list. Add it under Type rules first.")
                })?,
        ),
    };
    let updated = set_type(&state.db, org.id, id, category.as_deref())
        .await
        .map_err(db_error)?;
    if !updated {
        return Err(not_found("Row not found."));
    }
    tracing::info!(org = %org.slug, row = id, r#type = ?category, user = user.id, "row type set by hand");
    Ok(Json(RowTypeResult {
        type_manual: category.is_some(),
        category,
    }))
}

/// Picking a type marks the row hand-set; clearing it doesn't (the next save may type it).
pub async fn set_type(
    db: &SqlitePool,
    org_id: i64,
    row_id: i64,
    category: Option<&str>,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE statement_transactions
         SET type = ?1, type_manual = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?3 AND org_id = ?4",
    )
    .bind(category)
    .bind(category.is_some())
    .bind(row_id)
    .bind(org_id)
    .execute(db)
    .await?
    .rows_affected()
        > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(clean_name("  FEES ").unwrap(), "FEES");
        assert!(clean_name("  ").is_err());
        assert!(clean_name(&"x".repeat(101)).is_err());
        assert!(clean_name("A\u{7}B").is_err());
    }
}
