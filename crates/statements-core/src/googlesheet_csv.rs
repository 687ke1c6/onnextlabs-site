//! A Google Sheet of statement rows, downloaded as CSV: the tab n8n (and this app's "Send to Google Sheets")
//! appends to, `id, month, date, effective_date, amount, description, currency, type` (`row.rs`). Not a bank's
//! format, so the file doesn't state an account number.
//!
//! Rows get this app's ids like any statement's (`id.rs`); the sheet's own ids are only checked, in an
//! [`IdCheck`]. Old sheets hold n8n ids, which were made unique by appending `_` to the description; those
//! suffixes are removed (the occurrence count replaces them), so the rows match the bank's own statements.
//!
//! Values are kept as the sheet shows them, including amounts like `-1,798.00` and hand edits; `type` becomes the
//! row's category and `effective_date` is kept. Columns are found by header name, so their order doesn't matter;
//! other columns (`month`, `rate`, notes) are ignored.

use std::collections::HashSet;

use anyhow::{Context, Result, bail, ensure};

use crate::hash::{DateFormat, row_id as n8n_id};
use crate::row::Column;
use crate::statement::{
    IdCheck, IdMismatch, Parsed, StatedId, Transaction, check_effective_date, check_iso_date,
};

/// Where each column this format reads is in the file.
struct Columns {
    id: usize,
    date: usize,
    amount: usize,
    description: usize,
    effective_date: Option<usize>,
    currency: Option<usize>,
    category: Option<usize>,
}

impl Columns {
    fn find(header: &csv::StringRecord) -> Option<Self> {
        let index = |column: Column| {
            header
                .iter()
                .position(|h| Column::from_header(h) == Some(column))
        };
        Some(Self {
            id: index(Column::Id)?,
            date: index(Column::Date)?,
            amount: index(Column::Amount)?,
            description: index(Column::Description)?,
            effective_date: index(Column::EffectiveDate),
            currency: index(Column::Currency),
            category: index(Column::Type),
        })
    }
}

/// The header row (row 1) has `id`, `date`, `amount` and `description` columns.
pub fn matches(rows: &[csv::StringRecord]) -> bool {
    rows.first().and_then(Columns::find).is_some()
}

pub fn parse(rows: &[csv::StringRecord]) -> Result<Parsed> {
    let Some(columns) = rows.first().and_then(Columns::find) else {
        bail!("the header row needs id, date, amount and description columns");
    };

    let mut stated = Vec::new();
    let mut transactions = Vec::new();
    for (index, row) in rows.iter().enumerate().skip(1) {
        let line = index + 1;
        // Sheets exports often end with empty rows.
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let (id, t) = read_row(row, &columns).with_context(|| format!("line {line}"))?;
        stated.push(StatedId {
            line,
            id,
            description: t.description.clone(),
        });
        transactions.push(t);
    }
    strip_n8n_suffixes(&mut transactions);

    Ok(Parsed {
        account_number: None,
        transactions,
        stated_ids: Some(stated),
    })
}

/// n8n made same-day duplicates unique by appending `_` (`X`, `X_`, `X__`). A description ending in `_` loses
/// the run of `_` when the sheet has a row on the same day with the bare description. (Amounts aren't compared:
/// a duplicate may have been edited since, e.g. zeroed.)
fn strip_n8n_suffixes(transactions: &mut [Transaction]) {
    let bare: HashSet<(String, String)> = transactions
        .iter()
        .filter(|t| !t.description.ends_with('_'))
        .map(|t| (t.date.clone(), t.description.clone()))
        .collect();
    for t in transactions {
        let base = t.description.trim_end_matches('_');
        if base.len() != t.description.len() && bare.contains(&(t.date.clone(), base.to_string())) {
            t.description.truncate(base.len());
        }
    }
}

fn read_row(row: &csv::StringRecord, c: &Columns) -> Result<(Option<i64>, Transaction)> {
    let cell = |i: usize| row.get(i).unwrap_or("");
    let optional = |i: Option<usize>| i.map(cell).map(str::trim).filter(|v| !v.is_empty());

    let id = match cell(c.id).trim() {
        "" => None,
        raw => Some(
            raw.parse::<i64>()
                .with_context(|| format!("id {raw:?} isn't a whole number"))?,
        ),
    };
    let date = cell(c.date).trim();
    let date = check_iso_date(date).with_context(|| format!("bad date {date:?}"))?;
    let effective_date = optional(c.effective_date)
        .map(|d| {
            check_effective_date(&date, d).with_context(|| format!("bad effective_date {d:?}"))
        })
        .transpose()?;
    let amount = cell(c.amount);
    ensure!(!amount.trim().is_empty(), "missing amount");

    Ok((
        id,
        Transaction {
            id: 0,
            occurrence: 0,
            date,
            description: cell(c.description).to_string(),
            amount: amount.to_string(),
            // Every format so far is ZAR; a hand-added row may leave it blank.
            currency: optional(c.currency).unwrap_or("ZAR").to_string(),
            category: optional(c.category).map(str::to_string),
            effective_date,
            // No id: added to the sheet by hand (e.g. half of a split), not from a bank statement.
            hand_added: id.is_none(),
        },
    ))
}

/// How the sheet's ids compare with the rows (`stated` and `transactions` are in the same order, ids assigned).
pub(crate) fn check_ids(stated: &[StatedId], transactions: &[Transaction]) -> IdCheck {
    let mut check = IdCheck::default();
    for (s, t) in stated.iter().zip(transactions) {
        let Some(id) = s.id else {
            check.no_id.push(s.line);
            continue;
        };
        if id == t.id {
            check.current += 1;
            continue;
        }
        match i32::try_from(id)
            .ok()
            .and_then(|id| n8n_hashes_to(id, t, &s.description))
        {
            Some(true) => check.n8n_matched += 1,
            Some(false) => check.n8n_reformatted += 1,
            None => check.mismatched.push(IdMismatch { line: s.line, id }),
        }
    }
    check
}

/// Whether `id` is the n8n id of the row (with its description as the sheet wrote it, `_` suffix included):
/// `Some(true)` with the amount as written, `Some(false)` with the amount written another way, `None` if
/// neither.
///
/// Sheets stores amounts as numbers, so what it shows (and exports) needn't be the string n8n hashed: `-1798`
/// comes back as `-1,798.00`, `-85.00` as `-85`. The renderings tried: as written, without thousands separators,
/// JS `String(number)` (n8n's own number to string), and two decimal places.
fn n8n_hashes_to(id: i32, t: &Transaction, description: &str) -> Option<bool> {
    let hashes =
        |amount: &str| n8n_id(&t.date, description, amount, DateFormat::UtcTimestamp) == id;
    if hashes(&t.amount) {
        return Some(true);
    }
    let plain = t.amount.trim().replace(',', "");
    let mut others = vec![plain.clone()];
    if let Ok(n) = plain.parse::<f64>() {
        // Rust's shortest round-trip form is JS's for amounts (no exponents at these magnitudes).
        others.push(n.to_string());
        others.push(format!("{n:.2}"));
    }
    others.iter().any(|a| hashes(a)).then_some(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id;
    use crate::statement::{self, Format};

    fn parse_str(csv: &str) -> statement::Statement {
        statement::parse(csv.as_bytes()).unwrap()
    }

    /// Made-up sheet: rows with n8n ids (some with amounts Sheets has since reformatted), an n8n duplicate
    /// (`_`) zeroed by hand afterwards, hand-added rows without ids (a same-day pair), an effective date, types,
    /// a blank currency, extra columns and a trailing empty row. The first five ids are `hash.rs`'s n8n JS
    /// vectors.
    #[test]
    fn synthetic_sheet() {
        let s = statement::parse(
            &include_bytes!("../tests/fixtures/googlesheet_csv_synthetic.csv")[..],
        )
        .unwrap();
        assert_eq!(s.format, Format::GoogleSheetCsv);
        assert_eq!(s.account_number, None);

        let rows: Vec<_> = s
            .transactions
            .iter()
            .map(|t| {
                (
                    t.date.as_str(),
                    t.effective_date.as_deref(),
                    t.amount.as_str(),
                    t.description.as_str(),
                    t.occurrence,
                    t.currency.as_str(),
                    t.category.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "2027-12-29",
                    None,
                    "-250.75",
                    "IB PAYMENT TO EXAMPLE MUNICIPALITY ACC 001",
                    0,
                    "ZAR",
                    None
                ),
                (
                    "2027-12-31",
                    None,
                    "-120.00",
                    "FIXED MONTHLY FEE ",
                    0,
                    "ZAR",
                    Some("FEES")
                ),
                // No bare "OVERDRAFT SERVICE FEE " that day, so the `_` stays.
                (
                    "2027-12-31",
                    None,
                    "-57.50",
                    "OVERDRAFT SERVICE FEE _",
                    0,
                    "ZAR",
                    Some("FEES")
                ),
                (
                    "2028-02-01",
                    Some("2028-01-31"),
                    "-899.99",
                    "REGISTERED DC DEBIT HOME LOAN DC   0000001       280201",
                    0,
                    "ZAR",
                    Some("BOND REPAYMENT")
                ),
                ("2026-12-31", None, "1,234.00", "Café ☕ 😀", 0, "ZAR", None),
                (
                    "2028-01-05",
                    None,
                    "-1,798.00",
                    "DEBIT CARD PURCHASE FROM DECOR 1234*5678 03 JAN",
                    0,
                    "ZAR",
                    None
                ),
                // The zeroed duplicate: suffix removed; a different amount now, so its own key.
                (
                    "2027-12-31",
                    None,
                    "0.00",
                    "FIXED MONTHLY FEE ",
                    0,
                    "ZAR",
                    None
                ),
                (
                    "2028-03-01",
                    Some("2028-02-29"),
                    "-100",
                    "Split rent",
                    0,
                    "ZAR",
                    Some("RENT")
                ),
                (
                    "2028-03-01",
                    None,
                    "-100",
                    "Split rent",
                    1,
                    "ZAR",
                    Some("RENT")
                ),
            ]
        );
        let hand: Vec<bool> = s.transactions.iter().map(|t| t.hand_added).collect();
        assert_eq!(
            hand,
            [false, false, false, false, false, false, false, true, true]
        );
        assert_eq!(
            s.transactions[0].id,
            id::row_id(
                "2027-12-29",
                -25_075,
                "IB PAYMENT TO EXAMPLE MUNICIPALITY ACC 001",
                0
            )
        );

        // As written: lines 2, 5 and 6. Reformatted by Sheets: 3 (-120), 4 (-57.5) and 7 (-1798).
        assert_eq!(
            s.id_check.unwrap(),
            IdCheck {
                current: 0,
                n8n_matched: 3,
                n8n_reformatted: 3,
                // Hashed at -120, zeroed by hand afterwards.
                mismatched: vec![IdMismatch {
                    line: 8,
                    id: 793040786
                }],
                no_id: vec![9, 10],
            }
        );
    }

    #[test]
    fn a_sheet_this_app_wrote_has_current_ids() {
        let id = id::row_id("2028-01-01", -100, "FEE", 0);
        let s = parse_str(&format!(
            "id,date,amount,description\n{id},2028-01-01,-1.00,FEE \n"
        ));
        assert_eq!(s.transactions[0].id, id);
        assert_eq!(s.id_check.unwrap().current, 1);
    }

    #[test]
    fn detection() {
        let rows = |csv: &str| {
            csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .from_reader(csv.as_bytes())
                .records()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(matches(&rows(
            "id,month,date,effective_date,amount,description,currency,type,rate,\n"
        )));
        assert!(matches(&rows("description, id ,amount,date\n")));
        assert!(!matches(&rows("month,date,amount,description\n")));
        // Standard Bank PDF-as-CSV has no id column.
        assert!(!matches(&rows("description,amount,date,balance\n")));
        assert!(!matches(&[]));
    }

    #[test]
    fn rejects_malformed_rows() {
        let err = |csv: &str| format!("{:#}", statement::parse(csv.as_bytes()).unwrap_err());
        let head = "id,date,effective_date,amount,description\n";
        assert!(err(&format!("{head}1,2028-02-30,,-1,X\n")).contains("line 2: bad date"));
        assert!(err(&format!("{head}1,01/02/2028,,-1,X\n")).contains("line 2: bad date"));
        assert!(
            err(&format!("{head}1,2028-02-01,2028-02-30,-1,X\n"))
                .contains("line 2: bad effective_date")
        );
        assert!(err(&format!("{head}1,2028-02-01,, ,X\n")).contains("line 2: missing amount"));
        assert!(err(&format!("{head}abc,2028-02-01,,-1,X\n")).contains("line 2: id \"abc\""));
    }
}
