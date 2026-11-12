export type User = {
  id: number;
  email: string | null;
  name: string | null;
  picture: string | null;
  /** Admins can do everything, in every organisation. */
  admin: boolean;
  /** Why they're an admin: their account's admin flag (set by an admin, or `grant-admin`),
   * and/or ADMIN_EMAILS. Empty when they aren't one. */
  admin_via?: ("flag" | "admin-emails")[];
};

/** The signed-in user, or null if there's no valid session. Throws on network/server errors. */
export async function fetchMe(): Promise<User | null> {
  const res = await fetch("/api/auth/me", { credentials: "same-origin" });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`/api/auth/me: ${res.status}`);
  return res.json();
}

export async function fetchProviders(): Promise<string[]> {
  const res = await fetch("/api/auth/providers");
  return res.ok ? res.json() : [];
}

export async function logout(): Promise<void> {
  await fetch("/api/auth/logout", { method: "POST", credentials: "same-origin" });
}

/** Full-page navigation: the OAuth flow leaves the SPA and comes back via redirects. */
export const loginUrl = (provider: string, returnTo?: string) =>
  `/api/auth/login/${provider}` + (returnTo ? `?return_to=${encodeURIComponent(returnTo)}` : "");

/** Admins can do everything, in every organisation. */
export const isAdmin = (user: User) => user.admin;
