import { createResource, createSignal, For, Show } from "solid-js";
import { Link, useNavigate } from "@tanstack/solid-router";
import FileSpreadsheet from "lucide-solid/icons/file-spreadsheet";
import Loader2 from "lucide-solid/icons/loader-circle";
import UploadCloud from "lucide-solid/icons/cloud-upload";
import X from "lucide-solid/icons/x";
import type { ColumnDef } from "@tanstack/solid-table";
import { DataGrid, dateRangeFilter, distinct, type GridColumnMeta, pickFilter } from "~/components/DataGrid";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { fetchAllFolders } from "~/lib/documents";
import { can } from "~/lib/orgs";
import {
  fetchUploads,
  formatDate,
  formatPeriod,
  type Preview,
  previewStatement,
  saveStatement,
  type UploadSummary,
} from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed, PageHeader } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import {
  AccountPicker,
  Fact,
  StatementsTabs,
  TransactionsTable,
  useStatementAccount,
} from "~/routes/portal/org/StatementParts";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);

/** Drop a statement, see what it holds, then save it (optionally with a copy in Documents). */
function UploadPanel(props: { onSaved: (account: string) => void }) {
  const org = useOrg();
  const navigate = useNavigate();
  let fileInput!: HTMLInputElement;

  const [file, setFile] = createSignal<File | null>(null);
  const [preview, setPreview] = createSignal<Preview | null>(null);
  const [busy, setBusy] = createSignal<"reading" | "saving" | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [dragging, setDragging] = createSignal(false);
  const canSaveDocs = () => can(org, "write-docs");
  const [toDocuments, setToDocuments] = createSignal(false);
  const [folder, setFolder] = createSignal("");
  const [folders] = createResource(() => (toDocuments() ? org.slug : undefined), fetchAllFolders);

  const reset = () => {
    setFile(null);
    setPreview(null);
    setError(null);
    setToDocuments(false);
    if (fileInput) fileInput.value = "";
  };

  const pick = async (files: FileList | null | undefined) => {
    const f = files?.[0];
    if (!f) return;
    setError(null);
    setPreview(null);
    setFile(f);
    setBusy("reading");
    try {
      const p = await previewStatement(org.slug, f);
      setPreview(p);
      setFolder(p.documents_folder);
    } catch (e) {
      setError(errorText(e, "Couldn't read that statement."));
      setFile(null);
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    const f = file();
    if (!f || busy()) return;
    setError(null);
    setBusy("saving");
    try {
      const saved = await saveStatement(org.slug, f, toDocuments(), folder());
      // The Statements pages now show the account this statement belongs to.
      props.onSaved(saved.account_number);
      void navigate({ to: "/portal/$org/statements/$upload", params: { org: org.slug, upload: String(saved.id) } });
    } catch (e) {
      setError(errorText(e, "Couldn't save the statement."));
    } finally {
      setBusy(null);
    }
  };

  // The default folder is offered even before it exists: saving creates it.
  const folderOptions = () => {
    const list = settled(folders) ?? [];
    const def = preview()?.documents_folder;
    return def && !list.includes(def) ? [def, ...list] : list;
  };

  return (
    <section class="mt-6">
      <Show
        when={preview()}
        fallback={
          <>
            <button
              type="button"
              onClick={() => fileInput.click()}
              onDragEnter={(e) => {
                e.preventDefault();
                setDragging(true);
              }}
              onDragOver={(e) => {
                e.preventDefault();
                if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
              }}
              onDragLeave={(e) => {
                if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
              }}
              onDrop={(e) => {
                e.preventDefault();
                setDragging(false);
                void pick(e.dataTransfer?.files);
              }}
              disabled={!!busy()}
              class={cn(
                "flex w-full flex-col items-center justify-center gap-2 rounded-xl border-2 border-dashed px-4 py-10 text-center transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                dragging() ? "border-ring bg-accent/60" : "border-border hover:bg-accent/40",
              )}
            >
              <Show
                when={busy() === "reading"}
                fallback={
                  <>
                    <UploadCloud size={28} class="text-muted-foreground" />
                    <span class="text-sm font-medium">Drop a bank statement here, or click to choose</span>
                    <span class="text-xs text-muted-foreground">CSV only for now (Standard Bank). Nothing is saved until you confirm.</span>
                  </>
                }
              >
                <Loader2 size={28} class="animate-spin text-muted-foreground" />
                <span class="text-sm font-medium">Reading {file()?.name}…</span>
              </Show>
            </button>
            <input
              ref={fileInput}
              type="file"
              accept=".csv,text/csv"
              class="hidden"
              onChange={(e) => void pick(e.currentTarget.files)}
            />
          </>
        }
      >
        {(p) => (
          <div class="rounded-xl border border-border p-5">
            <div class="flex items-start justify-between gap-4">
              <div class="flex min-w-0 items-center gap-3">
                <FileSpreadsheet size={20} class="shrink-0 text-muted-foreground" />
                <div class="min-w-0">
                  <h2 class="truncate text-base font-medium">{p().filename}</h2>
                  <p class="text-xs text-muted-foreground">Review it, then save.</p>
                </div>
              </div>
              <button
                type="button"
                onClick={reset}
                disabled={!!busy()}
                class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
                aria-label="Discard this statement"
                title="Discard"
              >
                <X size={18} />
              </button>
            </div>

            <dl class="mt-4 grid grid-cols-2 gap-4 sm:grid-cols-4">
              <Fact label="Format" value={p().format_name} />
              <Fact label="Account" value={p().account_number ?? "Not stated"} />
              <Fact label="Period" value={formatPeriod(p().first_date, p().last_date)} />
              <Fact label="Rows" value={`${p().rows} (${p().typed} typed)`} />
            </dl>
            <p class="mt-3 text-sm text-muted-foreground">
              {p().new} new {p().new === 1 ? "row" : "rows"}
              <Show when={p().existing > 0}>
                {", "}
                {p().existing} already saved (they'll be updated, never duplicated)
              </Show>
              .
            </p>

            <div class="mt-4 max-h-[28rem] overflow-y-auto">
              <TransactionsTable rows={p().transactions} showStored />
            </div>

            <Show when={canSaveDocs()}>
              <div class="mt-5 flex flex-col gap-3 rounded-lg bg-secondary/20 p-4">
                <label class="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={toDocuments()}
                    onChange={(e) => setToDocuments(e.currentTarget.checked)}
                    disabled={!!busy()}
                    class="h-4 w-4 rounded border-input"
                  />
                  Also save the file to Documents
                </label>
                <Show when={toDocuments()}>
                  <div class="flex flex-col gap-1.5 pl-6">
                    <label for="statement-folder" class="text-xs text-muted-foreground">Folder</label>
                    <select
                      id="statement-folder"
                      value={(settled(folders), folder())}
                      onChange={(e) => setFolder(e.currentTarget.value)}
                      disabled={!!busy() || folders.loading}
                      class="h-9 max-w-sm rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <option value="">Documents (top level)</option>
                      <For each={folderOptions()}>{(path) => <option value={path}>Documents / {path.split("/").join(" / ")}</option>}</For>
                    </select>
                  </div>
                </Show>
              </div>
            </Show>

            <div class="mt-5 flex flex-wrap items-center justify-end gap-2">
              <Button variant="ghost" onClick={reset} disabled={!!busy()}>
                Cancel
              </Button>
              <Button onClick={save} disabled={!!busy()}>
                <Show when={busy() === "saving"}>
                  <Loader2 size={16} class="animate-spin" />
                </Show>
                Save statement
              </Button>
            </div>
          </div>
        )}
      </Show>
      <Show when={error()}>
        <p role="alert" class="mt-3 text-sm text-red-500">{error()}</p>
      </Show>
    </section>
  );
}

type UploadMeta = GridColumnMeta<UploadSummary>;

function UploadLink(props: { upload: UploadSummary }) {
  const org = useOrg();
  return (
    <Link
      to="/portal/$org/statements/$upload"
      params={{ org: org.slug, upload: String(props.upload.id) }}
      class="block max-w-[18rem] truncate font-medium underline-offset-2 hover:underline focus-visible:outline-none focus-visible:underline"
      title={props.upload.filename}
    >
      {props.upload.filename}
    </Link>
  );
}

const uploadColumns: ColumnDef<UploadSummary, any>[] = [
  {
    accessorKey: "filename",
    header: "File",
    cell: (c) => <UploadLink upload={c.row.original} />,
    enableHiding: false,
  },
  {
    id: "period",
    accessorFn: (u) => u.first_date ?? "",
    header: "Period",
    cell: (c) => (
      <span class="whitespace-nowrap text-muted-foreground">{formatPeriod(c.row.original.first_date, c.row.original.last_date)}</span>
    ),
    meta: { width: "w-56" } satisfies UploadMeta,
  },
  {
    accessorKey: "rows_parsed",
    header: "Rows",
    cell: (c) => <span class="tabular-nums">{c.getValue<number>()}</span>,
    meta: { width: "w-20", align: "right" } satisfies UploadMeta,
  },
  {
    accessorKey: "rows_new",
    header: "New",
    cell: (c) => <span class="tabular-nums text-muted-foreground">{c.getValue<number>()}</span>,
    meta: { width: "w-20", align: "right", label: "New rows" } satisfies UploadMeta,
  },
  {
    accessorKey: "rows_updated",
    header: "Updated",
    cell: (c) => <span class="tabular-nums text-muted-foreground">{c.getValue<number>()}</span>,
    meta: { width: "w-24", align: "right", label: "Already saved rows" } satisfies UploadMeta,
  },
  {
    id: "uploaded_by",
    accessorFn: (u) => u.uploaded_by ?? "",
    header: "Saved by",
    cell: (c) => <span class="truncate text-muted-foreground">{c.row.original.uploaded_by ?? "—"}</span>,
    filterFn: pickFilter,
    meta: {
      width: "w-40",
      pick: { options: (rows) => distinct(rows, (u) => u.uploaded_by), emptyLabel: "Unknown" },
    } satisfies UploadMeta,
  },
  {
    accessorKey: "uploaded_at",
    header: "Saved",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{formatDate(c.getValue<string>())}</span>,
    filterFn: dateRangeFilter,
    meta: { width: "w-32" } satisfies UploadMeta,
  },
];

function UploadsList(props: { acct: ReturnType<typeof useStatementAccount> }) {
  const org = useOrg();
  // Each result carries its account, so one account's statements never show under another.
  const [uploads] = createResource(
    () => {
      const a = props.acct.account();
      return a ? { account: a.number } : undefined;
    },
    async ({ account }) => ({ account, list: await fetchUploads(org.slug, account) }),
  );
  const current = () => {
    const data = settled(uploads);
    const a = props.acct.account();
    return data && a && data.account === a.number ? data : undefined;
  };
  const error = () => uploads.error ?? props.acct.accounts.error;

  return (
    <section class="mt-8">
      <h2 class="mb-3 text-sm font-medium">Saved statements</h2>
      <Show when={error()}>
        <p class="text-sm text-red-500">{errorText(error(), "Couldn't load statements.")}</p>
      </Show>
      <Show when={props.acct.account() === null || current()?.list.length === 0}>
        <p class="rounded-xl border border-dashed border-border px-6 py-10 text-center text-sm text-muted-foreground">
          No statements saved yet.
        </p>
      </Show>
      {/* Keyed: a new account starts with fresh filters and paging. */}
      <Show when={current()?.list.length ? current() : undefined} keyed>
        {(data) => (
          <DataGrid
            rows={data.list}
            columns={uploadColumns}
            initialSorting={[{ id: "uploaded_at", desc: true }]}
            search={(u, q) => [u.filename, u.uploaded_by ?? ""].some((v) => v.toLowerCase().includes(q))}
            searchPlaceholder="Search file, who saved it"
            storageKey="statements-uploads-columns"
            defaultHidden={["rows_new", "rows_updated"]}
            noun={["statement", "statements"]}
            toolbar={(grid) => {
              const range = () => grid.filterValue<[string, string]>("uploaded_at", ["", ""]);
              const set = (from: string, to: string) => grid.setFilter("uploaded_at", from || to ? [from, to] : undefined);
              return (
                <div class="flex min-w-0 flex-[2_1_16rem] items-center gap-1" title="When the statement was saved">
                  <Input type="date" class="h-9 min-w-0" value={range()[0]} onInput={(e) => set(e.currentTarget.value, range()[1])} aria-label="Saved from" />
                  <span class="text-muted-foreground">–</span>
                  <Input type="date" class="h-9 min-w-0" value={range()[1]} onInput={(e) => set(range()[0], e.currentTarget.value)} aria-label="Saved until" />
                </div>
              );
            }}
            summary={(shown) => <span>{shown.reduce((n, u) => n + u.rows_parsed, 0)} rows in these statements</span>}
          />
        )}
      </Show>
    </section>
  );
}

function Uploads() {
  const org = useOrg();
  const acct = useStatementAccount();
  return (
    <>
      <PageHeader
        title="Statements"
        description="Statement files saved for this account. Open one to see its rows or send them to your own Google Sheet."
        actions={<AccountPicker state={acct} />}
      />
      <StatementsTabs />
      <Show when={can(org, "write-statements")}>
        <UploadPanel onSaved={(account) => acct.pick(account)} />
      </Show>
      <UploadsList acct={acct} />
    </>
  );
}

/** /portal/<org>/statements/uploads. Needs `read-statements`; uploading needs `write-statements`. */
export function UploadsPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <Uploads />
    </Allowed>
  );
}
