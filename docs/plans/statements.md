# Plan: Statements

Brings the statements-iq proof of concept into the portal as a new org section,
`/portal/<org>/statements`, next to Documents. Each step leaves the app working and can be
reviewed and committed on its own.

## What it does

1. Someone with `write-statements` drops a bank statement CSV on the Statements page.
2. The server detects the format (only Standard Bank CSV for now), parses it, gives every
   transaction its n8n-compatible id, and sets `type` from the org's **type rules**. The page
   shows a preview.
3. **Save** stores the upload and its transactions in SQLite. A transaction already stored
   for the same org and account (same hash) is updated in place, never duplicated.
   Optionally, it also saves the original file to the org's Documents. The default folder is
   `Statements`, and this needs `write-docs` as well.
4. Optionally, from a saved upload: **Add to Google Sheet**. Connect Google (separate
   consent, Sheets scopes only), pick or create a spreadsheet and tab, and send **every
   row of that statement**, whether or not it was already in the database or the sheet. Rows
   whose id is already in the tab get their `type` overwritten in place (see step 4), and
   the rest are appended. The
   header row is written to an empty tab, as in the POC.

The database is the record. The Google Sheet is an optional copy for each person, going to
a sheet they own. Having a row in the database never stops it going to a sheet.

## Decisions

| Topic | Decision |
|---|---|
| Code layout | The repo becomes a Cargo workspace. `crates/statements-core` is copied from `statements-iq/crates/core`: parsing, hashing, rows, the Sheets client, and the upload into a sheet. The POC's server crate isn't copied: its routes are rewritten into `src/statements.rs` on this app's sessions and RBAC. The CLI isn't copied. |
| Categories | The built-in `categories.toml` (personal payee names) is **not** copied. Its matching logic stays in core, but the rules come from the database. |
| Type rules | Per organisation, ordered, and the first match wins. Each rule has a `type`, a list of `contains` substrings (case-insensitive), an optional min amount (≥), an optional max amount (<), and optionally a set of formats. This is the POC's rule shape without `accounts`. Rules apply when a statement is saved. A "re-apply to stored transactions" action is a later option. |
| Permissions | `read-statements` covers viewing uploads and transactions and adding them to your own Google Sheet. `write-statements` covers uploading, deleting uploads and editing type rules. Both go to `Admin-org` in a migration, and neither goes to `Member`. |
| Google access | A separate "Connect Google Sheets" OAuth flow from the Statements page, asking for `spreadsheets` and `drive.metadata.readonly` only. It uses the same OAuth client and a new redirect URI, `<PUBLIC_URL>/api/statements/google/callback`. The access token is held in server memory, keyed by the portal user, for about an hour, with no refresh token. It never reaches the browser. A restart drops it and the user connects again. |
| In-memory state | Apart from the Google token, there is none. Preview and save are both stateless multipart posts (the browser keeps the file between them), so nothing has to survive the Google redirect. That's why Sheets works from a *saved* upload. |
| Blocking I/O | Core keeps blocking `ureq`, called through `spawn_blocking`. |
| Raw statements | Not stored unless the user ticks "Save to Documents". Then the file goes through the existing documents code, so it gets the same duplicate-name numbering and permissions. The upload row links to the document. |
| Test data | Only the synthetic fixture comes across. The tests against real statements and n8n output are dropped (accepted). |

## Schema

```sql
-- One per saved statement file.
CREATE TABLE statement_uploads (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    org_id         INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    format         TEXT    NOT NULL,             -- e.g. 'sb_csv'
    account_number TEXT,                         -- when the file states one
    filename       TEXT    NOT NULL,             -- as uploaded
    sha256         TEXT    NOT NULL,             -- of the file
    rows_parsed    INTEGER NOT NULL,
    rows_new       INTEGER NOT NULL,             -- not stored before this upload
    rows_updated   INTEGER NOT NULL,             -- already stored; updated in place
    document_id    INTEGER REFERENCES documents (id) ON DELETE SET NULL,
    uploaded_by    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    uploaded_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- The canonical rows: the n8n shape plus org and account. One row per (org, account, hash),
-- however many uploads contained it.
CREATE TABLE statement_transactions (
    id             INTEGER PRIMARY KEY,
    org_id         INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    account_number TEXT    NOT NULL DEFAULT '',  -- '' when the format doesn't state one
    txn_id         INTEGER NOT NULL,             -- the prod n8n i32 hash, as written to Sheets
    date           TEXT    NOT NULL,             -- YYYY-MM-DD
    amount         TEXT    NOT NULL,             -- raw string, part of the hash
    description    TEXT    NOT NULL,             -- incl. any '_' suffix and trailing spaces
    currency       TEXT    NOT NULL,
    type           TEXT,                         -- from the type rules at the latest save
    updated_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, account_number, txn_id)
) STRICT;

-- Which transactions each upload contained (all of them, new or already stored).
CREATE TABLE statement_upload_transactions (
    upload_id      INTEGER NOT NULL REFERENCES statement_uploads (id) ON DELETE CASCADE,
    transaction_id INTEGER NOT NULL REFERENCES statement_transactions (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,             -- row order in the file, for the sheet
    PRIMARY KEY (upload_id, transaction_id)
) STRICT;
CREATE INDEX statement_upload_transactions_txn ON statement_upload_transactions (transaction_id);

CREATE TABLE statement_type_rules (
    id          INTEGER PRIMARY KEY,
    org_id      INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,                -- evaluation order
    type        TEXT    NOT NULL,
    contains    TEXT    NOT NULL,                -- JSON array of strings
    min_amount  REAL,
    max_amount  REAL,
    formats     TEXT    NOT NULL DEFAULT '[]',   -- JSON array; empty = all
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
```

Decided:
- **The transaction id is the prod n8n hash, unchanged.** `txn_id` is exactly the id written
  to Google Sheets today, from the same inputs (date, description, amount) and the same
  function, so it matches the existing prod data. Org and account are never hashed in.
- **Uniqueness lives in the database, not the id.** The constraint
  `UNIQUE (org_id, account_number, txn_id)` only stops a row from being stored twice for
  the same org and account. The same hash in another org or account is fine. For formats
  with no account number (none yet), every file of that format shares `''`.
- **Re-uploading overwrites.** Saving upserts on `(org_id, account_number, txn_id)`. The
  hash inputs (date, description, amount) are identical by definition, so in practice this
  refreshes `type` from the current rules (see the blank-type rule below) and `updated_at`. Re-uploading a statement is
  also how edited rules get applied to stored rows. (If manual type edits are added later,
  this will need revisiting.)
- **Rows are shared between uploads.** `statement_upload_transactions` links each upload to
  every row it contained, so "Add to Google Sheet" sends the whole statement.
- **Deleting an upload** removes its links and any transactions no other upload still
  contains. A linked document is left alone.
- **A blank type never replaces a type.** When no rule matches, the stored `type` (in the
  database, and in the sheet) is kept. A type from a rule always overwrites. This protects
  existing prod categories when an org has no rules yet, and types typed into the sheet.
- **Columns:** `org_id` and the upload links are enough for now.

## Step 0: Workspace and core crate (no visible change) ✅

Done as below. The site doesn't depend on the crate until step 2 (cargo warns about unused
dependencies). Notes:
- Type rules: `CategoryRule::new` validates a rule, and `categorise(rules, statement)`
  takes them from the caller. The TOML loading and `accounts` scoping are gone.
- `just check` and `just test` now pass `--workspace`, so they cover the crate.
- The Docker build isn't checked here (no Docker in the devcontainer). Its caching stage
  stubs the crate the same way as `src/main.rs`.

- Make the root `Cargo.toml` a workspace with `crates/statements-core` as a member, and
  copy core across: `statement`, `sb_csv`, `hash`, `row`, `categories` (rules passed in, no
  `include_str!`), `google`, `upload`, and `oauth` (only what the web flow needs).
- Copy `tests/fixtures/sb_csv_synthetic.*`. Drop the tests that read `references/`.
- Update the Dockerfile's dependency-caching stage for the workspace.
- `just check` and `cargo test` pass, and the synthetic n8n parity test still runs.

## Step 1: Schema and permissions ✅

- Migration `0011_statements.sql`: the tables above, plus the two permissions granted to
  `Admin-org`.
- Add `ReadStatements` and `WriteStatements` to `src/rbac.rs`, and update the
  permission table in the README.

## Step 2: Upload, preview, save ✅

Done as below. Checked against a real server and rclone with a scratch copy of the dev
database: permissions, preview, save, re-save (updates, never duplicates), Save to
Documents, delete. The pages weren't clicked through in a browser here (none in the
devcontainer). Notes:
- The upload's `account_number` is `''` rather than NULL when the file doesn't state one,
  matching the transactions.
- `documents::save_bytes` stores a file that's already in memory through the same path as
  an upload, so it gets the same numbering and cleanup.
- The org nav and dashboard get a Statements entry (`read-statements`).


- `POST /api/orgs/{slug}/statements/preview` (multipart): parse, apply the rules, and
  return format, account number, row count, how many rows got a type, the first rows,
  and how many rows are new and how many will be updated.
- `POST /api/orgs/{slug}/statements` (multipart, plus `save_to_documents` and `folder`):
  parse again, then in one database transaction insert the upload, upsert every row and
  link them all to the upload, and optionally
  store the file in Documents (creating `Statements` if it's missing).
- `GET /api/orgs/{slug}/statements` lists uploads, and `GET …/statements/{id}` returns
  one with its transactions. `DELETE …/statements/{id}` deletes one.
- Size cap: the existing `MAX_UPLOAD_MB`.
- Frontend: a Statements entry in the org nav, and the page with the dropzone, preview,
  options and Save, then the uploads list and the upload detail page.

## Step 3: Type rules screen ✅

Done as below. "Contains" is edited one text per line, and lines are kept as typed apart
from dropping blank ones. Each rule can be limited to one format, or left on "any".


- `/portal/<org>/statements/rules`: list, add, edit, delete and reorder rules. A "test
  against a description and amount" box shows which rule matches.
- API: `GET/PUT /api/orgs/{slug}/statement-rules`. PUT replaces the whole ordered list,
  which keeps reordering simple.

## Step 4: Google Sheets ✅ (built; the live Google run is yours)

Done as below, with these refinements:
- **Existing rows: only `type` is written,** and only where it differs. Every other column
  this app fills is the id's own input (date, description, amount) or derived from it, so
  rewriting them could only change how existing prod cells are formatted. A blank type never
  replaces one, and a tab with no `type` column gets no updates. Every copy of a repeated id
  is updated. Updates go in one `values:batchUpdate`; new ids are appended as before.
- The result reports rows added, types updated, and existing rows left as they were.
- Connections are kept per portal user (not per browser session), in memory, until the
  token expires. A callback only completes for the portal user who started it.
- `Transaction.currency` in core is now a `String`, so stored rows convert back.
- The last spreadsheet and tab used are remembered in the browser.

Checked here: the consent redirect (scopes, PKCE, redirect URI), the refusals (not signed
in, no statements access, someone else's or a replayed `state`, cancelled consent), and the
pickers answering 401 when not connected. **Not checked here** (needs a real Google client):
the token exchange, listing and creating spreadsheets and tabs, and sending, including the
type overwrite on a tab that already has the rows.

## Step 5: Docs and clean-up ✅

Done: README "Statements" section (including the Google Cloud setup), the extra redirect
URI in the sign-in steps and `.env.example`, and the POC spec in
`docs/statements-iq-spec.md` with a note on what changed.


- README section for Statements, and the POC spec kept as `docs/statements-iq-spec.md`
  for history.
- Remove the `statements-iq/` mount from the devcontainer (yours to do; never committed).

## Step 6: Transactions grid ✅

Added after the first review: the section now opens on **Transactions**, every stored row
in the org (`GET /api/orgs/{slug}/statement-transactions`, `read-statements`), shown with
TanStack Table (`@tanstack/solid-table`). Uploads moved to their own tab
(`/portal/<org>/statements/uploads`), next to Type rules.

- Grid features: search (description, type, account, amount), filters for account (when there's more
  than one) and a date range in the toolbar, a type filter (any one type, or "no type") in a popover
  on the Type column header, sorting (shift-click for multi-column),
  pagination (25–250 rows), and a column picker remembered per browser. In/out/net totals for the
  filtered rows, per currency. (The API also returns each row's latest upload; the grid
  doesn't show it.)
- Everything happens in the browser on the full list, which is fine for thousands of rows. If an org
  grows to hundreds of thousands, move filtering and paging to the server.

## Step 7: Month column, header filters, Uploads grid ✅

- One grid component (`web/src/components/DataGrid.tsx`) serves both tabs: search, header filters,
  multi-column sorting, pagination, a column picker remembered per browser, a toolbar slot (the date
  range) and a summary line.
- Header filters (a funnel in the column header: tick any number of values, or blanks; "All" clears) on Month and Type
  in Transactions, and on Account and Saved by in Uploads. The toolbar account select is gone.
- **Month** isn't stored: the API derives it from `date` with core's `row::month_label`, the same
  function that fills the sheet's `month` column, so the two always agree. It sorts and lists
  chronologically.
- Uploads: file, account, period, rows (plus new/already-saved counts, hidden by default), saved by,
  saved date (with a date range filter); the summary totals the rows in the statements shown.


## Step 8: Compact and Wide switches ✅

Two switches at the right of every grid's toolbar, remembered per browser (`web/src/lib/view.ts`):
- **Compact:** one line per row, every row the same height. Long text is cut off with an ellipsis
  (hover a cell to see all of it); the table switches to fixed column widths so this works.
- **Wide:** the portal's content area drops its `max-w-5xl` cap and uses the window's width. It only
  applies while a grid is on the page, so other portal pages keep their usual width.

## Step 9: One account at a time ✅

People only ever look at one bank account's statements at a time, so the account is a page-level
choice, not a column:
- An **Account** picker in the Statements page header (Transactions and Uploads tabs), listing the
  org's accounts (`GET /api/orgs/{slug}/statement-accounts`: uploads, rows, last upload; most
  recent first). The choice is remembered per org in the browser; otherwise the most recently
  uploaded account is shown. Saving a statement switches to its account.
- `?account=` on `statements` and `statement-transactions` returns only that account's rows
  (`''` = statements stating none); without it, everything (kept for the API's other uses).
- The Account column and its filter are gone from both grids. Switching account starts the grid
  afresh (filters and paging reset).
- Type rules stay per organisation.


## Step 10: Status bar ✅

Each grid ends in a status bar that sticks to the bottom of the window: the row count ("12 of 240
rows"), the page's own figures, paging, and the Compact and Wide switches (moved from the toolbar).
Transactions shows a figure for Amount over the rows that pass the filters (all rows when none
are on), per currency: **Sum, Average, Min or Max**, chosen by clicking it as in a spreadsheet's
status bar (remembered per browser). Uploads shows how many rows the listed statements
hold. The separate summary line under the toolbar is gone; the multi-column sort hint is now the
header buttons' tooltip.

## Step 11: Only the grid scrolls ✅

On pages with a grid, the portal frame is exactly the window's height: the page header, tabs and
toolbar stay put, the grid fills what's left, and only its rows scroll, under pinned column headers
and above the status bar. The grid keeps a minimum height (24rem); when the content above it is
taller than the window allows (the statement preview on Uploads, or a short window), the page
scrolls as before until the grid is in view. Other portal pages scroll normally.
