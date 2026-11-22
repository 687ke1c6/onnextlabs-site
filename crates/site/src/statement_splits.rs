//! Splits, deleting rows added by hand, and a row's effective date.
//!
//! A split moves part of a row's amount to another type or date (e.g. the utilities share of a rent
//! payment) by adding rows by hand: ordinary rows in `statement_transactions`, with `split_of` set
//! to the original row's `txn_id`. They sort, filter and total like any other row. A split's rows
//! add up to zero, so the account's balance never changes; the original row is never touched.
//!
//! A split is edited as a whole, from the original row: the rows sent replace the split's rows
//! (rows sent with their `id` are updated in place, keeping their attached documents). No rows
//! removes the split. A split's rows can't themselves be split.
//!
//! Rows added by hand (`hand_added`: split rows, and Google Sheet rows without an id) can be
//! deleted on their own; rows from bank statements go only with their statement. A split row is
//! removed by changing its split, so a split always adds up to zero.
//!
//! Needs `write-statements`.

use crate::db::{self, Db};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::Response,
    routing::{patch, put},
    Json, Router,
};
use serde::Deserialize;
use statements_core::{
    id::{amount_cents, format_cents, split_row_id},
    statement::check_iso_date,
};

use crate::{
    api::{bad_request, db_error, not_found},
    auth::CurrentUser,
    params,
    rbac::{self, Permission},
    statement_rows::{self, RowDetail},
    statement_types, AppState,
};

/// More than this many rows in one split is almost certainly a mistake.
const MAX_SPLIT_ROWS: usize = 20;
const MAX_DESCRIPTION_CHARS: usize = 300;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/orgs/{org}/statement-transactions/{id}/split",
            put(put_split),
        )
        .route(
            "/api/orgs/{org}/statement-transactions/{id}/effective-date",
            patch(set_effective_date),
        )
}

/// One row of a split, as sent by the page.
#[derive(Deserialize, Debug, Clone)]
pub struct SplitRowInput {
    /// An existing row of this split (updated in place); absent for a new one.
    pub id: Option<i64>,
    pub date: String,
    pub effective_date: Option<String>,
    pub description: String,
    pub amount: String,
    /// A type on the list, or null / "" for none.
    #[serde(rename = "type")]
    pub category: Option<String>,
}

#[derive(Deserialize)]
struct SplitBody {
    rows: Vec<SplitRowInput>,
}

#[derive(Debug)]
enum SplitError {
    NotFound,
    Invalid(String),
    Db(db::Error),
}

impl From<db::Error> for SplitError {
    fn from(e: db::Error) -> Self {
        SplitError::Db(e)
    }
}

impl From<SplitError> for Response {
    fn from(e: SplitError) -> Self {
        match e {
            SplitError::NotFound => not_found("Row not found."),
            SplitError::Invalid(msg) => bad_request(&msg),
            SplitError::Db(e) => db_error(e),
        }
    }
}

/// PUT /api/orgs/{org}/statement-transactions/{id}/split — set the split of row `id`: its rows
/// become exactly the ones sent (none removes it). Returns the row's details.
async fn put_split(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<SplitBody>,
) -> Result<Json<RowDetail>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let rows = body.rows.len();
    save_split(&state.db, org.id, id, body.rows).await?;
    tracing::info!(org = %org.slug, row = id, rows, user = user.id, "split saved");
    Ok(Json(statement_rows::detail(&state, &org, id).await?))
}

/// A split row, checked and ready to store.
struct CheckedRow {
    id: Option<i64>,
    date: String,
    effective_date: Option<String>,
    description: String,
    amount: String,
    category: Option<String>,
}

#[derive(Deserialize)]
struct Parent {
    account_id: i64,
    txn_id: i64,
    currency: String,
    split_of: Option<i64>,
}

#[derive(Deserialize)]
struct ExistingSplitRow {
    id: i64,
    txn_id: i64,
}

async fn save_split(
    db: &Db,
    org_id: i64,
    parent_id: i64,
    rows: Vec<SplitRowInput>,
) -> Result<(), SplitError> {
    let parent: Parent = db::query_as(
        "SELECT account_id, txn_id, currency, split_of FROM statement_transactions
         WHERE id = ? AND org_id = ?",
    )
    .bind(parent_id)
    .bind(org_id)
    .fetch_optional(db)
    .await?
    .ok_or(SplitError::NotFound)?;
    if parent.split_of.is_some() {
        return Err(SplitError::Invalid(
            "This row is part of a split. Change the split from its original row.".into(),
        ));
    }
    let existing: Vec<ExistingSplitRow> = db::query_as(
        "SELECT id, txn_id FROM statement_transactions WHERE account_id = ? AND split_of = ?",
    )
    .bind(parent.account_id)
    .bind(parent.txn_id)
    .fetch_all(db)
    .await?;
    let existing: HashMap<i64, i64> = existing.into_iter().map(|r| (r.id, r.txn_id)).collect();
    let rows = check_rows(db, org_id, &existing, rows).await?;

    let kept: HashSet<i64> = rows.iter().filter_map(|r| r.id).collect();
    // Ids stay with their rows; new rows take the first free position.
    let mut taken: HashSet<i64> = kept.iter().map(|id| existing[id]).collect();
    let mut next = 0u32;

    let mut batch = db::Batch::new();
    for (&id, _) in existing.iter().filter(|(id, _)| !kept.contains(id)) {
        batch.push(
            "DELETE FROM statement_transactions WHERE id = ?",
            params![id],
        );
    }
    for r in rows {
        let type_manual = r.category.is_some();
        match r.id {
            Some(id) => batch.push(
                "UPDATE statement_transactions
                 SET date = ?, effective_date = ?, description = ?, amount = ?, type = ?,
                     type_manual = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ? AND account_id = ? AND split_of = ?",
                params![
                    r.date,
                    r.effective_date,
                    r.description,
                    r.amount,
                    r.category,
                    type_manual,
                    id,
                    parent.account_id,
                    parent.txn_id,
                ],
            ),
            None => {
                let txn_id = loop {
                    let candidate = split_row_id(parent.txn_id, next);
                    next += 1;
                    if taken.insert(candidate) {
                        break candidate;
                    }
                };
                batch.push(
                    "INSERT INTO statement_transactions
                         (org_id, account_id, txn_id, date, amount, description, currency, type,
                          type_manual, effective_date, occurrence, split_of, hand_added)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, 1)",
                    params![
                        org_id,
                        parent.account_id,
                        txn_id,
                        r.date,
                        r.amount,
                        r.description,
                        parent.currency.clone(),
                        r.category,
                        type_manual,
                        r.effective_date,
                        parent.txn_id,
                    ],
                )
            }
        };
    }
    db::run(db, batch).await?;
    Ok(())
}

/// Every row valid, ids belonging to this split, types on the list (in its spelling), and the
/// amounts adding up to zero.
async fn check_rows(
    db: &Db,
    org_id: i64,
    existing: &HashMap<i64, i64>,
    rows: Vec<SplitRowInput>,
) -> Result<Vec<CheckedRow>, SplitError> {
    let invalid = |msg: String| Err(SplitError::Invalid(msg));
    if rows.len() > MAX_SPLIT_ROWS {
        return invalid(format!("A split can have at most {MAX_SPLIT_ROWS} rows."));
    }
    let mut seen = HashSet::new();
    let mut total = 0i64;
    let mut checked = Vec::with_capacity(rows.len());
    for (i, r) in rows.into_iter().enumerate() {
        let n = i + 1;
        if let Some(id) = r.id {
            if !existing.contains_key(&id) || !seen.insert(id) {
                return invalid(format!("Row {n} isn't part of this split."));
            }
        }
        let Ok(date) = check_iso_date(r.date.trim()) else {
            return invalid(format!("Row {n}: give a date."));
        };
        let effective_date = match r.effective_date.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(d) => match check_iso_date(d) {
                Ok(d) => Some(d),
                Err(_) => return invalid(format!("Row {n}: the effective date isn't a date.")),
            },
        };
        let description = r.description.trim().to_string();
        if description.is_empty()
            || description.chars().count() > MAX_DESCRIPTION_CHARS
            || description.chars().any(char::is_control)
        {
            return invalid(format!(
                "Row {n}: give a description of up to {MAX_DESCRIPTION_CHARS} characters."
            ));
        }
        let cents = match amount_cents(&r.amount) {
            Ok(0) => return invalid(format!("Row {n}: the amount can't be zero.")),
            Ok(c) => c,
            Err(_) => {
                return invalid(format!(
                    "Row {n}: the amount isn't a number (e.g. -1234.50)."
                ))
            }
        };
        total += cents;
        let category = match r
            .category
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            None => None,
            Some(t) => match statement_types::canonical_name(db, org_id, t).await? {
                Some(name) => Some(name),
                None => return invalid(format!("Row {n}: {t:?} isn't on the types list.")),
            },
        };
        checked.push(CheckedRow {
            id: r.id,
            date,
            effective_date,
            description,
            amount: format_cents(cents),
            category,
        });
    }
    if total != 0 {
        return invalid(format!(
            "The rows add up to {}; a split's rows must add up to 0.",
            format_cents(total)
        ));
    }
    Ok(checked)
}

#[derive(Deserialize)]
struct DeletableRow {
    account_id: i64,
    txn_id: i64,
    hand_added: bool,
    split_of: Option<i64>,
}

/// DELETE /api/orgs/{org}/statement-transactions/{id} — delete a row added by hand, with its
/// split's rows if it has a split. (Routed by `statement_rows`, with the row's other endpoints.)
pub async fn delete_row(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<StatusCode, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let removed = delete_hand_added(&state.db, org.id, id).await?;
    tracing::info!(org = %org.slug, row = id, removed, user = user.id, "hand-added row deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the hand-added row `id` and its split's rows; returns how many rows went.
async fn delete_hand_added(db: &Db, org_id: i64, id: i64) -> Result<usize, SplitError> {
    let row: DeletableRow = db::query_as(
        "SELECT account_id, txn_id, hand_added, split_of FROM statement_transactions
         WHERE id = ? AND org_id = ?",
    )
    .bind(id)
    .bind(org_id)
    .fetch_optional(db)
    .await?
    .ok_or(SplitError::NotFound)?;
    if row.split_of.is_some() {
        return Err(SplitError::Invalid(
            "This row is part of a split. Change or remove the split from its original row.".into(),
        ));
    }
    if !row.hand_added {
        return Err(SplitError::Invalid(
            "Only rows added by hand can be deleted. Rows from a bank statement go when their statement is deleted.".into(),
        ));
    }
    let mut batch = db::Batch::new();
    let splits = batch.push(
        "DELETE FROM statement_transactions WHERE account_id = ? AND split_of = ?",
        params![row.account_id, row.txn_id],
    );
    batch.push(
        "DELETE FROM statement_transactions WHERE id = ?",
        params![id],
    );
    let results = db::run(db, batch).await?;
    Ok(results[splits].rows_affected as usize + 1)
}

#[derive(Deserialize)]
struct EffectiveDateBody {
    /// `YYYY-MM-DD`, or null / "" to clear it.
    effective_date: Option<String>,
}

/// PATCH /api/orgs/{org}/statement-transactions/{id}/effective-date — set or clear a row's
/// effective date (any row). Returns the row's details.
async fn set_effective_date(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<EffectiveDateBody>,
) -> Result<Json<RowDetail>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let date = match body.effective_date.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(d) => Some(check_iso_date(d).map_err(|_| bad_request("That isn't a date."))?),
    };
    let updated = db::query(
        "UPDATE statement_transactions
         SET effective_date = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ? AND org_id = ?",
    )
    .bind(date.clone())
    .bind(id)
    .bind(org.id)
    .execute(&state.db)
    .await
    .map_err(db_error)?
    .rows_affected();
    if updated == 0 {
        return Err(not_found("Row not found."));
    }
    tracing::info!(org = %org.slug, row = id, effective_date = ?date, user = user.id, "effective date set");
    Ok(Json(statement_rows::detail(&state, &org, id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An org with a "Main" account, the types RENT and UTILITIES, and one statement row
    /// (+10000.00 RENT). Returns (db, org, parent row id, parent txn_id).
    async fn setup() -> (Db, i64, i64, i64) {
        let db = db::test_db().await;
        let org: i64 = db::query_scalar(
            "INSERT INTO organisations (slug, name) VALUES ('a', 'A') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let account: i64 = db::query_scalar(
            "INSERT INTO statement_accounts (org_id, name) VALUES (?, 'Main') RETURNING id",
        )
        .bind(org)
        .fetch_one(&db)
        .await
        .unwrap();
        db::query(
            "INSERT INTO statement_types (org_id, name) VALUES (?1, 'RENT'), (?1, 'UTILITIES')",
        )
        .bind(org)
        .execute(&db)
        .await
        .unwrap();
        let parent: i64 = db::query_scalar(
            "INSERT INTO statement_transactions
                 (org_id, account_id, txn_id, date, amount, description, currency, type)
             VALUES (?, ?, 77, '2025-06-02', '10,000.00', 'TENANT', 'ZAR', 'RENT') RETURNING id",
        )
        .bind(org)
        .bind(account)
        .fetch_one(&db)
        .await
        .unwrap();
        (db, org, parent, 77)
    }

    fn row(description: &str, amount: &str, category: Option<&str>) -> SplitRowInput {
        SplitRowInput {
            id: None,
            date: "2025-06-02".into(),
            effective_date: None,
            description: description.into(),
            amount: amount.into(),
            category: category.map(Into::into),
        }
    }

    async fn rows_of(db: &Db, account_parent: i64) -> Vec<statement_rows::SplitRow> {
        let account: i64 = db::query_scalar("SELECT id FROM statement_accounts")
            .fetch_one(db)
            .await
            .unwrap();
        statement_rows::split_rows(db, account, account_parent)
            .await
            .unwrap()
    }

    fn message(e: SplitError) -> String {
        match e {
            SplitError::Invalid(m) => m,
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_split_adds_rows_that_add_up_to_zero() {
        let (db, org, parent, txn) = setup().await;
        let mut utilities = row("Utilities payment", "4,726.16", Some("utilities"));
        utilities.effective_date = Some("2025-05-31".into());
        save_split(
            &db,
            org,
            parent,
            vec![
                row("Split out utilities from rent", "-4726.16", Some("RENT")),
                utilities,
            ],
        )
        .await
        .unwrap();

        let rows = rows_of(&db, txn).await;
        let got: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.amount.as_str(),
                    r.category.as_deref(),
                    r.effective_date.as_deref(),
                    r.currency.as_str(),
                )
            })
            .collect();
        // Amounts in one format; the list's spelling of the type.
        assert_eq!(
            got,
            [
                ("-4726.16", Some("RENT"), None, "ZAR"),
                ("4726.16", Some("UTILITIES"), Some("2025-05-31"), "ZAR")
            ]
        );
        assert_eq!(rows[0].txn_id, split_row_id(txn, 0));
        assert_eq!(rows[1].txn_id, split_row_id(txn, 1));
        let manual: i64 = db::query_scalar(
            "SELECT count(*) FROM statement_transactions WHERE split_of IS NOT NULL AND type_manual = 1",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(manual, 2);

        // What "Send to Google Sheets" adds after the row.
        let account: i64 = db::query_scalar("SELECT id FROM statement_accounts")
            .fetch_one(&db)
            .await
            .unwrap();
        let sent = crate::statements::split_rows_of(&db, account, &[txn, 12345])
            .await
            .unwrap();
        let sent: Vec<_> = sent
            .iter()
            .map(|t| (t.split_of, t.amount.as_str()))
            .collect();
        assert_eq!(sent, [(Some(txn), "-4726.16"), (Some(txn), "4726.16")]);
    }

    #[tokio::test]
    async fn bad_splits_are_refused() {
        let (db, org, parent, _) = setup().await;
        let err = |rows| async { message(save_split(&db, org, parent, rows).await.unwrap_err()) };
        assert!(err(vec![row("A", "-1", None), row("B", "2", None)])
            .await
            .contains("add up to 1.00"));
        assert!(err(vec![row("A", "-1", Some("NOPE")), row("B", "1", None)])
            .await
            .contains("isn't on the types list"));
        assert!(err(vec![row("A", "0", None)])
            .await
            .contains("can't be zero"));
        assert!(err(vec![row(" ", "-1", None), row("B", "1", None)])
            .await
            .contains("Row 1: give a description"));
        let mut bad_date = row("A", "-1", None);
        bad_date.date = "2025-02-30".into();
        assert!(err(vec![bad_date, row("B", "1", None)])
            .await
            .contains("Row 1: give a date"));
        let mut stranger = row("A", "-1", None);
        stranger.id = Some(parent);
        assert!(err(vec![stranger, row("B", "1", None)])
            .await
            .contains("isn't part of this split"));
        assert!(matches!(
            save_split(&db, org + 1, parent, vec![]).await,
            Err(SplitError::NotFound)
        ));
    }

    #[tokio::test]
    async fn editing_keeps_ids_and_removing_all_rows_removes_the_split() {
        let (db, org, parent, txn) = setup().await;
        save_split(
            &db,
            org,
            parent,
            vec![row("A", "-100", None), row("B", "100", None)],
        )
        .await
        .unwrap();
        let before = rows_of(&db, txn).await;

        // Keep A (changed), drop B, add C and D.
        let mut a = row("A changed", "-300", Some("RENT"));
        a.id = Some(before[0].id);
        save_split(
            &db,
            org,
            parent,
            vec![a, row("C", "100", None), row("D", "200", None)],
        )
        .await
        .unwrap();
        let after = rows_of(&db, txn).await;
        assert_eq!(after.len(), 3);
        assert_eq!(
            (after[0].id, after[0].txn_id),
            (before[0].id, before[0].txn_id)
        );
        assert_eq!(after[0].description, "A changed");
        // B's position is free again, so C takes it.
        assert_eq!(after[1].txn_id, split_row_id(txn, 1));
        assert_eq!(after[2].txn_id, split_row_id(txn, 2));

        // A split row can't be split itself.
        let err = save_split(&db, org, after[0].id, vec![]).await.unwrap_err();
        assert!(message(err).contains("part of a split"));

        save_split(&db, org, parent, vec![]).await.unwrap();
        assert!(rows_of(&db, txn).await.is_empty());
    }

    #[tokio::test]
    async fn splits_outlive_their_statement_and_link_up_again() {
        let (db, org, parent, txn) = setup().await;
        save_split(
            &db,
            org,
            parent,
            vec![row("A", "-1", None), row("B", "1", None)],
        )
        .await
        .unwrap();
        // The statement is deleted (its row with it), then saved again.
        db::query("DELETE FROM statement_transactions WHERE id = ?")
            .bind(parent)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(rows_of(&db, txn).await.len(), 2);
        db::query(
            "INSERT INTO statement_transactions (org_id, account_id, txn_id, date, amount, description, currency)
             SELECT ?, id, ?, '2025-06-02', '10,000.00', 'TENANT', 'ZAR' FROM statement_accounts",
        )
        .bind(org)
        .bind(txn)
        .execute(&db)
        .await
        .unwrap();
        let again: i64 = db::query_scalar("SELECT id FROM statement_transactions WHERE txn_id = ?")
            .bind(txn)
            .fetch_one(&db)
            .await
            .unwrap();
        // Editing from the new row sees the same split.
        save_split(&db, org, again, vec![]).await.unwrap();
        assert!(rows_of(&db, txn).await.is_empty());
    }

    #[tokio::test]
    async fn only_hand_added_rows_can_be_deleted_and_take_their_split_with_them() {
        let (db, org, bank_row, _) = setup().await;
        let err = delete_hand_added(&db, org, bank_row).await.unwrap_err();
        assert!(message(err).contains("Only rows added by hand"));

        // A row added by hand in a Google Sheet, which has been split.
        let hand: i64 = db::query_scalar(
            "INSERT INTO statement_transactions
                 (org_id, account_id, txn_id, date, amount, description, currency, hand_added)
             SELECT ?, id, 88, '2025-06-30', '-50.00', 'Hand', 'ZAR', 1 FROM statement_accounts
             RETURNING id",
        )
        .bind(org)
        .fetch_one(&db)
        .await
        .unwrap();
        save_split(
            &db,
            org,
            hand,
            vec![row("A", "-1", None), row("B", "1", None)],
        )
        .await
        .unwrap();
        let split_row = rows_of(&db, 88).await[0].id;
        let err = delete_hand_added(&db, org, split_row).await.unwrap_err();
        assert!(message(err).contains("part of a split"));
        assert!(matches!(
            delete_hand_added(&db, org + 1, hand).await,
            Err(SplitError::NotFound)
        ));

        assert_eq!(delete_hand_added(&db, org, hand).await.unwrap(), 3);
        let left: Vec<i64> = db::query_scalar("SELECT id FROM statement_transactions")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(left, [bank_row]);
    }
}
