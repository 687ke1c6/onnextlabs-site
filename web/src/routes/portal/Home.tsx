import { createMemo, createSignal, For, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import Building2 from "lucide-solid/icons/building";
import ChevronRight from "lucide-solid/icons/chevron-right";
import FileText from "lucide-solid/icons/file-text";
import Landmark from "lucide-solid/icons/landmark";
import Search from "lucide-solid/icons/search";
import { buttonVariants } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { isAdmin } from "~/lib/auth";
import { can, type Org } from "~/lib/orgs";
import { cn } from "~/lib/utils";
import { PageHeader, usePortal } from "~/routes/portal/Layout";

/** Show a search box once there are more organisations than this. */
const SEARCH_FROM = 6;

/** "ONNEXT LABS" → "OL", "Acme" → "AC". */
function initials(name: string): string {
  const [first = "", second] = name.match(/[\p{L}\p{N}]+/gu) ?? [];
  if (!first) return "?";
  return (second ? first.charAt(0) + second.charAt(0) : first.slice(0, 2)).toUpperCase();
}

// Literal class names, so Tailwind keeps them. Picked from the slug, so an org keeps its colour.
const SWATCHES = [
  "bg-sky-500/15 text-sky-700 dark:text-sky-300",
  "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  "bg-violet-500/15 text-violet-700 dark:text-violet-300",
  "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  "bg-rose-500/15 text-rose-700 dark:text-rose-300",
  "bg-teal-500/15 text-teal-700 dark:text-teal-300",
  "bg-indigo-500/15 text-indigo-700 dark:text-indigo-300",
  "bg-orange-500/15 text-orange-700 dark:text-orange-300",
];

function swatch(slug: string): string {
  let h = 0;
  for (const ch of slug) h = (h * 31 + ch.charCodeAt(0)) | 0;
  return SWATCHES[Math.abs(h) % SWATCHES.length];
}

const SECTION_LINK =
  "relative z-10 inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function OrgCard(props: { org: Org }) {
  const org = props.org;
  const sections = () => can(org, "read-docs") || can(org, "read-statements");
  return (
    // The whole card opens the org (a stretched link); the section links sit above it.
    <div class="group relative flex flex-col rounded-xl border border-border bg-secondary/20 p-4 transition-colors focus-within:border-ring hover:border-ring hover:bg-accent/40">
      <div class="flex items-center gap-3">
        <div class={cn("flex h-10 w-10 shrink-0 items-center justify-center rounded-lg text-sm font-semibold", swatch(org.slug))} aria-hidden="true">
          {initials(org.name)}
        </div>
        <div class="min-w-0 flex-1">
          <Link
            to="/portal/$org"
            params={{ org: org.slug }}
            class="block truncate text-sm font-medium after:absolute after:inset-0 after:rounded-xl focus-visible:outline-none"
          >
            {org.name}
          </Link>
          <p class="truncate text-xs text-muted-foreground">
            <Show when={org.role} fallback="Admin access">
              {(role) => role().name}
            </Show>
          </p>
        </div>
        <ChevronRight size={16} class="shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
      </div>
      <Show when={sections()}>
        <div class="-mb-1 -ml-2 mt-3 flex flex-wrap gap-1">
          <Show when={can(org, "read-docs")}>
            <Link to="/portal/$org/documents" params={{ org: org.slug }} class={SECTION_LINK}>
              <FileText size={14} /> Documents
            </Link>
          </Show>
          <Show when={can(org, "read-statements")}>
            <Link to="/portal/$org/statements" params={{ org: org.slug }} class={SECTION_LINK}>
              <Landmark size={14} /> Statements
            </Link>
          </Show>
        </div>
      </Show>
    </div>
  );
}

function OrgGroup(props: { title?: string; hint?: string; orgs: Org[] }) {
  return (
    <Show when={props.orgs.length > 0}>
      <section class="mt-8">
        <Show when={props.title}>
          <div class="mb-3">
            <h2 class="text-sm font-medium">{props.title}</h2>
            <Show when={props.hint}>
              <p class="text-xs text-muted-foreground">{props.hint}</p>
            </Show>
          </div>
        </Show>
        <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          <For each={props.orgs}>{(org) => <OrgCard org={org} />}</For>
        </div>
      </section>
    </Show>
  );
}

/** /portal: the organisations the user can open. */
export function PortalHomePage() {
  const portal = usePortal();
  const admin = () => isAdmin(portal.user);
  const canManage = () => portal.canAnywhere("manage-orgs");
  const [query, setQuery] = createSignal("");

  const sorted = createMemo(() => [...portal.orgs()].sort((a, b) => a.name.localeCompare(b.name)));
  const shown = createMemo(() => {
    const q = query().trim().toLowerCase();
    return q ? sorted().filter((o) => o.name.toLowerCase().includes(q) || o.slug.includes(q)) : sorted();
  });
  const mine = () => shown().filter((o) => o.role);
  // Only admins see orgs they aren't members of.
  const others = () => shown().filter((o) => !o.role);
  const grouped = () => admin() && sorted().some((o) => o.role) && sorted().some((o) => !o.role);

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
        <Show when={portal.orgs().length > SEARCH_FROM}>
          <div class="relative mt-8 max-w-sm">
            <Search size={16} class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
            <Input
              class="h-9 pl-9"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              placeholder="Find an organisation"
              aria-label="Find an organisation"
            />
          </div>
        </Show>

        <Show
          when={grouped()}
          fallback={<OrgGroup orgs={shown()} />}
        >
          <OrgGroup title="Your organisations" orgs={mine()} />
          <OrgGroup title="Other organisations" hint="You can open these because you're an admin." orgs={others()} />
        </Show>

        <Show when={shown().length === 0}>
          <p class="mt-8 text-sm text-muted-foreground">No organisation matches "{query().trim()}".</p>
        </Show>
      </Show>
    </>
  );
}
