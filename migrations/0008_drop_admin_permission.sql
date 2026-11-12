-- The `admin` permission is gone: org roles no longer grant site-wide admin (that comes only
-- from ADMIN_EMAILS or the `grant-role` CLI). Holders of the Admin role (which carried it)
-- move to Admin-org, which has the same org permissions; then the Admin role is removed.
UPDATE org_members SET role_id = (SELECT id FROM roles WHERE name = 'Admin-org')
    WHERE role_id = (SELECT id FROM roles WHERE name = 'Admin')
      AND EXISTS (SELECT 1 FROM role_permissions p
                  JOIN roles r ON r.id = p.role_id
                  WHERE r.name = 'Admin' AND p.permission = 'admin')
      AND EXISTS (SELECT 1 FROM roles WHERE name = 'Admin-org');
DELETE FROM role_permissions WHERE permission = 'admin';
DELETE FROM roles
    WHERE name = 'Admin'
      AND description = 'Full access to everything, in every organisation.'
      AND NOT EXISTS (SELECT 1 FROM org_members m WHERE m.role_id = roles.id);
