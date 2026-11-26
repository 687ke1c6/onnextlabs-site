// The built-in statement template. Everything comes from /data.json (statements-pdf data.rs):
// strings are ready to print, and totals are already worked out.
//
// To make your own, start from a copy of this file: keep the `data` fields you need and change
// the layout freely.

#let data = json("/data.json")
#let opt = data.options
#let muted = luma(95)
#let rule = 0.5pt + luma(170)

#set document(title: (if data.title != "" { data.title } else { "Statement" }) + " – " + data.account.name, author: data.organisation)
#set text(font: "Libertinus Serif", size: 9.5pt, lang: "en")
// A letterhead PDF behind the pages, full size (scaled to fit if its page size differs): one of
// its pages behind the first page, and maybe one behind the rest. The margins keep the
// statement clear of it.
#let letterhead = data.letterhead
#set page(
  paper: opt.paper,
  margin: (x: 18mm, top: opt.margin_top_mm * 1mm, bottom: opt.margin_bottom_mm * 1mm),
  background: if letterhead != none {
    context {
      let p = if here().page() == 1 { letterhead.first } else { letterhead.later }
      if p != none { image("/" + letterhead.file, page: p, width: 100%, height: 100%, fit: "contain") }
    }
  },
  footer: context {
    set text(size: 8pt, fill: muted)
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
      [#data.account.name · #data.provenance],
      [Page #counter(page).display("1 of 1", both: true)],
    )
  },
)

// Figures line up in columns.
#let amt(s) = text(number-width: "tabular", s)
#let label(s) = text(fill: muted, s)

// ---- header ----

#grid(
  columns: (1fr, auto),
  column-gutter: 12pt,
  align: (left + bottom, right + bottom),
  {
    // The title is optional (e.g. a letterhead already says what this is).
    if data.title != "" { text(size: 18pt, weight: "semibold", data.title) }
    if data.title != "" and data.subtitle != "" { linebreak() }
    if data.subtitle != "" { text(size: 11pt, fill: muted, data.subtitle) }
  },
  if data.logo != none { image("/" + data.logo, height: 16mm) },
)

#v(4pt)
#line(length: 100%, stroke: rule)
#v(2pt)

// The template's header fields, filled in.
#grid(
  columns: (auto, 1fr),
  column-gutter: 14pt,
  row-gutter: 6pt,
  ..data.fields.map(f => (label(f.label), f.value)).flatten(),
)

// ---- totals ----

#v(10pt)
#let total-box(name, value) = block(
  width: 100%,
  inset: (x: 8pt, y: 7pt),
  stroke: rule,
  radius: 2pt,
  {
    label(text(size: 8pt, name))
    linebreak()
    text(size: 12pt, weight: "semibold", amt(value))
  },
)
// The template's summary figures (balances come from all the account's rows, whatever was
// printed), up to four to a row.
#let summary = data.summary
#if summary.len() > 0 {
  if opt.totals.summary_title != "" {
    text(size: 11pt, weight: "semibold", opt.totals.summary_title)
    v(2pt)
  }
  grid(
    columns: (1fr,) * calc.min(summary.len(), 4),
    column-gutter: 8pt,
    row-gutter: 6pt,
    ..summary.map(f => total-box(f.label, f.value)),
  )
}

// ---- totals by type ----

#if opt.summary and data.by_type.len() > 0 {
  v(10pt)
  text(size: 11pt, weight: "semibold")[Totals by category]
  v(2pt)
  table(
    columns: (1fr, auto, auto, auto, auto),
    stroke: none,
    inset: (x: 5pt, y: 3.5pt),
    align: (x, y) => if x == 0 { left } else { right },
    fill: (x, y) => if y > 0 and calc.even(y) { luma(246) },
    table.header(..([Category], [Rows], [Money in], [Money out], [Net]).map(h => text(weight: "semibold", h))),
    table.hline(stroke: rule),
    ..data.by_type
      .map(t => (t.name, amt(str(t.count)), amt(t.money_in), amt(t.money_out), amt(t.net)))
      .flatten(),
    table.hline(stroke: rule),
    text(weight: "semibold")[Total],
    amt(str(data.totals.count)),
    amt(data.totals.money_in),
    amt(data.totals.money_out),
    text(weight: "semibold", amt(data.totals.net)),
  )
}

// ---- transactions ----

#v(10pt)
#text(size: 11pt, weight: "semibold")[Transactions]
#v(2pt)

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
  inset: (x: 5pt, y: 3.5pt),
  align: (x, y) => if is-num(cols.at(x)) { right } else { left },
  table.header(..cols.map(c => text(weight: "semibold", c.label))),
  table.hline(stroke: rule),
  ..opening-line,
  ..body-rows,
  ..if totals-line != none { (table.hline(stroke: rule), ..totals-line) },
)

#if data.has_hand_added {
  v(4pt)
  text(size: 8pt, fill: muted)[\* Added by hand (e.g. part of a split), not from a bank statement.]
}
