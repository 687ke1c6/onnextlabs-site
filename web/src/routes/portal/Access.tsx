import { createResource, For, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import Building2 from "lucide-solid/icons/building";
import ShieldCheck from "lucide-solid/icons/shield-check";
import { Badge } from "~/components/ui/badge";
import { isAdmin } from "~/lib/auth";
import { fetchRoles } from "~/lib/roles";
import { settled } from "~/lib/utils";
import { PageHeader, usePortal } from "~/routes/portal/Layout";

const ADMIN_REASONS = {
  flag: "Your account is marked as an admin (Manage → Users).",
  "admin-emails": "Your email address is in ADMIN_EMAILS.",
} as const;

/** /portal/access: the signed-in user's roles and what they allow. */
export function AccessPage() {
  const portal = usePortal();
  const admin = isAdmin(portal.user);
  // Role definitions, for each role's own permissions (admins hold everything anyway, so
  // the org list's permissions wouldn't say what the role itself gives).
  const [roles] = createResource(fetchRoles);
  const memberships = () => portal.orgs().filter((o) => o.role);

  const roleDetails = (id: number) => settled(roles)?.roles.find((r) => r.id === id);

  return (
    <>
      <PageHeader title="Your access" description="Your role in each organisation, and what it lets you do." />

      <Show when={admin}>
        <section class="mt-8 flex items-start gap-3 rounded-xl border border-ring/60 bg-accent/30 p-4">
          <ShieldCheck size={20} class="mt-0.5 shrink-0" />
          <div class="min-w-0">
            <h2 class="text-sm font-medium">Admin</h2>
            <p class="mt-0.5 text-sm text-muted-foreground">
              You can do everything, in every organisation, whatever your roles below say.
            </p>
            <ul class="mt-2 list-disc pl-5 text-xs text-muted-foreground">
              <For each={portal.user.admin_via ?? []}>{(why) => <li>{ADMIN_REASONS[why]}</li>}</For>
            </ul>
          </div>
        </section>
      </Show>

      <h2 class="mt-8 text-sm font-medium">Your organisations</h2>
      <Show
        when={memberships().length > 0}
        fallback={
          <p class="mt-2 text-sm text-muted-foreground">
            You aren't a member of any organisation{admin ? ", but as an admin you can open them all" : ""}.
          </p>
        }
      >
        <ul class="mt-3 divide-y divide-border rounded-xl border border-border">
          <For each={memberships()}>
            {(org) => {
              const role = () => roleDetails(org.role!.id);
              return (
                <li class="flex flex-col gap-2 p-4 sm:flex-row sm:items-start sm:gap-4">
                  <div class="flex min-w-0 items-center gap-3 sm:w-64 sm:shrink-0">
                    <Building2 size={18} class="shrink-0 text-muted-foreground" />
                    <div class="min-w-0">
                      <Link to="/portal/$org" params={{ org: org.slug }} class="block truncate text-sm font-medium hover:underline">
                        {org.name}
                      </Link>
                      <p class="truncate text-xs text-muted-foreground">{org.role!.name}</p>
                    </div>
                  </div>
                  <div class="flex min-w-0 flex-1 flex-wrap gap-1.5">
                    <Show when={role()} fallback={<span class="h-6 w-40 animate-pulse rounded-full bg-secondary/60" />}>
                      {(r) => (
                        <For each={r().permissions} fallback={<span class="text-xs text-muted-foreground">No permissions</span>}>
                          {(p) => <Badge class="font-mono">{p}</Badge>}
                        </For>
                      )}
                    </Show>
                  </div>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
      <Show when={admin && memberships().length > 0}>
        <p class="mt-3 text-xs text-muted-foreground">
          As an admin you can also open every organisation you aren't a member of.
        </p>
      </Show>
    </>
  );
}
