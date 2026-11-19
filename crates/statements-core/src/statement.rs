//! A bank statement, whatever its format: detect the format from the file's shape, parse it, then give every row
//! its n8n-compatible id (n8n "Determine Statement Type" -> "Parse ..." -> "Enrich csv (id & unique descriptions)").

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::hash::{DateFormat, row_id};
use crate::sb_csv;

/// A statement format this app can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Standard Bank CSV export.
    SbCsv,
}

impl Format {
    /// Every known format, in detection order.
    pub const ALL: [Format; 1] = [Format::SbCsv];

    /// The name used in the config and messages.
    pub fn id(self) -> &'static str {
        match self {
            Format::SbCsv => "sb_csv",
        }
    }

    /// The format with this config name (e.g. `sb_csv`).
    pub fn from_id(id: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.id() == id)
    }

    pub fn description(self) -> &'static str {
        match self {
            Format::SbCsv => "Standard Bank CSV",
        }
    }

    /// How the format is recognised, for the "unrecognised statement" error.
    fn recognised_by(self) -> &'static str {
        match self {
            Format::SbCsv => "first cell is \"0\"",
        }
    }

    fn matches(self, rows: &[csv::StringRecord]) -> bool {
        match self {
            Format::SbCsv => sb_csv::matches(rows),
        }
    }

    fn parse(self, rows: &[csv::StringRecord]) -> Result<Parsed> {
        match self {
            Format::SbCsv => sb_csv::parse(rows),
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.description(), self.id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub id: i32,
    /// `YYYY-MM-DD`
    pub date: String,
    /// As the format builds it, plus any `_` suffixes added to make same-day ids unique.
    pub description: String,
    /// Raw string from the file, never reformatted (it is part of the id).
    pub amount: String,
    pub currency: &'static str,
    /// Goes in the `type` column; set by the category rules (`categories.rs`), if any match.
    pub category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub format: Format,
    /// Only when the file states it (e.g. Standard Bank CSV row 1). Many formats don't, so the account has to come
    /// from configuration or the user.
    pub account_number: Option<String>,
    pub transactions: Vec<Transaction>,
}

/// What a format parser returns: transactions without ids yet.
#[derive(Debug)]
pub struct Parsed {
    pub account_number: Option<String>,
    pub transactions: Vec<Transaction>,
}

pub fn parse_file(path: &Path, date_format: DateFormat) -> Result<Statement> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    parse(file, date_format).with_context(|| format!("reading statement {}", path.display()))
}

/// Detects the format, parses the rows and assigns ids. Cells are used exactly as read: no trimming.
pub fn parse<R: Read>(reader: R, date_format: DateFormat) -> Result<Statement> {
    let rows = read_rows(reader)?;
    let Some(format) = detect(&rows) else {
        let known: Vec<String> = Format::ALL
            .iter()
            .map(|f| format!("{f}: {}", f.recognised_by()))
            .collect();
        bail!(
            "unrecognised statement format. Known formats:\n  {}",
            known.join("\n  ")
        );
    };
    let Parsed {
        account_number,
        mut transactions,
    } = format
        .parse(&rows)
        .with_context(|| format!("parsing {format}"))?;
    assign_unique_ids(&mut transactions, date_format);
    Ok(Statement {
        format,
        account_number,
        transactions,
    })
}

/// The first format whose detector accepts the rows.
fn detect(rows: &[csv::StringRecord]) -> Option<Format> {
    Format::ALL.into_iter().find(|f| f.matches(rows))
}

/// All rows, without treating any as a header (formats differ on where their header is, if any).
fn read_rows<R: Read>(reader: R) -> Result<Vec<csv::StringRecord>> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::None)
        .from_reader(reader)
        .records()
        .collect::<Result<_, _>>()
        .context("reading CSV")
}

/// n8n `uniqDescriptionsByDay` + `getRowId`: within each day, append `_` to the description until the id is
/// unique, then use that id. Days are independent, so processing in file order gives the same result.
fn assign_unique_ids(transactions: &mut [Transaction], date_format: DateFormat) {
    let mut ids_by_day: HashMap<String, HashSet<i32>> = HashMap::new();
    for t in transactions {
        let seen = ids_by_day.entry(t.date.clone()).or_default();
        let mut id = row_id(&t.date, &t.description, &t.amount, date_format);
        while seen.contains(&id) {
            t.description.push('_');
            id = row_id(&t.date, &t.description, &t.amount, date_format);
        }
        seen.insert(id);
        t.id = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_standard_bank_csv() {
        let s = parse(
            &include_bytes!("../tests/fixtures/sb_csv_synthetic.csv")[..],
            DateFormat::UtcTimestamp,
        )
        .unwrap();
        assert_eq!(s.format, Format::SbCsv);
        assert_eq!(s.account_number.as_deref(), Some("123456789"));
    }

    #[test]
    fn unrecognised_file_lists_known_formats() {
        let err = parse(
            "Type,Product,Started Date\n".as_bytes(),
            DateFormat::UtcTimestamp,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unrecognised statement format"), "{err}");
        assert!(
            err.contains("Standard Bank CSV (sb_csv): first cell is \"0\""),
            "{err}"
        );
    }

    #[test]
    fn empty_file_is_unrecognised() {
        let err = parse("".as_bytes(), DateFormat::UtcTimestamp).unwrap_err();
        assert!(err.to_string().contains("unrecognised"), "{err}");
    }

    #[test]
    fn same_day_duplicates_get_underscore_suffix() {
        let csv = "0,5009,BRANCH,0,,X,0,0\n,123,ACC-NO,0,,,0,0\n,0,OPEN,1,OPEN BALANCE,,0,0\n\
                   HIST,20260101,,-5,FEE,,1,0\nHIST,20260101,,-5,FEE,,1,0\nHIST,20260101,,-5,FEE,,1,0\n\
                   HIST,20260102,,-5,FEE,,1,0\n,0,CLOSE,1,CLOSE BALANCE,,0,0";
        let s = parse(csv.as_bytes(), DateFormat::UtcTimestamp).unwrap();
        let descs: Vec<&str> = s
            .transactions
            .iter()
            .map(|t| t.description.as_str())
            .collect();
        assert_eq!(descs, ["FEE ", "FEE _", "FEE __", "FEE "]);
    }
}
