-- Statement types as a managed list, hand-set types on rows, and documents linked to rows.
-- (Rows and rules keep the type's name, so Google Sheets and the rules engine are unchanged;
-- renames and in-use checks are done by the app, in one transaction. A foreign key would mean
-- rebuilding statement_transactions, which would cascade-delete its upload links.)

-- The types an org uses. Names are unique ignoring case, so "FEES" and "Fees" can't both exist.
CREATE TABLE statement_types (
    id         INTEGER PRIMARY KEY,
    org_id     INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL COLLATE NOCASE,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, name)
) STRICT;

-- Start the list with every type already in use.
INSERT OR IGNORE INTO statement_types (org_id, name)
    SELECT DISTINCT org_id, type FROM statement_transactions WHERE type IS NOT NULL AND type <> '';
INSERT OR IGNORE INTO statement_types (org_id, name)
    SELECT DISTINCT org_id, type FROM statement_type_rules;

-- 1 = someone picked this row's type: saving a statement never changes it.
ALTER TABLE statement_transactions ADD COLUMN type_manual INTEGER NOT NULL DEFAULT 0;

-- Documents attached to a row (receipts, invoices...). Many to many; removing a link never
-- touches the document.
CREATE TABLE statement_transaction_documents (
    transaction_id INTEGER NOT NULL REFERENCES statement_transactions (id) ON DELETE CASCADE,
    document_id    INTEGER NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    attached_by    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    attached_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (transaction_id, document_id)
) STRICT;

CREATE INDEX statement_transaction_documents_document ON statement_transaction_documents (document_id);
