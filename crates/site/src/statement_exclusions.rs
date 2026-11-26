//! Upload exclusions: lines a bank statement states that are never saved.
//!
//! The case they're for: a card authorisation that isn't settled yet (`OUTSTANDING CARD
//! AUTHORISATION …`). It's on the statement now, but once it settles the bank shows it as a new
//! line and drops the authorisation, so saving it would count the transaction twice.
//!
//! Rules belong to an account. A line is left out when its description contains any of the
//! rule's texts, ignoring case. Previewing and saving a statement leave matching lines out
//! (`statements.rs`); the preview lists them.
//!
//! Rows saved before a rule existed stay until deleted: `…/exclusions/{rule}/matches` lists the
//! account's saved rows a rule matches, and they can be deleted from there (with
//! `delete-statement-rows`) or one at a time from the row. A deleted row isn't remembered: an
//! exclusion is what keeps it out of later uploads.
//!
//! Viewing needs `read-statements`; editing the rules `write-statements`.

use crate::db::{self, Db};
use std::{collections::HashSet, sync::Arc};

use axum::{
    extract::{Path as UrlPath, State},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use statements_core::statement::{Statement, Transaction};

use crate::{
    api::{bad_request, db_error, not_found},
    auth::CurrentUser,
    params,
    rbac::{self, Permission},
    statement_accounts, AppState,
};

const MAX_RULES: usize = 200;
const MAX_TEXTS_PER_RULE: usize = 50;
const MAX_TEXT_CHARS: usize = 200;
const MAX_NOTE_CHARS: usize = 200;
/// At most this many rows are deleted in one go.
const MAX_DELETE: usize = 5_000;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/exclusions",
            get(get_rules).put(put_rules),
        )
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/exclusions/{rule}/matches",
            get(matches),
        )
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/exclusions/{rule}/matches/delete",
            post(delete_matches),
        )
}

/// A rule as the API and the database hold it: `contains` as typed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RuleJson {
    pub contains: Vec<String>,
    #[serde(default)]
    pub note: String,
}

/// A checked rule, ready to match.
#[derive(Debug, Clone)]
pub struct Exclusion {
    /// Upper-cased.
    contains: Vec<String>,
}

impl Exclusion {
    /// The description contains any of the texts, ignoring case.
    pub fn matches(&self, description: &str) -> bool {
        let description = description.to_uppercase();
        self.contains
            .iter()
            .any(|c| description.contains(c.as_str()))
    }
}

impl RuleJson {
    fn check(&self) -> Result<Exclusion, String> {
        if self.contains.is_empty() {
            return Err("give at least one text to look for".into());
        }
        if self.contains.len() > MAX_TEXTS_PER_RULE {
            return Err(format!("at most {MAX_TEXTS_PER_RULE} texts to look for"));
        }
        // Blank (or only spaces) would match nearly every line.
        if self.contains.iter().any(|c| c.trim().is_empty()) {
            return Err("a text to look for is empty".into());
        }
        if self
            .contains
            .iter()
            .any(|c| c.chars().count() > MAX_TEXT_CHARS)
        {
            return Err("a text to look for is too long".into());
        }
        if self.note.chars().count() > MAX_NOTE_CHARS {
            return Err("the note is too long".into());
        }
        Ok(Exclusion {
            contains: self.contains.iter().map(|c| c.to_uppercase()).collect(),
        })
    }
}

/// Tests elsewhere: a rule, checked.
#[cfg(test)]
pub fn tests_check(rule: &RuleJson) -> Exclusion {
    rule.check().expect("a valid rule")
}

/// Takes the lines the rules exclude out of the statement, in file order, and returns them.
pub fn take_excluded(st: &mut Statement, rules: &[Exclusion]) -> Vec<Transaction> {
    if rules.is_empty() {
        return Vec::new();
    }
    let (out, keep) = std::mem::take(&mut st.transactions)
        .into_iter()
        .partition(|t| rules.iter().any(|r| r.matches(&t.description)));
    st.transactions = keep;
    out
}

#[derive(Deserialize)]
struct RuleRow {
    contains: String,
    note: String,
}

async fn load(db: &Db, account_id: i64) -> Result<Vec<RuleJson>, Response> {
    let rows: Vec<RuleRow> = db::query_as(
        "SELECT contains, note FROM statement_exclusion_rules
         WHERE account_id = ? ORDER BY position, id",
    )
    .bind(account_id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    Ok(rows
        .into_iter()
        .map(|r| RuleJson {
            contains: serde_json::from_str(&r.contains).unwrap_or_default(),
            note: r.note,
        })
        .collect())
}

/// The account's rules, ready to match. Stored rules were checked when saved; any that no
/// longer pass are skipped.
pub async fn account_exclusions(db: &Db, account_id: i64) -> Result<Vec<Exclusion>, Response> {
    Ok(load(db, account_id)
        .await?
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r.check() {
            Ok(rule) => Some(rule),
            Err(e) => {
                tracing::warn!(account_id, rule = i + 1, error = %e, "skipping invalid exclusion rule");
                None
            }
        })
        .collect())
}

#[derive(Serialize, Deserialize)]
struct RulesBody {
    rules: Vec<RuleJson>,
}

/// GET /api/orgs/{org}/statement-accounts/{id}/exclusions
async fn get_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id)): UrlPath<(String, i64)>,
) -> Result<Json<RulesBody>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    Ok(Json(RulesBody {
        rules: load(&state.db, account_id).await?,
    }))
}

/// PUT /api/orgs/{org}/statement-accounts/{id}/exclusions — replaces the account's rules.
async fn put_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id)): UrlPath<(String, i64)>,
    Json(body): Json<RulesBody>,
) -> Result<Json<RulesBody>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    save(&state.db, org.id, account_id, body.rules).await?;
    tracing::info!(org = %org.slug, account = account_id, user = user.id, "exclusion rules saved");
    Ok(Json(RulesBody {
        rules: load(&state.db, account_id).await?,
    }))
}

async fn save(
    db: &Db,
    org_id: i64,
    account_id: i64,
    mut rules: Vec<RuleJson>,
) -> Result<(), Response> {
    if rules.len() > MAX_RULES {
        return Err(bad_request(&format!("At most {MAX_RULES} exclusions.")));
    }
    for (i, r) in rules.iter_mut().enumerate() {
        r.note = r.note.trim().to_string();
        r.check()
            .map_err(|e| bad_request(&format!("Exclusion {}: {e}.", i + 1)))?;
    }
    let mut batch = db::Batch::new();
    batch.push(
        "DELETE FROM statement_exclusion_rules WHERE account_id = ?",
        params![account_id],
    );
    for (position, r) in rules.iter().enumerate() {
        batch.push(
            "INSERT INTO statement_exclusion_rules
                 (org_id, account_id, position, contains, note)
             VALUES (?, ?, ?, ?, ?)",
            params![
                org_id,
                account_id,
                position,
                serde_json::to_string(&r.contains).expect("texts serialise"),
                r.note.clone(),
            ],
        );
    }
    db::run(db, batch).await.map_err(db_error)?;
    Ok(())
}

// ---- saved rows a rule matches ----

/// A saved row an exclusion matches.
#[derive(Serialize, Deserialize, Debug)]
struct Match {
    id: i64,
    date: String,
    description: String,
    amount: String,
    currency: String,
    /// Rows in its split (deleted with it), and documents attached (the links go; the
    /// documents stay).
    split_rows: i64,
    attachments: i64,
}

/// The account's saved rows rule `rule` (0-based) matches, oldest first; 404 if there's no such
/// rule. Split rows are left out: they go with their original row.
async fn saved_matches(db: &Db, account_id: i64, rule: usize) -> Result<Vec<Match>, Response> {
    let rules = load(db, account_id).await?;
    let rule = rules
        .get(rule)
        .ok_or_else(|| not_found("That exclusion isn't there any more."))?
        .check()
        .map_err(|e| bad_request(&format!("That exclusion isn't valid: {e}.")))?;
    let rows: Vec<Match> = db::query_as(
        "SELECT t.id, t.date, t.description, t.amount, t.currency,
                (SELECT count(*) FROM statement_transactions s
                 WHERE s.account_id = t.account_id AND s.split_of = t.txn_id) AS split_rows,
                (SELECT count(*) FROM statement_transaction_documents d WHERE d.transaction_id = t.id)
                    AS attachments
         FROM statement_transactions t
         WHERE t.account_id = ? AND t.split_of IS NULL
         ORDER BY t.date, t.id",
    )
    .bind(account_id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    Ok(rows
        .into_iter()
        .filter(|r| rule.matches(&r.description))
        .collect())
}

#[derive(Serialize)]
struct MatchesResponse {
    rows: Vec<Match>,
}

/// GET /api/orgs/{org}/statement-accounts/{id}/exclusions/{rule}/matches — saved rows the
/// exclusion matches.
async fn matches(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id, rule)): UrlPath<(String, i64, usize)>,
) -> Result<Json<MatchesResponse>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    Ok(Json(MatchesResponse {
        rows: saved_matches(&state.db, account_id, rule).await?,
    }))
}

#[derive(Deserialize)]
struct DeleteBody {
    /// Row ids, as listed by `matches`.
    ids: Vec<i64>,
}

#[derive(Serialize)]
struct DeleteResponse {
    /// Rows deleted, split rows included.
    deleted: u64,
}

/// POST /api/orgs/{org}/statement-accounts/{id}/exclusions/{rule}/matches/delete — deletes the
/// rows given that the exclusion still matches (others are left alone), with their splits.
async fn delete_matches(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id, rule)): UrlPath<(String, i64, usize)>,
    Json(body): Json<DeleteBody>,
) -> Result<Json<DeleteResponse>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::DeleteStatementRows).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    if body.ids.len() > MAX_DELETE {
        return Err(bad_request(&format!("At most {MAX_DELETE} rows at once.")));
    }
    let deleted = delete_matching(&state.db, account_id, rule, &body.ids).await?;
    tracing::info!(org = %org.slug, account = account_id, rule, deleted, user = user.id, "excluded rows deleted");
    Ok(Json(DeleteResponse { deleted }))
}

async fn delete_matching(
    db: &Db,
    account_id: i64,
    rule: usize,
    ids: &[i64],
) -> Result<u64, Response> {
    let wanted: HashSet<i64> = ids.iter().copied().collect();
    let still: Vec<i64> = saved_matches(db, account_id, rule)
        .await?
        .into_iter()
        .map(|m| m.id)
        .filter(|id| wanted.contains(id))
        .collect();
    if still.is_empty() {
        return Ok(0);
    }
    let ids_json = serde_json::to_string(&still).expect("ids serialise");
    let mut batch = db::Batch::new();
    // Their splits first (split rows point at the original row's txn_id), then the rows.
    let splits = batch.push(
        "DELETE FROM statement_transactions
         WHERE account_id = ?1 AND split_of IN
             (SELECT txn_id FROM statement_transactions
              WHERE account_id = ?1 AND id IN (SELECT value FROM json_each(?2)))",
        params![account_id, ids_json.clone()],
    );
    let rows = batch.push(
        "DELETE FROM statement_transactions
         WHERE account_id = ? AND id IN (SELECT value FROM json_each(?))",
        params![account_id, ids_json],
    );
    let results = db::run(db, batch).await.map_err(db_error)?;
    Ok(results[splits].rows_affected + results[rows].rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(contains: &[&str]) -> RuleJson {
        RuleJson {
            contains: contains.iter().map(|s| s.to_string()).collect(),
            note: String::new(),
        }
    }

    #[test]
    fn matches_any_text_ignoring_case() {
        let r = rule(&["outstanding card authorisation", "PENDING"])
            .check()
            .unwrap();
        assert!(r.matches(
            "OUTSTANDING CARD AUTHORISATION WOOLWORTHS BELVEDERE W ZAF 30-10-2026 12H15:35"
        ));
        assert!(r.matches("Pending payment"));
        assert!(!r.matches("WOOLWORTHS BELVEDERE"));
    }

    #[test]
    fn bad_rules_are_refused() {
        assert!(rule(&[]).check().is_err());
        assert!(rule(&["  "]).check().is_err());
        let mut long_note = rule(&["A"]);
        long_note.note = "x".repeat(MAX_NOTE_CHARS + 1);
        assert!(long_note.check().is_err());
    }

    #[tokio::test]
    async fn rules_round_trip_and_bad_ones_name_the_rule() {
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
        let mut r = rule(&["Outstanding Card Authorisation "]);
        r.note = "  Not settled yet  ".into();
        save(&db, org, account, vec![r.clone()]).await.unwrap();
        let back = load(&db, account).await.unwrap();
        r.note = "Not settled yet".into();
        assert_eq!(back, [r]);

        let err = save(&db, org, account, vec![rule(&["A"]), rule(&[])])
            .await
            .unwrap_err();
        let body = axum::body::to_bytes(err.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Exclusion 2"));
        // Nothing changed.
        assert_eq!(load(&db, account).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn saved_rows_a_rule_matches_can_be_deleted_with_their_splits() {
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
        // An authorisation (split in two), the settled purchase, and a fee.
        db::query(
            "INSERT INTO statement_transactions
                 (org_id, account_id, txn_id, date, amount, description, currency, split_of, hand_added)
             VALUES (?1, ?2, 1, '2026-10-30', '-50.00', 'OUTSTANDING CARD AUTHORISATION WOOLWORTHS', 'ZAR', NULL, 0),
                    (?1, ?2, 2, '2026-10-30', '-20.00', 'Card authorisation part', 'ZAR', 1, 1),
                    (?1, ?2, 3, '2026-10-30', '20.00', 'Card authorisation part', 'ZAR', 1, 1),
                    (?1, ?2, 4, '2026-11-01', '-50.00', 'WOOLWORTHS BELVEDERE', 'ZAR', NULL, 0),
                    (?1, ?2, 5, '2026-11-02', '-7.50', 'MONTHLY FEE', 'ZAR', NULL, 0)",
        )
        .bind(org)
        .bind(account)
        .execute(&db)
        .await
        .unwrap();
        let id = |txn: i64| {
            let db = db.clone();
            async move {
                db::query_scalar::<i64>("SELECT id FROM statement_transactions WHERE txn_id = ?")
                    .bind(txn)
                    .fetch_one(&db)
                    .await
                    .unwrap()
            }
        };
        let (auth, fee) = (id(1).await, id(5).await);
        save(
            &db,
            org,
            account,
            vec![rule(&["MONTHLY FEE"]), rule(&["card authorisation"])],
        )
        .await
        .unwrap();
        assert!(saved_matches(&db, account, 2).await.is_err());

        // Split rows aren't listed: they go with their original row.
        let found = saved_matches(&db, account, 1).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].id, found[0].split_rows), (auth, 2));

        // Rows the rule doesn't match are left alone, even when asked for.
        assert_eq!(
            delete_matching(&db, account, 1, &[auth, fee])
                .await
                .unwrap(),
            3
        );
        let left: Vec<i64> =
            db::query_scalar("SELECT txn_id FROM statement_transactions ORDER BY txn_id")
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(left, [4, 5]);
        assert_eq!(delete_matching(&db, account, 1, &[auth]).await.unwrap(), 0);
    }
}
