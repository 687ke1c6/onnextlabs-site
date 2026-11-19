import { createContext, createEffect, createMemo, createResource, createSignal, For, on, Show, useContext } from "solid-js";
import { Link } from "@tanstack/solid-router";
import type { ColumnDef } from "@tanstack/solid-table";
import Check from "lucide-solid/icons/check";
import ChevronUp from "lucide-solid/icons/chevron-up";
import Hand from "lucide-solid/icons/hand";
import Paperclip from "lucide-solid/icons/paperclip";
import { DataGrid, distinct, type GridColumnMeta, pickFilter } from "~/components/DataGrid";
import { Menu, menuItemClass, useMenu } from "~/components/ui/menu";
import { can } from "~/lib/orgs";
import {
  accountLabel,
  amountValue,
  fetchOrgTransactions,
  fetchTypes,
  formatAmount,
  formatDate,
  isNegative,
  type OrgTransaction,
  setRowType,
} from "~/lib/statements";
import { cn, selectValue, settled } from "~/lib/utils";
import { Allowed, PageHeader } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { RowPanel } from "~/routes/portal/org/RowPanel";
import { AccountPicker, StatementsTabs, useStatementAccount } from "~/routes/portal/org/StatementParts";

type Meta = GridColumnMeta<OrgTransaction>;

/** Months newest first (by their dates, not alphabetically). */
const monthOptions = (rows: OrgTransaction[]) => {
  const latest = new Map<string, string>();
  for (const r of rows) if ((latest.get(r.month) ?? "") < r.date) latest.set(r.month, r.date);
  return [...latest.entries()].sort((a, b) => b[1].localeCompare(a[1])).map(([m]) => ({ value: m, label: m }));
};

const columns: ColumnDef<OrgTransaction, any>[] = [
  {
    accessorKey: "date",
    header: "Date",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{formatDate(c.getValue<string>())}</span>,
    meta: { width: "w-32" } satisfies Meta,
  },
  {
    accessorKey: "month",
    header: "Month",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{c.getValue<string>()}</span>,
    // Chronological, not alphabetical.
    sortingFn: (a, b) => a.original.date.slice(0, 7).localeCompare(b.original.date.slice(0, 7)),
    filterFn: pickFilter,
    meta: { width: "w-32", pick: { options: monthOptions } } satisfies Meta,
  },
  {
    accessorKey: "description",
    header: "Description",
    // Kept exactly as stored (trailing spaces and "_" suffixes are part of the id).
    cell: (c) => <span class="whitespace-pre-wrap break-words">{c.getValue<string>()}</span>,
    enableHiding: false,
  },
  {
    id: "amount",
    accessorFn: (t) => amountValue(t.amount),
    header: "Amount",
    cell: (c) => (
      <span class={cn("whitespace-nowrap tabular-nums", isNegative(c.row.original.amount) ? "" : "text-emerald-600 dark:text-emerald-400")}>
        {formatAmount(c.row.original.amount)}
      </span>
    ),
    sortingFn: "basic",
    meta: { width: "w-32", align: "right" } satisfies Meta,
  },
  {
    accessorKey: "currency",
    header: "Currency",
    cell: (c) => <span class="text-muted-foreground">{c.getValue<string>()}</span>,
    meta: { width: "w-24" } satisfies Meta,
  },
  {
    id: "type",
    accessorFn: (t) => t.type ?? "",
    header: "Type",
    cell: (c) => <TypeCell row={c.row.original} />,
    filterFn: pickFilter,
    meta: { width: "w-44", pick: { options: (rows) => distinct(rows, (r) => r.type), emptyLabel: "No type" } } satisfies Meta,
  },
  {
    accessorKey: "attachments",
    header: "Docs",
    cell: (c) => (
      <Show when={c.getValue<number>() > 0} fallback={<span class="text-muted-foreground/50">—</span>}>
        <span class="inline-flex items-center gap-1 text-muted-foreground" title={`${c.getValue<number>()} attached`}>
          <Paperclip size={12} />
          <span class="tabular-nums">{c.getValue<number>()}</span>
        </span>
      </Show>
    ),
    meta: { width: "w-20", label: "Documents attached" } satisfies Meta,
  },
  {
    accessorKey: "txn_id",
    header: "Id",
    cell: (c) => <span class="font-mono text-xs text-muted-foreground">{c.getValue<number>()}</span>,
    meta: { width: "w-32" } satisfies Meta,
  },
  {
    accessorKey: "updated_at",
    header: "Last saved",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{formatDate(c.getValue<string>())}</span>,
    meta: { width: "w-32" } satisfies Meta,
  },
];

/** What the Type cells need from the page: the types list and how to save a change. */
type RowEditing = {
  canEdit: boolean;
  types: () => string[];
  setType: (rowId: number, type: string | null) => Promise<void>;
};
const RowEditingContext = createContext<RowEditing>();

/** A row's type: a dropdown of the org's types for editors, a label for everyone else. */
function TypeCell(props: { row: OrgTransaction }) {
  const edit = useContext(RowEditingContext);
  const [busy, setBusy] = createSignal(false);
  const manual = () =>
    props.row.type_manual && (
      <span title="Set by hand: saving a statement won't change it" class="shrink-0 text-muted-foreground">
        <Hand size={11} />
      </span>
    );
  return (
    <Show
      when={edit?.canEdit}
      fallback={
        <span class="inline-flex items-center gap-1">
          <Show when={props.row.type} fallback={<span class="text-muted-foreground">—</span>}>
            <span class="whitespace-nowrap rounded-full border border-border bg-secondary/40 px-2 py-0.5 text-xs font-medium">
              {props.row.type}
            </span>
          </Show>
          {manual()}
        </span>
      }
    >
      <span class="inline-flex w-full items-center gap-1">
        <select
          value={selectValue(props.row.type ?? "", edit!.types())}
          disabled={busy()}
          onClick={(e) => e.stopPropagation()}
          onChange={async (e) => {
            const select = e.currentTarget;
            setBusy(true);
            try {
              await edit!.setType(props.row.id, select.value || null);
            } catch {
              select.value = props.row.type ?? ""; // put it back; the page shows the error
            } finally {
              setBusy(false);
            }
          }}
          aria-label="Type"
          title={props.row.type_manual ? "Set by hand" : props.row.type ? "Set by a type rule" : "No type"}
          class={cn(
            "h-7 min-w-0 flex-1 rounded border border-transparent bg-transparent px-1 text-xs hover:border-input focus:border-input focus:outline-none disabled:opacity-50",
            props.row.type ? "font-medium" : "text-muted-foreground",
          )}
        >
          <option value="">—</option>
          {/* A name no longer on the list still shows, so the row reads right. */}
          <Show when={props.row.type && !edit!.types().includes(props.row.type)}>
            <option value={props.row.type!}>{props.row.type}</option>
          </Show>
          <For each={edit!.types()}>{(t) => <option value={t}>{t}</option>}</For>
        </select>
        {manual()}
      </span>
    </Show>
  );
}

type Stat = "sum" | "average" | "min" | "max";
const STATS: { id: Stat; label: string }[] = [
  { id: "sum", label: "Sum" },
  { id: "average", label: "Average" },
  { id: "min", label: "Min" },
  { id: "max", label: "Max" },
];
const STAT_KEY = "statements-amount-stat";

function initialStat(): Stat {
  try {
    const saved = localStorage.getItem(STAT_KEY);
    if (STATS.some((s) => s.id === saved)) return saved as Stat;
  } catch {
    // storage blocked: use the default
  }
  return "sum";
}
// Shared by every Transactions grid, like the other view settings.
const [stat, setStat] = createSignal<Stat>(initialStat());
const chooseStat = (s: Stat) => {
  setStat(s);
  try {
    localStorage.setItem(STAT_KEY, s);
  } catch {
    // just don't remember
  }
};

function StatChoice(props: { id: Stat; label: string }) {
  const menu = useMenu();
  return (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={stat() === props.id}
      onClick={() => {
        chooseStat(props.id);
        menu.close();
      }}
      class={menuItemClass}
    >
      <Check size={14} class={cn("shrink-0", stat() !== props.id && "invisible")} />
      {props.label}
    </button>
  );
}

/**
 * Sum, average, min or max of Amount over the rows that pass the filters (all rows when none are
 * on), per currency; click to choose which, as in a spreadsheet's status bar.
 */
function AmountStat(props: { rows: OrgTransaction[] }) {
  const figures = createMemo(() => {
    const by = new Map<string, { sum: number; in: number; out: number; n: number; min: number; max: number }>();
    for (const t of props.rows) {
      const v = amountValue(t.amount);
      if (!Number.isFinite(v)) continue;
      const f = by.get(t.currency) ?? { sum: 0, in: 0, out: 0, n: 0, min: Infinity, max: -Infinity };
      f.sum += v;
      if (v >= 0) f.in += v;
      else f.out += v;
      f.n += 1;
      f.min = Math.min(f.min, v);
      f.max = Math.max(f.max, v);
      by.set(t.currency, f);
    }
    return [...by.entries()];
  });
  const fmt = (n: number) =>
    `${n < 0 ? "−" : ""}${Math.abs(n).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
  const value = (f: { sum: number; n: number; min: number; max: number }) =>
    stat() === "sum" ? f.sum : stat() === "average" ? f.sum / f.n : stat() === "min" ? f.min : f.max;
  const label = () => STATS.find((s) => s.id === stat())!.label;

  return (
    <Show when={figures().length > 0}>
      <Menu
        label={`Amount: ${label()} (choose sum, average, min or max)`}
        title={`${label()} of Amount for the rows shown. Click to change.`}
        trigger={
          <>
            <span>{label()}</span>
            <For each={figures()}>
              {([currency, f]) => (
                <span class="whitespace-nowrap">
                  <span class={cn("font-semibold tabular-nums", value(f) < 0 ? "text-foreground" : "text-emerald-600 dark:text-emerald-400")}>
                    {fmt(value(f))}
                  </span>{" "}
                  {currency}
                </span>
              )}
            </For>
            <ChevronUp size={12} class="opacity-60" />
          </>
        }
        triggerClass="inline-flex items-center gap-1 rounded px-1 hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        class="w-40"
      >
        <For each={STATS}>{(s) => <StatChoice id={s.id} label={s.label} />}</For>
      </Menu>
    </Show>
  );
}

function NoTransactions() {
  const org = useOrg();
  return (
    <section class="rounded-xl border border-dashed border-border px-6 py-14 text-center">
      <h2 class="text-base font-medium">No transactions yet</h2>
      <p class="mt-1 text-sm text-muted-foreground">Rows appear here once a statement is saved.</p>
      <Show when={can(org, "write-statements")}>
        <Link
          to="/portal/$org/statements/uploads"
          params={{ org: org.slug }}
          class="mt-4 inline-flex h-9 items-center rounded-md border border-input px-3 text-sm font-medium hover:bg-accent"
        >
          Upload a statement
        </Link>
      </Show>
    </section>
  );
}

function TransactionsView(props: { acct: ReturnType<typeof useStatementAccount> }) {
  const org = useOrg();
  // Each result carries its account, so rows from the previous account never show under the new one.
  const [rows, { mutate }] = createResource(
    () => {
      const a = props.acct.account();
      return a ? { account: a.number } : undefined;
    },
    async ({ account }) => ({ account, rows: await fetchOrgTransactions(org.slug, account) }),
  );
  const current = () => {
    const data = settled(rows);
    const a = props.acct.account();
    return data && a && data.account === a.number ? data : undefined;
  };
  const error = () => rows.error ?? props.acct.accounts.error;
  // Rebuilt only when the account changes, so editing a row keeps filters, sorting and page.
  const gridKey = createMemo(() => (current() ? `account:${current()!.account}` : undefined));

  const [types, { refetch: refetchTypes }] = createResource(() => org.slug, fetchTypes);
  /** Updates one row in place (the grid keeps its filters, sorting and page). */
  const patchRow = (id: number, patch: Partial<OrgTransaction>) =>
    mutate((d) => d && { ...d, rows: d.rows.map((r) => (r.id === id ? { ...r, ...patch } : r)) });
  // The row whose details are open in the side panel.
  const [openRow, setOpenRow] = createSignal<number | null>(null);
  createEffect(on(gridKey, () => setOpenRow(null)));
  const [editError, setEditError] = createSignal<string | null>(null);
  const editing: RowEditing = {
    canEdit: can(org, "write-statements"),
    types: () => (settled(types) ?? []).map((t) => t.name),
    setType: async (rowId, type) => {
      setEditError(null);
      try {
        const saved = await setRowType(org.slug, rowId, type);
        patchRow(rowId, { type: saved.type, type_manual: saved.type_manual });
      } catch (e) {
        setEditError(e instanceof Error ? e.message : "Couldn't change the type.");
        void refetchTypes(); // e.g. the type was renamed or deleted meanwhile
        throw e;
      }
    },
  };

  return (
    <div class="mt-6 flex min-h-0 flex-1 flex-col">
      <Show when={error()}>
        {(e) => <p class="text-sm text-red-500">{e() instanceof Error ? (e() as Error).message : "Couldn't load transactions."}</p>}
      </Show>
      <Show when={props.acct.account() === null}>
        <NoTransactions />
      </Show>
      <Show when={props.acct.account() && !current() && !error()}>
        <div class="h-64 animate-pulse rounded-xl bg-secondary/40" />
      </Show>
      <Show when={editError()}>
        <p role="alert" class="mb-2 text-sm text-red-500">{editError()}</p>
      </Show>
      {/* Keyed by account: a new account starts with fresh filters and paging. */}
      <Show when={gridKey()} keyed>
        {(_account) => (
          <Show when={(current()?.rows.length ?? 0) > 0} fallback={<NoTransactions />}>
            <RowEditingContext.Provider value={editing}>
              <DataGrid
                rows={current()?.rows ?? []}
                columns={columns}
                initialSorting={[{ id: "date", desc: false }]}
                search={(t, q) => [t.description, t.type ?? "", t.amount, t.month].some((v) => v.toLowerCase().includes(q))}
                searchPlaceholder="Search description, type, amount, month"
                storageKey="statements-grid-columns"
                defaultHidden={["currency", "txn_id", "updated_at"]}
                noun={["row", "rows"]}
                status={(shown) => <AmountStat rows={shown} />}
                onRowClick={(r) => setOpenRow(r.id)}
                // Rows with documents attached: a light blue tint and a thin bar on the left.
                rowClass={(r) =>
                  r.attachments > 0
                    ? "bg-sky-500/[0.07] shadow-[inset_3px_0_0_var(--color-sky-500)] hover:bg-sky-500/[0.12]"
                    : undefined
                }
                isSelected={(r) => r.id === openRow()}
              />
            </RowEditingContext.Provider>
          </Show>
        )}
      </Show>
      <Show when={openRow()} keyed>
        {(id) => (
          <RowPanel
            rowId={id}
            types={editing.types()}
            setType={editing.setType}
            onChanged={(c) => patchRow(c.id, { type: c.type, type_manual: c.type_manual, attachments: c.attachments })}
            onClose={() => setOpenRow(null)}
          />
        )}
      </Show>
    </div>
  );
}

function Transactions() {
  const org = useOrg();
  const acct = useStatementAccount();
  const description = () => {
    const a = acct.account();
    return a ? `Every transaction saved for account ${accountLabel(a.number)}.` : `Transactions saved for ${org.name}.`;
  };
  return (
    // Hands its leftover height down to the grid (see `pageHasGrid`).
    <div class="flex min-h-0 flex-1 flex-col">
      <PageHeader title="Statements" description={description()} actions={<AccountPicker state={acct} />} />
      <StatementsTabs />
      <TransactionsView acct={acct} />
    </div>
  );
}

/** /portal/<org>/statements: every saved row for the chosen account. Needs `read-statements`. */
export function TransactionsPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <Transactions />
    </Allowed>
  );
}
