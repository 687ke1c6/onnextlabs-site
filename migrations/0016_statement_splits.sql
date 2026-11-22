-- Splits: rows added by hand to move part of a row's amount to another type or date (e.g. the
-- utilities share of a rent payment). Ordinary rows otherwise, so they sort, filter and total like
-- any other. A split's rows add up to zero; the original row is never changed.
--
-- `split_of` is the original row's `txn_id` in the same account (not its row id), so deleting a
-- statement and saving it again links its splits back up. NULL for rows from statements.
ALTER TABLE statement_transactions ADD COLUMN split_of INTEGER;

CREATE INDEX statement_transactions_split_of ON statement_transactions (account_id, split_of)
    WHERE split_of IS NOT NULL;
