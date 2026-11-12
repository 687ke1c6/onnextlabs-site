import type { Permission } from "~/lib/roles";

/** An organisation the signed-in user can open. */
export type Org = {
  id: number;
  slug: string;
  name: string;
  /** The user's role in the org; null when an admin opens an org they aren't a member of. */
  role: { id: number; name: string } | null;
  /** What the user may do here (everything for admins). Only for showing or hiding UI: the
   * API checks again on every request. */
  permissions: Permission[];
};

export const can = (org: Org | undefined, permission: Permission) => !!org?.permissions.includes(permission);

export type OrgSummary = { id: number; slug: string; name: string; created_at: string; member_count: number };

export type Member = {
  user_id: number;
  email: string | null;
  name: string | null;
  picture: string | null;
  role_id: number;
  role_name: string;
  added_at: string;
};

export async function json<T>(res: Response, fallback: string): Promise<T> {
  if (res.ok) return res.status === 204 ? (undefined as T) : res.json();
  let msg = fallback;
  try {
    const body = await res.json();
    if (typeof body?.error === "string") msg = body.error;
  } catch {
    // not JSON
  }
  throw new Error(msg);
}

export const send = (method: string, url: string, body?: unknown) =>
  fetch(url, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });

/** Orgs the user can open: their memberships, or every org for admins. */
export const fetchMyOrgs = async () => json<Org[]>(await fetch("/api/orgs"), "Couldn't load your organisations.");

// ---- managing orgs ----

/** Orgs the user can manage (`manage-orgs`; every org for admins). */
export const fetchManageableOrgs = async () =>
  json<OrgSummary[]>(await fetch("/api/manage/orgs"), "Couldn't load organisations.");

/** Admins only. */
export const createOrg = async (name: string, slug: string) =>
  json<Org>(await send("POST", "/api/manage/orgs", { name, slug }), "Couldn't create the organisation.");

const membersUrl = (org: string) => `/api/orgs/${encodeURIComponent(org)}/members`;

export const fetchMembers = async (org: string) => json<Member[]>(await fetch(membersUrl(org)), "Couldn't load members.");

export const addMember = async (org: string, userId: number, roleId: number) =>
  json<Member[]>(
    await send("POST", membersUrl(org), { user_id: userId, role_id: roleId }),
    "Couldn't add that member.",
  );

export const changeMemberRole = async (org: string, userId: number, roleId: number) =>
  json<Member[]>(await send("PATCH", `${membersUrl(org)}/${userId}`, { role_id: roleId }), "Couldn't change their role.");

export const removeMember = async (org: string, userId: number) =>
  json<void>(await send("DELETE", `${membersUrl(org)}/${userId}`), "Couldn't remove that member.");

/** "World Health Organisation" → "world-health-organisation" (accents folded to ASCII). */
export function slugify(name: string): string {
  return name
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 63)
    .replace(/-+$/, "");
}

export const SLUG_PATTERN = /^[a-z0-9]+(-[a-z0-9]+)*$/;
