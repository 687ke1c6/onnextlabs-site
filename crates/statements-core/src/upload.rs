//! Appending a statement to the chosen tab: read the header row and the existing ids, skip rows already there,
//! append the rest (with the header row first when the tab is empty). Existing rows are never changed.

use std::collections::HashSet;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::google::{Google, Tab};
use crate::row::{Cell, Layout};
use crate::statement::{Statement, Transaction};

/// Where the rows go.
#[derive(Debug, Clone)]
pub struct Target {
    pub spreadsheet_id: String,
    pub spreadsheet_title: String,
    pub tab: Tab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub parsed: usize,
    /// Rows skipped because their id is already in the tab.
    pub existing: usize,
    pub appended: usize,
    /// Appended rows that got a category (`type`).
    pub categorised: usize,
    pub wrote_header: bool,
    /// Non-blank cells in the id column that aren't ids (e.g. notes or totals); ignored.
    pub unreadable_ids: usize,
}

pub fn upload(google: &Google, target: &Target, statement: &Statement) -> Result<Summary> {
    let id = &target.spreadsheet_id;
    let tab = a1_tab(&target.tab.title);

    let headers: Vec<String> = google
        .read_values(id, &format!("{tab}!1:1"))?
        .into_iter()
        .next()
        .unwrap_or_default()
        .iter()
        .map(cell_text)
        .collect();
    let empty_header = headers.iter().all(|h| h.trim().is_empty());
    let layout = Layout::from_headers(&headers)?;

    let (existing_ids, unreadable_ids) = if empty_header {
        // Writing a header row is only safe if the tab really is empty.
        if !google
            .read_values(id, &format!("{tab}!A2:Z3"))?
            .iter()
            .all(|r| r.iter().all(is_blank))
        {
            bail!(
                "tab {:?} has data but no header row in row 1; add the headers (including `id`) or pick another tab",
                target.tab.title
            );
        }
        (HashSet::new(), 0)
    } else {
        let col = column_letter(layout.id_index());
        let column = google.read_values(id, &format!("{tab}!{col}2:{col}"))?;
        parse_ids(column.into_iter().filter_map(|row| row.into_iter().next()))
    };

    let plan = plan(
        &layout,
        empty_header,
        &existing_ids,
        &statement.transactions,
    );
    let appended = plan.new_rows;
    let categorised = plan.categorised;
    if !plan.rows.is_empty() {
        let rows = plan
            .rows
            .iter()
            .map(|r| r.iter().map(Cell::to_json).collect())
            .collect();
        let written = google.append_rows(id, &format!("{tab}!A1"), rows)?;
        let expected = plan.rows.len() as u64;
        if written != expected {
            bail!("Google reports {written} rows written, expected {expected}; check the tab");
        }
    }
    Ok(Summary {
        parsed: statement.transactions.len(),
        existing: statement.transactions.len() - appended,
        appended,
        categorised,
        wrote_header: empty_header && !plan.rows.is_empty(),
        unreadable_ids,
    })
}

struct Plan {
    /// Rows to append, header row first when there is one.
    rows: Vec<Vec<Cell>>,
    new_rows: usize,
    categorised: usize,
}

/// Which rows to append: transactions whose id isn't in the tab yet, in file order, after the header row for an
/// empty tab. Nothing at all (not even a header) when there are no new transactions.
fn plan(layout: &Layout, write_header: bool, existing: &HashSet<i64>, txs: &[Transaction]) -> Plan {
    let new: Vec<&Transaction> = txs
        .iter()
        .filter(|t| !existing.contains(&i64::from(t.id)))
        .collect();
    let mut rows = Vec::with_capacity(new.len() + 1);
    if write_header && !new.is_empty() {
        rows.push(layout.header_cells());
    }
    rows.extend(new.iter().map(|t| layout.cells(t)));
    Plan {
        rows,
        new_rows: new.len(),
        categorised: new.iter().filter(|t| t.category.is_some()).count(),
    }
}

/// Ids from the id column. They may be stored as numbers (what this app and, most likely, n8n write) or as text.
/// Returns the ids and the number of non-blank cells that aren't ids.
fn parse_ids(cells: impl Iterator<Item = Value>) -> (HashSet<i64>, usize) {
    let mut ids = HashSet::new();
    let mut unreadable = 0;
    for cell in cells {
        if is_blank(&cell) {
            continue;
        }
        match parse_id(&cell) {
            Some(id) => {
                ids.insert(id);
            }
            None => unreadable += 1,
        }
    }
    (ids, unreadable)
}

fn parse_id(cell: &Value) -> Option<i64> {
    match cell {
        Value::Number(n) => n.as_i64().or_else(|| {
            let f = n.as_f64()?;
            (f.fract() == 0.0 && f.abs() < 2f64.powi(53)).then_some(f as i64)
        }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn is_blank(cell: &Value) -> bool {
    match cell {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

fn cell_text(cell: &Value) -> String {
    match cell {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A tab name quoted for A1 notation: `'My tab'`, with any `'` doubled.
fn a1_tab(title: &str) -> String {
    format!("'{}'", title.replace('\'', "''"))
}

/// 0 -> `A`, 25 -> `Z`, 26 -> `AA`.
fn column_letter(index: usize) -> String {
    let mut n = index + 1;
    let mut letters = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        letters.push(b'A' + rem as u8);
        n = (n - 1) / 26;
    }
    letters.reverse();
    String::from_utf8(letters).expect("ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tx(id: i32, date: &str, description: &str, amount: &str) -> Transaction {
        Transaction {
            id,
            date: date.into(),
            description: description.into(),
            amount: amount.into(),
            currency: "ZAR",
            category: None,
        }
    }

    #[test]
    fn column_letters() {
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(25), "Z");
        assert_eq!(column_letter(26), "AA");
        assert_eq!(column_letter(701), "ZZ");
        assert_eq!(column_letter(702), "AAA");
    }

    #[test]
    fn quotes_tab_names() {
        assert_eq!(a1_tab("Transactions"), "'Transactions'");
        assert_eq!(a1_tab("Brad's 2026"), "'Brad''s 2026'");
    }

    #[test]
    fn reads_ids_stored_as_numbers_or_text() {
        let cells = [
            json!(1339188638),
            json!(-1575916464.0),
            json!(" 2038276903 "),
            json!(""),
            Value::Null,
            json!("TOTAL"),
            json!(1.5),
        ];
        let (ids, unreadable) = parse_ids(cells.into_iter());
        assert_eq!(ids, HashSet::from([1339188638, -1575916464, 2038276903]));
        assert_eq!(unreadable, 2);
    }

    #[test]
    fn skips_existing_ids_and_keeps_file_order() {
        let txs = [
            tx(1, "2026-05-29", "A", "-1"),
            tx(2, "2026-05-30", "B", "-2"),
            tx(3, "2026-05-31", "C", "-3"),
        ];
        let layout = Layout::from_headers(&["id", "amount"]).unwrap();
        let plan = plan(&layout, false, &HashSet::from([2]), &txs);
        assert_eq!(plan.new_rows, 2);
        assert_eq!(
            plan.rows,
            [
                vec![Cell::Integer(1), Cell::Decimal(-1.0)],
                vec![Cell::Integer(3), Cell::Decimal(-3.0)]
            ]
        );
    }

    #[test]
    fn empty_tab_gets_header_row_first() {
        let txs = [tx(1, "2026-05-29", "A", "-1")];
        let plan = plan(&Layout::default_headers(), true, &HashSet::new(), &txs);
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.rows[0][0], Cell::Text("id".into()));
        assert_eq!(plan.rows[1][0], Cell::Integer(1));
    }

    #[test]
    fn nothing_new_means_nothing_written() {
        let txs = [tx(1, "2026-05-29", "A", "-1")];
        let plan = plan(&Layout::default_headers(), true, &HashSet::from([1]), &txs);
        assert!(plan.rows.is_empty());
        assert_eq!(plan.new_rows, 0);
    }
}
