import { createMemo, createResource, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Link } from "@tanstack/solid-router";
import FileX from "lucide-solid/icons/file-x";
import Hand from "lucide-solid/icons/hand";
import Loader2 from "lucide-solid/icons/loader-circle";
import Paperclip from "lucide-solid/icons/paperclip";
import Plus from "lucide-solid/icons/plus";
import Search from "lucide-solid/icons/search";
import Split from "lucide-solid/icons/split";
import Trash from "lucide-solid/icons/trash";
import X from "lucide-solid/icons/x";
import { FileTypeIcon } from "~/components/FileTypeIcon";
import { Button } from "~/components/ui/button";
import { addDays, DatePicker } from "~/components/ui/date-picker";
import { Input } from "~/components/ui/input";
import { amountFigures, formatFigure, statLabel, statValue } from "~/lib/amountStat";
import { fetchAllDocuments } from "~/lib/documents";
import { can } from "~/lib/orgs";
import {
  amountCents,
  attachDocument,
  deleteRow,
  detachDocument,
  fetchRow,
  formatAmount,
  formatCents,
  formatDate,
  formatPeriod,
  isNegative,
  type OrgTransaction,
  type RowDetail,
  saveSplit,
  setEffectiveDate,
  type SplitRow,
  type SplitRowInput,
} from "~/lib/statements";
import { cn, selectValue, settled } from "~/lib/utils";
import { useOrg } from "~/routes/portal/org/OrgLayout";

/** How far an effective date may be from the row's date, in days (statements-core `MAX_EFFECTIVE_DAYS`). */
const MAX_EFFECTIVE_DAYS = 366;

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);
const downloadUrl = (documentId: number) => `/api/documents/${documentId}/download`;
/** "Documents / Statements / 123" for a folder path ("" is the top level). */
const folderLabel = (folder: string) => `Documents / ${folder ? folder.split("/").join(" / ") : "(top level)"}`;

/** What the panel reports back to the grid after a change, so its row stays in step. */
export type RowChange = {
  id: number;
  type: string | null;
  type_manual: boolean;
  effective_date: string | null;
  attachments: number;
};

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
  /** Rows were added or removed (a split saved): the grid reloads. */
  onRowsChanged: () => void;
  /** Shows another row (e.g. a split's original row) in the panel. */
  onOpenRow: (id: number) => void;
  onClose: () => void;
}) {
  const org = useOrg();
  const canEdit = can(org, "write-statements");
  const canAttach = canEdit && can(org, "read-docs");
  const slide = slideIn(() => props.onClose());
  // Rows added by hand go with write-statements; rows from a bank statement need their own permission.
  const canDelete = (r: RowDetail) => r.split_of === null && (r.hand_added ? canEdit : can(org, "delete-statement-rows"));
  const [row, { mutate, refetch }] = createResource(() => props.rowId, (id) => fetchRow(org.slug, id));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [picking, setPicking] = createSignal(false);
  const [splitting, setSplitting] = createSignal(false);

  const report = (r: RowDetail) =>
    props.onChanged({
      id: r.id,
      type: r.type,
      type_manual: r.type_manual,
      effective_date: r.effective_date,
      attachments: r.documents.length,
    });

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
      setError(errorText(e, "Couldn't change the category."));
    } finally {
      setBusy(false);
    }
  };

  // A row with a split takes its split's rows with it.
  const remove = async (r: RowDetail) => {
    const extra = r.splits.length ? ` Its split's ${r.splits.length} rows are deleted too.` : "";
    const again = r.hand_added
      ? ""
      : " Uploading a statement that contains it again brings it back, unless an upload exclusion leaves it out.";
    if (!confirm(`Delete this ${r.hand_added ? "row added by hand" : "row from a bank statement"}?${extra}${again}`)) return;
    setBusy(true);
    setError(null);
    try {
      await deleteRow(org.slug, props.rowId);
      props.onRowsChanged();
      slide.close();
    } catch (e) {
      setError(errorText(e, "Couldn't delete the row."));
      setBusy(false);
    }
  };

  const saveSplitRows = async (rows: SplitRowInput[]) => {
    const r = await saveSplit(org.slug, props.rowId, rows);
    mutate(r);
    setSplitting(false);
    props.onRowsChanged();
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !e.defaultPrevented && !splitting()) slide.close();
  };
  document.addEventListener("keydown", onKey);
  onCleanup(() => document.removeEventListener("keydown", onKey));

  return (
    <Portal>
      <Backdrop onClose={() => !busy() && slide.close()} />
      <aside
        class={cn("fixed inset-y-0 right-0 z-40 flex w-full max-w-md flex-col border-l border-border bg-background text-foreground shadow-2xl", slide.panelClass())}
        aria-label="Row details"
      >
        <header class="flex items-center justify-between gap-3 border-b border-border px-5 py-3">
          <h2 class="text-sm font-medium">Row details</h2>
          <button
            type="button"
            onClick={slide.close}
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
                      "mt-1 text-xl font-semibold font-mono tabular-nums",
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
                      <dt class="text-xs text-muted-foreground">Bank account</dt>
                      <dd class="truncate">
                        {r().account_name}
                        <Show when={r().account_identifier}>
                          <span class="ml-1 font-mono text-xs text-muted-foreground">{r().account_identifier}</span>
                        </Show>
                      </dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">Id</dt>
                      <dd class="font-mono text-xs">{r().txn_id}</dd>
                    </div>
                  </dl>
                </section>

                {/* Type */}
                <section>
                  <label for="row-type" class="text-xs text-muted-foreground">Category</label>
                  <div class="mt-1 flex items-center gap-2">
                    <select
                      id="row-type"
                      value={selectValue(r().type ?? "", props.types)}
                      onChange={(e) => void changeType(e.currentTarget.value || null)}
                      disabled={!canEdit || busy()}
                      class="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
                    >
                      <option value="">No category</option>
                      <Show when={r().type && !props.types.includes(r().type!)}>
                        <option value={r().type!}>{r().type}</option>
                      </Show>
                      <For each={props.types}>{(t) => <option value={t}>{t}</option>}</For>
                    </select>
                  </div>
                  <p class="mt-1 flex items-center gap-1 text-xs text-muted-foreground">
                    <Show
                      when={r().type_manual}
                      fallback={r().type ? "Set by a category rule. Pick one to set it by hand." : "No rule matched."}
                    >
                      <Hand size={11} /> Set by hand: saving a statement won't change it.
                    </Show>
                  </p>
                </section>

                {/* Effective date */}
                <section>
                  <label for="row-effective-date" class="text-xs text-muted-foreground">Effective date</label>
                  <div class="mt-1 flex items-center gap-2">
                    <DatePicker
                      id="row-effective-date"
                      value={r().effective_date ?? ""}
                      opensOn={r().date}
                      min={addDays(r().date, -MAX_EFFECTIVE_DAYS)}
                      max={addDays(r().date, MAX_EFFECTIVE_DAYS)}
                      onChange={(d) => void run(() => setEffectiveDate(org.slug, props.rowId, d), "Couldn't change the effective date.")}
                      disabled={!canEdit || busy()}
                      class="h-9 min-w-0 flex-1 cursor-pointer rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-60"
                    />
                    <Show when={canEdit && r().effective_date}>
                      <Button
                        size="sm"
                        variant="ghost"
                        class="h-9"
                        disabled={busy()}
                        onClick={() => void run(() => setEffectiveDate(org.slug, props.rowId, null), "Couldn't change the effective date.")}
                      >
                        Clear
                      </Button>
                    </Show>
                  </div>
                  <p class="mt-1 text-xs text-muted-foreground">
                    The date it counts towards, when that isn't {formatDate(r().date)}. Pick it from the calendar: within a year of that date.
                  </p>
                </section>

                {/* Split */}
                <Show
                  when={r().split_of === null}
                  fallback={
                    <section>
                      <h3 class="flex items-center gap-1.5 text-xs text-muted-foreground">
                        <Split size={12} /> Part of a split
                      </h3>
                      <Show
                        when={r().split_parent}
                        fallback={<p class="mt-1 text-sm text-muted-foreground">Its original row isn't stored any more (its statement was deleted).</p>}
                      >
                        {(p) => (
                          <>
                            <ul class="mt-1">
                              <SplitRowItem row={p()} onOpen={props.onOpenRow} />
                            </ul>
                            <p class="mt-1 text-xs text-muted-foreground">Change the split from its original row.</p>
                          </>
                        )}
                      </Show>
                    </section>
                  }
                >
                  <section>
                    <div class="flex items-center justify-between gap-2">
                      <h3 class="flex items-center gap-1.5 text-xs text-muted-foreground">
                        <Split size={12} /> Split
                      </h3>
                      <Show when={canEdit}>
                        <Button size="sm" variant="outline" class="h-7" onClick={() => setSplitting(true)} disabled={busy()}>
                          {r().splits.length ? "Edit split…" : "Split…"}
                        </Button>
                      </Show>
                    </div>
                    <ul class="mt-2 flex flex-col gap-1">
                      <For
                        each={r().splits}
                        fallback={<li class="text-sm text-muted-foreground">Not split. Split it to move part of the amount to another category or date.</li>}
                      >
                        {(s) => <SplitRowItem row={s} onOpen={props.onOpenRow} />}
                      </For>
                    </ul>
                    <Show when={splitting()}>
                      <SplitDialog row={r()} types={props.types} onSave={saveSplitRows} onClose={() => setSplitting(false)} />
                    </Show>
                  </section>
                </Show>

                {/* Where it came from */}
                <section>
                  <h3 class="text-xs text-muted-foreground">
                    {r().uploads.length === 0
                      ? "Added by hand"
                      : r().hand_added
                        ? `Added by hand, in ${r().uploads.length === 1 ? "this upload" : "these uploads"}`
                        : `From ${r().uploads.length === 1 ? "statement" : "statements"}`}
                  </h3>
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

                {/* Split rows go by changing their split. */}
                <Show when={canDelete(r())}>
                  <section class="border-t border-border pt-4">
                    <Button size="sm" variant="ghost" class="h-8 text-red-500 hover:text-red-600" disabled={busy()} onClick={() => void remove(r())}>
                      <Trash size={14} /> Delete row
                    </Button>
                    <p class="mt-1 text-xs text-muted-foreground">
                      {r().hand_added
                        ? "It was added by hand, not from a bank statement."
                        : "It's from a bank statement: uploading that statement again brings it back, unless an upload exclusion leaves it out."}
                    </p>
                  </section>
                </Show>
              </div>
            )}
          </Show>
        </div>
      </aside>
    </Portal>
  );
}

const SLIDE_MS = 200;

/**
 * A side panel sliding in from the right when it opens, and out before it closes: `close` slides
 * it out, then calls `onClose`. Without motion (reduced-motion setting) it just appears and goes.
 */
function slideIn(onClose: () => void) {
  const [shown, setShown] = createSignal(false);
  // Two frames: the panel is painted off-screen first, so the change to on-screen transitions.
  onMount(() => requestAnimationFrame(() => requestAnimationFrame(() => setShown(true))));
  let closing = false;
  const close = () => {
    if (closing) return;
    closing = true;
    setShown(false);
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    setTimeout(onClose, reduced ? 0 : SLIDE_MS);
  };
  const panelClass = () =>
    cn("transition-transform duration-200 ease-out motion-reduce:transition-none", shown() ? "translate-x-0" : "translate-x-full");
  return { close, panelClass };
}

/**
 * Behind a side panel: covers the page (see-through, so the grid stays in view) so the grid can't be
 * used while the panel is open, and a click anywhere on it closes the panel. The grid's selection stays (`data-keeps-selection`).
 */
function Backdrop(props: { onClose: () => void }) {
  return <div data-keeps-selection class="fixed inset-0 z-40" onClick={props.onClose} aria-hidden="true" />;
}

/** One row of a split (or a split's original row): opens it in the panel. */
function SplitRowItem(props: { row: SplitRow; onOpen: (id: number) => void }) {
  return (
    <li>
      <button
        type="button"
        onClick={() => props.onOpen(props.row.id)}
        class="flex w-full items-center gap-2 rounded-md px-1 py-1 text-left text-sm hover:bg-accent/40 focus-visible:bg-accent/40 focus-visible:outline-none"
        title="Open this row"
      >
        <span class="w-24 shrink-0 text-xs text-muted-foreground">{formatDate(props.row.effective_date ?? props.row.date)}</span>
        <span class="min-w-0 flex-1 truncate">{props.row.description}</span>
        <Show when={props.row.type}>
          <span class="shrink-0 rounded-full border border-border bg-secondary/40 px-2 py-0.5 text-xs font-medium">{props.row.type}</span>
        </Show>
        <span class={cn("shrink-0 font-mono tabular-nums", !isNegative(props.row.amount) && "text-emerald-600 dark:text-emerald-400")}>
          {formatAmount(props.row.amount)}
        </span>
      </button>
    </li>
  );
}

/**
 * A dialog for a row's split: the amount to move, the type it moves to, and optionally the date it
 * counts towards. "Split" adds the usual pair of rows: the amount taken off the row's own type, and
 * put on the chosen type (and effective date). They add up to zero; the original row stays as it is.
 */
function SplitDialog(props: {
  row: RowDetail;
  types: string[];
  onSave: (rows: SplitRowInput[]) => Promise<void>;
  onClose: () => void;
}) {
  let dialog!: HTMLDialogElement;
  onMount(() => dialog.showModal());

  const p = props.row;
  const sign = isNegative(p.amount) ? -1 : 1;
  // An existing pair (as this dialog makes): the row moved (same sign as the original) and the
  // row taking it off. Editing it keeps their ids, so their documents stay.
  const pair = (() => {
    if (p.splits.length !== 2) return null;
    const moved = p.splits.find((s) => isNegative(s.amount) === (sign < 0));
    const offset = p.splits.find((s) => s !== moved);
    return moved && offset ? { moved, offset } : null;
  })();
  const movedCents = pair ? amountCents(pair.moved.amount) : null;

  const [amount, setAmount] = createSignal(movedCents !== null ? formatCents(Math.abs(movedCents)) : "");
  const [type, setType] = createSignal(pair?.moved.type ?? "");
  const [effective, setEffective] = createSignal(pair?.moved.effective_date ?? "");
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const cents = () => amountCents(amount());
  const canSave = () => !saving() && !!cents();

  const submit = async (remove = false) => {
    if (remove && !confirm("Remove this split? Its rows are deleted (the original row stays).")) return;
    const portion = Math.abs(cents() ?? 0);
    if (!remove && !portion) return;
    setSaving(true);
    setError(null);
    const description = p.description.trim();
    try {
      await props.onSave(
        remove
          ? []
          : [
              {
                id: pair?.offset.id,
                date: p.date,
                effective_date: p.effective_date,
                description: `(Split) ${description}`,
                amount: formatCents(-sign * portion),
                type: p.type,
              },
              {
                id: pair?.moved.id,
                date: p.date,
                effective_date: effective() || null,
                description,
                amount: formatCents(sign * portion),
                type: type() || null,
              },
            ],
      );
    } catch (e) {
      setError(errorText(e, "Couldn't save the split."));
      setSaving(false);
    }
  };

  const fieldClass =
    "h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

  return (
    <dialog
      ref={dialog}
      onClose={props.onClose}
      onCancel={(e) => saving() && e.preventDefault()}
      class="m-auto w-[calc(100%-2rem)] max-w-md rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="split-title"
    >
      <form
        class="flex flex-col gap-5 p-6"
        onSubmit={(e) => {
          e.preventDefault();
          if (canSave()) void submit();
        }}
      >
        <div class="flex items-start justify-between gap-4">
          <div class="min-w-0">
            <h2 id="split-title" class="flex items-center gap-2 text-lg font-semibold tracking-tight">
              <Split size={18} /> {p.splits.length ? "Edit split" : "Split row"}
            </h2>
          </div>
          <button
            type="button"
            onClick={() => dialog.close()}
            disabled={saving()}
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
            aria-label="Close"
          >
            <X size={18} />
          </button>
        </div>

        {/* The row being split: its amount and type matter most. */}
        <div class="flex flex-col gap-3 rounded-lg border border-border bg-secondary/20 p-4">
          <div class="flex items-start justify-between gap-4">
            <div class="min-w-0">
              <span class="text-xs text-muted-foreground">Amount</span>
              <p class={cn("text-2xl font-semibold font-mono tabular-nums", !isNegative(p.amount) && "text-emerald-600 dark:text-emerald-400")}>
                {formatAmount(p.amount)}
              </p>
            </div>
            <div class="flex min-w-0 flex-col items-end">
              <span class="text-xs text-muted-foreground">Category</span>
              <Show when={p.type} fallback={<span class="mt-1 text-sm text-muted-foreground">No category</span>}>
                <span class="mt-1 rounded-full border border-border bg-secondary/60 px-2.5 py-0.5 text-sm font-medium">{p.type}</span>
              </Show>
            </div>
          </div>
          <p class="break-words text-sm">{p.description.trim()}</p>
          <p class="text-xs text-muted-foreground">
            {formatDate(p.date)}
            <Show when={p.effective_date && p.effective_date !== p.date}> · counts towards {formatDate(p.effective_date!)}</Show>
          </p>
        </div>

        <Show when={p.splits.length && !pair}>
          <p class="text-sm text-muted-foreground">Its current split's {p.splits.length} rows are replaced.</p>
        </Show>

        <div class="flex flex-col gap-3 text-sm">
          <label class="flex flex-col gap-1">
            <span class="text-xs text-muted-foreground">Amount</span>
            <input
              class={cn(fieldClass, "font-mono tabular-nums", amount() && cents() === null && "border-red-500")}
              inputmode="decimal"
              placeholder="0.00"
              value={amount()}
              onInput={(e) => setAmount(e.currentTarget.value)}
              ref={(el) => queueMicrotask(() => el.focus())}
            />
          </label>
          <label class="flex flex-col gap-1">
            <span class="text-xs text-muted-foreground">Category</span>
            <select class={fieldClass} value={selectValue(type(), props.types)} onChange={(e) => setType(e.currentTarget.value)}>
              <option value="">No category</option>
              <Show when={type() && !props.types.includes(type())}>
                <option value={type()}>{type()}</option>
              </Show>
              <For each={props.types}>{(t) => <option value={t}>{t}</option>}</For>
            </select>
          </label>
          <div class="flex flex-col gap-1">
            <span class="text-xs text-muted-foreground">Effective date (optional)</span>
            <div class="flex items-center gap-2">
              <DatePicker
                class={cn(fieldClass, "flex-1 cursor-pointer")}
                aria-label="Effective date"
                value={effective()}
                opensOn={p.date}
                min={addDays(p.date, -MAX_EFFECTIVE_DAYS)}
                max={addDays(p.date, MAX_EFFECTIVE_DAYS)}
                onChange={setEffective}
              />
              <Show when={effective()}>
                <Button type="button" size="sm" variant="ghost" class="h-9" onClick={() => setEffective("")}>
                  Clear
                </Button>
              </Show>
            </div>
          </div>
        </div>

        <Show when={error()}>
          <p role="alert" class="text-sm text-red-500">{error()}</p>
        </Show>
        <div class="flex flex-wrap items-center gap-2">
          <Show when={p.splits.length}>
            <Button type="button" variant="ghost" class="text-red-500 hover:text-red-600" disabled={saving()} onClick={() => void submit(true)}>
              <Trash size={16} /> Remove split
            </Button>
          </Show>
          <span class="flex-1" />
          <Button type="button" variant="outline" disabled={saving()} onClick={() => dialog.close()}>
            Cancel
          </Button>
          <Button type="submit" disabled={!canSave()}>
            <Show when={saving()}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            Split
          </Button>
        </div>
      </form>
    </dialog>
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

/** Runs `fn` on each item, a few at a time; returns the errors (an item that fails doesn't stop the rest). */
async function eachOf<T>(items: T[], fn: (item: T) => Promise<void>, onProgress: (done: number) => void) {
  let next = 0;
  let done = 0;
  const failures: unknown[] = [];
  const worker = async () => {
    while (next < items.length) {
      const item = items[next++];
      try {
        await fn(item);
      } catch (e) {
        failures.push(e);
      }
      onProgress(++done);
    }
  };
  await Promise.all(Array.from({ length: Math.min(6, items.length) }, worker));
  return failures;
}

/**
 * The side panel for several rows selected in the grid: change their category or effective date,
 * attach a document to all of them, or delete them, in one go. Splitting is one row at a time.
 * The panel follows the selection, so it stays up to date as the rows change.
 */
export function RowsPanel(props: {
  rows: OrgTransaction[];
  types: string[];
  /** Saves a row's category by hand (null clears it); updates the grid. */
  setType: (rowId: number, type: string | null) => Promise<void>;
  /** Updates one row in the grid after a change here. */
  onPatched: (id: number, patch: Partial<OrgTransaction>) => void;
  /** Rows were removed: the grid reloads. */
  onRowsChanged: () => void;
  onClose: () => void;
}) {
  const org = useOrg();
  const canEdit = can(org, "write-statements");
  const canAttach = canEdit && can(org, "read-docs");
  const canDeleteStatementRows = can(org, "delete-statement-rows");
  const slide = slideIn(() => props.onClose());
  const [progress, setProgress] = createSignal<{ what: string; done: number; of: number } | null>(null);
  const [message, setMessage] = createSignal<{ text: string; error: boolean } | null>(null);
  const [picking, setPicking] = createSignal(false);
  const busy = () => progress() !== null;

  const n = () => props.rows.length;
  const rowsWord = (k: number) => `${k.toLocaleString()} ${k === 1 ? "row" : "rows"}`;
  // The same figure as the grid's status bar shows (Sum, Average, Min or Max), per currency.
  const figures = createMemo(() => amountFigures(props.rows));
  const dates = createMemo(() => props.rows.map((r) => r.date).sort());
  /** The value every row shares, or undefined when they differ. */
  const common = <V,>(get: (r: OrgTransaction) => V): V | undefined => {
    const values = new Set(props.rows.map(get));
    return values.size === 1 ? [...values][0] : undefined;
  };
  const MIXED = "\u0000mixed";
  const commonType = () => common((r) => r.type ?? "");
  const commonEffective = () => common((r) => r.effective_date ?? "");
  // An effective date has to be within a year of each row's date: the window they all allow.
  const effectiveMin = () => (n() ? addDays(dates()[n() - 1], -MAX_EFFECTIVE_DAYS) : "");
  const effectiveMax = () => (n() ? addDays(dates()[0], MAX_EFFECTIVE_DAYS) : "");
  const effectivePossible = () => n() > 0 && effectiveMin() <= effectiveMax();
  // Rows from a bank statement need their own permission; split rows go with their original row.
  const deletable = createMemo(() =>
    props.rows.filter((r) => r.split_of === null && (r.hand_added ? canEdit : canDeleteStatementRows)),
  );

  /** Changes `rows` one by one, showing progress, then says how it went. */
  const apply = async (what: string, rows: OrgTransaction[], fn: (r: OrgTransaction) => Promise<void>, done: (k: number) => string) => {
    setMessage(null);
    if (rows.length === 0) {
      setMessage({ text: "Nothing to change: every row already has that.", error: false });
      return;
    }
    setProgress({ what, done: 0, of: rows.length });
    const failures = await eachOf(rows, fn, (k) => setProgress({ what, done: k, of: rows.length }));
    setProgress(null);
    const ok = rows.length - failures.length;
    if (failures.length === 0) setMessage({ text: done(ok), error: false });
    else
      setMessage({
        text: `${ok ? `${done(ok)} ` : ""}${rowsWord(failures.length)} couldn't be changed: ${errorText(failures[0], "something went wrong")}`,
        error: true,
      });
  };

  const setType = (type: string | null) =>
    apply(
      "Setting the category",
      props.rows.filter((r) => r.type !== type || !r.type_manual),
      (r) => props.setType(r.id, type),
      (k) => (type ? `${type} set on ${rowsWord(k)}.` : `Category cleared on ${rowsWord(k)}.`),
    );

  const setEffective = (d: string | null) =>
    apply(
      d ? "Setting the effective date" : "Clearing the effective date",
      props.rows.filter((r) => r.effective_date !== d),
      async (r) => {
        const saved = await setEffectiveDate(org.slug, r.id, d);
        props.onPatched(r.id, { effective_date: saved.effective_date });
      },
      (k) => (d ? `Effective date ${formatDate(d)} set on ${rowsWord(k)}.` : `Effective date cleared on ${rowsWord(k)}.`),
    );

  const attach = (documentId: number) => {
    setPicking(false);
    return apply(
      "Attaching the document",
      props.rows,
      async (r) => {
        const saved = await attachDocument(org.slug, r.id, documentId);
        props.onPatched(r.id, { attachments: saved.documents.length });
      },
      (k) => `Attached to ${rowsWord(k)}.`,
    );
  };

  const remove = async () => {
    const rows = deletable();
    const fromBank = rows.filter((r) => !r.hand_added).length;
    const splits = rows.reduce((sum, r) => sum + r.split_rows, 0);
    const skipped = n() - rows.length;
    const lines = [
      `Delete ${rowsWord(rows.length)}?`,
      splits ? `Their splits' ${rowsWord(splits)} are deleted too.` : "",
      fromBank ? `${rowsWord(fromBank)} came from bank statements: uploading those statements again brings them back, unless an upload exclusion leaves them out.` : "",
      skipped ? `${rowsWord(skipped)} can't be deleted here and stay.` : "",
    ];
    if (!confirm(lines.filter(Boolean).join("\n\n"))) return;
    await apply("Deleting", rows, (r) => deleteRow(org.slug, r.id), (k) => `Deleted ${rowsWord(k)}.`);
    props.onRowsChanged();
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !e.defaultPrevented && !busy()) slide.close();
  };
  document.addEventListener("keydown", onKey);
  onCleanup(() => document.removeEventListener("keydown", onKey));

  const fieldClass =
    "h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60";

  return (
    <Portal>
      <Backdrop onClose={() => !busy() && slide.close()} />
      <aside
        data-keeps-selection
        class={cn("fixed inset-y-0 right-0 z-40 flex w-full max-w-md flex-col border-l border-border bg-background text-foreground shadow-2xl", slide.panelClass())}
        aria-label="Selected rows"
      >
        <header class="flex items-center justify-between gap-3 border-b border-border px-5 py-3">
          <h2 class="text-sm font-medium">{rowsWord(n())} selected</h2>
          <button
            type="button"
            onClick={slide.close}
            disabled={busy()}
            class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
            aria-label="Close"
            title="Close (Esc)"
          >
            <X size={18} />
          </button>
        </header>

        <div class="flex-1 overflow-y-auto px-5 py-4">
          <div class="flex flex-col gap-6">
            {/* What's selected */}
            <section>
              <For each={figures()}>
                {([currency, f]) => (
                  <p
                    class={cn(
                      "text-xl font-semibold font-mono tabular-nums",
                      statValue(f) >= 0 && "text-emerald-600 dark:text-emerald-400",
                    )}
                  >
                    {formatFigure(statValue(f))} <span class="text-sm font-normal text-muted-foreground">
                      {currency} ({statLabel()})
                    </span>
                  </p>
                )}
              </For>
              <p class="mt-1 text-sm text-muted-foreground">
                {rowsWord(n())}, {formatPeriod(dates()[0], dates()[dates().length - 1])}
              </p>
              <ul class="mt-3 max-h-48 overflow-y-auto rounded-md border border-border text-xs">
                <For each={props.rows}>
                  {(r) => (
                    <li class="flex items-center gap-2 border-b border-border px-2 py-1 last:border-0">
                      <span class="w-20 shrink-0 text-muted-foreground">{formatDate(r.date)}</span>
                      <span class="min-w-0 flex-1 truncate" title={r.description}>{r.description}</span>
                      <span class={cn("shrink-0 font-mono tabular-nums", !isNegative(r.amount) && "text-emerald-600 dark:text-emerald-400")}>
                        {formatAmount(r.amount)}
                      </span>
                    </li>
                  )}
                </For>
              </ul>
            </section>

            {/* Category */}
            <section>
              <label for="rows-type" class="text-xs text-muted-foreground">Category</label>
              <div class="mt-1 flex items-center gap-2">
                <select
                  id="rows-type"
                  value={commonType() === undefined ? MIXED : selectValue(commonType()!, props.types)}
                  onChange={(e) => {
                    const v = e.currentTarget.value;
                    if (v !== MIXED) void setType(v || null);
                  }}
                  disabled={!canEdit || busy()}
                  class={fieldClass}
                >
                  <Show when={commonType() === undefined}>
                    <option value={MIXED} disabled>
                      Mixed
                    </option>
                  </Show>
                  <option value="">No category</option>
                  <Show when={commonType() && !props.types.includes(commonType()!)}>
                    <option value={commonType()!}>{commonType()}</option>
                  </Show>
                  <For each={props.types}>{(t) => <option value={t}>{t}</option>}</For>
                </select>
              </div>
              <p class="mt-1 text-xs text-muted-foreground">Picking one sets it on every selected row, by hand: saving a statement won't change it.</p>
            </section>

            {/* Effective date */}
            <section>
              <label for="rows-effective-date" class="text-xs text-muted-foreground">Effective date</label>
              <Show
                when={effectivePossible()}
                fallback={
                  <p class="mt-1 text-sm text-muted-foreground">
                    The rows are more than two years apart: no one effective date is within a year of all of them.
                  </p>
                }
              >
                <div class="mt-1 flex items-center gap-2">
                  <DatePicker
                    id="rows-effective-date"
                    value={commonEffective() ?? ""}
                    opensOn={dates()[0] > effectiveMin() ? dates()[0] : effectiveMin()}
                    min={effectiveMin()}
                    max={effectiveMax()}
                    onChange={(d) => void setEffective(d)}
                    disabled={!canEdit || busy()}
                    class={cn(fieldClass, "cursor-pointer disabled:cursor-default")}
                  />
                  <Show when={canEdit && props.rows.some((r) => r.effective_date)}>
                    <Button size="sm" variant="ghost" class="h-9" disabled={busy()} onClick={() => void setEffective(null)}>
                      Clear
                    </Button>
                  </Show>
                </div>
                <p class="mt-1 text-xs text-muted-foreground">
                  {commonEffective() === undefined ? "The rows have different effective dates. " : ""}
                  Picking one sets it on every selected row: within a year of each row's date.
                </p>
              </Show>
            </section>

            {/* Documents */}
            <Show when={canAttach}>
              <section>
                <div class="flex items-center justify-between gap-2">
                  <h3 class="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <Paperclip size={12} /> Documents
                  </h3>
                  <Show when={!picking()}>
                    <Button size="sm" variant="outline" class="h-7" onClick={() => setPicking(true)} disabled={busy()}>
                      <Plus size={14} /> Attach to all
                    </Button>
                  </Show>
                </div>
                <p class="mt-1 text-sm text-muted-foreground">
                  {(() => {
                    const k = props.rows.filter((r) => r.attachments > 0).length;
                    return k ? `${rowsWord(k)} of ${n().toLocaleString()} ${k === 1 ? "has" : "have"} documents attached. Open a row to see or remove them.` : "None attached.";
                  })()}
                </p>
                <Show when={picking()}>
                  <DocumentPicker exclude={[]} busy={busy()} onPick={(id) => void attach(id)} onClose={() => setPicking(false)} />
                </Show>
              </section>
            </Show>

            {/* Split */}
            <section>
              <h3 class="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Split size={12} /> Split
              </h3>
              <p class="mt-1 text-sm text-muted-foreground">Rows are split one at a time: click a single row to open it.</p>
            </section>

            <Show when={progress()}>
              {(p) => (
                <p role="status" class="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 size={14} class="animate-spin" /> {p().what}: {p().done} of {p().of}…
                </p>
              )}
            </Show>
            <Show when={message()}>
              {(m) => (
                <p role={m().error ? "alert" : "status"} class={cn("text-sm", m().error ? "text-red-500" : "text-muted-foreground")}>
                  {m().text}
                </p>
              )}
            </Show>

            <Show when={deletable().length > 0}>
              <section class="border-t border-border pt-4">
                <Button size="sm" variant="ghost" class="h-8 text-red-500 hover:text-red-600" disabled={busy()} onClick={() => void remove()}>
                  <Trash size={14} /> Delete {rowsWord(deletable().length)}
                </Button>
                <Show when={deletable().length < n()}>
                  <p class="mt-1 text-xs text-muted-foreground">
                    {rowsWord(n() - deletable().length)} can't be deleted here (split rows go with their original row
                    {canDeleteStatementRows ? "" : ", and deleting rows from a bank statement needs its own permission"}).
                  </p>
                </Show>
              </section>
            </Show>
          </div>
        </div>
      </aside>
    </Portal>
  );
}
