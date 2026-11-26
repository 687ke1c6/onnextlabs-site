use super::*;

fn line(date: &str, description: &str, category: Option<&str>, cents: i64) -> Line {
    Line {
        date: date.into(),
        effective_date: None,
        description: description.into(),
        category: category.map(Into::into),
        cents,
        hand_added: false,
    }
}

fn header(group_by_type: bool) -> Header {
    Header {
        generated: "2026-11-22".into(),
        title: "Statement".into(),
        subtitle: "Tax year 2024/25".into(),
        footer: "Prepared for SARS".into(),
        fields: default_fields("standard"),
        organisation: "Acme Holdings".into(),
        account: AccountInfo {
            name: "House Account".into(),
            number: Some("123456789".into()),
            holder: Some("Mr J Smith".into()),
            product: None,
        },
        currency: "ZAR".into(),
        period: None,
        date_basis: DateBasis::Transaction,
        selection: vec!["Tax year: 2024/25".into()],
        options: Options {
            columns: default_columns("standard"),
            group_by_type,
            summary: true,
            running_balance: false,
            paper: "a4".into(),
            margin_top_mm: 18,
            margin_bottom_mm: 22,
            date_style: DateStyle::Short,
            currency_symbol: String::new(),
            totals: default_totals("standard"),
        },
        logo: None,
        letterhead: None,
        balances: None,
    }
}

/// A bank-style header: padded dates, `R`, a stated period and balances.
fn bank_header(rows_complete: bool) -> Header {
    let mut h = header(false);
    h.fields = default_fields("bank");
    h.title = "{account.product}".into();
    h.account.product = Some("Current Account".into());
    h.period = Some(("2024-02-01".into(), "2024-07-31".into()));
    h.options.date_style = DateStyle::Padded;
    h.options.currency_symbol = "R".into();
    h.options.columns = default_columns("bank");
    h.options.totals = default_totals("bank");
    h.balances = Some(LedgerBalances {
        opening: 1_000_000,
        closing: 1_000_000 - 1_682_388,
        rows_complete,
    });
    h
}

/// The rows in date order (as the server prints them with a running balance).
fn lines_by_date() -> Vec<Line> {
    let mut rows = lines();
    rows.sort_by(|a, b| a.date.cmp(&b.date));
    rows
}

/// Real-looking rows, including the awkward bits: an effective date, a split pair (hand-added),
/// no type, a long description, markup-like text, and non-ASCII.
pub(crate) fn lines() -> Vec<Line> {
    let mut bond = line(
        "2024-03-01",
        "STD BANK BOND REPAYMENT SBSA HOMEL 536771472 240301",
        Some("BOND REPAYMENT"),
        -4_262_208,
    );
    bond.effective_date = Some("2024-02-29".into());
    let mut split_out = line("2024-06-02", "(Split) TENANT RENT", Some("RENT"), -472_616);
    split_out.hand_added = true;
    let mut split_in = line("2024-06-02", "TENANT RENT", Some("UTILITIES"), 472_616);
    split_in.hand_added = true;
    let mut rows = vec![
        bond,
        line(
            "2024-06-01",
            "REAL TIME TRANSFER FROM 29STMICHAELS 10011835146",
            Some("RENT"),
            3_800_000,
        ),
        split_out,
        split_in,
        line(
            "2024-06-03",
            "MONTHLY MANAGEMENT FEE ##",
            Some("FEES"),
            -695,
        ),
        line(
            "2024-06-04",
            "Café ☕ #notmarkup *not bold* $x$ <tag> @ref",
            None,
            -12_345,
        ),
    ];
    for i in 0..120 {
        rows.push(line(
            "2024-07-01",
            &format!("DEBIT CARD PURCHASE {i}"),
            Some("EXP"),
            -10_000 - i,
        ));
    }
    rows
}

#[test]
fn totals_and_groups() {
    let d = build(header(true), &lines());
    assert_eq!(
        d.totals,
        Totals {
            count: 126,
            money_in: "42,726.16".into(),
            money_out: "−59,550.04".into(),
            net: "−16,823.88".into(),
            debits: "59,550.04".into(),
        }
    );
    let names: Vec<&str> = d.by_type.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "BOND REPAYMENT",
            "RENT",
            "UTILITIES",
            "FEES",
            "No category",
            "EXP"
        ]
    );
    assert_eq!(d.groups.len(), 6);
    assert_eq!(d.groups[1].subtotal.as_ref().unwrap().net, "33,273.84");
    let p = d.period.unwrap();
    // By transaction date, not the effective date (29 Feb).
    assert_eq!(
        (p.from.as_str(), p.to.as_str()),
        ("1 Mar 2024", "1 Jul 2024")
    );
    // The effective date is shown only when it differs.
    assert_eq!(
        d.groups[0].rows[0].effective_date.as_deref(),
        Some("29 Feb 2024")
    );
    assert!(d.has_hand_added);
}

#[test]
fn formats() {
    assert_eq!(format_cents(0), "0.00");
    assert_eq!(format_cents(-5), "−0.05");
    assert_eq!(format_cents(123_456_789), "1,234,567.89");
    assert_eq!(format_cents(100_000), "1,000.00");
    assert_eq!(date_label("2024-03-01"), "1 Mar 2024");
    assert_eq!(date_label("nope"), "nope");
}

#[test]
fn renders_the_default_template_grouped_and_not() {
    for grouped in [false, true] {
        let pdf = render(DEFAULT_TEMPLATE, &build(header(grouped), &lines()), vec![]).unwrap();
        assert!(pdf.starts_with(b"%PDF-"), "not a PDF");
        assert!(pdf.len() > 10_000);
    }
    // With balances and a running balance.
    let pdf = render(
        DEFAULT_TEMPLATE,
        &build(bank_header(true), &lines_by_date()),
        vec![],
    )
    .unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
}

#[test]
fn renders_summary_without_header_fields() {
    // The bank layout puts the fields and the summary side by side: either may be missing.
    let mut h = bank_header(true);
    h.fields.clear();
    let data = build(h, &lines_by_date());
    assert!(data.fields.is_empty() && !data.summary.is_empty());
    for layout in [DEFAULT_TEMPLATE, BANK_TEMPLATE] {
        let pdf = render(layout, &data, vec![]).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        if let Ok(dir) = std::env::var("PREVIEW_DIR") {
            let name = if layout == BANK_TEMPLATE { "bank" } else { "standard" };
            std::fs::write(format!("{dir}/no-fields-{name}.pdf"), pdf).unwrap();
        }
    }
}

#[test]
fn renders_with_no_rows_and_no_options() {
    let mut h = header(false);
    h.options = Options {
        columns: vec![Column {
            key: ColumnKey::Amount,
            label: "Only the amount".into(),
        }],
        group_by_type: false,
        summary: false,
        running_balance: true,
        paper: "us-letter".into(),
        margin_top_mm: 10,
        margin_bottom_mm: 10,
        date_style: DateStyle::Short,
        currency_symbol: String::new(),
        totals: TotalsOptions {
            summary: vec![],
            table_row: false,
            ..TotalsOptions::default()
        },
    };
    // No title or subtitle either (a letterhead can say what the document is).
    h.title = String::new();
    h.subtitle = String::new();
    h.fields.clear();
    h.account.number = None;
    h.selection.clear();
    let data = build(h, &[]);
    // No balances, so no running balance; no rows, so no period.
    assert!(!data.options.running_balance);
    assert!(data.period.is_none());
    for layout in [DEFAULT_TEMPLATE, BANK_TEMPLATE] {
        let pdf = render(layout, &data, vec![]).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }
}

#[test]
fn fields_fill_in_and_empty_ones_are_left_out() {
    let d = build(header(false), &lines());
    let labels: Vec<&str> = d.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Organisation",
            "Account",
            "Account number",
            "Period",
            "Selection",
            "Currency",
            "Generated"
        ]
    );
    assert_eq!(d.fields[3].value, "1 Mar 2024 to 1 Jul 2024");

    let mut h = header(false);
    h.account.number = None;
    h.selection.clear();
    h.title = "{{Draft}} for {account.holder}".into();
    let d = build(h, &[]);
    assert!(!d.fields.iter().any(|f| f.label == "Account number"));
    assert_eq!(
        d.fields
            .iter()
            .find(|f| f.label == "Selection")
            .unwrap()
            .value,
        "All transactions in the bank account"
    );
    assert_eq!(d.title, "{Draft} for Mr J Smith");
    assert!(
        d.provenance
            .starts_with("Prepared by Acme Holdings from imported bank data")
    );
}

#[test]
fn balances_formats_and_running_balance() {
    let d = build(bank_header(true), &lines_by_date());
    assert_eq!(d.title, "Current Account");
    let value = |label: &str| {
        d.fields
            .iter()
            .find(|f| f.label == label)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(value("Transaction date range"), "01 Feb 2024 - 31 Jul 2024");
    assert_eq!(value("Opening balance"), "R 10,000.00");
    assert_eq!(value("Available balance"), "−R 6,823.88");
    assert!(d.options.running_balance);
    // The last running balance is the closing balance.
    let last = d.groups[0].rows.last().unwrap();
    assert_eq!(last.balance.as_deref(), Some("−6,823.88"));
    assert_eq!(d.balances.as_ref().unwrap().closing_amount, "−6,823.88");

    // Not every row in the period: no running balance, but the balances still print.
    let d = build(bank_header(false), &lines_by_date()[1..]);
    assert!(!d.options.running_balance);
    assert!(d.groups[0].rows.iter().all(|r| r.balance.is_none()));
    assert_eq!(d.balances.unwrap().opening, "R 10,000.00");

    // Grouped rows aren't in date order: no running balance either (and no balance column).
    let mut h = bank_header(true);
    h.options.group_by_type = true;
    let d = build(h, &lines_by_date());
    assert!(!d.options.running_balance);
    assert!(
        !d.options
            .columns
            .iter()
            .any(|c| c.key == ColumnKey::Balance)
    );

    // Debit and credit split the amount; debits print without their sign.
    let d = build(bank_header(true), &lines_by_date());
    let bond = &d.groups[0].rows[0];
    assert_eq!(
        (bond.debit.as_deref(), bond.credit.as_deref()),
        (Some("42,622.08"), None)
    );
    let rent = &d.groups[0].rows[1];
    assert_eq!(
        (rent.debit.as_deref(), rent.credit.as_deref()),
        (None, Some("38,000.00"))
    );
    assert_eq!(d.totals.debits, "59,550.04");

    assert_eq!(format_money(-4_095_720, "R"), "−R 40,957.20");
    assert_eq!(format_money(5, " "), "0.05");
    assert_eq!(
        date_label_in("2026-10-04", DateStyle::Padded),
        "04 Oct 2026"
    );
}

#[test]
fn columns_are_dropped_when_they_cannot_print() {
    let mut h = header(true);
    h.options.columns.push(Column {
        key: ColumnKey::Balance,
        label: "Saldo".into(),
    });
    let d = build(h, &lines());
    let keys: Vec<ColumnKey> = d.options.columns.iter().map(|c| c.key).collect();
    // Grouped: no type column; no balances: no balance column.
    use ColumnKey::*;
    assert_eq!(keys, [Date, EffectiveDate, Description, Amount]);
    // Every layout renders any set of columns, grouped or not.
    let mut h = bank_header(true);
    h.options.columns = COLUMN_KEYS
        .iter()
        .map(|&(key, label, _)| Column {
            key,
            label: label.into(),
        })
        .collect();
    for layout in [DEFAULT_TEMPLATE, BANK_TEMPLATE] {
        for grouped in [false, true] {
            let mut h = h.clone();
            h.options.group_by_type = grouped;
            assert!(render(layout, &build(h, &lines_by_date()), vec![]).is_ok());
        }
    }
}

#[test]
fn the_period_can_go_by_effective_date() {
    let mut h = header(false);
    h.date_basis = DateBasis::Effective;
    let p = build(h, &lines()).period.unwrap();
    assert_eq!(
        (p.from.as_str(), p.to.as_str()),
        ("29 Feb 2024", "1 Jul 2024")
    );
    assert_eq!(
        DateBasis::Effective.pick("2024-03-01", Some("2024-02-29")),
        "2024-02-29"
    );
    assert_eq!(
        DateBasis::Transaction.pick("2024-03-01", Some("2024-02-29")),
        "2024-03-01"
    );
    assert_eq!(DateBasis::Effective.pick("2024-03-01", None), "2024-03-01");
}

#[test]
fn summary_figures_follow_the_template() {
    // Without balances, the balance figures drop out; labels fill in.
    let d = build(header(false), &lines());
    let keys: Vec<&str> = d.summary.iter().map(|f| f.key).collect();
    assert_eq!(keys, ["count", "money_in", "money_out", "net"]);
    assert_eq!(d.summary[2].value, "59,550.04");

    let mut h = bank_header(true);
    h.options.totals.summary = vec![
        SummaryItem {
            key: SummaryKey::ClosingBalance,
            label: "Balance on {period.to}".into(),
        },
        SummaryItem {
            key: SummaryKey::MoneyOut,
            label: "Spent".into(),
        },
    ];
    h.options.totals.table_row_label = "Total for {account.name}".into();
    let d = build(h, &lines_by_date());
    assert_eq!(
        d.summary,
        [
            SummaryFigure {
                key: "closing_balance",
                label: "Balance on 31 Jul 2024".into(),
                value: "−R 6,823.88".into(),
            },
            SummaryFigure {
                key: "money_out",
                label: "Spent".into(),
                value: "R 59,550.04".into(),
            },
        ]
    );
    assert_eq!(d.options.totals.table_row_label, "Total for House Account");

    // Every layout renders with no summary, no totals row and no subtotals.
    for layout in [DEFAULT_TEMPLATE, BANK_TEMPLATE] {
        for grouped in [false, true] {
            let mut h = bank_header(true);
            h.options.group_by_type = grouped;
            h.options.totals = TotalsOptions {
                summary: vec![],
                table_row: false,
                subtotals: false,
                ..TotalsOptions::default()
            };
            assert!(render(layout, &build(h, &lines_by_date()), vec![]).is_ok());
        }
    }
}

#[test]
fn every_layout_prints_the_provenance() {
    for (name, _) in LAYOUTS {
        assert!(layout_source(name).contains("data.provenance"), "{name}");
    }
}

#[test]
fn renders_the_bank_layout() {
    for complete in [true, false] {
        let pdf = render(
            BANK_TEMPLATE,
            &build(bank_header(complete), &lines_by_date()),
            vec![],
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }
    // Grouped data is flattened.
    let pdf = render(BANK_TEMPLATE, &build(header(true), &lines()), vec![]).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
}

#[test]
fn template_errors_name_the_line() {
    let err = render("#let x = 1\n#nope", &build(header(false), &[]), vec![]).unwrap_err();
    assert!(
        err.to_string().contains("line 2: unknown variable: nope"),
        "{err}"
    );
    assert!(
        render(
            "x",
            &build(header(false), &[]),
            vec![Asset {
                name: "../x".into(),
                bytes: vec![]
            }]
        )
        .is_err()
    );
}

/// A two-page A4 "letterhead": page 1 has a dark band across the top, page 2 a light one.
fn letterhead_pdf() -> Vec<u8> {
    render(
        r##"#set page(paper: "a4", margin: 0pt)
#rect(width: 100%, height: 30mm, fill: rgb("#1e3a5f"))
#pagebreak()
#rect(width: 100%, height: 30mm, fill: rgb("#c8d6e5"))"##,
        &build(header(false), &[]),
        vec![],
    )
    .unwrap()
}

/// The colour of the statement's `page` (0-based) near its top-left corner.
fn corner(data: &StatementData, assets: Vec<Asset>, page: usize) -> [u8; 3] {
    let doc = pages(DEFAULT_TEMPLATE, data, assets).unwrap();
    let pixmap = typst_render::render(&doc.pages()[page], &typst_render::RenderOptions::default());
    let px = pixmap.pixel(5, 5).unwrap().demultiply();
    [px.red(), px.green(), px.blue()]
}

#[test]
fn letterhead_pages() {
    assert_eq!(pdf_page_count(&letterhead_pdf()), Ok(2));
    assert!(pdf_page_count(b"not a pdf").is_err());
    let lh = |pages, every| Letterhead::new("letterhead.pdf".into(), pages, every);
    assert_eq!((lh(2, true).first, lh(2, true).later), (Some(1), Some(2)));
    assert_eq!(lh(1, true).later, Some(1));
    assert_eq!(lh(3, false).later, None);
}

#[test]
fn letterhead_goes_behind_the_pages() {
    let asset = || {
        vec![Asset {
            name: "letterhead.pdf".into(),
            bytes: letterhead_pdf(),
        }]
    };
    let with = |every_page| {
        let mut h = header(false);
        h.options.margin_top_mm = 40;
        h.letterhead = Some(Letterhead::new("letterhead.pdf".into(), 2, every_page));
        build(h, &lines())
    };
    let white = [255, 255, 255];
    let plain = build(header(false), &lines());
    assert_eq!(corner(&plain, vec![], 0), white);

    let every = with(true);
    assert_eq!(corner(&every, asset(), 0), [0x1e, 0x3a, 0x5f]);
    assert_eq!(corner(&every, asset(), 1), [0xc8, 0xd6, 0xe5]);

    let first_only = with(false);
    assert_eq!(corner(&first_only, asset(), 0), [0x1e, 0x3a, 0x5f]);
    assert_eq!(corner(&first_only, asset(), 1), white);

    // And it makes a real PDF.
    let pdf = render(DEFAULT_TEMPLATE, &every, asset()).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
}

/// Writes every page of the default template (grouped and on a letterhead, and not) and of the
/// bank layout as PNGs to `PREVIEW_DIR`, to look
/// at: `PREVIEW_DIR=/tmp/x cargo test -p statements-pdf preview -- --ignored`.
#[test]
#[ignore]
fn preview() {
    let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
    let logo = br##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40"><rect width="120" height="40" rx="6" fill="#1e3a5f"/><text x="60" y="26" font-size="16" text-anchor="middle" fill="white" font-family="sans-serif">ACME</text></svg>"##;
    for grouped in [false, true] {
        let mut h = header(grouped);
        h.logo = Some("logo.svg".into());
        let mut assets = vec![Asset {
            name: "logo.svg".into(),
            bytes: logo.to_vec(),
        }];
        // The grouped one on a letterhead.
        if grouped {
            h.options.margin_top_mm = 40;
            h.letterhead = Some(Letterhead::new("letterhead.pdf".into(), 2, true));
            assets.push(Asset {
                name: "letterhead.pdf".into(),
                bytes: letterhead_pdf(),
            });
        }
        let doc = pages(DEFAULT_TEMPLATE, &build(h, &lines()), assets).unwrap();
        write_pages(&dir, if grouped { "grouped" } else { "flat" }, &doc);
    }
    let doc = pages(
        BANK_TEMPLATE,
        &build(bank_header(true), &lines_by_date()),
        vec![],
    )
    .unwrap();
    write_pages(&dir, "bank", &doc);
}

fn write_pages(dir: &str, name: &str, doc: &PagedDocument) {
    for (i, page) in doc.pages().iter().enumerate() {
        let png = typst_render::render(page, &typst_render::RenderOptions::default())
            .encode_png()
            .unwrap();
        std::fs::write(format!("{dir}/{name}-{}.png", i + 1), png).unwrap();
    }
}
