> **History.** This is the spec of statements-iq, the proof of concept the portal's
> Statements section was ported from (see [plans/statements.md](plans/statements.md)). It
> describes the POC's CLI and standalone web app, which weren't brought over, and its
> behaviour at the time: the portal now overwrites `type` on rows already in a sheet, and
> its type rules are per organisation instead of built in.

# statements-iq — POC spec

A one-shot CLI that takes a bank statement CSV, turns it into rows with the same shape and ids as the
n8n "Parse CSV" workflow, and appends the new rows to a Google Sheets tab you pick interactively.

The POC is deliberately narrow. Each step below is built and checked on its own before moving on.

## Usage

```
statements-iq upload <path/to/statement.csv> [--config ~/.statements-iq/config.toml]
```

1. Parse the CSV (only the Standard Bank CSV format for now; see below).
2. Sign in to Google in the browser, then choose a spreadsheet from the ones you can see.
3. Prompt: choose a tab within that spreadsheet.
4. Read the tab's existing ids, skip rows whose id already exists, and append the rest.
5. Print a summary: rows parsed, skipped as existing, and appended.

## Decisions

| Topic | Decision |
|---|---|
| Invocation | One-shot CLI. No watcher, email, or Telegram yet. |
| Input format | Only Standard Bank CSV (synthetic example: `tests/fixtures/sb_csv_synthetic.csv`). Statement-type detection comes later. |
| Target selection | Interactive: list spreadsheets, then list tabs. |
| Google auth | Browser sign-in on every run (OAuth installed-app flow: loopback redirect to `127.0.0.1:<random port>`, PKCE). The access token is kept in memory only; nothing is stored between runs. Needs a one-time "Desktop app" OAuth client (id + secret in the config; not confidential per Google). No service-account keys, and sheets need no extra sharing. Uploads are rare (about every 6 months), so signing in each run is fine. |
| Config | TOML file for the key path and later the account and category rules. |
| Duplicates | Skip rows whose `id` already exists in the tab. Never update or delete existing rows. |
| Column mapping | Match by the tab's header row. The app errors if the tab has headers but no `id` column. |
| Empty tab | Write the header row first, then the data. |
| Categories (`type`) | Left blank in the POC. |
| Hash compatibility | Must match n8n **exactly**, verified by a unit test against rows from the live sheet. |

## Standard Bank CSV format (`SB_CSV`)

The CSV has no header row. Columns are referenced by index.

```
0,1234,BRANCH,0,,EXAMPLE BRANCH,0,0                    <- row 0: col0 == "0" identifies SB_CSV
,123456789,ACC-NO,0,,,0,0                              <- row 1: col1 = account number
,0,OPEN,1000.00,OPEN BALANCE,,0,0                      <- row 2: opening balance
HIST,20271229,,-250.75,IB PAYMENT TO,EXAMPLE ...,00377,0    <- transactions
...
,0,CLOSE,15108.46,CLOSE BALANCE,,0,0                   <- last row: closing balance
```

Transactions are `rows[3..len-1]`, keeping only rows where `col1` is non-empty. They map as follows:

| Field | Source |
|---|---|
| date | `col1` as `YYYYMMDD` |
| amount | `col3` **raw string**, not reformatted (`-57.5` stays `-57.5`) |
| description | `format!("{} {}", col4, col5)`. **Keeps the trailing space** when `col5` is empty. |
| currency | `"ZAR"` |

## Row id: must match n8n byte for byte

n8n (server TZ = **UTC**):

```js
hashString(`${date.toISOString()}-${description}-${amount}`)
// hash = ((hash << 5) - hash) + str.charCodeAt(i); hash |= 0;   over UTF-16 code units
```

Rust equivalent:
- Input string: `{YYYY-MM-DD}T00:00:00.000Z-{description}-{amount}`
- Iterate over `str.encode_utf16()`, `hash = hash.wrapping_shl(5).wrapping_sub(hash).wrapping_add(unit as i32)` on `i32`.
- The id is the signed `i32`, written as a decimal (it can be negative).

Duplicate rows within one file: rows are grouped by date. If a row's id collides with an earlier row
on the same day, append `_` to its description and re-hash until the id is unique. The **stored**
description keeps the `_` suffix.

Example hash input for the first row of the synthetic CSV:
`2027-12-29T00:00:00.000Z-IB PAYMENT TO EXAMPLE MUNICIPALITY ACC 001--250.75`

Test data: real statements and prod sheet exports are **never committed**. They live in the git-ignored
`references/` folder (with the n8n workflow exports), and the tests that use them are skipped when it is absent.
Committed tests use the synthetic statement `tests/fixtures/sb_csv_synthetic.csv`, whose expected output
`tests/fixtures/sb_csv_synthetic.n8n.json` comes from running the real n8n code (`just oracle`). Locally, every
`references/<name>.csv` with a `references/<name>.n8n.json` (from `just oracle references/<name>.csv`) is checked too.

**Confirmed against prod** (`references/hash-ref.txt`, 29 rows): every id matches `utc` with the amount and
description exactly as stored, and none match `date`. Trailing spaces matter: one row with `##  ` only matches
with both spaces kept.

Amount assumption: the amount is hashed as the raw CSV string (`-57.5`), and future uploads always use this
CSV format. The 2023 prod rows hashed `-1,798.00`-style strings, so they came from a differently formatted
source. Rows that overlap that older data would get new ids and be added again.

Two date forms are still supported, as requested, and are
selected with `--hash-date utc|date` (default `utc`), implemented in `src/hash.rs` as `DateFormat`:
- `utc`: `2026-06-01T00:00:00.000Z-...`, which is what the n8n code does
- `date`: `2026-06-01-...`, the plain sheet date string

## Output row (n8n shape)

Header order when writing to an empty tab:

| Column | Value |
|---|---|
| id | hash above |
| month | en-US short month and year, for example `May 2026` |
| date | `YYYY-MM-DD` |
| effective_date | blank (next to `date`, as in the prod sheet) |
| amount | raw string |
| description | as above, including any `_` suffix |
| currency | `ZAR` |
| type | blank (POC) |

When the tab already has headers, values go under matching header names, and unknown headers are left blank.

## Google APIs

- Sheets API v4: read the header row and the `id` column (`values:batchGet`, unformatted), read tab names, create
  spreadsheets and tabs, and append values (`values:append`, `insertDataOption=INSERT_ROWS`, so no cell is ever
  overwritten). Values are entered as `USER_ENTERED` with typed cells: `id` and `amount` as JSON numbers (immune to
  the sheet locale's decimal separator), `date` as `YYYY-MM-DD` (becomes a real date), and `month`, `description`,
  `currency` as text with a leading `'` (never parsed as a date, number or formula; trailing spaces kept). Existing
  ids are read whether stored as numbers or text.
- An empty tab (blank row 1) gets the header row first, but only if rows 2-3 are blank too; otherwise it's an error.
- Drive API v3 `files.list` (`mimeType='application/vnd.google-apps.spreadsheet'`) to list spreadsheets.
  This returns the files the signed-in user can see.
- Scopes: `spreadsheets`, `drive.metadata.readonly`.

## Out of scope (later)

Other banks (Revolut, ABN AMRO, Standard Bank PDF), installing the binary, file watching or email triggers,
notifications, and the invoice reconciliation flow. Account routing and categorisation are phase 2 (below).

## Build order

1. ~~Cargo scaffold~~
2. ~~Hash function and the n8n-compatibility unit test (live-sheet fixture)~~
3. ~~Standard Bank CSV parser and the in-file duplicate suffixing, tested against n8n output~~
   (oracle: `tools/n8n-oracle.js` runs the real n8n node code; see "Test data" above)
3b. ~~Basic CLI: `statements-iq parse <file> [--hash-date utc|date]` prints parsed rows to the console~~
4. ~~Output row shaping (month, date, column order)~~ (`src/row.rs`; `month` checked against the n8n fixture)
5. ~~Config loading (TOML)~~ (`src/config.rs`, `statements-iq.example.toml`; check with `just config`)
6. ~~Google auth (browser sign-in) and listing spreadsheets and tabs~~ (`src/auth.rs`, `src/google.rs`; check with `just sheets`)
7. ~~Interactive selection~~ (`src/select.rs`; also creates spreadsheets and tabs)
8. ~~Read existing ids, then append new rows~~ (`src/upload.rs`)
9. End-to-end run against a DEV sheet
   (run by you: create a DEV spreadsheet in the picker, upload, re-run and expect 0 appended)

# Phase 2: accounts and categories

## Problem

Where a statement goes depends on whose account it is, and a statement doesn't always say. Standard Bank CSVs
carry an account number (row 1, column 1); other formats are recognised only by their column shape (Revolut:
first cell `Type`; ABN AMRO: second header `mutationcode`; Standard Bank PDF: first cell `description`) and carry
no account number. n8n handled this with a fixed account-number map plus a hard-coded account per format
("joint, probably"), and failed on unknown numbers.

## Decisions

| Topic | Decision |
|---|---|
| Statement detection | Each format is a detector + parser. The parsed statement has a `format` (e.g. `sb_csv`) and an optional `account_number` (only when the file states one). Only `sb_csv` is implemented in this phase; the others plug in later without config changes. |
| Accounts | Named accounts in the config, each with a target `spreadsheet` id and `tab`, and match rules: `format` (required) and `number` (optional). |
| Routing | A rule with a `number` matches only statements stating that number; a rule without one matches any statement of that format. The most specific match wins (number beats format-only). |
| No match, or a tie | Ask: a picker of the configured accounts, plus "choose a spreadsheet and tab" (the phase 1 pickers). Nothing is guessed. |
| Override | `--account <name>` skips routing; `--spreadsheet`/`--tab` still work and take precedence. |
| Unknown account number | Not an error (unlike n8n): it falls through to the format-only rules, then to the picker. |
| Categories (`type`) | Ordered rules in `categories.toml` (committed), built into the binary; changing them means a rebuild. `[[categories]]` in the user's config is an error. The first match sets `type`, no match leaves it blank. A rule has `type`, `contains` (any of these substrings, case-insensitive), and optionally `min_amount` (>=), `max_amount` (<), and `accounts` (only for these accounts). |
| Category parity | The user's rules are a port of the n8n "Add SB Catagories (type)" node and must give the same `type` for every row of the local real statements (oracle extended to emit `type`). |
| Existing rows | Categories apply to newly appended rows only; existing rows are never changed. |
| Privacy | Account numbers and sheet ids live only in `~/.statements-iq/config.toml`, outside the repo. The category rules, including payee names, are committed in `categories.toml` (the user's choice; keep the repo private). The example config and other committed tests use made-up values. |

## Config sketch

```toml
[accounts.joint]
spreadsheet = "1AbC..."            # id from `just sheets`
tab = "Transactions"
match = [{ format = "sb_csv", number = "123456789" }]

[accounts.travel]
spreadsheet = "1XyZ..."
tab = "2027"
match = [{ format = "revolut_csv" }]   # no number in these files: any Revolut CSV
```

`categories.toml` (built in):

```toml
[[categories]]
type = "BOND REPAYMENT"
contains = ["BOND REPAYMENT", "DEBIT ORDER - DO"]

[[categories]]
type = "RENT"
contains = ["TRANSFER FROM EXAMPLE TENANT"]
min_amount = 38000
accounts = ["joint"]
```

## Open questions

- ~~n8n matches `FEE` as a plain substring, so `COFFEE SHOP` is categorised `FEES`.~~ Decided: keep substring
  matching (parity with n8n); no whole-word option for now.
- ~~Should choosing from the picker offer to save the route to the config?~~ Decided: not for now; `upload` keeps
  printing the `[accounts.NAME]` block to paste.

## Build order (phase 2)

10. ~~Statement detection: `format` + optional `account_number` on the parsed statement; detector registry with
    `sb_csv` (error lists the known formats when nothing matches)~~ (`src/statement.rs`; a new format is a
    `Format` variant plus a module with `matches` and `parse`)
11. ~~Accounts config and routing (match rules, specificity, `--account`, picker fallback), unit-tested on made-up
    accounts; `upload` prints which account and why~~ (`src/route.rs`, `[accounts]` in `src/config.rs`; two
    accounts with the same rule is a config error; after a manual choice, `upload` prints an `[accounts.NAME]`
    block to paste, in place of saving routes automatically)
12. ~~Categories config and matching; `parse` and `upload` show `type`; oracle emits n8n's `type`~~
    (`src/categories.rs`, `[[categories]]` in `src/config.rs`; optional `formats` scope added, since n8n's rules
    only ever ran on Standard Bank statements)
13. ~~Port the n8n SB categories into the user's config; local parity test against the real statements~~
    (all 7 rules, scoped to `sb_csv`; `local_rules_match_n8n_categories` checks every row of every
    `references/*.csv` with an n8n output, including a local probe statement that hits each rule and both sides
    of the amount thresholds: 239 rows, all equal)
14. ~~End-to-end: upload both real statements with routing, check `type` in the DEV sheet~~ (verified by the user)

## Later

- More statement formats: Revolut CSV, ABN AMRO, Standard Bank PDF (detection rules above). Each is a `Format`
  variant plus a module with `matches` and `parse`; no config changes needed.


# Phase 3: the web app

## Problem

The CLI needs a terminal and a local OAuth client, so only its author can use it. A small web app at a subdomain
of the user's domain lets them, and a few interested people, drop a statement in a browser and append it to their
own Google Sheet, with the same choices the CLI offers.

## Flow

1. Drag and drop a CSV (CSV only for now) onto the page.
2. The server parses it and shows a preview: format, account number (when stated), number of rows, and how many
   got a `type`, plus the first few rows.
3. "Sign in with Google": the regular web OAuth flow (redirect to Google, back to the server's callback).
4. Choose a spreadsheet (or create one), then a tab (or create one): the same choices as the CLI pickers.
5. Upload: append the new rows, then show parsed, skipped as existing, and appended.

## Decisions

| Topic | Decision |
|---|---|
| Users | The author plus a few others. The Google OAuth app stays in "Testing" mode with them added as test users (limit 100), so the sensitive Sheets scope needs no Google verification. Test users see Google's "unverified app" notice once. |
| Hosting | Public, on a subdomain of the user's domain (`https`). TLS and the subdomain are set up by the user, outside this repo (reverse proxy in front of the server). |
| Shape | A Cargo workspace: `core` (library: parsing, detection, categories, hashing, Sheets client, upload), `cli` (the existing CLI, kept), `server` (axum). The frontend lives in `web/`. |
| Frontend | SolidJS + TypeScript + Vite + Tailwind CSS, with Bun as package manager and script runtime (pinned 1.4.x in the dev container), with shadcn-style components from a Solid port built on Kobalte (shadcn itself is React-only). In release builds the server embeds the built `web/dist`, so it ships as one binary; in development Vite proxies `/api` and `/auth` to the server. |
| Google auth | Regular web OAuth (authorization code + PKCE + `state`) with a new OAuth client of type "Web application", its redirect URI registered exactly (`https://<subdomain>/auth/callback`, plus `http://localhost:<port>/auth/callback` for development). Same scopes as the CLI: Sheets and `drive.metadata.readonly`. Online access only: no refresh tokens are asked for or kept; signing in again after about an hour is fine for an occasional upload. |
| Secrets | The web client id and secret (now a real secret) and the public base URL come from environment variables (`.env` locally, git-ignored), never from a file in the repo. Session ids are random 256-bit values held server-side, so there is no session signing key. |
| Sessions | An HttpOnly, `Secure`, `SameSite=Lax` cookie holding a random session id. Session data stays in server memory: the parsed statement and the Google access token. Tokens never reach the browser. Sessions expire after inactivity (e.g. 1 hour); a restart signs everyone out and drops pending uploads. |
| Upload before sign-in | Allowed: the parsed statement is kept in the session, so it survives the OAuth redirect. |
| Statements | Parsed in memory, never written to disk or logged. Request size capped (e.g. 5 MB); anything that isn't a detected statement format is rejected with the known formats listed. |
| Routing | Pickers only. No `[accounts]` config on the server. |
| Categories | The built-in `categories.toml` rules apply to every upload (as in the CLI, only the account-scoped rules are skipped, since there are no accounts). |
| Duplicates, columns, empty tab | Exactly as the CLI: the same `core` upload code. |
| CSRF | State-changing endpoints are `POST` with a JSON body (or multipart for the file) and require a custom header the browser sets (e.g. `X-Requested-With`), on top of `SameSite=Lax`. |
| Blocking I/O | The Google client stays on blocking `ureq` at first, called through `spawn_blocking`; moving to async is a later option, not a requirement. |
| CLI | Kept, unchanged in behaviour, on top of `core`. |

## API sketch

| Method and path | Does |
|---|---|
| `POST /api/statement` (multipart) | Parse the file into the session; returns the preview. |
| `GET /api/session` | Whether signed in (and as whom), and the pending statement's preview if any. |
| `GET /auth/login` | Redirect to Google's consent page. |
| `GET /auth/callback` | Check `state`, exchange the code, store the token, redirect to `/`. |
| `POST /auth/logout` | Revoke the token and end the session. |
| `GET /api/spreadsheets` | The signed-in user's spreadsheets, most recently modified first. |
| `GET /api/spreadsheets/{id}` | Its title and tabs. |
| `POST /api/spreadsheets` | Create a spreadsheet with one tab. |
| `POST /api/spreadsheets/{id}/tabs` | Add a tab. |
| `POST /api/upload` | Append the pending statement to `{spreadsheet_id, tab}`; returns the summary and clears the statement. |

## Open questions

- Other people's uploads are categorised with the author's rules (payee names and amounts). Fine for "a few
  interested people" at first; per-user rules would need per-user storage.
- Deployment packaging (container image or plain binary plus a service file) is decided when it's deployed.

## Running it

- `cp .env.example .env` and fill in the "Web application" client; its redirect URIs must include
  `http://localhost:3000/auth/callback`.
- `just dev` while developing: Vite (on Bun) serves the page on http://localhost:3000 with hot reload and forwards
  `/api` and `/auth` to the axum server on :3001, so the registered redirect URI works unchanged. Frontend edits
  show up immediately; restart `just dev` after Rust changes.
- `just serve` runs it as it ships: the frontend built into `web/dist` and served by the server itself on :3000.
- Release: `just build-server` gives `target/release/statements-iq-server` with the frontend embedded. Behind the
  reverse proxy, set `PUBLIC_URL=https://<subdomain>` and `BIND` to the address the proxy forwards to.

## Build order (phase 3)

15. ~~Workspace split: `core`, `cli`; the CLI behaves exactly as before (`just check` passes, same commands).~~
    (`crates/core`: parsing, categories, Sheets client, upload and the shared OAuth pieces in `oauth.rs`;
    `crates/cli`: loopback sign-in and token cache, config, routing, pickers)
16. ~~`server` skeleton: axum, config from the environment, health check, serving a placeholder page; `just serve`.~~
    (`crates/server`; settings in `.env`, see `.env.example`)
17. ~~`web/` skeleton: Vite + Solid + TypeScript + Tailwind + the component library; dev proxy to the server;
    release build embedded in the server.~~ (shadcn-style components in `web/src/components/ui`, on Kobalte;
    `rust-embed` serves `web/dist`, read from disk in debug builds)
18. ~~Statement upload: drag-and-drop, `POST /api/statement`, sessions, preview. Size cap, format errors.~~
19. Web OAuth: login, callback, logout, session token. Built and unit-tested (state, PKCE, redirects, CSRF
    header); **to check by the user** against their "Web application" client.
20. ~~Pickers: list, choose and create spreadsheets and tabs.~~ (UI checked in a headless browser against mocked
    Google responses)
21. Upload: append through `core`, show the summary. Built; **end-to-end run against a DEV sheet locally, by the
    user**.
22. Deploy to the subdomain (by the user), add the production redirect URI, and run the same end-to-end check.
