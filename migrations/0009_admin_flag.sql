-- Admin is an attribute of an account, not a role: org roles do the fine-grained work.
-- Set by another admin in the portal (Manage → Users) or with `grant-admin`; verified
-- ADMIN_EMAILS addresses are admins regardless (checked live, not stored here).
ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;
UPDATE users SET is_admin = 1 WHERE id IN (SELECT user_id FROM user_roles WHERE role = 'admin');
DROP TABLE user_roles;
