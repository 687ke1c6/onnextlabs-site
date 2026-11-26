//! Accounts: what statements are uploaded into. An org needs at least one before anything else
//! in Statements. An account has a name (required, unique in the org ignoring case) and an
//! optional identifier, e.g. the bank account number (unique in the org when set), which is
//! checked against the account number a statement states when it's uploaded.
//!
//! For statement PDFs an account also has a holder, a product name and a starting balance: the
//! balance at the start of a date, which statement balances are worked out from (by transaction
//! date, over all its rows). The PDF template last used to print it is remembered.
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
const MAX_DETAIL_CHARS: usize = 100;

/// The columns an [`Account`] is read from.
const COLUMNS: &str = "id, name, identifier, holder_name, product_name, opening_balance_cents,
                       opening_balance_date, last_template_id";

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

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub identifier: Option<String>,
    pub holder_name: Option<String>,
    pub product_name: Option<String>,
    /// The balance at the start of `opening_balance_date` (both or neither).
    pub opening_balance_cents: Option<i64>,
    pub opening_balance_date: Option<String>,
    /// The statement template it was last printed with.
    pub last_template_id: Option<i64>,
}

/// The account, if it's in the org (a 404 otherwise).
pub async fn find(db: &Db, org_id: i64, id: i64) -> Result<Account, Response> {
    db::query_as(db::Trusted(format!(
        "SELECT {COLUMNS} FROM statement_accounts WHERE id = ? AND org_id = ?"
    )))
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
    db::query_as(db::Trusted(format!(
        "SELECT {COLUMNS} FROM statement_accounts WHERE org_id = ? AND identifier = ?"
    )))
    .bind(org_id)
    .bind(stated)
    .fetch_optional(db)
    .await
    .map_err(db_error)
}

#[derive(Serialize, Deserialize)]
pub struct AccountSummary {
    #[serde(flatten)]
    pub account: Account,
    pub uploads: i64,
    pub rows: i64,
    /// When a statement was last saved into it.
    pub last_uploaded_at: Option<String>,
}

pub async fn org_accounts(db: &Db, org_id: i64) -> db::Result<Vec<AccountSummary>> {
    db::query_as(
        "SELECT a.id, a.name, a.identifier, a.holder_name, a.product_name,
                a.opening_balance_cents, a.opening_balance_date, a.last_template_id,
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
    /// Left as they are when not sent.
    #[serde(default)]
    details: Option<DetailsBody>,
}

/// What statement PDFs show about the account. Blank = none.
#[derive(Deserialize, Default)]
struct DetailsBody {
    #[serde(default)]
    holder_name: Option<String>,
    #[serde(default)]
    product_name: Option<String>,
    /// As typed, e.g. `40,957.20` or `-1200`.
    #[serde(default)]
    opening_balance: Option<String>,
    /// `YYYY-MM-DD`
    #[serde(default)]
    opening_balance_date: Option<String>,
}

#[derive(Debug, PartialEq)]
struct Details {
    holder_name: Option<String>,
    product_name: Option<String>,
    opening_balance_cents: Option<i64>,
    opening_balance_date: Option<String>,
}

fn clean_details(d: &DetailsBody) -> Result<Details, Response> {
    let text = |v: &Option<String>, what: &str| -> Result<Option<String>, Response> {
        let v = v.as_deref().map(str::trim).filter(|v| !v.is_empty());
        if v.is_some_and(|v| {
            v.chars().count() > MAX_DETAIL_CHARS || v.chars().any(char::is_control)
        }) {
            return Err(bad_request(&format!(
                "Keep the {what} to {MAX_DETAIL_CHARS} characters, on one line."
            )));
        }
        Ok(v.map(str::to_owned))
    };
    let amount = d
        .opening_balance
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let date = d
        .opening_balance_date
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let (cents, date) = match (amount, date) {
        (None, None) => (None, None),
        (Some(a), Some(d)) => {
            let cents = statements_core::id::amount_cents(a).map_err(|_| {
                bad_request("The starting balance must be an amount, e.g. 40957.20 or -1200.")
            })?;
            if !is_iso_date(d) {
                return Err(bad_request("The starting balance's date isn't a date."));
            }
            (Some(cents), Some(d.to_owned()))
        }
        _ => {
            return Err(bad_request(
                "Give the starting balance and its date, or neither.",
            ))
        }
    };
    Ok(Details {
        holder_name: text(&d.holder_name, "account holder")?,
        product_name: text(&d.product_name, "product name")?,
        opening_balance_cents: cents,
        opening_balance_date: date,
    })
}

/// `YYYY-MM-DD`, a real day.
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| s[r].parse::<u32>().ok();
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
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
    let d = clean_details(body.details.as_ref().unwrap_or(&DetailsBody::default()))?;
    let account: Account = db::query_as(db::Trusted(format!(
        "INSERT INTO statement_accounts
             (org_id, name, identifier, holder_name, product_name, opening_balance_cents,
              opening_balance_date)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING {COLUMNS}"
    )))
    .bind(org.id)
    .bind(&name)
    .bind(&identifier)
    .bind(&d.holder_name)
    .bind(&d.product_name)
    .bind(d.opening_balance_cents)
    .bind(&d.opening_balance_date)
    .fetch_one(&state.db)
    .await
    .map_err(|e| conflict(&e).unwrap_or_else(|| db_error(e)))?;
    tracing::info!(org = %org.slug, account = account.id, %name, user = user.id, "account created");
    Ok((StatusCode::CREATED, Json(account)))
}

/// PATCH /api/orgs/{org}/statement-accounts/{id} — change its name and identifier, and its
/// details when they're sent.
async fn update(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<AccountBody>,
) -> Result<Json<Account>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let current = find(&state.db, org.id, id).await?;
    let (name, identifier) = clean(&body)?;
    let d = match &body.details {
        Some(d) => clean_details(d)?,
        None => Details {
            holder_name: current.holder_name,
            product_name: current.product_name,
            opening_balance_cents: current.opening_balance_cents,
            opening_balance_date: current.opening_balance_date,
        },
    };
    let account: Account = db::query_as(db::Trusted(format!(
        "UPDATE statement_accounts
         SET name = ?, identifier = ?, holder_name = ?, product_name = ?,
             opening_balance_cents = ?, opening_balance_date = ?
         WHERE id = ?
         RETURNING {COLUMNS}"
    )))
    .bind(&name)
    .bind(&identifier)
    .bind(&d.holder_name)
    .bind(&d.product_name)
    .bind(d.opening_balance_cents)
    .bind(&d.opening_balance_date)
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
            details: None,
        };
        assert_eq!(
            clean(&body(" Joint ", Some(" 123 "))).unwrap(),
            ("Joint".into(), Some("123".into()))
        );
        assert_eq!(clean(&body("Joint", Some("  "))).unwrap().1, None);
        assert!(clean(&body("  ", None)).is_err());
        assert!(clean(&body("x", Some(&"9".repeat(65)))).is_err());
    }

    #[tokio::test]
    async fn summaries_read_every_column() {
        let db = db::test_db().await;
        let org: i64 = db::query_scalar(
            "INSERT INTO organisations (slug, name) VALUES ('a', 'A') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        db::query(
            "INSERT INTO statement_accounts (org_id, name, holder_name, opening_balance_cents, opening_balance_date)
             VALUES (?, 'Main', 'Mr J Smith', -150, '2026-10-04'), (?, 'Other', NULL, NULL, NULL)",
        )
        .bind(org)
        .bind(org)
        .execute(&db)
        .await
        .unwrap();
        let list = org_accounts(&db, org).await.unwrap();
        assert_eq!(list[0].account.holder_name.as_deref(), Some("Mr J Smith"));
        assert_eq!(list[0].account.opening_balance_cents, Some(-150));
        assert_eq!(list[1].account.opening_balance_date, None);
        assert_eq!(list[1].rows, 0);
    }

    #[test]
    fn details_are_cleaned_and_balances_need_a_date() {
        let d = |amount: Option<&str>, date: Option<&str>| DetailsBody {
            holder_name: Some("  Mr J Smith ".into()),
            product_name: Some(" ".into()),
            opening_balance: amount.map(Into::into),
            opening_balance_date: date.map(Into::into),
        };
        assert_eq!(
            clean_details(&d(Some("40,957.20"), Some("2026-10-04"))).unwrap(),
            Details {
                holder_name: Some("Mr J Smith".into()),
                product_name: None,
                opening_balance_cents: Some(4_095_720),
                opening_balance_date: Some("2026-10-04".into()),
            }
        );
        assert_eq!(
            clean_details(&d(Some(" "), None))
                .unwrap()
                .opening_balance_cents,
            None
        );
        assert!(clean_details(&d(Some("10"), None)).is_err());
        assert!(clean_details(&d(None, Some("2026-10-04"))).is_err());
        assert!(clean_details(&d(Some("ten"), Some("2026-10-04"))).is_err());
        assert!(clean_details(&d(Some("10"), Some("2026-02-30"))).is_err());
        assert!(
            is_iso_date("2024-02-29") && !is_iso_date("2025-02-29") && !is_iso_date("2025-1-01")
        );
    }
}
