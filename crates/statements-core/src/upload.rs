//! Sending a statement to the chosen tab. Every row is sent: rows whose id isn't in the tab are appended (with the
//! header row first when the tab is empty); rows whose id is already there get their `type` cell overwritten with
//! the statement's type. Only `type` is ever written to an existing row: every other value this app fills is the
//! id's own input (date, description, amount) or derived from it, so it's already right. A blank type never
//! replaces one in the sheet.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use serde_json::Value;

use crate::google::{Google, Tab};
use crate::row::{Cell, Column, Layout};
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
    /// Rows whose id was already in the tab.
    pub existing: usize,
    pub appended: usize,
    /// Existing rows whose `type` cell was overwritten (every copy of a repeated id counts).
    pub updated: usize,
    /// Rows sent with a category (`type`).
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

    let mut existing = Existing::default();
    if empty_header {
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
    } else {
        let col = column_letter(layout.id_index());
        let ids = google.read_values(id, &format!("{tab}!{col}2:{col}"))?;
        existing = Existing::from_column(ids.into_iter().map(|row| row.into_iter().next()));
        if let Some(type_index) = layout.index_of(Column::Type) {
            let col = column_letter(type_index);
            let types = google.read_values(id, &format!("{tab}!{col}2:{col}"))?;
            existing.types = types
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .next()
                        .map_or_else(String::new, |c| cell_text(&c))
                })
                .collect();
        }
    };

    let plan = plan(&layout, empty_header, &existing, &statement.transactions);
    if !plan.updates.is_empty() {
        let col = column_letter(
            layout
                .index_of(Column::Type)
                .expect("updates need a type column"),
        );
        let data = plan
            .updates
            .iter()
            .map(|(row, category)| {
                let cell = Cell::Text(category.clone()).to_json();
                (format!("{tab}!{col}{row}"), vec![vec![cell]])
            })
            .collect();
        google.update_values(id, data)?;
    }
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
        existing: statement.transactions.len() - plan.new_rows,
        appended: plan.new_rows,
        updated: plan.updates.len(),
        categorised: statement
            .transactions
            .iter()
            .filter(|t| t.category.is_some())
            .count(),
        wrote_header: empty_header && !plan.rows.is_empty(),
        unreadable_ids: existing.unreadable,
    })
}

/// What the tab already holds: where each id is (sheet row numbers, 1-based) and the `type` cells.
#[derive(Default)]
struct Existing {
    rows: HashMap<i64, Vec<usize>>,
    /// The `type` column from row 2 down (index 0 = row 2); empty when the tab has no `type` column.
    types: Vec<String>,
    unreadable: usize,
}

impl Existing {
    /// From the id column, row 2 down. Blank rows come back as `None` and keep their place.
    fn from_column(cells: impl Iterator<Item = Option<Value>>) -> Self {
        let mut existing = Existing::default();
        for (i, cell) in cells.enumerate() {
            let Some(cell) = cell.filter(|c| !is_blank(c)) else {
                continue;
            };
            match parse_id(&cell) {
                Some(id) => existing.rows.entry(id).or_default().push(i + 2),
                None => existing.unreadable += 1,
            }
        }
        existing
    }

    fn type_at(&self, row: usize) -> &str {
        self.types.get(row - 2).map_or("", String::as_str)
    }
}

struct Plan {
    /// Rows to append, header row first when there is one.
    rows: Vec<Vec<Cell>>,
    new_rows: usize,
    /// `type` cells to overwrite: (sheet row, new type).
    updates: Vec<(usize, String)>,
}

/// New ids are appended in file order, after the header row for an empty tab (no header when there's nothing to
/// append). Ids already in the tab get their `type` overwritten wherever it differs, in every copy; a blank type
/// leaves the cell alone, and a tab without a `type` column gets no updates.
fn plan(layout: &Layout, write_header: bool, existing: &Existing, txs: &[Transaction]) -> Plan {
    let has_type = layout.index_of(Column::Type).is_some();
    let mut new = Vec::new();
    let mut updates = Vec::new();
    let mut seen = HashSet::new();
    for t in txs {
        match existing.rows.get(&t.id) {
            None => new.push(t),
            Some(rows) => {
                let Some(category) = t.category.as_ref().filter(|_| has_type) else {
                    continue;
                };
                for &row in rows {
                    if seen.insert(row) && existing.type_at(row) != category {
                        updates.push((row, category.clone()));
                    }
                }
            }
        }
    }
    let mut rows = Vec::with_capacity(new.len() + 1);
    if write_header && !new.is_empty() {
        rows.push(layout.header_cells());
    }
    rows.extend(new.iter().map(|t| layout.cells(t)));
    Plan {
        rows,
        new_rows: new.len(),
        updates,
    }
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

    fn tx(id: i64, date: &str, description: &str, amount: &str) -> Transaction {
        Transaction {
            id,
            occurrence: 0,
            date: date.into(),
            description: description.into(),
            amount: amount.into(),
            currency: "ZAR".into(),
            category: None,
            effective_date: None,
            hand_added: false,
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

    fn existing(ids: &[(i64, usize)], types: &[&str]) -> Existing {
        let mut e = Existing::default();
        for &(id, row) in ids {
            e.rows.entry(id).or_default().push(row);
        }
        e.types = types.iter().map(|t| t.to_string()).collect();
        e
    }

    fn typed(mut t: Transaction, category: &str) -> Transaction {
        t.category = Some(category.into());
        t
    }

    #[test]
    fn reads_ids_stored_as_numbers_or_text_and_keeps_rows() {
        let cells = [
            Some(json!(1339188638)),
            Some(json!(-1575916464.0)),
            None,
            Some(json!(" 2038276903 ")),
            Some(json!("")),
            Some(json!("TOTAL")),
            Some(json!(1.5)),
            Some(json!(1339188638)),
        ];
        let e = Existing::from_column(cells.into_iter());
        assert_eq!(e.rows[&1339188638], [2, 9]);
        assert_eq!(e.rows[&-1575916464], [3]);
        assert_eq!(e.rows[&2038276903], [5]);
        assert_eq!(e.rows.len(), 3);
        assert_eq!(e.unreadable, 2);
    }

    #[test]
    fn appends_new_ids_in_file_order() {
        let txs = [
            tx(1, "2026-05-29", "A", "-1"),
            tx(2, "2026-05-30", "B", "-2"),
            tx(3, "2026-05-31", "C", "-3"),
        ];
        let layout = Layout::from_headers(&["id", "amount"]).unwrap();
        let plan = plan(&layout, false, &existing(&[(2, 2)], &[]), &txs);
        assert_eq!(plan.new_rows, 2);
        assert_eq!(
            plan.rows,
            [
                vec![Cell::Integer(1), Cell::Decimal(-1.0)],
                vec![Cell::Integer(3), Cell::Decimal(-3.0)]
            ]
        );
        assert!(plan.updates.is_empty());
    }

    #[test]
    fn existing_rows_get_their_type_overwritten_in_every_copy() {
        let layout = Layout::from_headers(&["id", "type"]).unwrap();
        let txs = [
            typed(tx(1, "2026-05-29", "A", "-1"), "FEES"),
            typed(tx(2, "2026-05-30", "B", "-2"), "RENT"),
            typed(tx(3, "2026-05-31", "C", "-3"), "RATES"),
        ];
        // Id 1 is in rows 2 and 4 (one of them already FEES); id 2 in row 3 as OLD; id 3 is new.
        let e = existing(&[(1, 2), (1, 4), (2, 3)], &["", "OLD", "FEES"]);
        let plan = plan(&layout, false, &e, &txs);
        assert_eq!(plan.updates, [(2, "FEES".into()), (3, "RENT".into())]);
        assert_eq!(plan.new_rows, 1);
    }

    #[test]
    fn a_blank_type_never_replaces_one() {
        let layout = Layout::from_headers(&["id", "type"]).unwrap();
        let txs = [tx(1, "2026-05-29", "A", "-1")];
        let plan = plan(&layout, false, &existing(&[(1, 2)], &["FEES"]), &txs);
        assert!(plan.updates.is_empty());
        assert!(plan.rows.is_empty());
    }

    #[test]
    fn no_type_column_means_no_updates() {
        let layout = Layout::from_headers(&["id", "amount"]).unwrap();
        let txs = [typed(tx(1, "2026-05-29", "A", "-1"), "FEES")];
        let plan = plan(&layout, false, &existing(&[(1, 2)], &[]), &txs);
        assert!(plan.updates.is_empty());
    }

    #[test]
    fn empty_tab_gets_header_row_first() {
        let txs = [tx(1, "2026-05-29", "A", "-1")];
        let plan = plan(&Layout::default_headers(), true, &Existing::default(), &txs);
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.rows[0][0], Cell::Text("id".into()));
        assert_eq!(plan.rows[1][0], Cell::Integer(1));
    }

    #[test]
    fn nothing_new_means_nothing_appended() {
        let txs = [tx(1, "2026-05-29", "A", "-1")];
        let plan = plan(
            &Layout::default_headers(),
            true,
            &existing(&[(1, 2)], &[]),
            &txs,
        );
        assert!(plan.rows.is_empty());
        assert_eq!(plan.new_rows, 0);
    }
}
