-- Rows with no bank statement behind them: added by hand, either as rows of a split or in a Google
-- Sheet export (rows without an id). Only these can be deleted on their own. A bank statement
-- that later contains the same transaction clears it (the row is then the bank's).
ALTER TABLE statement_transactions ADD COLUMN hand_added INTEGER NOT NULL DEFAULT 0;

UPDATE statement_transactions SET hand_added = 1 WHERE split_of IS NOT NULL;
