/** An account (created the first time someone signs in). */
export type UserListItem = {
  id: number;
  email: string | null;
  name: string | null;
  picture: string | null;
  last_login_at: string;
};

/** Every account, sorted by name. Needs `manage-orgs` in some org. */
export async function fetchUsers(): Promise<UserListItem[]> {
  const res = await fetch("/api/users");
  if (!res.ok) throw new Error("Couldn't load people.");
  return res.json();
}

/** An account as the admin Users page shows it. */
export type AdminUser = {
  id: number;
  email: string | null;
  name: string | null;
  picture: string | null;
  created_at: string;
  last_login_at: string;
  /** The account's stored admin flag. */
  is_admin: boolean;
  /** Admin because of ADMIN_EMAILS, whatever the flag says; can't be changed here. */
  env_admin: boolean;
  orgs: { slug: string; name: string; role: string }[];
};

async function errorFrom(res: Response, fallback: string): Promise<Error> {
  try {
    const body = await res.json();
    if (typeof body?.error === "string") return new Error(body.error);
  } catch {
    // not JSON
  }
  return new Error(fallback);
}

/** Admins only. */
export async function fetchAdminUsers(): Promise<AdminUser[]> {
  const res = await fetch("/api/admin/users");
  if (!res.ok) throw await errorFrom(res, "Couldn't load people.");
  return res.json();
}

/** Admins only: set or clear an account's admin flag. Returns the updated account. */
export async function setAdmin(id: number, admin: boolean): Promise<AdminUser> {
  const res = await fetch(`/api/admin/users/${id}/admin`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ admin }),
  });
  if (!res.ok) throw await errorFrom(res, "Couldn't change their admin access.");
  return res.json();
}
