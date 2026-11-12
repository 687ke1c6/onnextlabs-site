import { createResource, For, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import type { LucideIcon } from "lucide-solid";
import Building2 from "lucide-solid/icons/building";
import ChevronRight from "lucide-solid/icons/chevron-right";
import Mail from "lucide-solid/icons/mail";
import Users from "lucide-solid/icons/users";
import { buttonVariants } from "~/components/ui/button";
import { isAdmin } from "~/lib/auth";
import { settled } from "~/lib/utils";
import { PageHeader, usePortal } from "~/routes/portal/Layout";

type Overview = { subscribers: number; users: number };

function Stat(props: { label: string; value?: number; icon: LucideIcon }) {
  return (
    <div class="rounded-xl border border-border bg-secondary/20 p-4">
      <div class="flex items-center justify-between text-muted-foreground">
        <p class="text-xs">{props.label}</p>
        <props.icon size={16} />
      </div>
      <p class="mt-1 text-2xl font-semibold tabular-nums">{props.value ?? "–"}</p>
    </div>
  );
}

/** Site-wide waitlist and account counts: admin-only data. */
function AdminOverview() {
  const [overview] = createResource<Overview>(async () => {
    const res = await fetch("/api/admin/overview");
    if (!res.ok) throw new Error(`overview: ${res.status}`);
    return res.json();
  });
  return (
    <>
      <section class="mt-8 grid gap-4 sm:grid-cols-2">
        <Stat label="Waitlist subscribers" value={settled(overview)?.subscribers} icon={Mail} />
        <Stat label="Accounts" value={settled(overview)?.users} icon={Users} />
      </section>
      <Show when={overview.error}>
        <p class="mt-4 text-sm text-red-500">Couldn't load the overview.</p>
      </Show>
    </>
  );
}

/** /portal: the organisations the user can open. */
export function PortalHomePage() {
  const portal = usePortal();
  const admin = () => isAdmin(portal.user);
  const canManage = () => portal.canAnywhere("manage-orgs");

  return (
    <>
      <PageHeader
        title="Organisations"
        description={admin() ? "You're an admin: every organisation is listed." : "Choose an organisation to open."}
        actions={
          <Show when={canManage()}>
            <Link to="/portal/organisations" class={buttonVariants({ variant: "outline", size: "sm" })}>
              <Building2 size={16} /> Manage
            </Link>
          </Show>
        }
      />

      <Show when={admin()}>
        <AdminOverview />
      </Show>

      <Show
        when={portal.orgs().length > 0}
        fallback={
          <section class="mt-8 flex flex-col items-center justify-center rounded-xl border border-dashed border-border px-6 py-16 text-center">
            <Building2 size={32} class="text-muted-foreground" />
            <h2 class="mt-4 text-base font-medium">No organisations yet</h2>
            <p class="mt-1 max-w-sm text-sm text-muted-foreground">Create one to start adding members and documents.</p>
            <Link to="/portal/organisations" class={buttonVariants({ size: "sm", class: "mt-4" })}>
              Create an organisation
            </Link>
          </section>
        }
      >
        <section class="mt-8 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          <For each={portal.orgs()}>
            {(org) => (
              <Link
                to="/portal/$org"
                params={{ org: org.slug }}
                class="group flex items-center gap-3 rounded-xl border border-border bg-secondary/20 p-4 transition-colors hover:border-ring hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-secondary text-muted-foreground">
                  <Building2 size={20} />
                </div>
                <div class="min-w-0 flex-1">
                  <p class="truncate text-sm font-medium">{org.name}</p>
                  <p class="truncate text-xs text-muted-foreground">/{org.slug}</p>
                </div>
                <ChevronRight size={16} class="shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
              </Link>
            )}
          </For>
        </section>
      </Show>
    </>
  );
}
