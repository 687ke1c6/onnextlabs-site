//! Standard Bank PDF statements: the "Transaction History Statement" as the bank's app and website give it,
//! in the layout whose footer reads `PPB00067436 2026-01` (form PPB00067436, version 2026-01). The bank changes
//! this layout now and then, so this format reads exactly that version: another version is refused by name,
//! and needs its own format (with its own checks) rather than a guess.
//!
//! Rows are rebuilt to read exactly as the same transaction does in a Standard Bank CSV export (`sb_csv`), so
//! it gets the same row id and uploading both never saves it twice. The PDF writes a row's description as
//! `<detail> [##] - <description, lower-cased>`, where the CSV has the description and detail in their own
//! columns (and the `##` fee marker in another). When there's no detail, the PDF repeats the description, in
//! capitals, before the dash. Rebuilding it:
//!
//! - the description is upper-cased, and a word the PDF leaves off is put back: `ib payment` is
//!   `IB PAYMENT TO` paying out and `IB PAYMENT FROM` paying in (the CSVs seen so far always agree with the
//!   amount's sign). Only descriptions in [`DESCRIPTIONS`] are known; any other is refused, since a word the
//!   PDF dropped can't be known, and guessing wrong would save the transaction twice.
//! - the `##` / `#` fee marker is dropped (the CSV keeps it in its own column);
//! - the row reads `<DESCRIPTION> <detail>`, as `sb_csv` builds it. Runs of spaces in the CSV's detail come out
//!   as one space here; row ids treat them the same.
//!
//! Every row's amount must take the running balance from the row before to its own: a row read wrongly (or
//! missed) fails that check, and the whole statement is refused.
//!
//! Amounts are stored without thousands separators or `+` (`-20319.46`). The account number comes from the
//! "Account number" line, without its spaces (`10197691372`, as the CSV states it).

use anyhow::{Context, Result, bail, ensure};

use crate::statement::{Parsed, Transaction, to_iso_date};

/// The footer every page of this layout carries.
pub const FORM: &str = "PPB00067436";
pub const VERSION: &str = "2026-01";

/// Descriptions this layout is known to write: the PDF's (lower-case) text, and the CSV's when the amount is
/// paid out and paid in. They differ only where the PDF leaves a word off.
const DESCRIPTIONS: &[(&str, &str, &str)] = &[
    ("account payment", "ACCOUNT PAYMENT", "ACCOUNT PAYMENT"),
    (
        "autobank cash withdrawal at",
        "AUTOBANK CASH WITHDRAWAL AT",
        "AUTOBANK CASH WITHDRAWAL AT",
    ),
    ("bank charges", "BANK CHARGES", "BANK CHARGES"),
    (
        "cash withdrawal fee",
        "CASH WITHDRAWAL FEE",
        "CASH WITHDRAWAL FEE",
    ),
    (
        "cellphone instantmon cash",
        "CELLPHONE INSTANTMON CASH TO",
        "CELLPHONE INSTANTMON CASH TO",
    ),
    (
        "cheque card purchase",
        "CHEQUE CARD PURCHASE",
        "CHEQUE CARD PURCHASE",
    ),
    ("credit transfer", "CREDIT TRANSFER", "CREDIT TRANSFER"),
    (
        "debit card purchase",
        "DEBIT CARD PURCHASE FROM",
        "DEBIT CARD PURCHASE FROM",
    ),
    (
        "debit card refund",
        "DEBIT CARD REFUND FROM",
        "DEBIT CARD REFUND FROM",
    ),
    ("debit transfer", "DEBIT TRANSFER", "DEBIT TRANSFER"),
    (
        "electricity purchase",
        "ELECTRICITY PURCHASE",
        "ELECTRICITY PURCHASE",
    ),
    (
        "fee - debit order",
        "FEE - DEBIT ORDER",
        "FEE - DEBIT ORDER",
    ),
    (
        "fee - instant money",
        "FEE - INSTANT MONEY",
        "FEE - INSTANT MONEY",
    ),
    (
        "fee immediate payment",
        "FEE IMMEDIATE PAYMENT",
        "FEE IMMEDIATE PAYMENT",
    ),
    (
        "fee-electronic account payment",
        "FEE-ELECTRONIC ACCOUNT PAYMENT",
        "FEE-ELECTRONIC ACCOUNT PAYMENT",
    ),
    (
        "fee-replacement atm card",
        "FEE-REPLACEMENT ATM CARD",
        "FEE-REPLACEMENT ATM CARD",
    ),
    (
        "fee: electricity purchase",
        "FEE: ELECTRICITY PURCHASE",
        "FEE: ELECTRICITY PURCHASE",
    ),
    (
        "fee: emailed information",
        "FEE: EMAILED INFORMATION",
        "FEE: EMAILED INFORMATION",
    ),
    ("ib payment", "IB PAYMENT TO", "IB PAYMENT FROM"),
    ("ib transfer", "IB TRANSFER TO", "IB TRANSFER FROM"),
    (
        "immediate payment",
        "IMMEDIATE PAYMENT",
        "IMMEDIATE PAYMENT",
    ),
    (
        "insurance premium",
        "INSURANCE PREMIUM",
        "INSURANCE PREMIUM",
    ),
    ("magtape credit", "MAGTAPE CREDIT", "MAGTAPE CREDIT"),
    ("membership fee", "MEMBERSHIP FEE", "MEMBERSHIP FEE"),
    (
        "monthly management fee",
        "MONTHLY MANAGEMENT FEE",
        "MONTHLY MANAGEMENT FEE",
    ),
    (
        "reversal: bank fee fee: electricity purchase",
        "REVERSAL: BANK FEE FEE: ELECTRICITY PURCHASE",
        "REVERSAL: BANK FEE FEE: ELECTRICITY PURCHASE",
    ),
    ("reverse fee", "REVERSE FEE", "REVERSE FEE"),
    (
        "service agreement",
        "SERVICE AGREEMENT",
        "SERVICE AGREEMENT",
    ),
    (
        "std bank bond repayment",
        "STD BANK BOND REPAYMENT",
        "STD BANK BOND REPAYMENT",
    ),
    (
        "transaction reversal electricity purchase",
        "TRANSACTION REVERSAL ELECTRICITY PURCHASE",
        "TRANSACTION REVERSAL ELECTRICITY PURCHASE",
    ),
];

/// What a PDF is, by its text.
#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    /// This layout.
    Current,
    /// A Standard Bank statement in another version of the layout (`PPB00067436 2027-03`, say).
    OtherVersion(String),
    /// Something else.
    Unknown,
}

/// Which layout the PDF's text is.
pub fn kind(text: &str) -> Kind {
    if !text.contains("Transaction History Statement") {
        return Kind::Unknown;
    }
    let current = format!("{FORM} {VERSION}");
    if text.contains(&current) {
        return Kind::Current;
    }
    // The footer's form and version, whatever they are.
    let other = text.lines().find_map(|l| {
        let at = l.find("PPB")?;
        let mut words = l[at..].split_whitespace();
        Some(
            format!("{} {}", words.next()?, words.next().unwrap_or(""))
                .trim()
                .to_string(),
        )
    });
    other.map_or(Kind::Unknown, Kind::OtherVersion)
}

/// The text of a PDF, page by page as the bank laid it out.
pub fn text(pdf: &[u8]) -> Result<String> {
    pdf_extract::extract_text_from_mem(pdf).context("reading the PDF's text")
}

/// Lines of the page header and footer, repeated on every page.
fn is_page_furniture(line: &str) -> bool {
    const STARTS: &[&str] = &[
        "Customer Care:",
        "Website:",
        "Transaction History Statement",
        "Account holder:",
        "Account number:",
        "Product name:",
        "Transaction date range:",
        "Available balance:",
        "Date Description",
        "# These fees",
        "## These fees",
        "Please verify all transactions",
        FORM,
    ];
    STARTS.iter().any(|s| line.starts_with(s)) || is_long_date(line)
}

/// `25 November 2026`: the date the statement was made, at the top of each page.
fn is_long_date(line: &str) -> bool {
    let mut w = line.split(' ');
    let (Some(d), Some(m), Some(y), None) = (w.next(), w.next(), w.next(), w.next()) else {
        return false;
    };
    d.len() <= 2
        && d.bytes().all(|b| b.is_ascii_digit())
        && m.len() > 3
        && month(&m[..3]).is_some()
        && y.len() == 4
        && y.bytes().all(|b| b.is_ascii_digit())
}

fn month(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS.iter().position(|m| *m == name).map(|i| i as u32 + 1)
}

/// `28 Sep 2026 rest…` -> (`2026-09-28`, `rest…`).
fn row_start(line: &str) -> Option<(String, &str)> {
    let mut parts = line.splitn(4, ' ');
    let (d, m, y, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if d.len() != 2 || !d.bytes().all(|b| b.is_ascii_digit()) || y.len() != 4 {
        return None;
    }
    let m = month(m)?;
    let date = to_iso_date(&format!("{y}{m:02}{d}")).ok()?;
    Some((date, rest))
}

/// An amount as the PDF writes it (`-20,319.46`, `+1,283.25`, `35,275.08`), in cents.
fn cents(s: &str) -> Option<i64> {
    let s = s.replace(',', "");
    let (negative, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s.strip_prefix('+').unwrap_or(&s)),
    };
    let (whole, frac) = digits.split_once('.')?;
    if whole.is_empty()
        || frac.len() != 2
        || !(whole.bytes().chain(frac.bytes())).all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let c = whole.parse::<i64>().ok()? * 100 + frac.parse::<i64>().ok()?;
    Some(if negative { -c } else { c })
}

/// A row's text ends with its amount (signed) and the balance after it: (body, amount, balance).
fn split_amounts(text: &str) -> Option<(&str, &str, &str)> {
    let (rest, balance) = text.rsplit_once(' ')?;
    let (body, amount) = rest.rsplit_once(' ')?;
    (amount.starts_with(['+', '-']) && cents(amount).is_some() && cents(balance).is_some())
        .then_some((body, amount, balance))
}

/// The CSV's description and detail for a PDF row's body (`<detail> [##] - <description>`).
fn rebuild(body: &str, out: bool) -> Result<String> {
    // The description is the part after one of the " - "s that's a known one. Tried from the left, so the
    // longest known description wins (`fee - debit order` over `debit order`).
    let found = body.match_indices(" - ").find_map(|(at, _)| {
        let written = &body[at + 3..];
        DESCRIPTIONS
            .iter()
            .find(|(pdf, _, _)| *pdf == written)
            .map(|d| (&body[..at], d))
    });
    let Some((left, (pdf, paid_out, paid_in))) = found else {
        let written = body.rsplit_once(" - ").map_or(body, |(_, d)| d);
        bail!(
            "the description {written:?} isn't one this app knows how to match with Standard Bank's CSV export \
             (the PDF can leave words off it). Upload the CSV statement instead, or have this description added."
        );
    };
    let description = if out { paid_out } else { paid_in };
    // The fee marker, which the CSV has in its own column.
    let detail = left.trim_end_matches(['#', ' ']).trim();
    // No detail: the PDF repeats the description instead.
    let detail = if detail.eq_ignore_ascii_case(pdf) || detail == *description {
        ""
    } else {
        detail
    };
    Ok(format!("{description} {detail}"))
}

/// The rows of a statement's text.
pub fn parse_text(text: &str) -> Result<Parsed> {
    ensure!(
        kind(text) == Kind::Current,
        "not a {FORM} {VERSION} statement"
    );
    let account_number = text.lines().find_map(|l| {
        let n: String = l
            .trim()
            .strip_prefix("Account number:")?
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        (!n.is_empty()).then_some(n)
    });

    // Each row starts with its date; a long description wraps onto the next line(s).
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut open = false; // the last row hasn't reached its amounts yet
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some((date, rest)) = row_start(line) {
            ensure!(!open, "the row before {line:?} has no amount");
            open = split_amounts(rest).is_none();
            rows.push((date, rest.to_string()));
        } else if open && !is_page_furniture(line) {
            let (_, text) = rows.last_mut().expect("open only after a row");
            text.push(' ');
            text.push_str(line);
            open = split_amounts(text).is_none();
        }
    }
    ensure!(!open, "the last row has no amount");
    ensure!(!rows.is_empty(), "no transactions found");

    let mut transactions = Vec::with_capacity(rows.len());
    let mut balance: Option<i64> = None;
    for (n, (date, text)) in rows.iter().enumerate() {
        let row = n + 1;
        let (body, amount, after) = split_amounts(text).expect("checked above");
        let (amount_cents, after_cents) = (
            cents(amount).expect("checked"),
            cents(after).expect("checked"),
        );
        if let Some(before) = balance {
            ensure!(
                before + amount_cents == after_cents,
                "row {row} ({date}, {amount}) doesn't take the balance from {} to {after}: a row may have been \
                 read wrongly or missed",
                format_cents(before)
            );
        }
        balance = Some(after_cents);
        let description =
            rebuild(body, amount_cents < 0).with_context(|| format!("row {row} ({date})"))?;
        transactions.push(Transaction {
            id: 0,
            occurrence: 0,
            date: date.clone(),
            description,
            amount: amount.trim_start_matches('+').replace(',', ""),
            currency: "ZAR".into(),
            category: None,
            effective_date: None,
            hand_added: false,
        });
    }
    Ok(Parsed {
        account_number,
        transactions,
        stated_ids: None,
    })
}

fn format_cents(c: i64) -> String {
    format!(
        "{}{}.{:02}",
        if c < 0 { "-" } else { "" },
        c.abs() / 100,
        c.abs() % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statement;

    /// A made-up statement in this layout: two pages, a wrapped row, a fee with no detail.
    const TEXT: &str = "
Customer Care: 0860 123 000
Website: www.standardbank.co.za
25 November 2026

Transaction History Statement

Account holder: MR. A TEST

Account number: 12 34 567 890 1

Product name: MYMO CURRENT ACCOUNT

Transaction date range: 26 Sep 2026 - 25 Nov 2026

Available balance: R 1,000.00

Date Description In (R) Out (R) Bank fees (R) Balance (R)

# These fees are zero rated
## These fees include VAT at the applicable prevailing rate in accordance with the VAT Act
Please verify all transactions reflected on this statement and notify discrepancies to the Bank as soon as possible

PPB00067436 2026-01 The Standard Bank of South Africa Limited (Reg. No. 1962/000738/06)  Page 1 of 2

28 Sep 2026 SBSA HOMEL 536771472 260928 - std bank bond repayment -2,000.00 3,000.00

28 Sep 2026 FEE - DEBIT ORDER ## - fee - debit order -3.50 2,996.50

29 Sep 2026 SHOP 5196*8256 05 OCT - debit card purchase -96.50 2,900.00

30 Sep 2026 MONTHLY MANAGEMENT FEE ## - monthly management
fee -7.50 2,892.50

Customer Care: 0860 123 000
Website: www.standardbank.co.za
25 November 2026

Transaction History Statement

PPB00067436 2026-01 The Standard Bank of South Africa Limited (Reg. No. 1962/000738/06)  Page 2 of 2

01 Oct 2026 FromMe - ib transfer +1,000.00 3,892.50

01 Oct 2026 Rent - ib payment -2,892.50 1,000.00
";

    #[test]
    fn rows_read_as_the_csv_export_has_them() {
        let p = parse_text(TEXT).unwrap();
        assert_eq!(p.account_number.as_deref(), Some("12345678901"));
        let rows: Vec<(&str, &str, &str)> = p
            .transactions
            .iter()
            .map(|t| (t.date.as_str(), t.amount.as_str(), t.description.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "2026-09-28",
                    "-2000.00",
                    "STD BANK BOND REPAYMENT SBSA HOMEL 536771472 260928"
                ),
                ("2026-09-28", "-3.50", "FEE - DEBIT ORDER "),
                (
                    "2026-09-29",
                    "-96.50",
                    "DEBIT CARD PURCHASE FROM SHOP 5196*8256 05 OCT"
                ),
                ("2026-09-30", "-7.50", "MONTHLY MANAGEMENT FEE "),
                ("2026-10-01", "1000.00", "IB TRANSFER FROM FromMe"),
                ("2026-10-01", "-2892.50", "IB PAYMENT TO Rent"),
            ]
        );
    }

    #[test]
    fn a_row_that_breaks_the_running_balance_is_refused() {
        let text = TEXT.replace("-96.50 2,900.00", "-95.50 2,900.00");
        let err = format!("{:#}", parse_text(&text).unwrap_err());
        assert!(err.contains("row 3") && err.contains("balance"), "{err}");
    }

    #[test]
    fn an_unknown_description_is_refused() {
        let text = TEXT.replace("ib transfer", "telepathic transfer");
        let err = format!("{:#}", parse_text(&text).unwrap_err());
        assert!(
            err.contains("telepathic transfer") && err.contains("row 5"),
            "{err}"
        );
    }

    #[test]
    fn other_versions_are_told_apart() {
        assert_eq!(kind(TEXT), Kind::Current);
        assert_eq!(
            kind(&TEXT.replace("PPB00067436 2026-01", "PPB00067436 2027-03")),
            Kind::OtherVersion("PPB00067436 2027-03".into())
        );
        assert_eq!(kind("Some other bank"), Kind::Unknown);
    }

    /// The real statement and its CSV export, if they're on this machine (`statements/` isn't committed):
    /// every row has the same id from both.
    #[test]
    fn the_real_pdf_matches_its_csv_export() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../statements/");
        let (Ok(pdf), Ok(csv)) = (
            std::fs::read(format!("{dir}statement-10-19-769-137-2.pdf")),
            std::fs::read(format!("{dir}statement-10-19-769-137-2.csv")),
        ) else {
            eprintln!("skipped: the real statement isn't here");
            return;
        };
        let from_pdf = statement::parse(&pdf[..]).unwrap();
        let from_csv = statement::parse(&csv[..]).unwrap();
        assert_eq!(from_pdf.account_number, from_csv.account_number);
        let ids =
            |s: &statement::Statement| s.transactions.iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(&from_pdf), ids(&from_csv));
    }
}
