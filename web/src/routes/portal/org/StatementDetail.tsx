import { createResource, createSignal, Show } from "solid-js";
import { Link, useNavigate, useParams } from "@tanstack/solid-router";
import ArrowLeft from "lucide-solid/icons/arrow-left";
import Download from "lucide-solid/icons/download";
import Trash from "lucide-solid/icons/trash";
import { Button } from "~/components/ui/button";
import { can } from "~/lib/orgs";
import { deleteUpload, fetchUpload, formatDate, formatPeriod } from "~/lib/statements";
import { settled } from "~/lib/utils";
import { Allowed, PageHeader } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { GoogleSheetPanel } from "~/routes/portal/org/GoogleSheetPanel";
import { Fact, TransactionsTable } from "~/routes/portal/org/StatementParts";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);

function Detail() {
  const org = useOrg();
  const params = useParams({ from: "/portal/$org/statements/$upload" });
  const navigate = useNavigate();
  const id = () => Number(params().upload);
  const [upload] = createResource(id, (n) => fetchUpload(org.slug, n));
  const [deleting, setDeleting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const remove = async () => {
    const u = settled(upload);
    if (!u) return;
    const msg =
      `Delete ${u.filename}? Rows that no other saved statement contains are deleted too.` +
      (u.document_id ? " The copy in Documents stays." : "");
    if (!confirm(msg)) return;
    setDeleting(true);
    setError(null);
    try {
      await deleteUpload(org.slug, u.id);
      void navigate({ to: "/portal/$org/statements", params: { org: org.slug } });
    } catch (e) {
      setError(errorText(e, "Couldn't delete the statement."));
      setDeleting(false);
    }
  };

  return (
    <>
      <Link
        to="/portal/$org/statements"
        params={{ org: org.slug }}
        class="mb-4 inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft size={14} /> Statements
      </Link>

      <Show when={upload.error}>
        <p class="text-sm text-red-500">{errorText(upload.error, "Couldn't load this statement.")}</p>
      </Show>

      <Show when={settled(upload)}>
        {(u) => (
          <>
            <PageHeader
              title={u().filename}
              description={`${u().format_name}, saved ${formatDate(u().uploaded_at)}${u().uploaded_by ? ` by ${u().uploaded_by}` : ""}`}
              actions={
                <div class="flex flex-wrap gap-2">
                  <Show when={u().document_id}>
                    {(docId) => (
                      <a
                        href={`/api/documents/${docId()}/download`}
                        class="inline-flex h-9 items-center gap-2 rounded-md border border-input px-3 text-sm font-medium hover:bg-accent hover:text-accent-foreground"
                      >
                        <Download size={16} /> Original file
                      </a>
                    )}
                  </Show>
                  <Show when={can(org, "write-statements")}>
                    <Button size="sm" variant="outline" onClick={remove} disabled={deleting()}>
                      <Trash size={16} /> Delete
                    </Button>
                  </Show>
                </div>
              }
            />
            <Show when={error()}>
              <p role="alert" class="mt-4 text-sm text-red-500">{error()}</p>
            </Show>

            <dl class="mt-6 grid grid-cols-2 gap-4 sm:grid-cols-4">
              <Fact label="Account" value={u().account_number || "Not stated"} />
              <Fact label="Period" value={formatPeriod(u().first_date, u().last_date)} />
              <Fact label="Rows" value={u().rows_parsed} />
              <Fact label="When saved" value={`${u().rows_new} new, ${u().rows_updated} already saved`} />
            </dl>

            <GoogleSheetPanel org={org.slug} upload={u().id} />

            <section class="mt-8">
              <h2 class="mb-3 text-sm font-medium">Rows</h2>
              <TransactionsTable rows={u().transactions} />
            </section>
          </>
        )}
      </Show>
    </>
  );
}

/** /portal/<org>/statements/<id>. Needs `read-statements`. */
export function StatementDetailPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <Detail />
    </Allowed>
  );
}
