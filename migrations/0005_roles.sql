-- Org roles. A role is a named set of permissions, defined once and shared by every org;
-- each org membership points at one role. The permission keys are fixed in code
-- (src/rbac.rs); unknown keys are ignored. The global 'admin' user role still overrides all.
CREATE TABLE roles (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT    NOT NULL DEFAULT '',
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE role_permissions (
    role_id    INTEGER NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    permission TEXT    NOT NULL,
    PRIMARY KEY (role_id, permission)
) STRICT;

INSERT INTO roles (id, name, description) VALUES
    (1, 'Admin', 'Full control of the organisation.'),
    (2, 'Member', 'Can browse and download documents.');
INSERT INTO role_permissions (role_id, permission) VALUES
    (1, 'read-docs'), (1, 'write-docs'), (1, 'manage-orgs'), (1, 'write-roles'),
    (2, 'read-docs');

-- Memberships now carry a role id instead of free text. Everyone existing becomes a Member.
-- RESTRICT: a role can't be deleted while anyone holds it.
CREATE TABLE org_members_new (
    org_id   INTEGER NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    user_id  INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role_id  INTEGER NOT NULL REFERENCES roles (id) ON DELETE RESTRICT,
    added_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (org_id, user_id)
) STRICT;
INSERT INTO org_members_new (org_id, user_id, role_id, added_at)
    SELECT org_id, user_id, 2, added_at FROM org_members;
DROP TABLE org_members;
ALTER TABLE org_members_new RENAME TO org_members;
CREATE INDEX org_members_user_id ON org_members (user_id);
CREATE INDEX org_members_role_id ON org_members (role_id);
