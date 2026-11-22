//! Standard Bank PDF statements already converted to CSV: a separate tool has scraped the PDF into a CSV with
//! the header `description,amount,date,balance`. (Not the PDF itself; that will be its own format.) Matches the n8n "SB_PDF" path: Determine Statement Type -> Parse SB_PDF csv.
//! Ids are assigned afterwards, the same way for every format (`statement.rs`).
//!
//! Amounts keep their thousands separators (`-5,009.49`), exactly as n8n hashed them; the balance column is not
//! used. The file doesn't state an account number.

use anyhow::{Context, Result};

use crate::statement::{Parsed, Transaction, to_iso_date};

/// n8n: `firstRow['0'] === 'description'`. Also requires the next two header cells, since they are the columns
/// the parser reads.
pub fn matches(rows: &[csv::StringRecord]) -> bool {
    rows.first().is_some_and(|header| {
        header.get(0) == Some("description")
            && header.get(1) == Some("amount")
            && header.get(2) == Some("date")
    })
}

/// A header row, then one row per transaction: description, amount, date (`YYYYMMDD`), balance.
pub fn parse(rows: &[csv::StringRecord]) -> Result<Parsed> {
    // n8n: `statement.slice(1)`: everything after the header.
    let mut transactions = Vec::new();
    for (index, row) in rows.iter().enumerate().skip(1) {
        let line = index + 1; // 1-based line number in the file
        // A missing column would become the string "undefined" (or an invalid date) in n8n; refuse instead.
        let description = row
            .get(0)
            .with_context(|| format!("line {line}: missing description column"))?;
        let amount = row
            .get(1)
            .filter(|a| !a.trim().is_empty())
            .with_context(|| format!("line {line}: missing amount"))?;
        let yyyymmdd = row
            .get(2)
            .with_context(|| format!("line {line}: missing date column"))?;
        let date =
            to_iso_date(yyyymmdd).with_context(|| format!("line {line}: bad date {yyyymmdd:?}"))?;

        transactions.push(Transaction {
            id: 0,
            occurrence: 0,
            date,
            description: description.to_string(),
            amount: amount.to_string(),
            currency: "ZAR".into(),
            category: None,
            effective_date: None,
            hand_added: false,
        });
    }
    Ok(Parsed {
        account_number: None,
        transactions,
        stated_ids: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statement::{self, Format};

    #[derive(serde::Deserialize)]
    struct N8nRow {
        id: i32,
        month: String,
        date: String,
        description: String,
        amount: String,
        currency: String,
    }

    /// Made-up statement covering thousands separators, quoted commas in descriptions, a trailing space,
    /// same-day duplicates, month/year/leap-day boundaries and non-ASCII text. The expected output comes from
    /// running the real n8n node code (Determine Statement Type -> Parse SB_PDF csv -> Enrich) over it.
    #[test]
    fn synthetic_csv_matches_n8n_output() {
        let expected: Vec<N8nRow> = serde_json::from_str(include_str!(
            "../tests/fixtures/sb_pdf_csv_synthetic.n8n.json"
        ))
        .unwrap();
        let statement =
            statement::parse(&include_bytes!("../tests/fixtures/sb_pdf_csv_synthetic.csv")[..])
                .unwrap();

        assert_eq!(statement.format, Format::SbPdfCsv);
        assert_eq!(statement.account_number, None);
        assert_eq!(statement.transactions.len(), expected.len());
        // n8n's ids and `_`-suffixed descriptions, reproduced; the rows themselves keep the bare description.
        let n8n = statement::n8n_ids(&statement.transactions);
        for (i, (got, want)) in statement.transactions.iter().zip(&expected).enumerate() {
            let (n8n_id, n8n_description) = &n8n[i];
            assert_eq!(
                want.description.trim_end_matches('_'),
                got.description.trim_end_matches('_'),
                "transaction {i}"
            );
            assert_eq!(
                (
                    *n8n_id,
                    crate::row::month_label(&got.date),
                    got.date.as_str(),
                    n8n_description.as_str(),
                    got.amount.as_str(),
                    got.currency.as_str()
                ),
                (
                    want.id,
                    want.month.clone(),
                    want.date.as_str(),
                    want.description.as_str(),
                    want.amount.as_str(),
                    want.currency.as_str()
                ),
                "transaction {i}"
            );
        }
    }

    fn rows(csv: &str) -> Vec<csv::StringRecord> {
        csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(csv.as_bytes())
            .records()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn detection() {
        assert!(matches(&rows("description,amount,date,balance\n")));
        assert!(matches(&rows("description,amount,date\n")));
        assert!(!matches(&rows("description,value,when\n")));
        assert!(!matches(&rows("0,5009,BRANCH,0,,X,0,0\n")));
        assert!(!matches(&[]));
    }

    #[test]
    fn header_only_has_no_transactions() {
        let parsed = parse(&rows("description,amount,date,balance\n")).unwrap();
        assert!(parsed.transactions.is_empty());
    }

    #[test]
    fn rejects_malformed_rows() {
        let err = |csv: &str| format!("{:#}", parse(&rows(csv)).unwrap_err());
        assert!(
            err("description,amount,date,balance\nFEE,-1.25,20241325,0\n")
                .contains("line 2: bad date")
        );
        assert!(
            err("description,amount,date,balance\nFEE,,20241025,0\n")
                .contains("line 2: missing amount")
        );
        assert!(
            err("description,amount,date,balance\nFEE,-1.25\n").contains("line 2: missing date")
        );
    }
}
