//! Accounts: what statements are uploaded into. An org needs at least one before anything else
//! in Statements. An account has a name (required, unique in the org ignoring case) and an
//! optional identifier, e.g. the bank account number (unique in the org when set), which is
//! checked against the account number a statement states when it's uploaded.
//!
//! Rows are unique per account; type rules belong to an account. Deleting an account deletes
//! its uploads, rows, rules and row-document links (documents stay), after a preview.

use crate::db::{self, Db};
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
    delete_impact,
    rbac::{self, Permission},
    AppState,
};

const MAX_NAME_CHARS: usize = 100;
const MAX_IDENTIFIER_CHARS: usize = 64;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/orgs/{org}/statement-accounts", get(list).post(create))
        .route(
            "/api/orgs/{org}/statement-accounts/{id}",
            patch(update).delete(remove),
        )
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/delete-impact",
            get(delete_preview),
        )
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub identifier: Option<String>,
}

/// The account, if it's in the org (a 404 otherwise).
pub async fn find(db: &Db, org_id: i64, id: i64) -> Result<Account, Response> {
    db::query_as("SELECT id, name, identifier FROM statement_accounts WHERE id = ? AND org_id = ?")
        .bind(id)
        .bind(org_id)
        .fetch_optional(db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Account not found."))
}

/// Another account in the org whose identifier is `stated`, if any.
pub async fn with_identifier(
    db: &Db,
    org_id: i64,
    stated: &str,
) -> Result<Option<Account>, Response> {
    db::query_as(
        "SELECT id, name, identifier FROM statement_accounts WHERE org_id = ? AND identifier = ?",
    )
    .bind(org_id)
    .bind(stated)
    .fetch_optional(db)
    .await
    .map_err(db_error)
}

#[derive(Serialize, Deserialize)]
pub struct AccountSummary {
    pub id: i64,
    pub name: String,
    pub identifier: Option<String>,
    pub uploads: i64,
    pub rows: i64,
    /// When a statement was last saved into it.
    pub last_uploaded_at: Option<String>,
}

pub async fn org_accounts(db: &Db, org_id: i64) -> db::Result<Vec<AccountSummary>> {
    db::query_as(
        "SELECT a.id, a.name, a.identifier,
                (SELECT count(*) FROM statement_uploads u WHERE u.account_id = a.id) AS uploads,
                (SELECT count(*) FROM statement_transactions t WHERE t.account_id = a.id) AS rows,
                (SELECT max(u.uploaded_at) FROM statement_uploads u WHERE u.account_id = a.id)
                    AS last_uploaded_at
         FROM statement_accounts a WHERE a.org_id = ? ORDER BY a.name COLLATE NOCASE",
    )
    .bind(org_id)
    .fetch_all(db)
    .await
}

/// GET /api/orgs/{org}/statement-accounts
async fn list(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<Vec<AccountSummary>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(
        org_accounts(&state.db, org.id).await.map_err(db_error)?,
    ))
}

#[derive(Deserialize)]
struct AccountBody {
    name: String,
    #[serde(default)]
    identifier: Option<String>,
}

/// Trimmed name (required) and identifier (optional; blank = none).
fn clean(body: &AccountBody) -> Result<(String, Option<String>), Response> {
    let name = body.name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(bad_request(
            "Give the account a name of up to 100 characters.",
        ));
    }
    let identifier = body
        .identifier
        .as_deref()
        .map(str::trim)
        .filter(|i| !i.is_empty());
    if identifier.is_some_and(|i| {
        i.chars().count() > MAX_IDENTIFIER_CHARS || i.chars().any(char::is_control)
    }) {
        return Err(bad_request("Keep the identifier to 64 characters."));
    }
    Ok((name.to_owned(), identifier.map(str::to_owned)))
}

/// Which uniqueness rule a write broke, as a message.
fn conflict(e: &db::Error) -> Option<Response> {
    if !e.is_unique_violation() {
        return None;
    }
    Some(error(
        StatusCode::CONFLICT,
        if e.message.contains("identifier") {
            "Another account already has that identifier."
        } else {
            "There's already an account with that name."
        },
    ))
}

/// POST /api/orgs/{org}/statement-accounts — returns the new account.
async fn create(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Json(body): Json<AccountBody>,
) -> Result<(StatusCode, Json<Account>), Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let (name, identifier) = clean(&body)?;
    let account: Account = db::query_as(
        "INSERT INTO statement_accounts (org_id, name, identifier) VALUES (?, ?, ?)
         RETURNING id, name, identifier",
    )
    .bind(org.id)
    .bind(&name)
    .bind(&identifier)
    .fetch_one(&state.db)
    .await
    .map_err(|e| conflict(&e).unwrap_or_else(|| db_error(e)))?;
    tracing::info!(org = %org.slug, account = account.id, %name, user = user.id, "account created");
    Ok((StatusCode::CREATED, Json(account)))
}

/// PATCH /api/orgs/{org}/statement-accounts/{id} — change its name and identifier.
async fn update(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<AccountBody>,
) -> Result<Json<Account>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    find(&state.db, org.id, id).await?;
    let (name, identifier) = clean(&body)?;
    let account: Account = db::query_as(
        "UPDATE statement_accounts SET name = ?, identifier = ? WHERE id = ?
         RETURNING id, name, identifier",
    )
    .bind(&name)
    .bind(&identifier)
    .bind(id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| conflict(&e).unwrap_or_else(|| db_error(e)))?;
    tracing::info!(org = %org.slug, account = id, %name, user = user.id, "account updated");
    Ok(Json(account))
}

/// GET /api/orgs/{org}/statement-accounts/{id}/delete-impact
async fn delete_preview(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<Json<delete_impact::Impact>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    find(&state.db, org.id, id).await?;
    Ok(Json(
        delete_impact::impact(&state.db, &[("statement_accounts", vec![id])])
            .await
            .map_err(db_error)?,
    ))
}

#[derive(Deserialize)]
struct DeleteBody {
    /// The account's name, typed to confirm.
    confirm: String,
}

/// DELETE /api/orgs/{org}/statement-accounts/{id} — the account and, through the foreign keys,
/// its uploads, rows, rules and row-document links. Documents stay.
async fn remove(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<DeleteBody>,
) -> Result<StatusCode, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let account = find(&state.db, org.id, id).await?;
    if body.confirm.trim() != account.name {
        return Err(bad_request("Type the account's name exactly to confirm."));
    }
    db::query("DELETE FROM statement_accounts WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::warn!(org = %org.slug, account = id, name = %account.name, user = user.id, "account deleted");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_identifiers_are_cleaned() {
        let body = |name: &str, identifier: Option<&str>| AccountBody {
            name: name.into(),
            identifier: identifier.map(Into::into),
        };
        assert_eq!(
            clean(&body(" Joint ", Some(" 123 "))).unwrap(),
            ("Joint".into(), Some("123".into()))
        );
        assert_eq!(clean(&body("Joint", Some("  "))).unwrap().1, None);
        assert!(clean(&body("  ", None)).is_err());
        assert!(clean(&body("x", Some(&"9".repeat(65)))).is_err());
    }
}
