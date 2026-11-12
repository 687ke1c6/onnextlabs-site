-- "Admin" is the site-wide role (has the `admin` permission); "Admin-org" runs one
-- organisation. Skipped where a name has already been changed or taken.
UPDATE roles SET name = 'Admin-org', description = 'Full control of the organisation.'
    WHERE name = 'Admin'
      AND NOT EXISTS (SELECT 1 FROM role_permissions p WHERE p.role_id = roles.id AND p.permission = 'admin')
      AND NOT EXISTS (SELECT 1 FROM roles r WHERE r.name = 'Admin-org');
UPDATE roles SET name = 'Admin', description = 'Full access to everything, in every organisation.'
    WHERE name = 'Site admin'
      AND NOT EXISTS (SELECT 1 FROM roles r WHERE r.name = 'Admin');
