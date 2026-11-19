import { createMemo, createResource, createSignal, For, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Link } from "@tanstack/solid-router";
import FileX from "lucide-solid/icons/file-x";
import Hand from "lucide-solid/icons/hand";
import Paperclip from "lucide-solid/icons/paperclip";
import Plus from "lucide-solid/icons/plus";
import Search from "lucide-solid/icons/search";
import X from "lucide-solid/icons/x";
import { FileTypeIcon } from "~/components/FileTypeIcon";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { fetchAllDocuments } from "~/lib/documents";
import { can } from "~/lib/orgs";
import {
  accountLabel,
  attachDocument,
  detachDocument,
  fetchRow,
  formatAmount,
  formatDate,
  isNegative,
  type RowDetail,
} from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { useOrg } from "~/routes/portal/org/OrgLayout";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);
const downloadUrl = (documentId: number) => `/api/documents/${documentId}/download`;
/** "Documents / Statements / 123" for a folder path ("" is the top level). */
const folderLabel = (folder: string) => `Documents / ${folder ? folder.split("/").join(" / ") : "(top level)"}`;

/** What the panel reports back to the grid after a change, so its row stays in step. */
export type RowChange = { id: number; type: string | null; type_manual: boolean; attachments: number };

/**
 * A row's details in a panel at the side of the window: what it is, its type (editable), the
 * statement(s) it came from, and the documents attached to it (attach from existing Documents,
 * or remove the link). Escape or ✕ closes it.
 */
export function RowPanel(props: {
  rowId: number;
  types: string[];
  /** Saves a row's type by hand (null clears it). */
  setType: (rowId: number, type: string | null) => Promise<void>;
  onChanged: (change: RowChange) => void;
  onClose: () => void;
}) {
  const org = useOrg();
  const canEdit = can(org, "write-statements");
  const canAttach = canEdit && can(org, "read-docs");
  const [row, { mutate, refetch }] = createResource(() => props.rowId, (id) => fetchRow(org.slug, id));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [picking, setPicking] = createSignal(false);

  const report = (r: RowDetail) =>
    props.onChanged({ id: r.id, type: r.type, type_manual: r.type_manual, attachments: r.documents.length });

  const run = async (fn: () => Promise<RowDetail>, fallback: string) => {
    setBusy(true);
    setError(null);
    try {
      const r = await fn();
      mutate(r);
      report(r);
    } catch (e) {
      setError(errorText(e, fallback));
    } finally {
      setBusy(false);
    }
  };

  const changeType = async (type: string | null) => {
    setBusy(true);
    setError(null);
    try {
      await props.setType(props.rowId, type);
      await refetch();
    } catch (e) {
      setError(errorText(e, "Couldn't change the type."));
    } finally {
      setBusy(false);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !e.defaultPrevented) props.onClose();
  };
  document.addEventListener("keydown", onKey);
  onCleanup(() => document.removeEventListener("keydown", onKey));

  return (
    <Portal>
      <aside
        class="fixed inset-y-0 right-0 z-40 flex w-full max-w-md flex-col border-l border-border bg-background text-foreground shadow-2xl"
        aria-label="Row details"
      >
        <header class="flex items-center justify-between gap-3 border-b border-border px-5 py-3">
          <h2 class="text-sm font-medium">Row details</h2>
          <button
            type="button"
            onClick={props.onClose}
            class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Close"
            title="Close (Esc)"
          >
            <X size={18} />
          </button>
        </header>

        <div class="flex-1 overflow-y-auto px-5 py-4">
          <Show when={row.error}>
            <p class="text-sm text-red-500">{errorText(row.error, "Couldn't load the row.")}</p>
          </Show>
          <Show when={!settled(row) && row.loading}>
            <div class="h-40 animate-pulse rounded-lg bg-secondary/40" />
          </Show>

          <Show when={settled(row)}>
            {(r) => (
              <div class="flex flex-col gap-6">
                {/* What it is */}
                <section>
                  <p class="whitespace-pre-wrap break-words text-base font-medium">{r().description}</p>
                  <p
                    class={cn(
                      "mt-1 text-xl font-semibold tabular-nums",
                      !isNegative(r().amount) && "text-emerald-600 dark:text-emerald-400",
                    )}
                  >
                    {formatAmount(r().amount)} <span class="text-sm font-normal text-muted-foreground">{r().currency}</span>
                  </p>
                  <dl class="mt-3 grid grid-cols-2 gap-3 text-sm">
                    <div>
                      <dt class="text-xs text-muted-foreground">Date</dt>
                      <dd>{formatDate(r().date)}</dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">Month</dt>
                      <dd>{r().month}</dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">Account</dt>
                      <dd>{accountLabel(r().account_number)}</dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">Id</dt>
                      <dd class="font-mono text-xs">{r().txn_id}</dd>
                    </div>
                  </dl>
                </section>

                {/* Type */}
                <section>
                  <label for="row-type" class="text-xs text-muted-foreground">Type</label>
                  <div class="mt-1 flex items-center gap-2">
                    <select
                      id="row-type"
                      value={r().type ?? ""}
                      onChange={(e) => void changeType(e.currentTarget.value || null)}
                      disabled={!canEdit || busy()}
                      class="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
                    >
                      <option value="">No type</option>
                      <Show when={r().type && !props.types.includes(r().type!)}>
                        <option value={r().type!}>{r().type}</option>
                      </Show>
                      <For each={props.types}>{(t) => <option value={t}>{t}</option>}</For>
                    </select>
                  </div>
                  <p class="mt-1 flex items-center gap-1 text-xs text-muted-foreground">
                    <Show
                      when={r().type_manual}
                      fallback={r().type ? "Set by a type rule. Pick one to set it by hand." : "No rule matched."}
                    >
                      <Hand size={11} /> Set by hand: saving a statement won't change it.
                    </Show>
                  </p>
                </section>

                {/* Where it came from */}
                <section>
                  <h3 class="text-xs text-muted-foreground">From {r().uploads.length === 1 ? "statement" : "statements"}</h3>
                  <ul class="mt-1 flex flex-col gap-1">
                    <For each={r().uploads}>
                      {(u) => (
                        <li class="flex items-center gap-2 text-sm">
                          <Link
                            to="/portal/$org/statements/$upload"
                            params={{ org: org.slug, upload: String(u.id) }}
                            class="min-w-0 flex-1 truncate underline-offset-2 hover:underline"
                          >
                            {u.filename}
                          </Link>
                          <span class="shrink-0 text-xs text-muted-foreground">{formatDate(u.uploaded_at)}</span>
                          <Show when={u.document_id && can(org, "read-docs")}>
                            <a href={downloadUrl(u.document_id!)} class="shrink-0 text-xs text-muted-foreground underline hover:text-foreground">
                              File
                            </a>
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </section>

                {/* Attached documents */}
                <section>
                  <div class="flex items-center justify-between gap-2">
                    <h3 class="flex items-center gap-1.5 text-xs text-muted-foreground">
                      <Paperclip size={12} /> Documents
                    </h3>
                    <Show when={canAttach && !picking()}>
                      <Button size="sm" variant="outline" class="h-7" onClick={() => setPicking(true)} disabled={busy()}>
                        <Plus size={14} /> Attach existing
                      </Button>
                    </Show>
                  </div>
                  <ul class="mt-2 flex flex-col gap-1">
                    <For
                      each={r().documents}
                      fallback={<li class="text-sm text-muted-foreground">None attached.</li>}
                    >
                      {(d) => (
                        <li class="flex items-center gap-2 rounded-md px-1 py-1 text-sm hover:bg-accent/40">
                          <Show
                            when={!d.missing}
                            fallback={<FileX size={16} class="shrink-0 text-red-500" aria-hidden="true" />}
                          >
                            <FileTypeIcon filename={d.filename} contentType={d.content_type} size={16} />
                          </Show>
                          <div class="min-w-0 flex-1">
                            <Show
                              when={!d.missing && can(org, "read-docs")}
                              fallback={<span class={cn("block truncate", d.missing && "text-muted-foreground line-through")}>{d.filename}</span>}
                            >
                              <a href={downloadUrl(d.document_id)} class="block truncate underline-offset-2 hover:underline" title="Download">
                                {d.filename}
                              </a>
                            </Show>
                            <span class="block truncate text-xs text-muted-foreground">
                              {d.missing ? "File missing · " : ""}
                              {folderLabel(d.folder)}
                            </span>
                          </div>
                          <Show when={canAttach}>
                            <button
                              type="button"
                              onClick={() => void run(() => detachDocument(org.slug, props.rowId, d.document_id), "Couldn't remove the document.")}
                              disabled={busy()}
                              class="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
                              aria-label={`Remove ${d.filename} from this row`}
                              title="Remove from this row (the document stays in Documents)"
                            >
                              <X size={14} />
                            </button>
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                  <Show when={picking()}>
                    <DocumentPicker
                      exclude={r().documents.map((d) => d.document_id)}
                      busy={busy()}
                      onPick={(id) => void run(() => attachDocument(org.slug, props.rowId, id), "Couldn't attach the document.")}
                      onClose={() => setPicking(false)}
                    />
                  </Show>
                </section>

                <Show when={error()}>
                  <p role="alert" class="text-sm text-red-500">{error()}</p>
                </Show>
              </div>
            )}
          </Show>
        </div>
      </aside>
    </Portal>
  );
}

/** Search the org's Documents and pick one to attach. */
function DocumentPicker(props: { exclude: number[]; busy: boolean; onPick: (id: number) => void; onClose: () => void }) {
  const org = useOrg();
  const [docs] = createResource(() => org.slug, fetchAllDocuments);
  const [query, setQuery] = createSignal("");
  const shown = createMemo(() => {
    const q = query().trim().toLowerCase();
    return (settled(docs) ?? [])
      .filter((d) => !props.exclude.includes(d.id))
      .filter((d) => !q || `${d.folder}/${d.filename}`.toLowerCase().includes(q))
      .slice(0, 100);
  });

  return (
    <div class="mt-3 rounded-lg border border-border p-3">
      <div class="flex items-center gap-2">
        <div class="relative min-w-0 flex-1">
          <Search size={14} class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <Input
            class="h-8 pl-8 text-sm"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault(); // close the picker, not the panel
                props.onClose();
              }
            }}
            placeholder="Find a document"
            aria-label="Find a document"
            ref={(el) => queueMicrotask(() => el.focus())}
          />
        </div>
        <Button size="sm" variant="ghost" class="h-8" onClick={props.onClose}>
          Done
        </Button>
      </div>
      <Show when={docs.error}>
        <p class="mt-2 text-sm text-red-500">{errorText(docs.error, "Couldn't load documents.")}</p>
      </Show>
      <ul class="mt-2 max-h-64 overflow-y-auto">
        <Show when={docs.loading}>
          <li class="px-2 py-1.5 text-sm text-muted-foreground">Loading…</li>
        </Show>
        <For each={shown()} fallback={<Show when={settled(docs)}><li class="px-2 py-1.5 text-sm text-muted-foreground">No documents match.</li></Show>}>
          {(d) => (
            <li>
              <button
                type="button"
                onClick={() => props.onPick(d.id)}
                disabled={props.busy}
                class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:bg-accent focus-visible:outline-none disabled:opacity-50"
              >
                <FileTypeIcon filename={d.filename} contentType={d.content_type} size={16} />
                <span class="min-w-0 flex-1">
                  <span class="block truncate">{d.filename}</span>
                  <span class="block truncate text-xs text-muted-foreground">{folderLabel(d.folder)}</span>
                </span>
                <Plus size={14} class="shrink-0 text-muted-foreground" />
              </button>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}
