-- Uploaded documents. The file itself lives at $DOCUMENTS_DIR/<folder>/<filename>; this row
-- is its metadata. Folders are plain directories on disk and have no row of their own.
CREATE TABLE documents (
    id           INTEGER PRIMARY KEY,
    -- Path relative to $DOCUMENTS_DIR, '/'-separated; '' is the top level.
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
    UNIQUE (folder, filename)
) STRICT;
