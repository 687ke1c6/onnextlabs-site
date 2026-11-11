-- People who asked to be notified at launch ("Notify me" on the home page).
CREATE TABLE subscribers (
    id              INTEGER PRIMARY KEY,
    -- Stored trimmed + lowercased; NOCASE keeps uniqueness case-insensitive regardless.
    email           TEXT NOT NULL UNIQUE COLLATE NOCASE,
    -- Client IP (v4 or v6, text form) and User-Agent of the first signup request.
    ip              TEXT,
    user_agent      TEXT,
    -- UTC timestamps, ISO 8601 with milliseconds.
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Set when someone unsubscribes; cleared if they sign up again. Rows are kept, not deleted.
    unsubscribed_at TEXT
) STRICT;

CREATE INDEX subscribers_created_at ON subscribers (created_at);
