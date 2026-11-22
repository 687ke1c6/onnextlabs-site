//! Standard Bank CSV statements, matching the n8n "SB_CSV" path: Determine Statement Type -> Parse SB Csv.
//! Ids are assigned afterwards, the same way for every format (`statement.rs`).

use anyhow::{Context, Result, ensure};

use crate::statement::{Parsed, Transaction, to_iso_date};

/// n8n: `firstRow['0'] === '0'` identifies a Standard Bank CSV.
pub fn matches(rows: &[csv::StringRecord]) -> bool {
    rows.first().and_then(|r| r.get(0)) == Some("0")
}

/// Rows of a Standard Bank CSV: BRANCH, ACC-NO (account number in column 1), OPEN, transactions, CLOSE.
pub fn parse(rows: &[csv::StringRecord]) -> Result<Parsed> {
    ensure!(
        rows.len() >= 4,
        "Standard Bank CSV too short ({} rows)",
        rows.len()
    );
    let account_number = rows[1]
        .get(1)
        .filter(|n| !n.trim().is_empty())
        .map(str::to_string);

    // n8n: `rows.slice(3, -1).filter(r => !!r['1'])`: skip BRANCH/ACC-NO/OPEN and the CLOSE row.
    let mut transactions = Vec::new();
    for (index, row) in rows[3..rows.len() - 1].iter().enumerate() {
        let line = index + 4; // 1-based line number in the file
        let Some(yyyymmdd) = row.get(1).filter(|s| !s.is_empty()) else {
            continue;
        };
        let date =
            to_iso_date(yyyymmdd).with_context(|| format!("line {line}: bad date {yyyymmdd:?}"))?;
        // A missing col3/col4 would become the string "undefined" in n8n; refuse instead.
        let amount = row
            .get(3)
            .with_context(|| format!("line {line}: missing amount column"))?;
        let desc_main = row
            .get(4)
            .with_context(|| format!("line {line}: missing description column"))?;
        // n8n: `${row['4']} ${row['5'] ?? ''}`, which keeps the trailing space when col5 is empty.
        let description = format!("{desc_main} {}", row.get(5).unwrap_or(""));

        transactions.push(Transaction {
            id: 0,
            occurrence: 0,
            date,
            description,
            amount: amount.to_string(),
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

    /// Parses `csv` and checks every row against `expected_json`, the output of the real n8n workflow code
    /// (`just oracle <csv>`).
    fn assert_matches_n8n(csv: &[u8], expected_json: &str, account: &str, label: &str) {
        let expected: Vec<N8nRow> = serde_json::from_str(expected_json).unwrap();
        let statement = statement::parse(csv).unwrap();

        assert_eq!(statement.format, Format::SbCsv, "{label}");
        assert_eq!(
            statement.account_number.as_deref(),
            Some(account),
            "{label}"
        );
        assert_eq!(statement.transactions.len(), expected.len(), "{label}");
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
                "{label}: transaction {i}"
            );
        }
    }

    /// Made-up statement covering trailing spaces, same-day duplicates, a blank body row, month/year/leap-day
    /// boundaries and non-ASCII text.
    #[test]
    fn synthetic_csv_matches_n8n_output() {
        assert_matches_n8n(
            include_bytes!("../tests/fixtures/sb_csv_synthetic.csv"),
            include_str!("../tests/fixtures/sb_csv_synthetic.n8n.json"),
            "123456789",
            "synthetic",
        );
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
        assert!(matches(&rows("0,5009,BRANCH,0,,X,0,0\n")));
        assert!(!matches(&rows("Type,Product\n")));
        assert!(!matches(&[]));
    }

    #[test]
    fn blank_account_number_is_none() {
        let parsed = parse(&rows(
            "0,1,BRANCH,0,,X,0,0\n, ,ACC-NO,0,,,0,0\n,0,OPEN,1,OPEN BALANCE,,0,0\n,0,CLOSE,1,CLOSE BALANCE,,0,0",
        ))
        .unwrap();
        assert_eq!(parsed.account_number, None);
        assert!(parsed.transactions.is_empty());
    }

    #[test]
    fn rejects_invalid_dates() {
        assert!(to_iso_date("20260231").is_err());
        assert!(to_iso_date("2026021").is_err());
        assert_eq!(to_iso_date("20240229").unwrap(), "2024-02-29");
    }
}
