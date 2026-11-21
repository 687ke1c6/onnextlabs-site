-- Bank statements (src/statements.rs). An upload is one statement file someone saved; its
-- rows are stored once per (org, account, n8n id) and linked to every upload that contained
-- them. The n8n id (`txn_id`) is the prod hash, exactly as written to Google Sheets.

CREATE TABLE statement_uploads (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    org_id         INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    -- Detected format, e.g. 'sb_csv'.
    format         TEXT    NOT NULL,
    -- As stated in the file; '' when the format doesn't state one.
    account_number TEXT    NOT NULL DEFAULT '',
    -- As uploaded.
    filename       TEXT    NOT NULL,
    -- Hex SHA-256 of the file.
    sha256         TEXT    NOT NULL,
    rows_parsed    INTEGER NOT NULL,
    -- Not stored before this upload / already stored and updated in place.
    rows_new       INTEGER NOT NULL,
    rows_updated   INTEGER NOT NULL,
    -- The copy saved to Documents, if the uploader asked for one.
    document_id    INTEGER REFERENCES documents (id) ON DELETE SET NULL,
    uploaded_by    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    uploaded_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX statement_uploads_org ON statement_uploads (org_id, uploaded_at);

-- The canonical rows: the n8n shape plus org and account.
CREATE TABLE statement_transactions (
    id             INTEGER PRIMARY KEY,
    org_id         INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    account_number TEXT    NOT NULL DEFAULT '',
    -- The n8n i32 hash of date, description and amount (prod ids, unchanged).
    txn_id         INTEGER NOT NULL,
    -- YYYY-MM-DD
    date           TEXT    NOT NULL,
    -- Raw string from the file, never reformatted (part of the hash).
    amount         TEXT    NOT NULL,
    -- Including any '_' suffix and trailing spaces (part of the hash).
    description    TEXT    NOT NULL,
    currency       TEXT    NOT NULL,
    -- From the type rules. A save with no matching rule keeps the existing type.
    type           TEXT,
    created_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, account_number, txn_id)
) STRICT;

-- Which rows each upload contained, new or already stored, in file order.
CREATE TABLE statement_upload_transactions (
    upload_id      INTEGER NOT NULL REFERENCES statement_uploads (id) ON DELETE CASCADE,
    transaction_id INTEGER NOT NULL REFERENCES statement_transactions (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,
    PRIMARY KEY (upload_id, transaction_id)
) STRICT;

CREATE INDEX statement_upload_transactions_txn ON statement_upload_transactions (transaction_id);

-- Per-org rules that set `type`, tried in `position` order; the first match wins.
CREATE TABLE statement_type_rules (
    id         INTEGER PRIMARY KEY,
    org_id     INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    type       TEXT    NOT NULL,
    -- JSON array of strings; matches when the description contains any (case-insensitive).
    contains   TEXT    NOT NULL,
    -- Amount >= min_amount, amount < max_amount (either may be absent).
    min_amount REAL,
    max_amount REAL,
    -- JSON array of format ids; empty = every format.
    formats    TEXT    NOT NULL DEFAULT '[]',
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX statement_type_rules_org ON statement_type_rules (org_id, position);

-- Org admins get the new permissions; Members don't.
INSERT OR IGNORE INTO role_permissions (role_id, permission)
    SELECT r.id, p.permission FROM roles r,
        (SELECT 'read-statements' AS permission UNION ALL SELECT 'write-statements') p
    WHERE r.name = 'Admin-org';
