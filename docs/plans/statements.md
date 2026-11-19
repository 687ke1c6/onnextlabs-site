# Plan: Statements

Brings the statements-iq proof of concept into the portal as a new org section,
`/portal/<org>/statements`, next to Documents. Each step leaves the app working and can be
reviewed and committed on its own.

## What it does

1. Someone with `write-statements` drops a bank statement CSV on the Statements page.
2. The server detects the format (only Standard Bank CSV for now), parses it, gives every
   transaction its n8n-compatible id, and sets `type` from the org's **type rules**. The page
   shows a preview.
3. **Save** stores the upload and its transactions in SQLite. Transactions already stored
   for the same account are skipped, never changed.
   Optionally, it also saves the original file to the org's Documents. The default folder is
   `Statements`, and this needs `write-docs` as well.
4. Optionally, from a saved upload: **Add to Google Sheet**. Connect Google (separate
   consent, Sheets scopes only), pick or create a spreadsheet and tab, and append the rows.
   The rules are the same as the POC: ids already in the tab are skipped, and the header row
   is written to an empty tab.

The database is the record. The Google Sheet is an optional copy for each person, going to
a sheet they own.

## Decisions

| Topic | Decision |
|---|---|
| Code layout | The repo becomes a Cargo workspace. `crates/statements-core` is copied from `statements-iq/crates/core`: parsing, hashing, rows, the Sheets client, and the upload into a sheet. The POC's server crate isn't copied: its routes are rewritten into `src/statements.rs` on this app's sessions and RBAC. The CLI isn't copied. |
| Categories | The built-in `categories.toml` (personal payee names) is **not** copied. Its matching logic stays in core, but the rules come from the database. |
| Type rules | Per organisation, ordered, and the first match wins. Each rule has a `type`, a list of `contains` substrings (case-insensitive), an optional min amount (≥), an optional max amount (<), and optionally a set of formats. This is the POC's rule shape without `accounts`. Rules apply when a statement is saved. A "re-apply to stored transactions" action is a later option. |
| Permissions | `read-statements` covers viewing uploads and transactions and adding them to your own Google Sheet. `write-statements` covers uploading, deleting uploads and editing type rules. Both go to `Admin-org` in a migration, and neither goes to `Member`. |
| Google access | A separate "Connect Google Sheets" OAuth flow from the Statements page, asking for `spreadsheets` and `drive.metadata.readonly` only. It uses the same OAuth client and a new redirect URI, `<PUBLIC_URL>/api/statements/google/callback`. The access token is held in server memory, keyed by the portal session, for about an hour, with no refresh token. It never reaches the browser. A restart drops it and the user connects again. |
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
    rows_added     INTEGER NOT NULL,             -- parsed minus already stored
    document_id    INTEGER REFERENCES documents (id) ON DELETE SET NULL,
    uploaded_by    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    uploaded_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- The canonical rows: the n8n shape plus where they came from.
CREATE TABLE statement_transactions (
    id             INTEGER PRIMARY KEY,
    org_id         INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    upload_id      INTEGER NOT NULL REFERENCES statement_uploads (id) ON DELETE CASCADE,
    account_number TEXT    NOT NULL DEFAULT '',  -- '' when the format doesn't state one
    txn_id         INTEGER NOT NULL,             -- the n8n i32 hash
    date           TEXT    NOT NULL,             -- YYYY-MM-DD
    amount         TEXT    NOT NULL,             -- raw string, part of the hash
    description    TEXT    NOT NULL,             -- incl. any '_' suffix and trailing spaces
    currency       TEXT    NOT NULL,
    type           TEXT,                         -- from the type rules at save time
    UNIQUE (org_id, account_number, txn_id)
) STRICT;

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
- **Dedupe key** is `(org, account_number, txn_id)`. The id hashes only date, description
  and amount, so without the account two accounts could produce the same id. For formats
  with no account number (none yet), every file of that format shares `''`.
- **Deleting an upload** removes its transactions, so re-uploading the statement adds them
  again. A linked document is left alone.
- **Columns:** `org_id` and `upload_id` are enough for now.

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

## Step 1: Schema and permissions

- Migration `0011_statements.sql`: the tables above, plus the two permissions granted to
  `Admin-org`.
- Add `ReadStatements` and `WriteStatements` to `src/rbac.rs`, and update the
  permission table in the README.

## Step 2: Upload, preview, save

- `POST /api/orgs/{slug}/statements/preview` (multipart): parse, apply the rules, and
  return format, account number, row count, how many rows got a type, the first rows,
  and how many rows are already stored.
- `POST /api/orgs/{slug}/statements` (multipart, plus `save_to_documents` and `folder`):
  parse again, insert the upload and new transactions in one transaction, and optionally
  store the file in Documents (creating `Statements` if it's missing).
- `GET /api/orgs/{slug}/statements` lists uploads, and `GET …/statements/{id}` returns
  one with its transactions. `DELETE …/statements/{id}` deletes one.
- Size cap: the existing `MAX_UPLOAD_MB`.
- Frontend: a Statements entry in the org nav, and the page with the dropzone, preview,
  options and Save, then the uploads list and the upload detail page.

## Step 3: Type rules screen

- `/portal/<org>/statements/rules`: list, add, edit, delete and reorder rules. A "test
  against a description and amount" box shows which rule matches.
- API: `GET/PUT /api/orgs/{slug}/statement-rules`. PUT replaces the whole ordered list,
  which keeps reordering simple.

## Step 4: Google Sheets

- `GET /api/statements/google/connect?return_to=…` and `…/callback`: the separate consent
  flow (state + PKCE), with the token stored by session.
  `GET /api/statements/google` reports whether you're connected and as which email.
  `POST …/disconnect` revokes the token.
- `GET/POST /api/statements/google/spreadsheets`, `GET …/{id}`, and `POST …/{id}/tabs`
  are the POC's pickers.
- `POST /api/orgs/{slug}/statements/{id}/sheet` with `{spreadsheet_id, tab}` builds the
  statement from the stored rows and appends it through core. It returns parsed, skipped
  and appended counts.
- Frontend: an "Add to Google Sheet" panel on the upload page, with the pickers ported
  from the POC.
- README: the extra redirect URI to register, and the Testing-mode/test-user note for
  the sensitive scope.

## Step 5: Docs and clean-up

- README section for Statements, and the POC spec kept as `docs/statements-iq-spec.md`
  for history.
- Remove the `statements-iq/` mount from the devcontainer (yours to do; never committed).
