import { Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import ChevronRight from "lucide-solid/icons/chevron-right";
import FileText from "lucide-solid/icons/file-text";
import { can } from "~/lib/orgs";
import { PageHeader, usePortalUser } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";

/** /portal/<slug>: the org's landing page. */
export function OrgDashboardPage() {
  const org = useOrg();
  const user = usePortalUser();
  const firstName = () => (user.name ?? "").split(" ")[0];

  return (
    <>
      <PageHeader title={org.name} description="Organisation dashboard" />

      <section class="mt-8 rounded-xl border border-border bg-secondary/20 p-6">
        <h2 class="text-base font-medium">Welcome{firstName() ? `, ${firstName()}` : ""}</h2>
        <p class="mt-1 text-sm text-muted-foreground">
          More sections will appear here as they're added and as you're given access.
        </p>
      </section>

      <Show when={can(org, "read-docs")}>
        <section class="mt-4 grid gap-3 sm:grid-cols-2">
          <Link
            to="/portal/$org/documents"
            params={{ org: org.slug }}
            class="group flex items-center gap-3 rounded-xl border border-border p-4 transition-colors hover:border-ring hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <FileText size={20} class="shrink-0 text-muted-foreground" />
            <div class="min-w-0 flex-1">
              <p class="text-sm font-medium">Documents</p>
              <p class="text-xs text-muted-foreground">Shared documents and brand assets</p>
            </div>
            <ChevronRight size={16} class="shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
          </Link>
        </section>
      </Show>
    </>
  );
}
