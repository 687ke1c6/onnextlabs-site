-- New row ids (statements-core `id.rs`): a 48-bit hash of the date, the amount in cents, the
-- description with formatting removed, and the row's occurrence (0, 1, ... for rows of one
-- statement with the same key). They replace the n8n ids, whose `_` description suffixes and
-- raw amounts gave one transaction different ids in different formats.
--
-- Stored rows can't be given new ids in SQL, so every saved statement and row is removed, with
-- the rows' attached documents (the links; the documents stay). Accounts, types and type rules
-- stay, and so do the statement files in Documents: save them again (or a Google Sheet export)
-- to rebuild. Nothing is in production; agreed to flush the dev data.

DELETE FROM statement_transaction_documents;
DELETE FROM statement_upload_transactions;
DELETE FROM statement_transactions;
DELETE FROM statement_uploads;

-- Part of the id: 0 for the first row of a statement with its key, 1 for the next, and so on.
ALTER TABLE statement_transactions ADD COLUMN occurrence INTEGER NOT NULL DEFAULT 0;
