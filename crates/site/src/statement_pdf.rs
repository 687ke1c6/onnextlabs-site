//! Statements as PDF: printing the rows the grid shows, with one of the org's statement
//! templates (`statement_templates.rs`).
//!
//! Printing takes the rows as the page shows them (its filters and order), so what's printed is
//! exactly what was on screen; `selection` says in words what that was (e.g. `Tax year: 2024/25`).
//! The **period** is stated, or else the rows' first and last dates.
//!
//! Which date counts (the **date basis**) is the user's choice when printing: the transaction
//! date, or the effective date where a row has one. It decides what's in the period, the balances
//! and the running balance's order.
//!
//! **Balances** come from the account's starting balance and *all* its rows, never from the
//! printed rows: filters change what prints, not the balances. A running balance (the template's
//! `balance` column) prints only when the printed rows are exactly the account's rows in the
//! period (they then print in date order). `…/pdf/summary` tells the print dialog all this
//! before printing.
//!
//! The PDF comes back to show (the page opens it in a new tab), and can also be saved to Documents
//! under `Statements/<account>/Printed`.
//! Printing needs `read-statements`, and saving the copy `write-statements`.

use crate::db::{self, Db};
use std::{collections::HashSet, sync::Arc};

use axum::{
    extract::{Path as UrlPath, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use statements_core::id::amount_cents;
use statements_pdf::{
    AccountInfo, Asset, ColumnKey, DateBasis, Header, LedgerBalances, Letterhead, Line, Options,
};

use crate::{
    api::{bad_request, db_error, error},
    auth::CurrentUser,
    documents,
    rbac::{self, Permission},
    statement_accounts::{self, is_iso_date, Account},
    statement_templates::{self, logo_extension, MAX_LOGO_BYTES},
    statements, AppState,
};

const MAX_ROWS: usize = 20_000;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/orgs/{org}/statement-accounts/{id}/pdf", post(print))
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/pdf/summary",
            post(summary),
        )
}

#[derive(Deserialize, Clone)]
struct PeriodBody {
    /// `YYYY-MM-DD`
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct PrintBody {
    /// Row ids (not `txn_id`s) in the order to print: what the grid shows.
    rows: Vec<i64>,
    /// What was chosen, in words, e.g. `Tax year: 2024/25`.
    #[serde(default)]
    selection: Vec<String>,
    /// Also save the PDF to Documents.
    #[serde(default)]
    save: bool,
    /// The org's template to print with; otherwise the account's last one.
    #[serde(default)]
    template_id: Option<i64>,
    /// Otherwise the rows' first and last dates.
    #[serde(default)]
    period: Option<PeriodBody>,
    #[serde(default)]
    date_basis: DateBasis,
}

#[derive(Deserialize)]
struct SummaryBody {
    rows: Vec<i64>,
    #[serde(default)]
    period: Option<PeriodBody>,
    #[serde(default)]
    date_basis: DateBasis,
}

#[derive(Deserialize)]
struct PrintRow {
    id: i64,
    date: String,
    effective_date: Option<String>,
    description: String,
    #[serde(rename = "type")]
    category: Option<String>,
    amount: String,
    currency: String,
    hand_added: bool,
}

/// What printing these rows for this period would show, worked out before printing.
#[derive(Serialize, Debug, PartialEq)]
pub struct Summary {
    /// The period, `YYYY-MM-DD` (`None` with no rows and none stated).
    pub from: Option<String>,
    pub to: Option<String>,
    /// The balance at the start and end of the period, in cents (`None` without a starting
    /// balance on the account).
    pub opening_cents: Option<i64>,
    pub closing_cents: Option<i64>,
    /// How many of the account's rows are dated in the period.
    pub period_rows: usize,
    /// The printed rows are exactly those, so a running balance is true.
    pub rows_complete: bool,
}

/// A row of the account's ledger: id, its date (on the date basis), amount in cents.
type Entry = (i64, String, i64);

/// Every row of the account, for balances, dated on `basis`.
async fn ledger(db: &Db, account_id: i64, basis: DateBasis) -> Result<Vec<Entry>, Response> {
    #[derive(Deserialize)]
    struct Row {
        id: i64,
        date: String,
        effective_date: Option<String>,
        amount: String,
    }
    let rows: Vec<Row> = db::query_as(
        "SELECT id, date, effective_date, amount FROM statement_transactions WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    rows.into_iter()
        .map(|r| {
            let cents = amount_cents(&r.amount).map_err(|_| {
                bad_request(&format!(
                    "The row {} has an amount that isn't a number, so the balance can't be worked out.",
                    r.date
                ))
            })?;
            let date = basis.pick(&r.date, r.effective_date.as_deref()).to_string();
            Ok((r.id, date, cents))
        })
        .collect()
}

/// The balances and completeness of printing `printed` for the period `from`–`to` (inclusive),
/// given the account's `ledger` and its starting balance `(cents, date)`: the balance at the start
/// of that date.
fn work_out(
    ledger: &[Entry],
    start: Option<(i64, &str)>,
    from: &str,
    to: &str,
    printed: &[i64],
) -> (Option<LedgerBalances>, usize, bool) {
    let sum = |keep: &dyn Fn(&str) -> bool| -> i64 {
        ledger
            .iter()
            .filter(|(_, d, _)| keep(d))
            .map(|(_, _, c)| c)
            .sum()
    };
    let in_period: HashSet<i64> = ledger
        .iter()
        .filter(|(_, d, _)| d.as_str() >= from && d.as_str() <= to)
        .map(|(id, _, _)| *id)
        .collect();
    let printed_set: HashSet<i64> = printed.iter().copied().collect();
    let complete = printed_set.len() == printed.len() && printed_set == in_period;
    let balances = start.map(|(cents, date)| {
        // Forwards from the starting date to the period, or backwards from it.
        let opening = cents + sum(&|d| d >= date && d < from) - sum(&|d| d >= from && d < date);
        let closing = opening + sum(&|d| d >= from && d <= to);
        LedgerBalances {
            opening,
            closing,
            rows_complete: complete,
        }
    });
    (balances, in_period.len(), complete)
}

/// The stated period, checked, or the rows' first and last dates (on `basis`).
fn period(
    stated: Option<&PeriodBody>,
    lines: &[Line],
    basis: DateBasis,
) -> Result<Option<(String, String)>, Response> {
    if let Some(p) = stated {
        let (from, to) = (p.from.trim(), p.to.trim());
        if !is_iso_date(from) || !is_iso_date(to) {
            return Err(bad_request("The period needs a start and an end date."));
        }
        if from > to {
            return Err(bad_request("The period ends before it starts."));
        }
        return Ok(Some((from.to_owned(), to.to_owned())));
    }
    let dates = || {
        lines
            .iter()
            .map(|l| basis.pick(&l.date, l.effective_date.as_deref()).to_string())
    };
    Ok(dates().min().zip(dates().max()))
}

async fn summarise(
    db: &Db,
    account: &Account,
    ids: &[i64],
    lines: &[Line],
    stated: Option<&PeriodBody>,
    basis: DateBasis,
) -> Result<Summary, Response> {
    let Some((from, to)) = period(stated, lines, basis)? else {
        return Ok(Summary {
            from: None,
            to: None,
            opening_cents: None,
            closing_cents: None,
            period_rows: 0,
            rows_complete: false,
        });
    };
    let ledger = ledger(db, account.id, basis).await?;
    let start = account
        .opening_balance_cents
        .zip(account.opening_balance_date.as_deref());
    let (balances, period_rows, rows_complete) = work_out(&ledger, start, &from, &to, ids);
    Ok(Summary {
        from: Some(from),
        to: Some(to),
        opening_cents: balances.map(|b| b.opening),
        closing_cents: balances.map(|b| b.closing),
        period_rows,
        rows_complete,
    })
}

fn check_rows(rows: &[i64]) -> Result<(), Response> {
    if rows.len() > MAX_ROWS {
        return Err(bad_request(&format!(
            "At most {MAX_ROWS} rows can be printed at once."
        )));
    }
    Ok(())
}

/// POST /api/orgs/{org}/statement-accounts/{id}/pdf/summary — the period, balances and whether
/// a running balance can print, for these rows.
async fn summary(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<SummaryBody>,
) -> Result<Json<Summary>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    let account = statement_accounts::find(&state.db, org.id, id).await?;
    check_rows(&body.rows)?;
    let (lines, _) = rows_in_order(&state.db, account.id, &body.rows).await?;
    Ok(Json(
        summarise(
            &state.db,
            &account,
            &body.rows,
            &lines,
            body.period.as_ref(),
            body.date_basis,
        )
        .await?,
    ))
}

/// POST /api/orgs/{org}/statement-accounts/{id}/pdf — the rows as a PDF. With
/// `save`, a copy also goes to Documents; its id is in `X-Document-Id`.
async fn print(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<PrintBody>,
) -> Result<Response, Response> {
    let (org, perms) = rbac::org_access(&state.db, &user, &slug).await?;
    perms.require(Permission::ReadStatements)?;
    if body.save {
        perms.require(Permission::WriteStatements)?;
    }
    let account = statement_accounts::find(&state.db, org.id, id).await?;
    check_rows(&body.rows)?;
    let selection: Vec<String> = body
        .selection
        .iter()
        .take(20)
        .map(|s| {
            s.trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect();

    let template = statement_templates::for_printing(
        &state.db,
        org.id,
        body.template_id,
        account.last_template_id,
    )
    .await?;
    let t = &template.settings;
    let (mut lines, currency) = rows_in_order(&state.db, account.id, &body.rows).await?;
    let currency = currency.unwrap_or_else(|| "ZAR".into());
    let summary = summarise(
        &state.db,
        &account,
        &body.rows,
        &lines,
        body.period.as_ref(),
        body.date_basis,
    )
    .await?;
    let balances = summary
        .opening_cents
        .zip(summary.closing_cents)
        .map(|(opening, closing)| LedgerBalances {
            opening,
            closing,
            rows_complete: summary.rows_complete,
        });
    // A running balance runs in date order on the basis (then the order rows were stored).
    let running = t.columns.iter().any(|c| c.key == ColumnKey::Balance)
        && !t.group_by_type
        && balances.is_some_and(|b| b.rows_complete);
    if running {
        let basis = body.date_basis;
        let mut order: Vec<(i64, Line)> = body.rows.iter().copied().zip(lines).collect();
        order.sort_by(|(a_id, a), (b_id, b)| {
            let day = |l: &Line| basis.pick(&l.date, l.effective_date.as_deref()).to_string();
            day(a).cmp(&day(b)).then(a_id.cmp(b_id))
        });
        lines = order.into_iter().map(|(_, l)| l).collect();
    }

    // The logo, if it's still there; a missing one just leaves it off.
    let mut assets = Vec::new();
    let mut logo = None;
    if let Some(doc) = t.logo_document_id {
        if let Some((folder, filename)) = documents::find_in_org(&state.db, org.id, doc).await? {
            if let Some(ext) = logo_extension(&filename) {
                match documents::read_bytes(&state, &org, &folder, &filename, MAX_LOGO_BYTES).await
                {
                    Ok(bytes) => {
                        let name = format!("logo.{ext}");
                        assets.push(Asset {
                            name: name.clone(),
                            bytes: bytes.to_vec(),
                        });
                        logo = Some(name);
                    }
                    Err(_) => {
                        tracing::warn!(org = %org.slug, document = doc, "statement logo couldn't be read; printing without it")
                    }
                }
            }
        }
    }

    // Likewise the letterhead.
    let mut letterhead = None;
    if let Some(doc) = t.letterhead_document_id {
        match statement_templates::letterhead(&state, &org, doc).await {
            Ok((bytes, pages)) => {
                let name = "letterhead.pdf".to_string();
                assets.push(Asset {
                    name: name.clone(),
                    bytes,
                });
                letterhead = Some(Letterhead::new(name, pages, t.letterhead_every_page));
            }
            Err(_) => {
                tracing::warn!(org = %org.slug, document = doc, "statement letterhead couldn't be used; printing without it")
            }
        }
    }

    let generated: String = db::query_scalar("SELECT date('now')")
        .fetch_one(&state.db)
        .await
        .map_err(db_error)?;
    let period = summary.from.clone().zip(summary.to.clone());
    let data = statements_pdf::build(
        Header {
            generated: generated.clone(),
            title: t.title.clone(),
            subtitle: t.subtitle.clone(),
            footer: t.footer.clone(),
            fields: t.header_fields.clone(),
            organisation: org.name.clone(),
            account: AccountInfo {
                name: account.name.clone(),
                number: account.identifier.clone(),
                holder: account.holder_name.clone(),
                product: account.product_name.clone(),
            },
            currency,
            period: period.clone(),
            date_basis: body.date_basis,
            selection,
            options: Options {
                columns: t.columns.clone(),
                group_by_type: t.group_by_type,
                summary: t.show_summary,
                running_balance: running,
                paper: t.paper.clone(),
                margin_top_mm: t.margin_top_mm,
                margin_bottom_mm: t.margin_bottom_mm,
                date_style: t.date_style,
                currency_symbol: t.currency_symbol.clone(),
                totals: t.totals.clone(),
            },
            logo,
            letterhead,
            balances,
        },
        &lines,
    );
    let filename = pdf_filename(&account.name, period.as_ref(), &generated);
    let layout = statements_pdf::layout_source(&t.layout);

    // Typst is CPU-bound: keep it off the async workers.
    let pdf = tokio::task::spawn_blocking(move || statements_pdf::render(layout, &data, assets))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "statement PDF task failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
        })?
        .map_err(|e| {
            tracing::error!(error = %e, "statement PDF didn't render");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("The PDF couldn't be made: {e}"),
            )
        })?;

    // The account remembers the template for next time.
    db::query("UPDATE statement_accounts SET last_template_id = ? WHERE id = ?")
        .bind(template.id)
        .bind(account.id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;

    let mut response = (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/pdf"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        pdf.clone(),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&documents::content_disposition(&filename)) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, v);
    }
    if body.save {
        let folder = format!("{}/Printed", statements::statement_folder(&account.name));
        let (doc, _, _) = documents::save_bytes(
            &state,
            &org,
            user.id,
            &folder,
            &filename,
            Some("application/pdf".into()),
            pdf.into(),
            false,
        )
        .await?;
        response
            .headers_mut()
            .insert("x-document-id", HeaderValue::from(doc));
    }
    tracing::info!(org = %org.slug, account = account.id, template = template.id, rows = lines.len(), saved = body.save, user = user.id, "statement PDF printed");
    Ok(response)
}

/// The rows, in the order asked for, and the currency of the first. Every id must be a row of
/// the account.
async fn rows_in_order(
    db: &Db,
    account_id: i64,
    ids: &[i64],
) -> Result<(Vec<Line>, Option<String>), Response> {
    let rows: Vec<PrintRow> = db::query_as(
        "SELECT id, date, effective_date, description, type, amount, currency, hand_added
         FROM statement_transactions
         WHERE account_id = ? AND id IN (SELECT value FROM json_each(?))",
    )
    .bind(account_id)
    .bind(serde_json::to_string(ids).expect("ids serialise"))
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    let by_id: std::collections::HashMap<i64, &PrintRow> = rows.iter().map(|r| (r.id, r)).collect();
    let mut lines = Vec::with_capacity(ids.len());
    for id in ids {
        let r = by_id.get(id).ok_or_else(|| {
            bad_request("Some of those rows aren't in this bank account any more. Reload and try again.")
        })?;
        let cents = amount_cents(&r.amount).map_err(|_| {
            bad_request(&format!(
                "The row {} {:?} has an amount that isn't a number.",
                r.date, r.description
            ))
        })?;
        lines.push(Line {
            date: r.date.clone(),
            effective_date: r.effective_date.clone(),
            description: r.description.clone(),
            category: r.category.clone(),
            cents,
            hand_added: r.hand_added,
        });
    }
    let currency = ids
        .first()
        .and_then(|id| by_id.get(id))
        .map(|r| r.currency.clone());
    Ok((lines, currency))
}

/// `House Account statement 2024-03-01 to 2025-02-28.pdf` (the period), or with the day it was
/// made when there's no period.
fn pdf_filename(account: &str, period: Option<&(String, String)>, generated: &str) -> String {
    let name: String = account
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    match period {
        Some((from, to)) if from == to => format!("{} statement {from}.pdf", name.trim()),
        Some((from, to)) => format!("{} statement {from} to {to}.pdf", name.trim()),
        None => format!("{} statement {generated}.pdf", name.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (Db, i64, i64) {
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
        (db, org, account)
    }

    async fn row(db: &Db, org: i64, account: i64, txn: i64, date: &str, amount: &str) -> i64 {
        db::query_scalar(
            "INSERT INTO statement_transactions (org_id, account_id, txn_id, date, amount, description, currency)
             VALUES (?, ?, ?, ?, ?, 'X', 'ZAR') RETURNING id",
        )
        .bind(org)
        .bind(account)
        .bind(txn)
        .bind(date)
        .bind(amount)
        .fetch_one(db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn rows_print_in_the_order_given_and_only_from_the_account() {
        let (db, org, account) = setup().await;
        let a = row(&db, org, account, 1, "2025-01-01", "-1,234.50").await;
        let b = row(&db, org, account, 2, "2025-01-02", "10").await;
        let (lines, currency) = rows_in_order(&db, account, &[b, a]).await.unwrap();
        assert_eq!(
            lines.iter().map(|l| l.cents).collect::<Vec<_>>(),
            [1000, -123_450]
        );
        assert_eq!(currency.as_deref(), Some("ZAR"));

        let other: i64 = db::query_scalar(
            "INSERT INTO statement_accounts (org_id, name) VALUES (?, 'Other') RETURNING id",
        )
        .bind(org)
        .fetch_one(&db)
        .await
        .unwrap();
        let theirs = row(&db, org, other, 3, "2025-01-03", "1").await;
        assert!(rows_in_order(&db, account, &[a, theirs]).await.is_err());
    }

    /// A ledger: a starting balance of 100.00 at the start of 2026-10-04; rows either side.
    fn sample() -> Vec<Entry> {
        vec![
            (1, "2026-09-30".into(), -2_000),
            (2, "2026-10-01".into(), 5_000),
            (3, "2026-10-04".into(), -1_000),
            (4, "2026-10-10".into(), 20_000),
            (5, "2026-11-03".into(), -500),
            (6, "2026-11-04".into(), 7_000),
        ]
    }

    #[test]
    fn balances_come_from_the_whole_ledger() {
        let l = sample();
        let start = Some((10_000, "2026-10-04"));
        // From the starting date: 100.00, then -10 +200 -5.
        let (b, n, complete) = work_out(&l, start, "2026-10-04", "2026-11-03", &[3, 4, 5]);
        assert_eq!(
            (b.unwrap().opening, b.unwrap().closing, n, complete),
            (10_000, 28_500, 3, true)
        );
        // A filter (only row 4 printed) changes nothing but completeness.
        let (f, _, complete) = work_out(&l, start, "2026-10-04", "2026-11-03", &[4]);
        assert_eq!(
            (f.unwrap().opening, f.unwrap().closing, complete),
            (10_000, 28_500, false)
        );
        // A period before the starting date works backwards: 100 - 50 = 50 at the start of 1 Oct.
        let (b, _, _) = work_out(&l, start, "2026-10-01", "2026-10-03", &[2]);
        assert_eq!((b.unwrap().opening, b.unwrap().closing), (5_000, 10_000));
        // And after it, forwards: 100 - 10 + 200 - 5 = 285 at the start of 4 Nov.
        let (b, _, _) = work_out(&l, start, "2026-11-04", "2026-11-30", &[6]);
        assert_eq!((b.unwrap().opening, b.unwrap().closing), (28_500, 35_500));
        // Without a starting balance there are none, but completeness still counts.
        let (b, _, complete) = work_out(&l, None, "2026-11-04", "2026-11-30", &[6]);
        assert!(b.is_none() && complete);
        // The same row twice isn't the whole period.
        assert!(!work_out(&l, start, "2026-11-04", "2026-11-30", &[6, 6]).2);
    }

    #[tokio::test]
    async fn the_ledger_and_summary_use_every_row_of_the_account() {
        let (db, org, account) = setup().await;
        let a = row(&db, org, account, 1, "2026-10-01", "50").await;
        let b = row(&db, org, account, 2, "2026-10-05", "-1,234.50").await;
        db::query("UPDATE statement_accounts SET opening_balance_cents = 10000, opening_balance_date = '2026-10-01' WHERE id = ?")
            .bind(account)
            .execute(&db)
            .await
            .unwrap();
        let acct = statement_accounts::find(&db, org, account).await.unwrap();
        let (lines, _) = rows_in_order(&db, account, &[b]).await.unwrap();
        let s = summarise(&db, &acct, &[b], &lines, None, DateBasis::Transaction)
            .await
            .unwrap();
        // The period is the printed row's date; the opening balance includes row a.
        assert_eq!(
            s,
            Summary {
                from: Some("2026-10-05".into()),
                to: Some("2026-10-05".into()),
                opening_cents: Some(15_000),
                closing_cents: Some(15_000 - 123_450),
                period_rows: 1,
                rows_complete: true,
            }
        );
        let stated = PeriodBody {
            from: "2026-10-01".into(),
            to: "2026-10-31".into(),
        };
        let s = summarise(
            &db,
            &acct,
            &[b],
            &lines,
            Some(&stated),
            DateBasis::Transaction,
        )
        .await
        .unwrap();
        assert_eq!(
            (s.opening_cents, s.period_rows, s.rows_complete),
            (Some(10_000), 2, false)
        );
        let (all, _) = rows_in_order(&db, account, &[a, b]).await.unwrap();
        assert!(
            summarise(
                &db,
                &acct,
                &[a, b],
                &all,
                Some(&stated),
                DateBasis::Transaction
            )
            .await
            .unwrap()
            .rows_complete
        );
        let backwards = PeriodBody {
            from: "2026-10-31".into(),
            to: "2026-10-01".into(),
        };
        assert!(summarise(
            &db,
            &acct,
            &[b],
            &lines,
            Some(&backwards),
            DateBasis::Transaction
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn the_date_basis_decides_the_period_and_balances() {
        let (db, org, account) = setup().await;
        let oct = row(&db, org, account, 1, "2026-10-10", "100").await;
        // Paid on 1 Nov, but for October.
        let rent = row(&db, org, account, 2, "2026-11-01", "-30").await;
        db::query("UPDATE statement_transactions SET effective_date = '2026-10-31' WHERE id = ?")
            .bind(rent)
            .execute(&db)
            .await
            .unwrap();
        db::query("UPDATE statement_accounts SET opening_balance_cents = 0, opening_balance_date = '2026-10-01' WHERE id = ?")
            .bind(account)
            .execute(&db)
            .await
            .unwrap();
        let acct = statement_accounts::find(&db, org, account).await.unwrap();
        let october = PeriodBody {
            from: "2026-10-01".into(),
            to: "2026-10-31".into(),
        };
        let (both, _) = rows_in_order(&db, account, &[oct, rent]).await.unwrap();
        let (just_oct, _) = rows_in_order(&db, account, &[oct]).await.unwrap();

        // By transaction date, October is the one row, ending on 100.00.
        let s = summarise(
            &db,
            &acct,
            &[oct],
            &just_oct,
            Some(&october),
            DateBasis::Transaction,
        )
        .await
        .unwrap();
        assert_eq!(
            (s.closing_cents, s.period_rows, s.rows_complete),
            (Some(10_000), 1, true)
        );
        // By effective date the rent is October's too.
        let s = summarise(
            &db,
            &acct,
            &[oct, rent],
            &both,
            Some(&october),
            DateBasis::Effective,
        )
        .await
        .unwrap();
        assert_eq!(
            (s.closing_cents, s.period_rows, s.rows_complete),
            (Some(7_000), 2, true)
        );
        // And with no period stated, the rows' dates on the basis.
        let s = summarise(&db, &acct, &[rent], &both[1..], None, DateBasis::Effective)
            .await
            .unwrap();
        assert_eq!(s.to.as_deref(), Some("2026-10-31"));
    }

    #[test]
    fn filenames_name_the_period() {
        let p = ("2024-02-29".to_string(), "2025-02-28".to_string());
        assert_eq!(
            pdf_filename("House / Main", Some(&p), "2026-11-22"),
            "House - Main statement 2024-02-29 to 2025-02-28.pdf"
        );
        let day = ("2024-02-29".to_string(), "2024-02-29".to_string());
        assert_eq!(
            pdf_filename("H", Some(&day), "2026-11-22"),
            "H statement 2024-02-29.pdf"
        );
        assert_eq!(
            pdf_filename("H", None, "2026-11-22"),
            "H statement 2026-11-22.pdf"
        );
    }
}
