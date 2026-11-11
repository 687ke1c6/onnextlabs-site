-- Accounts created on first sign-in via an OpenID Connect provider (Google for now).
-- Signing in grants nothing by itself: access comes from user_roles.
CREATE TABLE users (
    id             INTEGER PRIMARY KEY,
    -- Identity = (issuer, subject): the provider's stable ID for this person.
    -- Email can change, so it's stored for display/lookup only.
    issuer         TEXT    NOT NULL,
    subject        TEXT    NOT NULL,
    email          TEXT    COLLATE NOCASE,
    email_verified INTEGER NOT NULL DEFAULT 0,
    name           TEXT,
    picture        TEXT,
    created_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_login_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (issuer, subject)
) STRICT;

CREATE INDEX users_email ON users (email);

-- Granted out-of-band (`onnextlabs-site grant-role <email> admin`). Placeholder for real RBAC.
CREATE TABLE user_roles (
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role       TEXT    NOT NULL,
    granted_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (user_id, role)
) STRICT;

-- Server-side sessions. The cookie holds a random token; only its SHA-256 is stored,
-- so a leaked database can't be used to hijack sessions.
CREATE TABLE sessions (
    token_hash BLOB    PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    ip         TEXT,
    user_agent TEXT,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at TEXT    NOT NULL
) STRICT;

CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE INDEX sessions_expires_at ON sessions (expires_at);
