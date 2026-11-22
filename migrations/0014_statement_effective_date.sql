-- The date a row counts towards when it isn't its own date (e.g. a bond repayment on the 1st that
-- belongs to the month before). YYYY-MM-DD, or NULL. Set by hand; comes in from Google Sheet
-- exports (format `googlesheet_csv`). A statement without one never clears it.
ALTER TABLE statement_transactions ADD COLUMN effective_date TEXT;
