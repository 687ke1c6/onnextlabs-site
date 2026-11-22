-- Quick filters: named filters shown as toggles above the statement grid, per organisation (the
-- same for every account). They're in groups: toggled filters in one group are combined with OR,
-- groups with AND. A "yearly" group has no filters of its own: it offers one per year (e.g. tax
-- years starting 1 March), worked out from the rows. Saved as a whole list (statement_filters.rs).

CREATE TABLE statement_filter_groups (
    id          INTEGER PRIMARY KEY,
    org_id      INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    name        TEXT    NOT NULL,
    -- 'filters' (its own filters) or 'yearly' (one per year from start_month/start_day).
    kind        TEXT    NOT NULL CHECK (kind IN ('filters', 'yearly')),
    start_month INTEGER,
    start_day   INTEGER,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX statement_filter_groups_org ON statement_filter_groups (org_id, position);

-- A filter matches a row when every condition it has holds. Dates are compared with the row's
-- effective date, or its date when it has none.
CREATE TABLE statement_quick_filters (
    id          INTEGER PRIMARY KEY,
    org_id      INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    group_id    INTEGER NOT NULL REFERENCES statement_filter_groups (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    name        TEXT    NOT NULL,
    -- YYYY-MM-DD, inclusive.
    date_from   TEXT,
    date_to     TEXT,
    -- JSON array of type names (the list's spelling); any of them. Renames follow (statement_types.rs).
    types       TEXT    NOT NULL DEFAULT '[]',
    -- JSON array of strings; the description contains any of them (ignoring case).
    contains    TEXT    NOT NULL DEFAULT '[]',
    -- Inclusive.
    min_amount  REAL,
    max_amount  REAL
) STRICT;

CREATE INDEX statement_quick_filters_group ON statement_quick_filters (group_id, position);
