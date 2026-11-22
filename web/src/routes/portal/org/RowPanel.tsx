import { createMemo, createResource, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { createStore } from "solid-js/store";
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
import { Input } from "~/components/ui/input";
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
  isNegative,
  type RowDetail,
  saveSplit,
  setEffectiveDate,
  type SplitRow,
  type SplitRowInput,
} from "~/lib/statements";
import { cn, selectValue, settled } from "~/lib/utils";
import { useOrg } from "~/routes/portal/org/OrgLayout";

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
      setError(errorText(e, "Couldn't change the type."));
    } finally {
      setBusy(false);
    }
  };

  // Only rows added by hand; one with a split takes its split's rows with it.
  const remove = async (r: RowDetail) => {
    const extra = r.splits.length ? ` Its split's ${r.splits.length} rows are deleted too.` : "";
    if (!confirm(`Delete this row added by hand?${extra}`)) return;
    setBusy(true);
    setError(null);
    try {
      await deleteRow(org.slug, props.rowId);
      props.onRowsChanged();
      props.onClose();
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
    if (e.key === "Escape" && !e.defaultPrevented && !splitting()) props.onClose();
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
                  <label for="row-type" class="text-xs text-muted-foreground">Type</label>
                  <div class="mt-1 flex items-center gap-2">
                    <select
                      id="row-type"
                      value={selectValue(r().type ?? "", props.types)}
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

                {/* Effective date */}
                <section>
                  <label for="row-effective-date" class="text-xs text-muted-foreground">Effective date</label>
                  <div class="mt-1 flex items-center gap-2">
                    <input
                      id="row-effective-date"
                      type="date"
                      value={r().effective_date ?? ""}
                      onChange={(e) =>
                        void run(() => setEffectiveDate(org.slug, props.rowId, e.currentTarget.value || null), "Couldn't change the effective date.")
                      }
                      disabled={!canEdit || busy()}
                      class="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
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
                  <p class="mt-1 text-xs text-muted-foreground">The date it counts towards, when that isn't {formatDate(r().date)}.</p>
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
                        fallback={<li class="text-sm text-muted-foreground">Not split. Split it to move part of the amount to another type or date.</li>}
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

                {/* Rows added by hand can go; split rows go by changing their split. */}
                <Show when={canEdit && r().hand_added && r().split_of === null}>
                  <section class="border-t border-border pt-4">
                    <Button size="sm" variant="ghost" class="h-8 text-red-500 hover:text-red-600" disabled={busy()} onClick={() => void remove(r())}>
                      <Trash size={14} /> Delete row
                    </Button>
                    <p class="mt-1 text-xs text-muted-foreground">It was added by hand, not from a bank statement.</p>
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
        <span class={cn("shrink-0 tabular-nums", !isNegative(props.row.amount) && "text-emerald-600 dark:text-emerald-400")}>
          {formatAmount(props.row.amount)}
        </span>
      </button>
    </li>
  );
}

type Draft = SplitRowInput & {
  key: number;
  /** The effective date follows the date until it's changed by hand. */
  effectiveEdited: boolean;
};
let draftKey = 0;

/**
 * A dialog for a row's split: rows that move part of its amount to another type or date. They
 * must add up to zero. "Fill" makes the usual pair: the amount out of the row's type, and into
 * another. Each row's effective date starts as its date.
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
  const blank = (over: Partial<SplitRowInput> = {}): Draft => ({
    key: ++draftKey,
    date: p.date,
    effective_date: p.date,
    description: "",
    amount: "",
    type: null,
    effectiveEdited: false,
    ...over,
  });
  // A store, so editing a field changes it in place: the row's inputs (and focus) stay put.
  const [rows, setRows] = createStore<Draft[]>(
    p.splits.length
      ? p.splits.map((s) => ({
          key: ++draftKey,
          id: s.id,
          date: s.date,
          effective_date: s.effective_date ?? s.date,
          description: s.description,
          amount: s.amount,
          type: s.type,
          effectiveEdited: s.effective_date !== null && s.effective_date !== s.date,
        }))
      : [],
  );
  const [moveAmount, setMoveAmount] = createSignal("");
  const [moveType, setMoveType] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const update = (key: number, patch: Partial<Draft>) => setRows((r) => r.key === key, patch);
  const setDate = (r: Draft, date: string) => update(r.key, r.effectiveEdited ? { date } : { date, effective_date: date });
  const cents = () => rows.map((r) => amountCents(r.amount));
  const total = () => cents().reduce<number>((sum, c) => sum + (c ?? 0), 0);
  const complete = () => rows.every((r, i) => r.description.trim() && r.date && cents()[i] !== null && cents()[i] !== 0);
  const canSave = () => !saving() && rows.length >= 2 && complete() && total() === 0;

  // The usual split, from the portion to move: the portion taken off the original row (same
  // description, marked, same type) and put on the chosen type. They add up to 0.
  const fill = (e?: SubmitEvent) => {
    e?.preventDefault();
    const c = amountCents(moveAmount());
    if (c === null || c === 0) return;
    const sign = isNegative(p.amount) ? -1 : 1;
    const portion = Math.abs(c);
    const description = p.description.trim();
    setRows([
      blank({ description: `(Split) ${description}`, amount: formatCents(-sign * portion), type: p.type }),
      blank({ description, amount: formatCents(sign * portion), type: moveType() || null }),
    ]);
  };

  const submit = async (remove = false) => {
    if (remove && !confirm("Remove this split? Its rows are deleted (the original row stays).")) return;
    setSaving(true);
    setError(null);
    try {
      await props.onSave(
        remove
          ? []
          : rows.map(({ key: _key, effectiveEdited: _edited, ...r }) => ({
              ...r,
              type: r.type || null,
              effective_date: r.effective_date || null,
            })),
      );
    } catch (e) {
      setError(errorText(e, "Couldn't save the split."));
      setSaving(false);
    }
  };

  const fieldClass =
    "h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
  const columns = "grid grid-cols-[minmax(0,1fr)_7.5rem_9rem_9rem_9rem_2rem] items-center gap-2";

  return (
    <dialog
      ref={dialog}
      onClose={props.onClose}
      onCancel={(e) => saving() && e.preventDefault()}
      class="m-auto w-[calc(100%-2rem)] max-w-5xl rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="split-title"
    >
      <div class="flex flex-col gap-5 p-6">
        <div class="flex items-start justify-between gap-4">
          <div class="min-w-0">
            <h2 id="split-title" class="flex items-center gap-2 text-lg font-semibold tracking-tight">
              <Split size={18} /> {p.splits.length ? "Edit split" : "Split row"}
            </h2>
            <p class="mt-1 truncate text-sm text-muted-foreground">
              {formatDate(p.date)} · {p.description.trim()} ·{" "}
              <span class={cn("tabular-nums", !isNegative(p.amount) && "text-emerald-600 dark:text-emerald-400")}>{formatAmount(p.amount)}</span>
              <Show when={p.type}> · {p.type}</Show>
            </p>
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

        <p class="text-sm text-muted-foreground">
          Split rows are added alongside the original row, which stays as it is. They must add up to 0.
        </p>

        {/* A new split starts from the portion to move; the rows follow. */}
        <Show when={rows.length === 0}>
          <form onSubmit={fill} class="flex flex-wrap items-end gap-2 rounded-lg bg-secondary/20 p-3 text-sm">
            <label class="flex flex-col gap-1">
              <span class="text-xs text-muted-foreground">Portion to split off</span>
              <input
                class={cn(fieldClass, "w-36 tabular-nums")}
                inputmode="decimal"
                placeholder="4726.16"
                value={moveAmount()}
                onInput={(e) => setMoveAmount(e.currentTarget.value)}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
            </label>
            <label class="flex w-56 flex-col gap-1">
              <span class="text-xs text-muted-foreground">to type</span>
              <select class={fieldClass} value={moveType()} onChange={(e) => setMoveType(e.currentTarget.value)}>
                <option value="">No type</option>
                <For each={props.types}>{(t) => <option value={t}>{t}</option>}</For>
              </select>
            </label>
            <Button type="submit" size="sm" variant="outline" class="h-9" disabled={!amountCents(moveAmount())}>
              Fill rows
            </Button>
          </form>
        </Show>

        <Show when={rows.length > 0}>
          <div class="flex flex-col gap-2 overflow-x-auto">
            <div class={cn(columns, "min-w-[48rem] text-xs text-muted-foreground")}>
              <span>Description</span>
              <span class="text-right">Amount</span>
              <span>Type</span>
              <span>Date</span>
              <span>Effective date</span>
              <span />
            </div>
            <For each={rows}>
              {(r, i) => (
                <div class={cn(columns, "min-w-[48rem]")}>
                  <input
                    class={fieldClass}
                    placeholder="Description"
                    aria-label={`Row ${i() + 1} description`}
                    value={r.description}
                    onInput={(e) => update(r.key, { description: e.currentTarget.value })}
                  />
                  <input
                    class={cn(fieldClass, "text-right tabular-nums", r.amount && amountCents(r.amount) === null && "border-red-500")}
                    inputmode="decimal"
                    placeholder="0.00"
                    aria-label={`Row ${i() + 1} amount`}
                    value={r.amount}
                    onInput={(e) => update(r.key, { amount: e.currentTarget.value })}
                  />
                  <select
                    class={fieldClass}
                    aria-label={`Row ${i() + 1} type`}
                    value={selectValue(r.type ?? "", props.types)}
                    onChange={(e) => update(r.key, { type: e.currentTarget.value || null })}
                  >
                    <option value="">No type</option>
                    <Show when={r.type && !props.types.includes(r.type)}>
                      <option value={r.type!}>{r.type}</option>
                    </Show>
                    <For each={props.types}>{(t) => <option value={t}>{t}</option>}</For>
                  </select>
                  <input type="date" class={fieldClass} aria-label={`Row ${i() + 1} date`} value={r.date} onInput={(e) => setDate(r, e.currentTarget.value)} />
                  <input
                    type="date"
                    class={fieldClass}
                    aria-label={`Row ${i() + 1} effective date`}
                    value={r.effective_date ?? ""}
                    onInput={(e) => update(r.key, { effective_date: e.currentTarget.value || null, effectiveEdited: true })}
                  />
                  <button
                    type="button"
                    onClick={() => setRows((all) => all.filter((x) => x.key !== r.key))}
                    class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-500"
                    aria-label={`Remove row ${i() + 1}`}
                    title="Remove this row"
                  >
                    <X size={14} />
                  </button>
                </div>
              )}
            </For>
          </div>

          <div class="flex items-center justify-between gap-2 text-sm">
            <Button size="sm" variant="ghost" class="h-8" onClick={() => setRows((all) => [...all, blank()])} disabled={rows.length >= 20}>
              <Plus size={14} /> Add row
            </Button>
            <span class={cn("tabular-nums", total() === 0 ? "text-muted-foreground" : "text-red-500")}>
              {total() === 0 ? "Adds up to 0" : `Adds up to ${formatAmount(formatCents(total()))}; must be 0`}
            </span>
          </div>
        </Show>

        <Show when={error()}>
          <p role="alert" class="text-sm text-red-500">{error()}</p>
        </Show>
        <div class="flex flex-wrap items-center gap-2">
          <Show when={p.splits.length}>
            <Button variant="ghost" class="text-red-500 hover:text-red-600" disabled={saving()} onClick={() => void submit(true)}>
              <Trash size={16} /> Remove split
            </Button>
          </Show>
          <span class="flex-1" />
          <Button variant="outline" disabled={saving()} onClick={() => dialog.close()}>
            Cancel
          </Button>
          <Button disabled={!canSave()} onClick={() => void submit()}>
            <Show when={saving()}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            Save split
          </Button>
        </div>
      </div>
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
