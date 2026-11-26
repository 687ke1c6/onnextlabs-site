// The built-in bank-style template: the header fields beside a summary of the balances, then
// the rows in the template's columns (usually money out and money in apart and, when the account
// has a starting balance and every row in the period is printed, the balance after each one).
//
// Everything comes from /data.json (statements-pdf data.rs). The summary by type is the standard
// layout's; this one leaves it out.

#let data = json("/data.json")
#let opt = data.options
#let muted = luma(95)
#let rule = 0.5pt + luma(175)
#let band = rgb("#1f3b57")
#let shade = luma(245)

#set document(title: (if data.title != "" { data.title } else { "Statement" }) + " – " + data.account.name, author: data.organisation)
#set text(font: "Libertinus Serif", size: 9.5pt, lang: "en")

// A letterhead PDF behind the pages, as in the standard layout.
#let letterhead = data.letterhead
#set page(
  paper: opt.paper,
  margin: (x: 16mm, top: opt.margin_top_mm * 1mm, bottom: opt.margin_bottom_mm * 1mm),
  background: if letterhead != none {
    context {
      let p = if here().page() == 1 { letterhead.first } else { letterhead.later }
      if p != none { image("/" + letterhead.file, page: p, width: 100%, height: 100%, fit: "contain") }
    }
  },
  footer: context {
    set text(size: 7.5pt, fill: muted)
    line(length: 100%, stroke: rule)
    v(-4pt)
    if data.footer != "" {
      data.footer
      linebreak()
    }
    grid(
      columns: (1fr, auto),
      column-gutter: 12pt,
      // Every layout prints the provenance line.
      data.provenance,
      [Page #counter(page).display("1 of 1", both: true)],
    )
  },
)

#let amt(s) = text(number-width: "tabular", s)
#let label(s) = text(fill: muted, s)

// ---- header ----

#grid(
  columns: (1fr, auto),
  column-gutter: 12pt,
  align: (left + bottom, right + bottom),
  {
    // The title is optional (e.g. a letterhead already says what this is).
    if data.title != "" { text(size: 17pt, weight: "semibold", data.title) }
    if data.title != "" and data.subtitle != "" { linebreak() }
    if data.subtitle != "" { text(size: 10.5pt, fill: muted, data.subtitle) }
  },
  if data.logo != none { image("/" + data.logo, height: 15mm) },
)
#v(8pt)

// The closing balance (or net) stands out.
#let strong-keys = ("closing_balance", "net")
#let fields-block = block(
  width: 100%,
  inset: 9pt,
  stroke: rule,
  radius: 2pt,
  grid(
    columns: (auto, 1fr),
    column-gutter: 12pt,
    row-gutter: 6pt,
    ..data.fields.map(f => (label(f.label), text(weight: "semibold", f.value))).flatten(),
  ),
)
// The template's summary figures beside the fields (balances come from all the account's rows,
// whatever was printed).
#let summary-block = block(
  width: 100%,
  inset: 9pt,
  fill: shade,
  radius: 2pt,
  {
    if opt.totals.summary_title != "" {
      text(weight: "semibold", opt.totals.summary_title)
      v(2pt)
    }
    grid(
      columns: (1fr, auto),
      row-gutter: 6pt,
      ..data.summary
        .map(f => (
          label(f.label),
          align(right, text(weight: if strong-keys.contains(f.key) { "semibold" } else { "regular" }, amt(f.value))),
        ))
        .flatten(),
    )
  },
)
// Either can be empty (no header fields set, or no summary figures): an empty one isn't drawn.
#let has-fields = data.fields.len() > 0
#let has-summary = data.summary.len() > 0
#if has-fields and has-summary {
  grid(columns: (1.35fr, 1fr), column-gutter: 10pt, fields-block, summary-block)
} else if has-fields {
  fields-block
} else if has-summary {
  // On the right, where it sits beside the fields.
  grid(columns: (1.35fr, 1fr), column-gutter: 10pt, [], summary-block)
}

// ---- transactions ----

#v(12pt)
// The template's columns, in order (data.rs leaves out balance unless a running balance can
// print, and type when grouped).
#let cols = opt.columns
#let tot = opt.totals
#let is-num(c) = ("amount", "debit", "credit", "balance").contains(c.key)
// Where a row's label (Opening balance, Subtotal, Totals) goes: the description column, else the
// first column that isn't a figure.
#let label-at = {
  let i = cols.position(c => c.key == "description")
  if i == none { cols.position(c => not is-num(c)) } else { i }
}
#let cell(r, key) = {
  let maybe(v) = if v != none { amt(v) } else { [] }
  if key == "date" { r.date }
  else if key == "effective_date" { if r.effective_date != none { r.effective_date } else { [] } }
  else if key == "description" { [#r.description#if r.hand_added [~\*]] }
  else if key == "type" { if r.type != none { r.type } else { label[—] } }
  else if key == "amount" { amt(r.amount) }
  else if key == "debit" { maybe(r.debit) }
  else if key == "credit" { maybe(r.credit) }
  else if key == "balance" { maybe(r.balance) }
}
// A row of figures under a label; `figures` maps column keys to amounts.
#let summary-line(name, figures, strong: false) = cols.enumerate().map(((i, c)) => {
  let body = if i == label-at { name } else if c.key in figures { amt(figures.at(c.key)) } else { [] }
  if strong { text(weight: "semibold", body) } else { body }
})
#let total-figures(t) = (amount: t.net, debit: t.debits, credit: t.money_in)
#let widths = cols.map(c => if c.key == "description" { 1fr } else { auto })
#let body-rows = data.groups.map(g => {
    let out = ()
    if g.name != none {
      out.push(table.cell(colspan: cols.len(), inset: (x: 5pt, top: 8pt, bottom: 3pt), text(weight: "semibold", g.name)))
    }
    for r in g.rows {
      out += cols.map(c => cell(r, c.key))
    }
    if g.subtotal != none and tot.subtotals {
      out += summary-line(label(tot.subtotal_label), total-figures(g.subtotal))
    }
    out
}).flatten()
#let opening-line = if opt.running_balance {
  summary-line(label(tot.opening_row_label), (balance: data.balances.opening_amount))
}
#let totals-line = if tot.table_row { summary-line(
  tot.table_row_label,
  total-figures(data.totals) + if opt.running_balance { (balance: data.balances.closing_amount) } else { (:) },
  strong: true,
) }

#table(
  columns: widths,
  stroke: none,
  inset: (x: 5pt, y: 4pt),
  align: (x, y) => if is-num(cols.at(x)) { right } else { left },
  fill: (x, y) => if y == 0 { band } else if calc.even(y) { shade },
  table.header(..cols.map(c => text(fill: white, weight: "semibold", c.label))),
  ..opening-line,
  ..body-rows,
  ..if totals-line != none { (table.hline(stroke: rule), ..totals-line) },
)

#if data.has_hand_added {
  v(4pt)
  text(size: 8pt, fill: muted)[\* Added by hand (e.g. part of a split), not from a bank statement.]
}
