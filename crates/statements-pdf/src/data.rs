//! What a template gets in `/data.json`. Everything is ready to print: dates and amounts are
//! formatted strings, and totals are worked out here (in cents), so templates only lay them out.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::vars;

/// The statement as the template sees it.
#[derive(Serialize, Debug, Clone)]
pub struct StatementData {
    /// The day it was made, `YYYY-MM-DD` (also Typst's `datetime.today()`).
    pub generated: String,
    /// The day it was made, for printing, e.g. `22 Nov 2026`.
    pub generated_label: String,
    /// Title, subtitle and footer, with their variables filled in.
    pub title: String,
    pub subtitle: String,
    pub footer: String,
    /// The header block: label/value rows, filled in. Rows whose value came out empty are left
    /// out (e.g. `Account number` for an account without one).
    pub fields: Vec<Field>,
    /// Who made it, from what. Every layout prints it, and a template can't change it.
    pub provenance: String,
    pub organisation: String,
    pub account: AccountInfo,
    pub currency: String,
    /// The statement period (`None` only with no rows and no period given).
    pub period: Option<Period>,
    /// What was chosen to print, in words (e.g. `Tax year: 2024/25`, `Type: RENT`). Empty: everything.
    pub selection: Vec<String>,
    pub options: Options,
    /// The logo's asset name (e.g. `logo.png`), if there is one.
    pub logo: Option<String>,
    /// A PDF drawn behind the pages (e.g. the organisation's letterhead), if there is one.
    pub letterhead: Option<Letterhead>,
    /// Of the printed rows (amounts without the currency symbol).
    pub totals: Totals,
    /// The template's summary figures that apply, filled in (balances with the currency symbol).
    pub summary: Vec<SummaryFigure>,
    /// The account's balance at the start and end of the period, with the currency symbol, if
    /// the account has a starting balance.
    pub balances: Option<Balances>,
    pub by_type: Vec<TypeTotal>,
    /// The rows, in one group (`name: None`) or grouped by type (`options.group_by_type`).
    pub groups: Vec<Group>,
    /// Some rows were added by hand rather than coming from a bank statement (they're marked).
    pub has_hand_added: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Field {
    pub label: String,
    pub value: String,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct AccountInfo {
    pub name: String,
    /// The identifier, e.g. the bank account number.
    pub number: Option<String>,
    pub holder: Option<String>,
    pub product: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct Period {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Balances {
    /// With the currency symbol, e.g. `R 40,957.20`.
    pub opening: String,
    pub closing: String,
    /// Without it, for table columns, e.g. `40,957.20`.
    pub opening_amount: String,
    pub closing_amount: String,
    pub opening_negative: bool,
    pub closing_negative: bool,
}

/// How dates print.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DateStyle {
    /// `4 Oct 2026`
    #[default]
    Short,
    /// `04 Oct 2026`
    Padded,
}

#[derive(Serialize, Debug, Clone)]
pub struct Options {
    /// The transaction table's columns, in order. [`build`] leaves out `balance` unless a
    /// running balance can print, and `type` when grouped by type.
    pub columns: Vec<Column>,
    pub group_by_type: bool,
    pub summary: bool,
    /// Set by [`build`]: the `balance` column prints (the account has balances, the rows are the
    /// whole period, in date order, and aren't grouped).
    pub running_balance: bool,
    /// Typst paper name: `a4` or `us-letter`.
    pub paper: String,
    /// Space kept clear at the top and bottom of every page (e.g. for a letterhead), in mm.
    pub margin_top_mm: u32,
    pub margin_bottom_mm: u32,
    pub date_style: DateStyle,
    /// Printed before balances and totals in variables and the summary, e.g. `R`. May be empty.
    pub currency_symbol: String,
    /// Which totals print, and their labels ([`build`] fills in the labels' variables).
    pub totals: TotalsOptions,
}

/// Which totals a statement has, and what they're called. Labels may use `{name}` variables.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct TotalsOptions {
    /// The summary near the top (boxes in the standard layout, a panel in the bank one), in
    /// order. Empty: no summary. Balances are left out when the account has none.
    pub summary: Vec<SummaryItem>,
    /// A heading over the summary (may be empty).
    pub summary_title: String,
    /// A totals row at the end of the transaction table.
    pub table_row: bool,
    pub table_row_label: String,
    /// A subtotal row after each type, when grouped by type.
    pub subtotals: bool,
    pub subtotal_label: String,
    /// The first row of the table when a running balance prints.
    pub opening_row_label: String,
}

impl Default for TotalsOptions {
    fn default() -> Self {
        default_totals("standard")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SummaryItem {
    pub key: SummaryKey,
    pub label: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SummaryKey {
    Count,
    MoneyIn,
    MoneyOut,
    Net,
    OpeningBalance,
    ClosingBalance,
}

/// Every summary figure, with its usual label and what it shows (for the template editor).
pub const SUMMARY_KEYS: [(SummaryKey, &str, &str); 6] = [
    (
        SummaryKey::Count,
        "Transactions",
        "How many rows are printed",
    ),
    (
        SummaryKey::MoneyIn,
        "Money in",
        "Money in, of the printed rows",
    ),
    (
        SummaryKey::MoneyOut,
        "Money out",
        "Money out, of the printed rows (without its minus sign)",
    ),
    (
        SummaryKey::Net,
        "Net",
        "Money in less money out, of the printed rows",
    ),
    (
        SummaryKey::OpeningBalance,
        "Opening balance",
        "The balance at the start of the period (when the bank account has a starting balance)",
    ),
    (
        SummaryKey::ClosingBalance,
        "Closing balance",
        "The balance at the end of the period (when the bank account has a starting balance)",
    ),
];

impl SummaryKey {
    pub fn name(self) -> &'static str {
        match self {
            SummaryKey::Count => "count",
            SummaryKey::MoneyIn => "money_in",
            SummaryKey::MoneyOut => "money_out",
            SummaryKey::Net => "net",
            SummaryKey::OpeningBalance => "opening_balance",
            SummaryKey::ClosingBalance => "closing_balance",
        }
    }

    /// Its usual label.
    pub fn label(self) -> &'static str {
        SUMMARY_KEYS
            .iter()
            .find(|(k, _, _)| *k == self)
            .map_or("", |(_, l, _)| l)
    }
}

/// The totals a new template starts with, for each layout.
pub fn default_totals(layout: &str) -> TotalsOptions {
    use SummaryKey::*;
    let item = |key: SummaryKey, label: &str| SummaryItem {
        key,
        label: label.into(),
    };
    let bank = layout == "bank";
    TotalsOptions {
        summary: if bank {
            vec![
                item(OpeningBalance, "Opening balance"),
                item(MoneyIn, "Money in"),
                item(MoneyOut, "Money out"),
                item(ClosingBalance, "Closing balance"),
                item(Count, "Transactions"),
            ]
        } else {
            vec![
                item(Count, "Transactions"),
                item(MoneyIn, "Money in"),
                item(MoneyOut, "Money out"),
                item(Net, "Net"),
                item(OpeningBalance, "Opening balance, {period.from}"),
                item(ClosingBalance, "Closing balance, {period.to}"),
            ]
        },
        summary_title: if bank { "Summary" } else { "" }.into(),
        table_row: true,
        table_row_label: "Totals".into(),
        subtotals: true,
        subtotal_label: "Subtotal".into(),
        opening_row_label: "Balance brought forward".into(),
    }
}

/// A summary figure, ready to print.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SummaryFigure {
    pub key: &'static str,
    pub label: String,
    pub value: String,
}

/// A column of the transaction table: what it shows, and its heading.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Column {
    pub key: ColumnKey,
    pub label: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKey {
    Date,
    /// The effective date, where it differs from the date.
    EffectiveDate,
    Description,
    Type,
    /// Signed: money in positive, money out negative.
    Amount,
    /// Money out, without its sign.
    Debit,
    /// Money in.
    Credit,
    /// The running balance after the row.
    Balance,
}

/// Every column, with its usual heading and what it shows (for the template editor).
pub const COLUMN_KEYS: [(ColumnKey, &str, &str); 8] = [
    (ColumnKey::Date, "Date", "The transaction date"),
    (
        ColumnKey::EffectiveDate,
        "Effective",
        "The effective date, where it differs from the date",
    ),
    (ColumnKey::Description, "Description", "The description"),
    (
        ColumnKey::Type,
        "Category",
        "The row's category (left out when grouped by category)",
    ),
    (
        ColumnKey::Amount,
        "Amount",
        "Money in positive, money out negative",
    ),
    (
        ColumnKey::Debit,
        "Debit",
        "Money out, without its minus sign",
    ),
    (ColumnKey::Credit, "Credit", "Money in"),
    (
        ColumnKey::Balance,
        "Balance",
        "The balance after each row: only when the bank account has a starting balance and every row in the period is printed",
    ),
];

impl ColumnKey {
    pub fn name(self) -> &'static str {
        match self {
            ColumnKey::Date => "date",
            ColumnKey::EffectiveDate => "effective_date",
            ColumnKey::Description => "description",
            ColumnKey::Type => "type",
            ColumnKey::Amount => "amount",
            ColumnKey::Debit => "debit",
            ColumnKey::Credit => "credit",
            ColumnKey::Balance => "balance",
        }
    }

    /// Its usual heading.
    pub fn label(self) -> &'static str {
        COLUMN_KEYS
            .iter()
            .find(|(k, _, _)| *k == self)
            .map_or("", |(_, l, _)| l)
    }
}

/// The columns a new template starts with, for each layout.
pub fn default_columns(layout: &str) -> Vec<Column> {
    use ColumnKey::*;
    let keys: &[(ColumnKey, &str)] = if layout == "bank" {
        &[
            (Date, "Date"),
            (Description, "Description"),
            (Debit, "Money out"),
            (Credit, "Money in"),
            (Balance, "Balance"),
        ]
    } else {
        &[
            (Date, "Date"),
            (EffectiveDate, "Effective"),
            (Description, "Description"),
            (Type, "Category"),
            (Amount, "Amount"),
        ]
    };
    keys.iter()
        .map(|&(key, label)| Column {
            key,
            label: label.into(),
        })
        .collect()
}

/// Which date decides what's in the period, the balances, and the running balance's order.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DateBasis {
    /// The transaction date.
    #[default]
    Transaction,
    /// The effective date where a row has one, else the transaction date.
    Effective,
}

impl DateBasis {
    /// `date`, or `effective` where it's set and this is [`DateBasis::Effective`].
    pub fn pick<'a>(self, date: &'a str, effective: Option<&'a str>) -> &'a str {
        match (self, effective) {
            (DateBasis::Effective, Some(e)) => e,
            _ => date,
        }
    }
}

/// A PDF asset drawn behind the pages, full size: which of its pages goes behind the first page
/// of the statement, and which behind the rest (1-based; `None` leaves those pages plain).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Letterhead {
    /// The asset's name, e.g. `letterhead.pdf`.
    pub file: String,
    pub first: Option<u32>,
    pub later: Option<u32>,
}

impl Letterhead {
    /// A letterhead of `pages` pages (at least one). Its first page goes behind the statement's
    /// first page. On `every_page`, its second page (or the first again if it has only one) goes
    /// behind the rest, the usual "first page and continuation" letterhead.
    pub fn new(file: String, pages: usize, every_page: bool) -> Self {
        Letterhead {
            file,
            first: Some(1),
            later: every_page.then_some(if pages >= 2 { 2 } else { 1 }),
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Totals {
    pub count: usize,
    pub money_in: String,
    pub money_out: String,
    pub net: String,
    /// Money out without its sign (the debit column's total).
    pub debits: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TypeTotal {
    /// `No type` for rows without one.
    pub name: String,
    pub count: usize,
    pub money_in: String,
    pub money_out: String,
    pub net: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct Group {
    pub name: Option<String>,
    pub rows: Vec<RowData>,
    /// The group's totals, when grouped.
    pub subtotal: Option<Totals>,
}

#[derive(Serialize, Debug, Clone)]
pub struct RowData {
    pub date: String,
    pub effective_date: Option<String>,
    pub description: String,
    #[serde(rename = "type")]
    pub category: Option<String>,
    pub amount: String,
    /// Money out, without its sign; or money in.
    pub debit: Option<String>,
    pub credit: Option<String>,
    pub negative: bool,
    /// The balance after this row (with `options.running_balance`).
    pub balance: Option<String>,
    /// Added by hand (a split row, or a Google Sheet row without an id).
    pub hand_added: bool,
}

/// A row to print, as stored.
#[derive(Debug, Clone)]
pub struct Line {
    /// `YYYY-MM-DD`
    pub date: String,
    pub effective_date: Option<String>,
    pub description: String,
    pub category: Option<String>,
    pub cents: i64,
    pub hand_added: bool,
}

/// The account's balance at the start and end of the period, in cents. `rows_complete`: the
/// printed rows are exactly the account's rows dated in the period, in date order, so a running
/// balance is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerBalances {
    pub opening: i64,
    pub closing: i64,
    pub rows_complete: bool,
}

/// What [`build`] needs besides the rows. Title, subtitle, footer and field values may contain
/// `{name}` variables (see [`crate::vars`]).
#[derive(Debug, Clone)]
pub struct Header {
    /// `YYYY-MM-DD`
    pub generated: String,
    pub title: String,
    pub subtitle: String,
    pub footer: String,
    pub fields: Vec<Field>,
    pub organisation: String,
    pub account: AccountInfo,
    pub currency: String,
    /// The statement period, `YYYY-MM-DD`. `None`: the rows' first and last dates.
    pub period: Option<(String, String)>,
    /// Which date the period goes by (the caller works out the balances on the same basis).
    pub date_basis: DateBasis,
    pub selection: Vec<String>,
    pub options: Options,
    pub logo: Option<String>,
    pub letterhead: Option<Letterhead>,
    pub balances: Option<LedgerBalances>,
}

const NO_TYPE: &str = "No category";
const EVERYTHING: &str = "All transactions in the bank account";

/// The header fields a new template starts with, for each layout.
pub fn default_fields(layout: &str) -> Vec<Field> {
    let f = |label: &str, value: &str| Field {
        label: label.into(),
        value: value.into(),
    };
    if layout == "bank" {
        vec![
            f("Account holder", "{account.holder}"),
            f("Account number", "{account.number}"),
            f("Product name", "{account.product}"),
            f("Transaction date range", "{period.from} - {period.to}"),
            f("Opening balance", "{balance.opening}"),
            f("Available balance", "{balance.closing}"),
        ]
    } else {
        vec![
            f("Organisation", "{org.name}"),
            f("Account", "{account.name}"),
            f("Account number", "{account.number}"),
            f("Period", "{period.from} to {period.to}"),
            f("Selection", "{selection}"),
            f("Currency", "{currency}"),
            f("Generated", "{generated}"),
        ]
    }
}

/// The statement for `lines`, in the order given (grouped by type, groups in order of first
/// appearance, when asked).
pub fn build(header: Header, lines: &[Line]) -> StatementData {
    let style = header.options.date_style;
    let symbol = header.options.currency_symbol.clone();
    let label = |iso: &str| date_label_in(iso, style);
    let money = |cents: i64| format_money(cents, &symbol);

    let period = header
        .period
        .clone()
        .or_else(|| {
            let basis = header.date_basis;
            let dates = || {
                lines
                    .iter()
                    .map(move |l| basis.pick(&l.date, l.effective_date.as_deref()).to_string())
            };
            dates().min().zip(dates().max())
        })
        .map(|(from, to)| Period {
            from: label(&from),
            to: label(&to),
        });

    let mut types: Vec<(String, Vec<&Line>)> = Vec::new();
    for l in lines {
        let name = l.category.clone().unwrap_or_else(|| NO_TYPE.to_string());
        match types.iter_mut().find(|(n, _)| *n == name) {
            Some((_, rows)) => rows.push(l),
            None => types.push((name, vec![l])),
        }
    }
    let by_type = types
        .iter()
        .map(|(name, rows)| {
            let t = totals(rows.iter().copied());
            TypeTotal {
                name: name.clone(),
                count: t.count,
                money_in: t.money_in,
                money_out: t.money_out,
                net: t.net,
            }
        })
        .collect();

    let mut options = header.options;
    options.running_balance = options.columns.iter().any(|c| c.key == ColumnKey::Balance)
        && !options.group_by_type
        && header.balances.is_some_and(|b| b.rows_complete);
    let (running_on, grouped) = (options.running_balance, options.group_by_type);
    options.columns.retain(|c| match c.key {
        ColumnKey::Balance => running_on,
        ColumnKey::Type => !grouped,
        _ => true,
    });
    let mut running = header.balances.map(|b| b.opening).unwrap_or(0);
    let mut row = |l: &Line| {
        running += l.cents;
        RowData {
            date: label(&l.date),
            effective_date: l
                .effective_date
                .as_ref()
                .filter(|e| **e != l.date)
                .map(|e| label(e)),
            description: l.description.trim().to_string(),
            category: l.category.clone(),
            amount: format_cents(l.cents),
            debit: (l.cents < 0).then(|| format_cents(-l.cents)),
            credit: (l.cents >= 0).then(|| format_cents(l.cents)),
            negative: l.cents < 0,
            balance: options.running_balance.then(|| format_cents(running)),
            hand_added: l.hand_added,
        }
    };
    let groups = if options.group_by_type {
        types
            .iter()
            .map(|(name, rows)| Group {
                name: Some(name.clone()),
                rows: rows.iter().map(|l| row(l)).collect(),
                subtotal: Some(totals(rows.iter().copied())),
            })
            .collect()
    } else {
        vec![Group {
            name: None,
            rows: lines.iter().map(row).collect(),
            subtotal: None,
        }]
    };

    let balances = header.balances.map(|b| Balances {
        opening: money(b.opening),
        closing: money(b.closing),
        opening_amount: format_cents(b.opening),
        closing_amount: format_cents(b.closing),
        opening_negative: b.opening < 0,
        closing_negative: b.closing < 0,
    });
    let sums = Sums::of(lines.iter());
    let generated_label = label(&header.generated);

    let vars: HashMap<&str, String> = HashMap::from([
        ("org.name", header.organisation.clone()),
        ("account.name", header.account.name.clone()),
        (
            "account.number",
            header.account.number.clone().unwrap_or_default(),
        ),
        (
            "account.holder",
            header.account.holder.clone().unwrap_or_default(),
        ),
        (
            "account.product",
            header.account.product.clone().unwrap_or_default(),
        ),
        ("currency", header.currency.clone()),
        (
            "period.from",
            period.as_ref().map(|p| p.from.clone()).unwrap_or_default(),
        ),
        (
            "period.to",
            period.as_ref().map(|p| p.to.clone()).unwrap_or_default(),
        ),
        (
            "balance.opening",
            balances
                .as_ref()
                .map(|b| b.opening.clone())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "balance.closing",
            balances
                .as_ref()
                .map(|b| b.closing.clone())
                .unwrap_or_else(|| "—".into()),
        ),
        ("total.in", money(sums.money_in)),
        ("total.out", money(sums.money_out)),
        ("total.net", money(sums.money_in + sums.money_out)),
        ("count", sums.count.to_string()),
        (
            "selection",
            if header.selection.is_empty() {
                EVERYTHING.to_string()
            } else {
                header.selection.join("; ")
            },
        ),
        ("generated", generated_label.clone()),
    ]);
    // Templates are checked when saved; anything that still doesn't fill in prints as typed.
    let fill = |text: &str| vars::fill(text, &vars).unwrap_or_else(|_| text.to_string());
    options.totals.summary_title = fill(&options.totals.summary_title);
    options.totals.table_row_label = fill(&options.totals.table_row_label);
    options.totals.subtotal_label = fill(&options.totals.subtotal_label);
    options.totals.opening_row_label = fill(&options.totals.opening_row_label);
    let summary = options
        .totals
        .summary
        .iter()
        .filter_map(|item| {
            let value = match item.key {
                SummaryKey::Count => sums.count.to_string(),
                SummaryKey::MoneyIn => money(sums.money_in),
                SummaryKey::MoneyOut => money(-sums.money_out),
                SummaryKey::Net => money(sums.money_in + sums.money_out),
                SummaryKey::OpeningBalance => balances.as_ref()?.opening.clone(),
                SummaryKey::ClosingBalance => balances.as_ref()?.closing.clone(),
            };
            Some(SummaryFigure {
                key: item.key.name(),
                label: fill(&item.label),
                value,
            })
        })
        .collect();
    let fields = header
        .fields
        .iter()
        .map(|f| Field {
            label: fill(&f.label),
            value: fill(&f.value).trim().to_string(),
        })
        .filter(|f| !f.value.is_empty())
        .collect();

    StatementData {
        provenance: format!(
            "Prepared by {} from imported bank data · {generated_label}",
            header.organisation
        ),
        generated_label,
        generated: header.generated,
        title: fill(&header.title),
        subtitle: fill(&header.subtitle),
        footer: fill(&header.footer),
        fields,
        organisation: header.organisation,
        account: header.account,
        currency: header.currency,
        period,
        selection: header.selection,
        options,
        logo: header.logo,
        letterhead: header.letterhead,
        totals: totals(lines.iter()),
        summary,
        balances,
        by_type,
        groups,
        has_hand_added: lines.iter().any(|l| l.hand_added),
    }
}

struct Sums {
    count: usize,
    money_in: i64,
    money_out: i64,
}

impl Sums {
    fn of<'a>(lines: impl Iterator<Item = &'a Line>) -> Sums {
        let mut s = Sums {
            count: 0,
            money_in: 0,
            money_out: 0,
        };
        for l in lines {
            s.count += 1;
            if l.cents >= 0 {
                s.money_in += l.cents;
            } else {
                s.money_out += l.cents;
            }
        }
        s
    }
}

fn totals<'a>(lines: impl Iterator<Item = &'a Line>) -> Totals {
    let s = Sums::of(lines);
    Totals {
        count: s.count,
        money_in: format_cents(s.money_in),
        money_out: format_cents(s.money_out),
        net: format_cents(s.money_in + s.money_out),
        debits: format_cents(-s.money_out),
    }
}

/// `-123456789` -> `−1,234,567.89` (a real minus sign).
pub fn format_cents(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    let whole = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!(
        "{}{grouped}.{:02}",
        if cents < 0 { "−" } else { "" },
        abs % 100
    )
}

/// `-4095720`, `R` -> `−R 40,957.20`; no symbol is [`format_cents`].
pub fn format_money(cents: i64, symbol: &str) -> String {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return format_cents(cents);
    }
    format!(
        "{}{symbol} {}",
        if cents < 0 { "−" } else { "" },
        format_cents(cents.abs())
    )
}

/// `2024-03-01` -> `1 Mar 2024` (anything else is returned as is).
pub fn date_label(iso: &str) -> String {
    date_label_in(iso, DateStyle::Short)
}

/// `2024-03-01` -> `1 Mar 2024` or `01 Mar 2024` (anything else is returned as is).
pub fn date_label_in(iso: &str, style: DateStyle) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let parts: Vec<&str> = iso.split('-').collect();
    match parts.as_slice() {
        [y, m, d] if y.len() == 4 => match (m.parse::<usize>(), d.parse::<u32>()) {
            (Ok(m @ 1..=12), Ok(d)) => match style {
                DateStyle::Short => format!("{d} {} {y}", MONTHS[m - 1]),
                DateStyle::Padded => format!("{d:02} {} {y}", MONTHS[m - 1]),
            },
            _ => iso.to_string(),
        },
        _ => iso.to_string(),
    }
}
