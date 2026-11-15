-- Files found in storage without a row get one when their folder is listed (documents.rs).
-- Their checksum isn't known: reading every such file just to list a folder is too slow.
-- SQLite can't drop NOT NULL in place, so the table is rebuilt (nothing references it).
-- Missing documents can now be deleted, so ids also become AUTOINCREMENT: download links
-- are /api/documents/<id>/download, and a deleted id must never point at another file.
CREATE TABLE documents_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    org_id       INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    -- Path relative to the org's documents folder, '/'-separated; '' is the top level.
    folder       TEXT    NOT NULL,
    filename     TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    -- As reported by the uploading browser; NULL for files found in storage.
    content_type TEXT,
    -- Hex SHA-256 of the contents; NULL for files found in storage.
    sha256       TEXT,
    -- NULL for files found in storage (or when the account is gone).
    uploaded_by  INTEGER REFERENCES users (id) ON DELETE SET NULL,
    -- When it was uploaded, or found in storage.
    uploaded_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- 1 = anyone with the link can download; 0 = members with read-docs only.
    public       INTEGER NOT NULL DEFAULT 1,
    UNIQUE (org_id, folder, filename)
) STRICT;

INSERT INTO documents_new (id, org_id, folder, filename, size, content_type, sha256,
                           uploaded_by, uploaded_at, public)
SELECT id, org_id, folder, filename, size, content_type, sha256, uploaded_by, uploaded_at, public
FROM documents;

DROP TABLE documents;
ALTER TABLE documents_new RENAME TO documents;
