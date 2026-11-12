import { json, send } from "~/lib/orgs";

/** Permission keys, fixed by the server (src/rbac.rs). */
export type Permission = "read-docs" | "write-docs" | "manage-orgs" | "write-roles";

export type PermissionInfo = { key: Permission; description: string };

/** A named set of permissions, shared by every org. */
export type Role = {
  id: number;
  name: string;
  description: string;
  permissions: Permission[];
  /** Memberships (across all orgs) that use this role. */
  member_count: number;
};

export type RoleInput = { name: string; description: string; permissions: Permission[] };

/** Every role, plus the permissions a role can be given. */
export const fetchRoles = async () =>
  json<{ permissions: PermissionInfo[]; roles: Role[] }>(await fetch("/api/roles"), "Couldn't load roles.");

export const createRole = async (role: RoleInput) =>
  json<Role>(await send("POST", "/api/roles", role), "Couldn't create the role.");

export const updateRole = async (id: number, role: RoleInput) =>
  json<Role>(await send("PUT", `/api/roles/${id}`, role), "Couldn't save the role.");

export const deleteRole = async (id: number) => json<void>(await send("DELETE", `/api/roles/${id}`), "Couldn't delete the role.");
