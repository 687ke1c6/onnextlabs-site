//! A bank statement, whatever its format: detect the format from the file's shape, parse it, then give every row
//! its id (`id.rs`). Detection and parsing of CSVs follow n8n's "Determine Statement Type" -> "Parse ..." nodes;
//! PDFs (`sb_pdf`) are told apart by the layout version they state.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::hash::{self, DateFormat};
use crate::id::{self, ID_LIMIT};
use crate::{googlesheet_csv, sb_csv, sb_pdf, sb_pdf_csv};

/// A statement format this app can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Standard Bank CSV export.
    SbCsv,
    /// Standard Bank PDF statement, scraped into a CSV by a separate tool (not the PDF itself).
    SbPdfCsv,
    /// A Google Sheet that n8n (or this app) wrote statement rows to, downloaded as CSV. Not a bank's format: its
    /// rows already have ids, which are kept.
    GoogleSheetCsv,
    /// Standard Bank PDF "Transaction History Statement", layout version 2026-01 (`sb_pdf`).
    SbPdf2026_01,
}

impl Format {
    /// Every known format, in detection order.
    pub const ALL: [Format; 4] = [
        Format::SbCsv,
        Format::SbPdfCsv,
        Format::GoogleSheetCsv,
        Format::SbPdf2026_01,
    ];

    /// The name used in the config and messages.
    pub fn id(self) -> &'static str {
        match self {
            Format::SbCsv => "sb_csv",
            Format::SbPdfCsv => "sb_pdf_csv",
            Format::GoogleSheetCsv => "googlesheet_csv",
            Format::SbPdf2026_01 => "sb_pdf_2026_01",
        }
    }

    /// A PDF, rather than a CSV.
    pub fn is_pdf(self) -> bool {
        matches!(self, Format::SbPdf2026_01)
    }

    /// The format with this config name (e.g. `sb_csv`).
    pub fn from_id(id: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.id() == id)
    }

    pub fn description(self) -> &'static str {
        match self {
            Format::SbCsv => "Standard Bank CSV",
            Format::SbPdfCsv => "Standard Bank PDF, converted to CSV",
            Format::GoogleSheetCsv => "Google Sheet export (CSV)",
            Format::SbPdf2026_01 => "Standard Bank PDF statement (2026-01 layout)",
        }
    }

    /// How the format is recognised, for the "unrecognised statement" error.
    fn recognised_by(self) -> &'static str {
        match self {
            Format::SbCsv => "first cell is \"0\"",
            Format::SbPdfCsv => "header row starts \"description,amount,date\"",
            Format::GoogleSheetCsv => "header row has id, date, amount and description columns",
            Format::SbPdf2026_01 => "a PDF whose footer reads \"PPB00067436 2026-01\"",
        }
    }

    fn matches(self, rows: &[csv::StringRecord]) -> bool {
        match self {
            Format::SbCsv => sb_csv::matches(rows),
            Format::SbPdfCsv => sb_pdf_csv::matches(rows),
            Format::GoogleSheetCsv => googlesheet_csv::matches(rows),
            // Not a CSV.
            Format::SbPdf2026_01 => false,
        }
    }

    fn parse(self, rows: &[csv::StringRecord]) -> Result<Parsed> {
        match self {
            Format::SbCsv => sb_csv::parse(rows),
            Format::SbPdfCsv => sb_pdf_csv::parse(rows),
            Format::GoogleSheetCsv => googlesheet_csv::parse(rows),
            Format::SbPdf2026_01 => unreachable!("a PDF format never matches CSV rows"),
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
    /// See `id.rs`: a hash of the date, amount, normalised description and `occurrence`.
    pub id: i64,
    /// 0 for the first row of the statement with this key (date, amount, normalised description), 1 for the
    /// next, and so on. Part of the id; replaces n8n's `_` description suffixes.
    pub occurrence: u32,
    /// `YYYY-MM-DD`
    pub date: String,
    /// As the format builds it; never changed to make it unique.
    pub description: String,
    /// As written in the file (the id uses it in cents).
    pub amount: String,
    pub currency: String,
    /// Goes in the `type` column: from the file when its format has one (`googlesheet_csv`), otherwise from the
    /// category rules (`categories.rs`), if any match.
    pub category: Option<String>,
    /// `YYYY-MM-DD`: the date the row counts towards when it isn't `date` (e.g. a bond repayment on the 1st that
    /// belongs to the month before). Only `googlesheet_csv` states one; set by hand there.
    pub effective_date: Option<String>,
    /// Not from a bank statement: added by hand (a Google Sheet row without an id). Such rows can be deleted
    /// on their own; a bank statement containing the same transaction later makes it a statement row.
    pub hand_added: bool,
}

/// A file that states its rows' ids (`googlesheet_csv`): how those ids compare with the rows. The stated ids are
/// replaced by computed ones either way; this checks the import. Old sheets hold n8n ids (`hash.rs`), newer ones
/// this app's.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct IdCheck {
    /// Already this app's id for the row.
    pub current: usize,
    /// The n8n id of the row exactly as written.
    pub n8n_matched: usize,
    /// The n8n id of the row with its amount written another way (`-1798` shown as `-1,798.00`): the sheet
    /// reformatted the number after n8n hashed it.
    pub n8n_reformatted: usize,
    /// Neither: the row was edited after its id was made (e.g. a duplicate charge zeroed by hand).
    pub mismatched: Vec<IdMismatch>,
    /// 1-based lines with a blank id (rows added by hand).
    pub no_id: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IdMismatch {
    /// 1-based line in the file.
    pub line: usize,
    /// As stated in the file.
    pub id: i64,
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub format: Format,
    /// Only when the file states it (e.g. Standard Bank CSV row 1). Many formats don't, so the account has to come
    /// from configuration or the user.
    pub account_number: Option<String>,
    pub transactions: Vec<Transaction>,
    /// Formats whose rows state their ids: how they compare with the rows.
    pub id_check: Option<IdCheck>,
}

/// What a format parser returns: transactions without ids yet.
#[derive(Debug)]
pub struct Parsed {
    pub account_number: Option<String>,
    pub transactions: Vec<Transaction>,
    /// Formats whose rows state their ids: what each row stated, one per transaction.
    pub stated_ids: Option<Vec<StatedId>>,
}

/// A row's id as the file states it, with what's needed to check it.
#[derive(Debug)]
pub struct StatedId {
    /// 1-based line in the file.
    pub line: usize,
    pub id: Option<i64>,
    /// The description as written, with any n8n `_` suffix (part of an n8n id).
    pub description: String,
}

pub fn parse_file(path: &Path) -> Result<Statement> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    parse(file).with_context(|| format!("reading statement {}", path.display()))
}

/// Detects the format, parses the rows and assigns ids. Cells are used exactly as read: no trimming.
pub fn parse<R: Read>(mut reader: R) -> Result<Statement> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .context("reading the statement")?;
    if bytes.starts_with(b"%PDF-") {
        return parse_pdf(&bytes);
    }
    let rows = read_rows(&bytes[..])?;
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
        stated_ids,
    } = format
        .parse(&rows)
        .with_context(|| format!("parsing {format}"))?;
    assign_ids(&mut transactions)?;
    let id_check = stated_ids.map(|stated| googlesheet_csv::check_ids(&stated, &transactions));
    Ok(Statement {
        format,
        account_number,
        transactions,
        id_check,
    })
}

/// A PDF statement: the layout it states decides the format.
fn parse_pdf(pdf: &[u8]) -> Result<Statement> {
    let text = sb_pdf::text(pdf)?;
    let format = match sb_pdf::kind(&text) {
        sb_pdf::Kind::Current => Format::SbPdf2026_01,
        sb_pdf::Kind::OtherVersion(version) => bail!(
            "this is a Standard Bank PDF statement in a layout this app doesn't read yet ({version}; it reads \
             {} {}). Upload the CSV statement instead until it does.",
            sb_pdf::FORM,
            sb_pdf::VERSION
        ),
        sb_pdf::Kind::Unknown => bail!(
            "unrecognised PDF. The only PDF statements this app reads are {}.",
            Format::SbPdf2026_01
        ),
    };
    let Parsed {
        account_number,
        mut transactions,
        ..
    } = sb_pdf::parse_text(&text).with_context(|| format!("parsing {format}"))?;
    assign_ids(&mut transactions)?;
    Ok(Statement {
        format,
        account_number,
        transactions,
        id_check: None,
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

/// Gives every transaction its occurrence and id (`id.rs`). Refuses an amount that isn't a number of cents.
fn assign_ids(transactions: &mut [Transaction]) -> Result<()> {
    let mut occurrences: HashMap<(String, i64, String), u32> = HashMap::new();
    let mut ids = HashSet::new();
    for t in transactions {
        let cents = id::amount_cents(&t.amount)
            .with_context(|| format!("{} {:?}", t.date, t.description))?;
        let description = id::key_description(&t.description);
        let n = occurrences
            .entry((t.date.clone(), cents, description.clone()))
            .or_default();
        t.occurrence = *n;
        *n += 1;
        t.id = id::row_id(&t.date, cents, &description, t.occurrence);
        debug_assert!((0..ID_LIMIT).contains(&t.id));
        // Different keys with the same 48-bit id: never expected, but two rows must never become one.
        ensure!(
            ids.insert(t.id),
            "two different rows got the same id ({} {:?}); please report this",
            t.date,
            t.description
        );
    }
    Ok(())
}

/// The ids n8n gave these transactions, with the descriptions it wrote: n8n `uniqDescriptionsByDay` +
/// `getRowId`, which append `_` to the description until the id is unique that day. For checking the parsers
/// against n8n's output.
pub fn n8n_ids(transactions: &[Transaction]) -> Vec<(i32, String)> {
    let mut ids_by_day: HashMap<&str, HashSet<i32>> = HashMap::new();
    transactions
        .iter()
        .map(|t| {
            let seen = ids_by_day.entry(&t.date).or_default();
            let mut description = t.description.clone();
            let id_of = |d: &str| hash::row_id(&t.date, d, &t.amount, DateFormat::UtcTimestamp);
            let mut n8n_id = id_of(&description);
            while seen.contains(&n8n_id) {
                description.push('_');
                n8n_id = id_of(&description);
            }
            seen.insert(n8n_id);
            (n8n_id, description)
        })
        .collect()
}

/// How far a row's effective date may be from its date, in days. Effective dates move a row to a nearby
/// month (a payment on the 1st that belongs to the month before); one a year or more away is a typing slip.
pub const MAX_EFFECTIVE_DAYS: i64 = 366;

/// An effective date for a row dated `date` (both `YYYY-MM-DD`): a real date within [`MAX_EFFECTIVE_DAYS`] of it.
pub fn check_effective_date(date: &str, effective: &str) -> Result<String> {
    let effective = check_iso_date(effective)?;
    let apart = (day_number(&effective) - day_number(&check_iso_date(date)?)).abs();
    ensure!(
        apart <= MAX_EFFECTIVE_DAYS,
        "{effective} is more than a year from the row's date ({date})"
    );
    Ok(effective)
}

/// Days since 1970-01-01 of a checked `YYYY-MM-DD` (proleptic Gregorian).
fn day_number(date: &str) -> i64 {
    let (y, m, d): (i64, i64, i64) = (
        date[0..4].parse().expect("checked"),
        date[5..7].parse().expect("checked"),
        date[8..10].parse().expect("checked"),
    );
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DD`, checked to be a real date.
pub fn check_iso_date(date: &str) -> Result<String> {
    let b = date.as_bytes();
    ensure!(
        b.len() == 10 && b[4] == b'-' && b[7] == b'-',
        "expected YYYY-MM-DD"
    );
    to_iso_date(&format!("{}{}{}", &date[0..4], &date[5..7], &date[8..10]))
}

/// `YYYYMMDD` -> `YYYY-MM-DD`. JS `new Date(y, m, d)` silently rolls invalid dates over (Feb 31 -> Mar 3);
/// this rejects them instead.
pub(crate) fn to_iso_date(yyyymmdd: &str) -> Result<String> {
    ensure!(
        yyyymmdd.len() == 8 && yyyymmdd.bytes().all(|b| b.is_ascii_digit()),
        "expected YYYYMMDD"
    );
    let year: u32 = yyyymmdd[0..4].parse()?;
    let month: u32 = yyyymmdd[4..6].parse()?;
    let day: u32 = yyyymmdd[6..8].parse()?;
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => bail!("month out of range"),
    };
    ensure!((1..=days_in_month).contains(&day), "day out of range");
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_standard_bank_csv() {
        let s = parse(&include_bytes!("../tests/fixtures/sb_csv_synthetic.csv")[..]).unwrap();
        assert_eq!(s.format, Format::SbCsv);
        assert_eq!(s.account_number.as_deref(), Some("123456789"));
    }

    #[test]
    fn unrecognised_file_lists_known_formats() {
        let err = parse("Type,Product,Started Date\n".as_bytes())
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
        let err = parse("".as_bytes()).unwrap_err();
        assert!(err.to_string().contains("unrecognised"), "{err}");
    }

    const DUPLICATES: &str = "0,5009,BRANCH,0,,X,0,0\n,123,ACC-NO,0,,,0,0\n,0,OPEN,1,OPEN BALANCE,,0,0\n\
        HIST,20260101,,-5,FEE,,1,0\nHIST,20260101,,-5,FEE,,1,0\nHIST,20260101,,-5.00,FEE  ##,,1,0\n\
        HIST,20260102,,-5,FEE,,1,0\n,0,CLOSE,1,CLOSE BALANCE,,0,0";

    #[test]
    fn same_key_rows_are_counted_not_renamed() {
        let s = parse(DUPLICATES.as_bytes()).unwrap();
        let rows: Vec<(&str, u32)> = s
            .transactions
            .iter()
            .map(|t| (t.description.as_str(), t.occurrence))
            .collect();
        // The third row differs only in formatting, so it's the same key.
        assert_eq!(
            rows,
            [("FEE ", 0), ("FEE ", 1), ("FEE  ## ", 2), ("FEE ", 0)]
        );
        let ids: HashSet<i64> = s.transactions.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 4);
        assert_eq!(
            s.transactions[0].id,
            id::row_id("2026-01-01", -500, "FEE", 0)
        );
    }

    #[test]
    fn n8n_ids_add_underscores_per_day() {
        let s = parse(DUPLICATES.as_bytes()).unwrap();
        let descs: Vec<String> = n8n_ids(&s.transactions)
            .into_iter()
            .map(|(_, d)| d)
            .collect();
        assert_eq!(descs, ["FEE ", "FEE _", "FEE  ## ", "FEE "]);
    }

    #[test]
    fn unreadable_amounts_are_refused() {
        let csv = DUPLICATES.replace("-5.00", "R5");
        let err = format!("{:#}", parse(csv.as_bytes()).unwrap_err());
        assert!(err.contains("amount \"R5\" isn't a number"), "{err}");
    }

    #[test]
    fn effective_dates_are_within_a_year_of_the_date() {
        assert_eq!(day_number("1970-01-01"), 0);
        assert_eq!(day_number("2026-03-01") - day_number("2026-02-28"), 1);
        assert_eq!(
            check_effective_date("2026-08-28", "2026-09-01").unwrap(),
            "2026-09-01"
        );
        assert!(check_effective_date("2026-08-28", "2025-08-27").is_ok());
        assert!(check_effective_date("2026-08-28", "2027-08-30").is_err());
        assert!(check_effective_date("2026-08-28", "0002-09-01").is_err());
        assert!(check_effective_date("2026-08-28", "2026-02-30").is_err());
    }
}
