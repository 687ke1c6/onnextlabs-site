-- Statement PDFs (statement_templates.rs, statement_pdf.rs), and upload exclusions
-- (statement_exclusions.rs) with the delete-statement-rows permission.
--
-- Templates belong to the org: any account prints with any of them, picked when printing. A
-- template picks a built-in layout and sets its text (title, subtitle, header fields, footer:
-- all may use {variables}), a logo and a letterhead from Documents, margins and paper, the
-- transaction table's columns, which totals print, and how dates and money look.
CREATE TABLE statement_templates (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    org_id                 INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    name                   TEXT    NOT NULL COLLATE NOCASE,
    layout                 TEXT    NOT NULL DEFAULT 'standard' CHECK (layout IN ('standard', 'bank')),
    title                  TEXT    NOT NULL DEFAULT 'Statement',
    subtitle               TEXT    NOT NULL DEFAULT '',
    -- JSON [{"label": ..., "value": ...}], values with {variables}.
    header_fields          TEXT    NOT NULL DEFAULT '[]',
    footer                 TEXT    NOT NULL DEFAULT '',
    logo_document_id       INTEGER REFERENCES documents (id) ON DELETE SET NULL,
    -- A PDF drawn behind the pages, on every page or the first only.
    letterhead_document_id INTEGER REFERENCES documents (id) ON DELETE SET NULL,
    letterhead_every_page  INTEGER NOT NULL DEFAULT 1,
    margin_top_mm          INTEGER NOT NULL DEFAULT 18 CHECK (margin_top_mm BETWEEN 5 AND 150),
    margin_bottom_mm       INTEGER NOT NULL DEFAULT 22 CHECK (margin_bottom_mm BETWEEN 5 AND 150),
    paper                  TEXT    NOT NULL DEFAULT 'a4' CHECK (paper IN ('a4', 'us-letter')),
    -- JSON [{"key": ..., "label": ...}]: the transaction table's columns, in order (keys: date,
    -- effective_date, description, type, amount, debit, credit, balance).
    columns                TEXT    NOT NULL DEFAULT '[]',
    -- JSON: the summary figures, the totals row, subtotals and their labels (statements-pdf
    -- TotalsOptions; '{}' is the standard layout's).
    totals                 TEXT    NOT NULL DEFAULT '{}',
    -- The totals by type table (standard layout).
    show_summary           INTEGER NOT NULL DEFAULT 1,
    group_by_type          INTEGER NOT NULL DEFAULT 0,
    date_style             TEXT    NOT NULL DEFAULT 'short' CHECK (date_style IN ('short', 'padded')),
    currency_symbol        TEXT    NOT NULL DEFAULT '',
    updated_at             TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, name)
) STRICT;

-- What a statement shows about an account: the holder, the product, and a starting balance (the
-- balance at the start of a date), which statement balances are worked out from. The template
-- it was last printed with is remembered.
ALTER TABLE statement_accounts ADD COLUMN holder_name TEXT;
ALTER TABLE statement_accounts ADD COLUMN product_name TEXT;
-- The balance at the start of opening_balance_date (YYYY-MM-DD), in cents: both or neither.
ALTER TABLE statement_accounts ADD COLUMN opening_balance_cents INTEGER;
ALTER TABLE statement_accounts ADD COLUMN opening_balance_date TEXT;
ALTER TABLE statement_accounts ADD COLUMN last_template_id INTEGER
    REFERENCES statement_templates (id) ON DELETE SET NULL;

-- Upload exclusions (statement_exclusions.rs): lines a statement states that aren't saved,
-- e.g. card authorisations that aren't settled yet (the settled transaction comes later as a
-- new line, and the authorisation drops off the bank's statement).
--
-- Rules belong to an account: a line is excluded when its description contains any of
-- `contains` (ignoring case).
CREATE TABLE statement_exclusion_rules (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    org_id     INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES statement_accounts (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    -- JSON array of texts, as typed.
    contains   TEXT    NOT NULL,
    -- Why the lines are left out (for whoever reads the rules later).
    note       TEXT    NOT NULL DEFAULT '',
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX statement_exclusion_rules_account ON statement_exclusion_rules (account_id, position);

-- Deleting rows that came from a bank statement is its own permission. Org admins get it.
INSERT OR IGNORE INTO role_permissions (role_id, permission)
    SELECT id, 'delete-statement-rows' FROM roles WHERE name = 'Admin-org';

-- Effective dates that can't be right (not a real date, or more than a year from the row's date,
-- like 0002-09-01 from a slip typing the year) are cleared. The app refuses them from now on
-- (statements-core `check_effective_date`).
UPDATE statement_transactions SET effective_date = NULL
    WHERE effective_date IS NOT NULL
      AND (date(effective_date) IS NOT effective_date
           OR abs(julianday(effective_date) - julianday(date)) > 366);
