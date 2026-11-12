-- A ready-made role carrying the new `admin` permission: giving it to someone in any
-- organisation makes them a site administrator (no ADMIN_EMAILS entry needed).
INSERT OR IGNORE INTO roles (name, description) VALUES
    ('Site admin', 'Full access to everything, in every organisation.');
INSERT OR IGNORE INTO role_permissions (role_id, permission)
    SELECT id, p.permission FROM roles,
        (SELECT 'read-docs' AS permission UNION ALL SELECT 'write-docs'
         UNION ALL SELECT 'manage-orgs' UNION ALL SELECT 'write-roles' UNION ALL SELECT 'admin') p
    WHERE roles.name = 'Site admin';
