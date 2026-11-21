//! The org's statement types (the values a row's `type` can take) and editing a row's type.
//!
//! Rows and type rules keep the type's *name*, so the rules engine and Google Sheets are
//! unchanged. Renaming a type renames it everywhere in one transaction; a type still used by a
//! row or rule can't be deleted.
//!
//! A type someone picks on a row is hand-set (`type_manual`): saving a statement never changes
//! it. Clearing it makes the row blank and not hand-set, so the next save can type it again.

use crate::db::Db;
use std::sync::Arc;

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::Response,
    routing::{get, patch},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    api::{bad_request, db_error, error, not_found},
    auth::CurrentUser,
    db, params,
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
}

#[derive(Serialize, Deserialize)]
pub struct TypeInfo {
    id: i64,
    name: String,
    /// Rows and rules using it.
    rows: i64,
    rules: i64,
}

pub async fn org_types(db: &Db, org_id: i64) -> db::Result<Vec<TypeInfo>> {
    db::query_as(
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
pub async fn canonical_name(db: &Db, org_id: i64, name: &str) -> db::Result<Option<String>> {
    db::query_scalar("SELECT name FROM statement_types WHERE org_id = ? AND name = ?")
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
    let count: i64 = db::query_scalar("SELECT count(*) FROM statement_types WHERE org_id = ?")
        .bind(org.id)
        .fetch_one(&state.db)
        .await
        .map_err(db_error)?;
    if count >= MAX_TYPES {
        return Err(bad_request("At most 500 types."));
    }
    match db::query("INSERT INTO statement_types (org_id, name) VALUES (?, ?)")
        .bind(org.id)
        .bind(&name)
        .execute(&state.db)
        .await
    {
        Ok(_) => {}
        Err(e) if e.is_unique_violation() => return Err(taken()),
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
        Err(e) if e.is_unique_violation() => return Err(taken()),
        Err(e) => return Err(db_error(e)),
    }
    Ok(Json(org_types(&state.db, org.id).await.map_err(db_error)?))
}

/// The old name, or None if there's no such type in the org.
async fn rename(db: &Db, org_id: i64, id: i64, name: &str) -> db::Result<Option<String>> {
    // One batch: rows and rules move to the new name first (matching the old name by
    // subquery), then the type itself. With no such type, nothing matches and nothing changes.
    let mut batch = db::Batch::new();
    let old = batch.push(
        "SELECT name FROM statement_types WHERE id = ? AND org_id = ?",
        params![id, org_id],
    );
    batch.push(
        "UPDATE statement_transactions SET type = ?1
         WHERE org_id = ?2 AND type = (SELECT name FROM statement_types WHERE id = ?3 AND org_id = ?2)",
        params![name, org_id, id],
    );
    batch.push(
        "UPDATE statement_type_rules SET type = ?1
         WHERE org_id = ?2 AND type = (SELECT name FROM statement_types WHERE id = ?3 AND org_id = ?2)",
        params![name, org_id, id],
    );
    batch.push(
        "UPDATE statement_types SET name = ?1 WHERE id = ?3 AND org_id = ?2",
        params![name, org_id, id],
    );
    let mut results = db::run(db, batch).await?;
    Ok(results
        .swap_remove(old)
        .into_first()
        .and_then(db::Value::into_string))
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
    db::query("DELETE FROM statement_types WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::info!(org = %org.slug, name = %t.name, user = user.id, "statement type deleted");
    Ok(Json(org_types(&state.db, org.id).await.map_err(db_error)?))
}

#[derive(Deserialize)]
pub struct RowTypeBody {
    /// A type on the list, or null / "" to clear it.
    #[serde(rename = "type")]
    category: Option<String>,
}

#[derive(Serialize)]
pub struct RowTypeResult {
    #[serde(rename = "type")]
    category: Option<String>,
    type_manual: bool,
}

/// PATCH /api/orgs/{org}/statement-transactions/{id} — set or clear a row's type by hand.
/// (Routed by `statement_rows`, with the row's other endpoints.)
pub async fn set_row_type(
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
    db: &Db,
    org_id: i64,
    row_id: i64,
    category: Option<&str>,
) -> db::Result<bool> {
    Ok(db::query(
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

    async fn exec(db: &Db, sql: &'static str) {
        db::query(sql).execute(db).await.unwrap();
    }

    async fn types_in_use(db: &Db) -> Vec<(String, String)> {
        db::query_as(
            "SELECT 'row', type FROM statement_transactions WHERE type IS NOT NULL
             UNION ALL SELECT 'rule', type FROM statement_type_rules ORDER BY 1, 2",
        )
        .fetch_all(db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn rename_moves_rows_and_rules_and_only_in_its_org() {
        let db = db::test_db().await;
        exec(
            &db,
            "INSERT INTO organisations (id, slug, name) VALUES (1, 'a', 'A'), (2, 'b', 'B')",
        )
        .await;
        exec(&db, "INSERT INTO statement_accounts (id, org_id, name) VALUES (1, 1, 'Main'), (2, 2, 'Main')").await;
        exec(
            &db,
            "INSERT INTO statement_types (id, org_id, name) VALUES (1, 1, 'Fees'), (2, 2, 'Fees')",
        )
        .await;
        exec(&db, "INSERT INTO statement_transactions (org_id, account_id, txn_id, date, amount, description, currency, type)
                   VALUES (1, 1, 1, '2026-01-01', '1', 'x', 'ZAR', 'Fees'),
                          (1, 1, 2, '2026-01-01', '1', 'y', 'ZAR', NULL),
                          (2, 2, 1, '2026-01-01', '1', 'x', 'ZAR', 'Fees')").await;
        exec(
            &db,
            "INSERT INTO statement_type_rules (org_id, account_id, position, type, contains)
                   VALUES (1, 1, 0, 'Fees', '[]'), (2, 2, 0, 'Fees', '[]')",
        )
        .await;

        assert_eq!(
            rename(&db, 1, 1, "Charges").await.unwrap(),
            Some("Fees".into())
        );
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert_eq!(
            types_in_use(&db).await,
            pairs(&[
                ("row", "Charges"),
                ("row", "Fees"),
                ("rule", "Charges"),
                ("rule", "Fees")
            ])
        );

        // Another org's id, or none at all: nothing found, nothing changed.
        assert_eq!(rename(&db, 1, 2, "Other").await.unwrap(), None);
        assert_eq!(rename(&db, 1, 99, "Other").await.unwrap(), None);
        let names: Vec<String> = db::query_scalar("SELECT name FROM statement_types ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(names, ["Charges", "Fees"]);

        // A name taken in the org fails and rolls the whole rename back.
        exec(
            &db,
            "INSERT INTO statement_types (id, org_id, name) VALUES (3, 1, 'Bank')",
        )
        .await;
        let err = rename(&db, 1, 1, "bank").await.unwrap_err();
        assert!(err.is_unique_violation());
        assert_eq!(
            types_in_use(&db).await,
            pairs(&[
                ("row", "Charges"),
                ("row", "Fees"),
                ("rule", "Charges"),
                ("rule", "Fees")
            ])
        );
    }
}
