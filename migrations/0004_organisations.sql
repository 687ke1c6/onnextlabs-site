-- Organisations: the top-level scope for everything in the portal. Each org's files live
-- under $DATA_DIR/orgs/<slug>/ (e.g. .../documents for the document store).
CREATE TABLE organisations (
    id         INTEGER PRIMARY KEY,
    -- URL segment and folder name, e.g. world-health-organisation. Fixed after creation.
    slug       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Who belongs to which org. Membership is what lets a non-admin into the portal.
CREATE TABLE org_members (
    org_id   INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    user_id  INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Org-scoped role. Only 'member' exists so far; finer roles/permissions come later.
    role     TEXT    NOT NULL DEFAULT 'member',
    added_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (org_id, user_id)
) STRICT;

CREATE INDEX org_members_user_id ON org_members (user_id);

-- Documents now belong to an org. Earlier uploads were test data with no org: dropped.
DROP TABLE documents;
CREATE TABLE documents (
    id           INTEGER PRIMARY KEY,
    org_id       INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    -- Path relative to the org's documents folder, '/'-separated; '' is the top level.
    folder       TEXT    NOT NULL,
    filename     TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    -- As reported by the uploading browser.
    content_type TEXT,
    -- Hex SHA-256 of the contents.
    sha256       TEXT    NOT NULL,
    uploaded_by  INTEGER REFERENCES users (id) ON DELETE SET NULL,
    uploaded_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- 1 = anyone with the link can download; 0 = admins only.
    public       INTEGER NOT NULL DEFAULT 1,
    UNIQUE (org_id, folder, filename)
) STRICT;

-- The global 'portal' role is replaced by org membership.
DELETE FROM user_roles WHERE role = 'portal';
