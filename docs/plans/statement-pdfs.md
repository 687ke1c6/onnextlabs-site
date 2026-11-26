# Plan: Statement PDFs

Print an account's transactions as a statement that reads like a bank's: account holder, account
number, product, the statement period, and balances. How it looks is set by **statement
templates** that belong to the org, chosen when printing.

Started as a per-account settings dialog with a **letterhead** (a PDF from Documents drawn behind
the pages, with top and bottom margins), then moved to org templates. It's all one migration,
`0019_statement_templates.sql`.

**Status key:** ✅ done · 🔜 next · ⬜ not started · ❓ needs a decision

## Decisions

| Topic | Decision |
|---|---|
| Templates | Belong to the **org** (`statement_templates`), not to an account. Any account prints with any template. The print dialog has a template picker; the account remembers the last one used. |
| Account details | Holder name and product name are properties of the **account** (new columns), edited on the Accounts tab with its name. The account number is the existing `identifier`. A template only refers to them, so one template suits every account. |
| Text with substitutions | Title, subtitle, footer and the **header fields** (a list of label/value rows, e.g. `Account holder` / `{account.holder}`) can contain `{name}` variables. Filled in on the server and passed to Typst as plain strings, so typed text is never Typst markup. An unknown variable is refused when the template is **saved**. `{{` and `}}` print a literal brace. |
| Statement period | Stated explicitly in the print dialog (from / to), by **transaction date**. It's prefilled from the active date chip if there is one, otherwise from the printed rows' first and last dates. It's what `{period.from}` / `{period.to}` print, and what the balances are for. (The grid's date filters use the *effective* date, so they can't be the period.) |
| Date basis | Chosen **when printing** ("Use effective dates", remembered per org in the browser). By default the transaction `date`; with effective dates, a row's `effective_date` where it has one. It decides the period's rows, the balances and the running balance's order. Within a day, by row id (the order rows were stored). |
| Table columns | A template lists its table's columns in order, each with its own heading: date, effective date, description, type, amount (signed), debit (money out, unsigned), credit (money in), balance (running). Balance prints only when a running balance is true; type drops out when grouped by type. |
| Totals | Per template: the summary figures (transactions, money in, money out, net, opening and closing balance), in order and each with its own label, under an optional heading (none: no summary); a totals row at the end of the table; subtotals when grouped; the opening-balance row's label; and the totals by type table (standard layout). Labels can use variables. Balance figures drop out when the account has no starting balance. |
| Designing | Its own page, `/portal/<org>/statements/templates/<id>`, not a dialog. |
| Printing | A dialog before printing: template, period, the date basis, the balances and whether a running balance prints, and saving a copy to Documents. **Preview** opens the PDF in a new tab (downloaded from there). |
| Balance source | Entered **by hand** on the account: the balance at the start of a given date. Checking it against the balances in imported statements comes later (see "Later"). |
| Balances are from the ledger | Computed on the server from **all** of the account's rows, never from the printed rows. Filters change which rows print, never the balances. Opening balance for a period = starting balance + rows dated from the starting date up to the period's start. Before the starting date it works backwards: starting balance − rows from the period's start up to the starting date. Closing = opening + rows dated in the period. |
| Running balance column | Only offered when the printed rows are **exactly** the account's rows dated in the period (the server checks). Otherwise the PDF prints without it and the dialog says why ("Running balance needs every row in the period: clear the filters"). Opening and closing balances can always print. |
| Splits and hand-added rows | Splits add up to zero, so balances don't change. Hand-added rows count: they're transactions. |
| Formats | A template setting each: date style (`4 Oct 2026` or `04 Oct 2026`) and currency symbol (`R`, blank, …), e.g. `R 40,957.20`. |
| Provenance | Two things a template can't turn off: the mark on hand-added rows, and the last line of the footer, "Prepared by {org.name} from imported bank data · {generated}", so a bank-style statement stays clearly the org's own. Every built-in layout prints it (a test checks). |
| Layouts | Built in: **Standard** (as before, with header fields and balances) and **Bank style** (header fields beside a summary box, then date, description, money out, money in, balance). Modelled on no particular bank; there are too many to copy one. |

### Variables

| Variable | Example |
|---|---|
| `{org.name}` | Acme Holdings |
| `{account.name}` | House Account |
| `{account.holder}` | Mr J Smith |
| `{account.number}` | 123456789 (the identifier) |
| `{account.product}` | Current Account |
| `{period.from}`, `{period.to}` | 04 Oct 2026, 03 Nov 2026 |
| `{balance.opening}`, `{balance.closing}` | R 12,000.00, R 40,957.20 |
| `{total.in}`, `{total.out}`, `{total.net}` | totals of the **printed** rows |
| `{count}` | printed rows |
| `{generated}` | the day it was printed |
| `{selection}` | the filters, in words |

`{account.holder}` and `{account.product}` print empty if the account doesn't have them. Balances
print as `—` if the account has no starting balance.

## Schema

One migration, `migrations/0019_statement_templates.sql`: the `statement_templates` table (layout,
text, header fields, logo, letterhead, margins, paper, `columns` and `totals` as JSON, totals by
type, grouping, formats) and the account columns (holder, product, starting balance and its date,
last template). While it was being built it went through four migrations (0019–0022); they were
collapsed before the first commit, and the dev database's migration history was rewritten to match
(backup in `backups/`).

## Steps

| # | Step | Done when | Status |
|---|---|---|---|
| 0 | Letterhead and margins (per account, for now) | A letterhead PDF prints behind page 1, and its page 2 behind the rest | ✅ |
| 1 | Org templates | Templates tab: create, rename, duplicate, delete, edit (today's settings dialog, moved). Print dialog picks one; the account remembers it | ✅ (pending your check) |
| 2 | Account details and starting balance | Accounts tab edits holder, product, and "balance at the start of [date]" | ✅ (pending your check) |
| 3 | Substitutions and header fields | Header fields editable in a template, with the variables listed to insert; unknown variables refused on save; they print filled in | ✅ (pending your check) |
| 4 | Statement period | Print dialog has from/to, prefilled from the date chip or the rows; the PDF prints that period | ✅ (pending your check) |
| 5 | Balances | Opening and closing balances print, and are the same whatever filters are on; the dialog shows them before printing | ✅ (pending your check) |
| 6 | Bank-style layout and running balance | A second built-in layout modelled on an example statement; running balance column when the rows are the whole period | ✅ (pending your check) |
| 7 | Formats | Date style and currency symbol per template | ✅ (pending your check) |

### Step 1: Org templates
- Migration as above. `statement_pdf.rs` routes move to `/api/orgs/{org}/statement-templates`
  (list, create, get, put, delete; `read-statements` to list and print, `write-statements` to edit).
  Printing takes `template_id`.
- The settings dialog becomes the template editor, opened from a **Templates** tab next to Accounts
  and Types. Accounts loses its "Statement PDF" action.
- Print dialog: template picker, defaulting to the account's last one, then "Standard".
- **Tests:** a template from another org can't be used or read; deleting a template clears
  `last_template_id`; an org with none gets "Standard".

### Step 2: Account details and starting balance
- Accounts tab edit form: holder, product, starting balance (amount and date, both or neither).
- **Tests:** amount parses like row amounts; a date without an amount is refused.

### Step 3: Substitutions and header fields
- `statements-pdf`: `fill(text, &Vars) -> Result<String, UnknownVariable>`, pure and unit-tested
  (escaped braces, unknown names, empty values). `check()` runs it on every text field with
  placeholder values.
- The template editor lists the header fields (label, value, reorder, remove) with a menu of
  variables to insert. The Typst template prints them as the two-column block that replaces today's
  fixed Organisation/Account/Period/… block. A new template starts with today's rows as fields.

### Step 4: Statement period
- The print body gains `period: {from, to}` (by `date`). Prefilled from an active quick-filter date
  range, otherwise the rows' min and max `date`. Editable in the dialog.
- The PDF filename uses the period.

### Step 5: Balances
- `balances(db, account, period) -> Option<(opening, closing)>` with SQL sums over **all** of the
  account's rows (one query; amounts are stored as text, so parsed with `amount_cents` as printing
  does today).
- **Tests:** the same period gives the same balances with any filter; a period before, spanning and
  after the starting date; hand-added and split rows.

### Step 6: Bank-style layout and running balance
- No example statement: the layout is a generic one (see Decisions).
- The running balance column: the server compares the printed row ids with the account's rows dated
  in the period. If they're the same, it prints the balance after each row, in date-then-id order.
  The rows print in that order too, not the grid's.
- **Tests:** a filtered print has no running balance; the last running balance equals the closing
  balance.

## Later
- **Check balances on import:** keep the OPEN/CLOSE BALANCE rows (Standard Bank CSV) and the balance
  column (PDF-as-CSV) as checkpoints, set the starting balance from the first import, and flag where
  the ledger and the bank disagree (usually a missing statement or a deleted row).
- Several currencies in one account.
- Custom Typst templates (`statements-pdf` can already render any template source).
