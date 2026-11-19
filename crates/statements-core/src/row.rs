//! Shapes parsed transactions into sheet rows, matching the n8n "Enrich csv" output
//! (`id, month, date, effective_date, amount, description, currency, type`).

use anyhow::{Result, ensure};

use crate::statement::Transaction;

/// A sheet column this app knows how to fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Id,
    Month,
    Date,
    Amount,
    Description,
    Currency,
    /// Category. Left blank in the POC.
    Type,
    /// Always blank (n8n writes `null`).
    EffectiveDate,
}

impl Column {
    /// Header order used when writing to an empty tab.
    pub const DEFAULT_ORDER: [Column; 8] = [
        Column::Id,
        Column::Month,
        Column::Date,
        Column::EffectiveDate,
        Column::Amount,
        Column::Description,
        Column::Currency,
        Column::Type,
    ];

    pub fn header(self) -> &'static str {
        match self {
            Column::Id => "id",
            Column::Month => "month",
            Column::Date => "date",
            Column::Amount => "amount",
            Column::Description => "description",
            Column::Currency => "currency",
            Column::Type => "type",
            Column::EffectiveDate => "effective_date",
        }
    }

    /// Matches a header cell by exact name, ignoring surrounding whitespace.
    fn from_header(header: &str) -> Option<Column> {
        let header = header.trim();
        Self::DEFAULT_ORDER
            .into_iter()
            .find(|c| c.header() == header)
    }

    /// The cell value for this column. Amount and description are written exactly as parsed (both are
    /// part of the id).
    pub fn value(self, t: &Transaction) -> String {
        match self {
            Column::Id => t.id.to_string(),
            Column::Month => month_label(&t.date),
            Column::Date => t.date.clone(),
            Column::Amount => t.amount.clone(),
            Column::Description => t.description.clone(),
            Column::Currency => t.currency.clone(),
            Column::Type => t.category.clone().unwrap_or_default(),
            Column::EffectiveDate => String::new(),
        }
    }

    /// The typed cell written to the sheet: `id` and `amount` as numbers, `date` as a date, the rest as text.
    pub fn cell(self, t: &Transaction) -> Cell {
        match self {
            Column::Id => Cell::Integer(i64::from(t.id)),
            Column::Amount => t
                .amount
                .parse::<f64>()
                .map_or_else(|_| Cell::Text(t.amount.clone()), Cell::Decimal),
            Column::Date => Cell::Date(t.date.clone()),
            other => Cell::Text(other.value(t)),
        }
    }
}

/// A value as written with `valueInputOption=USER_ENTERED` (as if typed into the sheet).
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// Sent as a JSON number, so the sheet's locale (e.g. a comma decimal separator) can't misread it.
    Integer(i64),
    Decimal(f64),
    /// `YYYY-MM-DD`, which Sheets turns into a real date in every locale.
    Date(String),
    /// Kept exactly as written: never parsed as a number, date or formula, and spaces are kept.
    Text(String),
}

impl Cell {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Cell::Integer(n) => serde_json::json!(n),
            Cell::Decimal(n) => serde_json::json!(n),
            Cell::Date(d) => serde_json::json!(d),
            Cell::Text(s) if s.is_empty() => serde_json::json!(""),
            // A leading apostrophe marks text when typed; Sheets doesn't store it.
            Cell::Text(s) => serde_json::json!(format!("'{s}")),
        }
    }
}

/// Where each value goes in a tab: one entry per header cell, `None` for headers this app doesn't fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    columns: Vec<Option<Column>>,
}

impl Layout {
    /// The layout for an empty tab, where the default header row is written first.
    pub fn default_headers() -> Self {
        Self {
            columns: Column::DEFAULT_ORDER.into_iter().map(Some).collect(),
        }
    }

    /// The layout for a tab's existing header row. An empty header row (or one with only blank cells)
    /// means the tab is empty and gets the default layout. A non-empty header row must contain `id`.
    pub fn from_headers<S: AsRef<str>>(headers: &[S]) -> Result<Self> {
        if headers.iter().all(|h| h.as_ref().trim().is_empty()) {
            return Ok(Self::default_headers());
        }
        let columns: Vec<Option<Column>> = headers
            .iter()
            .map(|h| Column::from_header(h.as_ref()))
            .collect();
        ensure!(
            columns.contains(&Some(Column::Id)),
            "the tab has a header row but no `id` column"
        );
        Ok(Self { columns })
    }

    /// Header cells, in sheet order. Unknown headers come back as empty strings.
    pub fn headers(&self) -> Vec<&'static str> {
        self.columns
            .iter()
            .map(|c| c.map_or("", Column::header))
            .collect()
    }

    /// Index of `column` in the tab, if it has one.
    pub fn index_of(&self, column: Column) -> Option<usize> {
        self.columns.iter().position(|c| *c == Some(column))
    }

    /// Index of the `id` column (always present).
    pub fn id_index(&self) -> usize {
        self.columns
            .iter()
            .position(|c| *c == Some(Column::Id))
            .expect("layout always has an id column")
    }

    /// The header row, as written to an empty tab.
    pub fn header_cells(&self) -> Vec<Cell> {
        self.headers()
            .into_iter()
            .map(|h| Cell::Text(h.to_string()))
            .collect()
    }

    /// One typed sheet row for a transaction, with blanks under headers this app doesn't fill.
    pub fn cells(&self, t: &Transaction) -> Vec<Cell> {
        self.columns
            .iter()
            .map(|c| c.map_or_else(|| Cell::Text(String::new()), |c| c.cell(t)))
            .collect()
    }

    /// One sheet row for a transaction, with blanks under headers this app doesn't fill.
    pub fn row(&self, t: &Transaction) -> Vec<String> {
        self.columns
            .iter()
            .map(|c| c.map_or_else(String::new, |c| c.value(t)))
            .collect()
    }
}

/// `YYYY-MM-DD` -> en-US short month and year, e.g. `May 2026` (n8n: `toLocaleDateString('en-US',
/// { month: 'short', year: 'numeric' })`). The date comes from the parser, so it is always well formed.
pub fn month_label(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let year = &date[0..4];
    let month: usize = date[5..7].parse().expect("parser produces YYYY-MM-DD");
    format!("{} {year}", MONTHS[month - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx() -> Transaction {
        Transaction {
            id: -1575916464,
            date: "2026-05-30".into(),
            description: "OVERDRAFT SERVICE FEE ".into(),
            amount: "-57.5".into(),
            currency: "ZAR".into(),
            category: None,
        }
    }

    #[test]
    fn typed_cells_for_user_entered() {
        let cells = Layout::default_headers().cells(&tx());
        let json: Vec<serde_json::Value> = cells.iter().map(Cell::to_json).collect();
        assert_eq!(
            serde_json::Value::Array(json),
            serde_json::json!([
                -1575916464,
                "'May 2026",
                "2026-05-30",
                "",
                -57.5,
                "'OVERDRAFT SERVICE FEE ",
                "'ZAR",
                ""
            ])
        );
    }

    #[test]
    fn text_that_looks_like_a_formula_or_number_stays_text() {
        assert_eq!(Cell::Text("=SUM(A1)".into()).to_json(), "'=SUM(A1)");
        assert_eq!(Cell::Text("-12".into()).to_json(), "'-12");
    }

    #[test]
    fn unparseable_amount_falls_back_to_text() {
        let mut t = tx();
        t.amount = "1,234.00".into();
        assert_eq!(Column::Amount.cell(&t), Cell::Text("1,234.00".into()));
    }

    #[test]
    fn month_labels() {
        assert_eq!(month_label("2026-05-30"), "May 2026");
        assert_eq!(month_label("2026-01-01"), "Jan 2026");
        assert_eq!(month_label("2025-12-31"), "Dec 2025");
    }

    #[test]
    fn default_layout_matches_prod_sheet_order() {
        let layout = Layout::default_headers();
        assert_eq!(
            layout.headers(),
            [
                "id",
                "month",
                "date",
                "effective_date",
                "amount",
                "description",
                "currency",
                "type"
            ]
        );
        assert_eq!(
            layout.row(&tx()),
            [
                "-1575916464",
                "May 2026",
                "2026-05-30",
                "",
                "-57.5",
                "OVERDRAFT SERVICE FEE ",
                "ZAR",
                ""
            ]
        );
    }

    #[test]
    fn existing_headers_map_by_name_and_blank_unknown_columns() {
        let layout =
            Layout::from_headers(&["date", " id ", "notes", "amount", "description"]).unwrap();
        assert_eq!(layout.id_index(), 1);
        assert_eq!(
            layout.row(&tx()),
            [
                "2026-05-30",
                "-1575916464",
                "",
                "-57.5",
                "OVERDRAFT SERVICE FEE "
            ]
        );
    }

    #[test]
    fn empty_header_row_uses_default_layout() {
        assert_eq!(
            Layout::from_headers::<&str>(&[]).unwrap(),
            Layout::default_headers()
        );
        assert_eq!(
            Layout::from_headers(&["", "  "]).unwrap(),
            Layout::default_headers()
        );
    }

    #[test]
    fn headers_without_id_are_rejected() {
        let err = Layout::from_headers(&["date", "amount"]).unwrap_err();
        assert!(err.to_string().contains("no `id` column"));
    }
}
