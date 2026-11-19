-- no-transaction
-- Accounts become a real thing: statements are uploaded *into* an account, rows are unique per
-- account, and type rules belong to an account (no more per-format scoping).
--
-- Rows' uniqueness moves from (org, stated account number, n8n id) to (org, account, n8n id),
-- which means rebuilding statement_transactions (and the uploads and rules tables, to add NOT
-- NULL foreign keys). Rebuilding a table that others reference must happen with foreign keys
-- off, or dropping the old table would cascade-delete its links; foreign_keys can't change
-- inside a transaction, hence "no-transaction" above and the explicit BEGIN/COMMIT below
-- (SQLite's documented procedure for changing a table's schema).

PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE statement_accounts (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    org_id     INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    -- Required; unique in the org, ignoring case.
    name       TEXT    NOT NULL COLLATE NOCASE,
    -- Optional, e.g. the bank account number; unique in the org when set. Checked against the
    -- account number a statement states.
    identifier TEXT,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, name)
) STRICT;

CREATE UNIQUE INDEX statement_accounts_identifier ON statement_accounts (org_id, identifier)
    WHERE identifier IS NOT NULL;

-- An account for every account number already in use (dev data; nothing is in production).
INSERT INTO statement_accounts (org_id, name, identifier)
    SELECT DISTINCT org_id,
           CASE WHEN account_number = '' THEN 'No account stated' ELSE account_number END,
           NULLIF(account_number, '')
    FROM (SELECT org_id, account_number FROM statement_uploads
          UNION SELECT org_id, account_number FROM statement_transactions);

-- Uploads: into an account; `account_number` is now what the file itself states ('' if none).
CREATE TABLE statement_uploads_new (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    org_id         INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    account_id     INTEGER NOT NULL REFERENCES statement_accounts (id) ON DELETE CASCADE,
    format         TEXT    NOT NULL,
    account_number TEXT    NOT NULL DEFAULT '',
    filename       TEXT    NOT NULL,
    sha256         TEXT    NOT NULL,
    rows_parsed    INTEGER NOT NULL,
    rows_new       INTEGER NOT NULL,
    rows_updated   INTEGER NOT NULL,
    document_id    INTEGER REFERENCES documents (id) ON DELETE SET NULL,
    uploaded_by    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    uploaded_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
INSERT INTO statement_uploads_new
    (id, org_id, account_id, format, account_number, filename, sha256, rows_parsed, rows_new,
     rows_updated, document_id, uploaded_by, uploaded_at)
SELECT u.id, u.org_id, a.id, u.format, u.account_number, u.filename, u.sha256, u.rows_parsed,
       u.rows_new, u.rows_updated, u.document_id, u.uploaded_by, u.uploaded_at
FROM statement_uploads u
JOIN statement_accounts a ON a.org_id = u.org_id
    AND (a.identifier = u.account_number OR (a.identifier IS NULL AND u.account_number = ''));
DROP TABLE statement_uploads;
ALTER TABLE statement_uploads_new RENAME TO statement_uploads;
CREATE INDEX statement_uploads_account ON statement_uploads (account_id, uploaded_at);

-- Rows: unique per account.
CREATE TABLE statement_transactions_new (
    id          INTEGER PRIMARY KEY,
    org_id      INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    account_id  INTEGER NOT NULL REFERENCES statement_accounts (id) ON DELETE CASCADE,
    -- The n8n i32 hash of date, description and amount (prod ids, unchanged).
    txn_id      INTEGER NOT NULL,
    date        TEXT    NOT NULL,
    amount      TEXT    NOT NULL,
    description TEXT    NOT NULL,
    currency    TEXT    NOT NULL,
    type        TEXT,
    type_manual INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (account_id, txn_id)
) STRICT;
INSERT INTO statement_transactions_new
    (id, org_id, account_id, txn_id, date, amount, description, currency, type, type_manual,
     created_at, updated_at)
SELECT t.id, t.org_id, a.id, t.txn_id, t.date, t.amount, t.description, t.currency, t.type,
       t.type_manual, t.created_at, t.updated_at
FROM statement_transactions t
JOIN statement_accounts a ON a.org_id = t.org_id
    AND (a.identifier = t.account_number OR (a.identifier IS NULL AND t.account_number = ''));
DROP TABLE statement_transactions;
ALTER TABLE statement_transactions_new RENAME TO statement_transactions;
CREATE INDEX statement_transactions_org ON statement_transactions (org_id);

-- Type rules: per account, no format scoping. Every account starts with none.
DROP TABLE statement_type_rules;
CREATE TABLE statement_type_rules (
    id         INTEGER PRIMARY KEY,
    org_id     INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES statement_accounts (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    type       TEXT    NOT NULL,
    -- JSON array of strings; matches when the description contains any (case-insensitive).
    contains   TEXT    NOT NULL,
    min_amount REAL,
    max_amount REAL,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX statement_type_rules_account ON statement_type_rules (account_id, position);

COMMIT;
PRAGMA foreign_keys = ON;
