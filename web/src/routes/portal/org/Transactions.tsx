import { createContext, createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show, useContext } from "solid-js";
import { Link } from "@tanstack/solid-router";
import type { ColumnDef } from "@tanstack/solid-table";
import CalendarClock from "lucide-solid/icons/calendar-clock";
import Check from "lucide-solid/icons/check";
import ChevronLeft from "lucide-solid/icons/chevron-left";
import Ellipsis from "lucide-solid/icons/ellipsis";
import ChevronUp from "lucide-solid/icons/chevron-up";
import Hand from "lucide-solid/icons/hand";
import Paperclip from "lucide-solid/icons/paperclip";
import PencilLine from "lucide-solid/icons/pencil-line";
import Printer from "lucide-solid/icons/printer";
import Settings2 from "lucide-solid/icons/settings-2";
import Split from "lucide-solid/icons/split";
import X from "lucide-solid/icons/x";
import { DataGrid, distinct, type GridColumnMeta, type GridView, pickFilter, ToggleButton } from "~/components/DataGrid";
import { Menu, menuItemClass, useMenu } from "~/components/ui/menu";
import { can } from "~/lib/orgs";
import { applyChips, type Chip, fetchFilterGroups, type FilterGroup, groupChips } from "~/lib/quickFilters";
import {
  amountValue,
  dateBasis,
  dayOf,
  fetchOrgTransactions,
  fetchTypes,
  formatAmount,
  formatDate,
  formatMoney,
  isNegative,
  ledgerOrder,
  type OrgTransaction,
  runningBalances,
  setRowType,
} from "~/lib/statements";
import { cn, selectValue, settled } from "~/lib/utils";
import { Allowed } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { type PrintJob, PrintDialog } from "~/routes/portal/org/PrintDialog";
import { amountFigures, chooseStat, formatFigure, type Stat, STATS, stat, statLabel, statValue } from "~/lib/amountStat";
import { RowPanel, RowsPanel } from "~/routes/portal/org/RowPanel";
import { type AccountState, StatementsShell } from "~/routes/portal/org/StatementsShell";

type Meta = GridColumnMeta<OrgTransaction>;

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/** `Feb 2026` -> `2026-02`, to put labels in date order. */
/** `2026-02-28` -> `Feb 2026`, as the server labels months. */
const monthLabel = (iso: string) => `${MONTHS[Number(iso.slice(5, 7)) - 1]} ${iso.slice(0, 4)}`;
const monthKey = (label: string) => `${label.slice(-4)}-${String(MONTHS.indexOf(label.slice(0, 3)) + 1).padStart(2, "0")}`;

/** Months newest first (in date order, not alphabetically). */
const monthOptions = (rows: OrgTransaction[]) =>
  [...new Set(rows.map((r) => r.month))]
    .sort((a, b) => monthKey(b).localeCompare(monthKey(a)))
    .map((m) => ({ value: m, label: m }));

/** Where the Balance column goes, when the account has an opening balance. */
const BALANCE_SLOT = null;

/**
 * The balance after each row (see `runningBalances`): over all the account's rows, whatever the
 * grid filters, so a filtered row still shows the account's true balance.
 */
const balanceColumn = (balances: () => Map<number, number>): ColumnDef<OrgTransaction, any> => ({
  id: "balance",
  accessorFn: (t) => balances().get(t.id) ?? null,
  header: "Balance",
  cell: (c) => (
    <Show when={balances().has(c.row.original.id)} fallback={<span class="text-muted-foreground/50">—</span>}>
      <span class="whitespace-nowrap font-mono tabular-nums text-muted-foreground">{formatMoney(balances().get(c.row.original.id)!)}</span>
    </Show>
  ),
  sortingFn: "basic",
  meta: { width: "w-36", align: "right", label: "Balance (running)" } satisfies Meta,
});

/** The grid's columns, with Balance only when there are balances to show. */
const columnsFor = (balances: (() => Map<number, number>) | null): ColumnDef<OrgTransaction, any>[] =>
  allColumns.flatMap((c) => (c === BALANCE_SLOT ? (balances ? [balanceColumn(balances)] : []) : [c]));

// Order, Month and balances always go by the transaction date, as the bank's statement does, so
// the balance matches it. Effective dates only change what quick filters pick.
const allColumns: (ColumnDef<OrgTransaction, any> | typeof BALANCE_SLOT)[] = [
  {
    accessorKey: "month",
    header: "Month",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{c.getValue<string>()}</span>,
    // Chronological, not alphabetical.
    sortingFn: (a, b) => monthKey(a.original.month).localeCompare(monthKey(b.original.month)),
    filterFn: pickFilter,
    meta: { width: "w-32", pick: { options: monthOptions } } satisfies Meta,
  },
  {
    accessorKey: "date",
    header: "Date",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{formatDate(c.getValue<string>())}</span>,
    // Same-day rows in the order they were stored (reversed when descending), the order a running
    // balance runs in, so each day's balances read in order either way.
    sortingFn: (a, b) => ledgerOrder("transaction")(a.original, b.original),
    meta: { width: "w-32" } satisfies Meta,
  },
  {
    id: "effective_date",
    accessorFn: (t) => t.effective_date ?? "",
    header: "Effective",
    cell: (c) => (
      <Show when={c.row.original.effective_date} fallback={<span class="text-muted-foreground/50">—</span>}>
        {(d) => <span class="whitespace-nowrap text-muted-foreground">{formatDate(d())}</span>}
      </Show>
    ),
    meta: { width: "w-32", label: "Effective date" } satisfies Meta,
  },
  {
    accessorKey: "description",
    header: "Description",
    // Kept exactly as the statement gives it. Split rows (added by hand) and rows that have a split
    // are marked.
    cell: (c) => (
      <span class="whitespace-pre-wrap break-words">
        <Show when={c.row.original.hand_added && c.row.original.split_of === null}>
          <span class="mr-1.5 inline-flex align-[-2px] text-muted-foreground" title="Added by hand (not from a bank statement)">
            <PencilLine size={13} />
          </span>
        </Show>
        <Show when={c.row.original.split_of !== null || c.row.original.split_rows > 0}>
          <span
            class="mr-1.5 inline-flex align-[-2px] text-muted-foreground"
            title={c.row.original.split_of !== null ? "Part of a split (added by hand)" : `Split into ${c.row.original.split_rows} rows`}
          >
            <Split size={13} />
          </span>
        </Show>
        {c.getValue<string>()}
      </span>
    ),
    enableHiding: false,
  },
  {
    id: "amount",
    accessorFn: (t) => amountValue(t.amount),
    header: "Amount",
    cell: (c) => (
      <span class={cn("whitespace-nowrap font-mono tabular-nums", isNegative(c.row.original.amount) ? "" : "text-emerald-600 dark:text-emerald-400")}>
        {formatAmount(c.row.original.amount)}
      </span>
    ),
    sortingFn: "basic",
    meta: { width: "w-32", align: "right" } satisfies Meta,
  },
  BALANCE_SLOT,
  {
    accessorKey: "currency",
    header: "Currency",
    cell: (c) => <span class="text-muted-foreground">{c.getValue<string>()}</span>,
    meta: { width: "w-24" } satisfies Meta,
  },
  {
    id: "type",
    accessorFn: (t) => t.type ?? "",
    header: "Category",
    cell: (c) => <TypeCell row={c.row.original} />,
    filterFn: pickFilter,
    meta: { width: "w-44", pick: { options: (rows) => distinct(rows, (r) => r.type), emptyLabel: "No category" } } satisfies Meta,
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
          aria-label="Category"
          title={props.row.type_manual ? "Set by hand" : props.row.type ? "Set by a category rule" : "No category"}
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
  const figures = createMemo(() => amountFigures(props.rows));
  const value = statValue;
  const fmt = formatFigure;
  const label = statLabel;

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

function readActive(key: string): Set<string> {
  try {
    const saved = JSON.parse(localStorage.getItem(key) ?? "[]");
    return new Set(Array.isArray(saved) ? saved.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function writeActive(key: string, active: Set<string>) {
  try {
    localStorage.setItem(key, JSON.stringify([...active]));
  } catch {
    // storage blocked: just don't remember
  }
}

/**
 * The quick filters, in a "⋯" menu by the grid's Columns button: the groups, each opening a
 * panel of its toggles. Within a group, toggled filters combine with OR; groups with AND. The
 * button shows how many are on. Toggles keep the menu open; Escape or a click outside closes it.
 */
function QuickFilterMenu(props: {
  groups: { group: FilterGroup; chips: Chip[] }[];
  active: Set<string>;
  onToggle: (id: string) => void;
  onClear: () => void;
}) {
  const org = useOrg();
  const [open, setOpen] = createSignal(false);
  // The group whose toggles are showing.
  const [shown, setShown] = createSignal<string | null>(null);
  let root!: HTMLDivElement;
  let button!: HTMLButtonElement;

  const activeChips = () => props.groups.flatMap((g) => g.chips.filter((c) => props.active.has(c.id)));
  const onIn = (g: { chips: Chip[] }) => g.chips.filter((c) => props.active.has(c.id)).length;
  const close = (refocus = false) => {
    setOpen(false);
    setShown(null);
    if (refocus) button.focus();
  };

  const onPointerDown = (e: PointerEvent) => {
    if (open() && !root.contains(e.target as Node)) close();
  };
  document.addEventListener("pointerdown", onPointerDown);
  onCleanup(() => document.removeEventListener("pointerdown", onPointerDown));

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || !open()) return;
    e.preventDefault();
    // Escape steps back: the group's panel first, then the menu.
    if (shown()) setShown(null);
    else close(true);
  };

  const panel = "absolute z-30 w-60 rounded-lg border border-border bg-background p-1 text-foreground shadow-xl";
  const item =
    "flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm outline-none transition-colors hover:bg-accent focus-visible:bg-accent";

  return (
    <div ref={root} class="relative" onKeyDown={onKeyDown}>
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open()}
        aria-label="Quick filters"
        title={activeChips().length ? `Quick filters: ${activeChips().map((c) => c.label).join(", ")}` : "Quick filters"}
        onClick={() => (open() ? close() : setOpen(true))}
        class={cn(
          "relative inline-flex h-9 w-9 items-center justify-center rounded-md border text-sm hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          activeChips().length ? "border-primary text-foreground" : "border-input",
        )}
      >
        <Ellipsis size={16} />
        <Show when={activeChips().length}>
          <span class="absolute -right-1.5 -top-1.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-semibold text-primary-foreground">
            {activeChips().length}
          </span>
        </Show>
      </button>

      <Show when={open()}>
        <div role="menu" aria-label="Quick filters" class={cn(panel, "right-0 top-full mt-1")}>
          <p class="px-2.5 pb-1 pt-2 text-xs font-medium text-muted-foreground">Quick filters</p>
          <For
            each={props.groups}
            fallback={<p class="px-2.5 py-2 text-sm text-muted-foreground">None set up yet.</p>}
          >
            {(g) => (
              <div class="relative">
                <button
                  type="button"
                  role="menuitem"
                  aria-haspopup="menu"
                  aria-expanded={shown() === g.group.name}
                  onClick={() => setShown(shown() === g.group.name ? null : g.group.name)}
                  class={cn(item, shown() === g.group.name && "bg-accent")}
                >
                  <ChevronLeft size={14} class="text-muted-foreground" />
                  <span class="min-w-0 flex-1 truncate">{g.group.name}</span>
                  <Show when={onIn(g)}>
                    <span class="rounded-full bg-primary px-1.5 text-[10px] font-semibold text-primary-foreground">{onIn(g)}</span>
                  </Show>
                </button>
                {/* The group's toggles, to the left (the menu sits at the grid's right edge). */}
                <Show when={shown() === g.group.name}>
                  <div role="menu" aria-label={g.group.name} class={cn(panel, "right-full top-0 mr-1 max-h-80 overflow-y-auto")}>
                    <p class="px-2.5 pb-1 pt-2 text-xs font-medium text-muted-foreground">{g.group.name}</p>
                    <For each={g.chips}>
                      {(c) => (
                        <button
                          type="button"
                          role="menuitemcheckbox"
                          aria-checked={props.active.has(c.id)}
                          title={c.title}
                          onClick={() => props.onToggle(c.id)}
                          class={item}
                        >
                          <span
                            class={cn(
                              "flex h-4 w-4 shrink-0 items-center justify-center rounded border",
                              props.active.has(c.id) ? "border-primary bg-primary text-primary-foreground" : "border-input",
                            )}
                          >
                            <Show when={props.active.has(c.id)}>
                              <Check size={12} />
                            </Show>
                          </span>
                          <span class="min-w-0 flex-1 truncate">{c.label}</span>
                        </button>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            )}
          </For>
          <div role="separator" class="my-1 h-px bg-border" />
          <Show when={activeChips().length}>
            <button type="button" role="menuitem" onClick={props.onClear} class={item}>
              <X size={14} class="text-muted-foreground" /> Clear quick filters
            </button>
          </Show>
          <Show when={can(org, "write-statements")}>
            <Link to="/portal/$org/statements/filters" params={{ org: org.slug }} role="menuitem" class={item} onClick={() => close()}>
              <Settings2 size={14} class="text-muted-foreground" /> Edit quick filters
            </Link>
          </Show>
        </div>
      </Show>
    </div>
  );
}

function TransactionsView(props: { acct: AccountState }) {
  const org = useOrg();
  // Each result carries its account, so rows from the previous account never show under the new one.
  const [rows, { mutate, refetch }] = createResource(
    () => {
      const a = props.acct.account();
      return a ? { account: a.id } : undefined;
    },
    async ({ account }) => ({ account, rows: await fetchOrgTransactions(org.slug, account) }),
  );
  const current = () => {
    const data = settled(rows);
    const a = props.acct.account();
    return data && a && data.account === a.id ? data : undefined;
  };
  const error = () => rows.error ?? props.acct.accounts.error;
  // Which date quick filters and Month go by (shared with printing). Order, shading and balances never change with it.
  const [basis, setBasis] = dateBasis(org.slug);
  // The account's opening balance, if it has one: the grid then offers a running Balance column.
  const opening = createMemo(
    () => {
      const a = props.acct.account();
      return a && a.opening_balance_cents !== null && a.opening_balance_date
        ? { cents: a.opening_balance_cents, date: a.opening_balance_date }
        : null;
    },
    null,
    { equals: (a, b) => a?.cents === b?.cents && a?.date === b?.date },
  );
  const balances = createMemo(() => {
    const o = opening();
    const data = current();
    return o && data ? runningBalances(data.rows, o, "transaction") : new Map<number, number>();
  });
  // Rebuilt only when the account (or its opening balance) changes, so editing a row keeps
  // filters, sorting and page.
  const gridKey = createMemo(() => {
    const o = opening();
    return current() ? `account:${current()!.account}:${o ? `${o.cents}@${o.date}` : "-"}` : undefined;
  });
  const columns = createMemo(() => columnsFor(opening() ? balances : null));

  const [types, { refetch: refetchTypes }] = createResource(() => org.slug, fetchTypes);

  // Quick filters: chips above the grid. Toggled ones are remembered per org in this browser.
  const [filterGroups] = createResource(() => org.slug, fetchFilterGroups);
  const chipGroups = createMemo(() =>
    (settled(filterGroups) ?? [])
      .map((group) => ({ group, chips: groupChips(group, current()?.rows ?? [], basis()) }))
      .filter((g) => g.chips.length > 0),
  );
  const activeKey = `quick-filters:${org.slug}`;
  const [active, setActive] = createSignal<Set<string>>(readActive(activeKey));
  const toggle = (id: string) => {
    const next = new Set(active());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setActive(next);
    writeActive(activeKey, next);
  };
  const clearChips = () => {
    setActive(new Set<string>());
    writeActive(activeKey, new Set());
  };
  // Oldest first (the server sends newest first): the grid's order when nothing is sorted, and
  // its tie-break when a sort leaves rows equal, so the Balance column reads top to bottom.
  // Order stays by transaction date; with effective dates on, a row's Month (and so the Month filter) is its
  // effective date's.
  const ledgerRows = createMemo(() => {
    const rows = [...(current()?.rows ?? [])].sort(ledgerOrder("transaction"));
    return basis() === "effective"
      ? rows.map((r) => (r.effective_date ? { ...r, month: monthLabel(r.effective_date) } : r))
      : rows;
  });
  const shownRows = createMemo(() => applyChips(ledgerRows(), chipGroups(), active(), basis()));

  // Printing: exactly what the grid shows, with the quick filters and grid filters in words.
  let gridView: (() => GridView<OrgTransaction>) | undefined;
  const [printJob, setPrintJob] = createSignal<PrintJob | null>(null);
  const openPrint = () => {
    const account = props.acct.account();
    const view = gridView?.();
    if (!account || !view) return;
    const quick = chipGroups()
      .map((g) => ({ name: g.group.name, on: g.chips.filter((c) => active().has(c.id)).map((c) => c.label) }))
      .filter((g) => g.on.length > 0)
      .map((g) => `${g.name}: ${g.on.join(" or ")}`);
    // The period: the dates of the active date quick filters (their whole range, even where no
    // row falls), otherwise the print dialog uses the rows' first and last dates.
    const dated = chipGroups()
      .flatMap((g) => g.chips)
      .filter((c) => active().has(c.id) && (c.filter.date_from || c.filter.date_to))
      .map((c) => c.filter);
    const dates = view.rows.map((r) => dayOf(r, basis())).sort();
    const bound = (values: (string | null)[], pick: (a: string, b: string) => string) =>
      values.every((v) => v) ? (values as string[]).reduce(pick) : null;
    const period = dated.length
      ? {
          from: bound(dated.map((f) => f.date_from), (a, b) => (a < b ? a : b)) ?? dates[0] ?? "",
          to: bound(dated.map((f) => f.date_to), (a, b) => (a > b ? a : b)) ?? dates[dates.length - 1] ?? "",
        }
      : null;
    setPrintJob({ account, rows: view.rows, selection: [...quick, ...view.filters], period: period?.from && period.to ? period : null });
  };
  /** Updates one row in place (the grid keeps its filters, sorting and page). */
  const patchRow = (id: number, patch: Partial<OrgTransaction>) =>
    mutate((d) => d && { ...d, rows: d.rows.map((r) => (r.id === id ? { ...r, ...patch } : r)) });
  // The row whose details are open in the side panel.
  const [openRow, setOpenRow] = createSignal<number | null>(null);
  // Rows selected by dragging in the grid, and whether the panel for them is open (instead of one row's).
  const [selection, setSelection] = createSignal<OrgTransaction[]>([]);
  const [selectionOpen, setSelectionOpen] = createSignal(false);
  const openSelection = () => {
    setOpenRow(null);
    setSelectionOpen(true);
  };
  const openOne = (id: number) => {
    setSelectionOpen(false);
    setOpenRow(id);
  };
  // Fewer than two rows left selected (deleted): the panel for them goes. (While a panel is open
  // its backdrop covers the grid, so the selection doesn't change any other way.)
  createEffect(() => {
    if (selection().length < 2) setSelectionOpen(false);
  });
  createEffect(
    on(gridKey, () => {
      setOpenRow(null);
      setSelectionOpen(false);
    }),
  );
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
        setEditError(e instanceof Error ? e.message : "Couldn't change the category.");
        void refetchTypes(); // e.g. the type was renamed or deleted meanwhile
        throw e;
      }
    },
  };

  return (
    <div class="mt-3 flex min-h-0 flex-1 flex-col">
      <Show when={error()}>
        {(e) => <p class="text-sm text-red-500">{e() instanceof Error ? (e() as Error).message : "Couldn't load transactions."}</p>}
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
                rows={shownRows()}
                viewRef={(v) => (gridView = v)}
                toolbarEnd={
                  <>
                    <QuickFilterMenu groups={chipGroups()} active={active()} onToggle={toggle} onClear={clearChips} />
                    <button
                      type="button"
                      onClick={openPrint}
                      aria-label="Print statement PDF"
                      title="Print what the grid shows as a PDF"
                      class="inline-flex h-9 w-9 items-center justify-center rounded-md border border-input hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <Printer size={16} />
                    </button>
                  </>
                }
                columns={columns()}
                initialSorting={[{ id: "date", desc: false }]}
                search={(t, q) =>
                  [t.description, t.type ?? "", t.amount, formatAmount(t.amount).replace("−", "-"), t.month].some((v) => v.toLowerCase().includes(q))
                }
                searchPlaceholder="Search description, category, amount, month"
                storageKey="statements-grid-columns"
                defaultHidden={["currency", "txn_id", "updated_at"]}
                noun={["row", "rows"]}
                status={(shown) => <AmountStat rows={shown} />}
                statusBar
                statusToggles={
                  <ToggleButton
                    label="Effective dates"
                    hint={
                      basis() === "effective"
                        ? "On: date quick filters, the Month column (and its filter) and printing count a row with an effective date on that day. Order, shading and balances stay by transaction date. Click to use transaction dates."
                        : "Off: date quick filters, the Month column and printing go by transaction date. Click to count a row with an effective date on that day instead. Order, shading and balances stay by transaction date."
                    }
                    icon={CalendarClock}
                    on={basis() === "effective"}
                    onChange={(on) => setBasis(on ? "effective" : "transaction")}
                  />
                }
                onRowClick={(r, selected) => (selected.length > 1 ? openSelection() : openOne(r.id))}
                rowKey={(r) => r.id}
                onSelectionChange={setSelection}
                selectionActions={(rows) => (
                  <Show when={rows.length > 1}>
                    <button
                      type="button"
                      onClick={openSelection}
                      class="rounded px-1.5 font-medium text-foreground underline-offset-2 hover:bg-accent hover:underline"
                      title="Change the selected rows' details together"
                    >
                      Edit…
                    </button>
                  </Show>
                )}
                // Rows with documents attached: a light blue tint and a thin bar on the left.
                rowClass={(r) =>
                  r.attachments > 0
                    ? "bg-sky-500/[0.07] shadow-[inset_3px_0_0_var(--color-sky-500)] hover:bg-sky-500/[0.12]"
                    : undefined
                }
                isSelected={(r) => r.id === openRow()}
                // Shading always follows the transaction month (as the order does), whatever Month shows.
                bandBy={(r) => r.date.slice(0, 7)}
              />
            </RowEditingContext.Provider>
          </Show>
        )}
      </Show>
      <PrintDialog job={printJob()} onClose={() => setPrintJob(null)} />
      <Show when={openRow()} keyed>
        {(id) => (
          <RowPanel
            rowId={id}
            types={editing.types()}
            setType={editing.setType}
            onChanged={(c) =>
              patchRow(c.id, { type: c.type, type_manual: c.type_manual, effective_date: c.effective_date, attachments: c.attachments })
            }
            onRowsChanged={() => void refetch()}
            onOpenRow={openOne}
            onClose={() => setOpenRow(null)}
          />
        )}
      </Show>
      {/* Goes with the selection, in the same update (an effect closing it would run after the
          panel had already recomputed for no rows). */}
      <Show when={selectionOpen() && selection().length > 1}>
        <RowsPanel
          rows={selection()}
          types={editing.types()}
          setType={editing.setType}
          onPatched={patchRow}
          onRowsChanged={() => void refetch()}
          onClose={() => setSelectionOpen(false)}
        />
      </Show>
    </div>
  );
}

function Transactions() {
  return (
    // Fills the window's height, so only the grid's rows scroll (see `pageHasGrid`).
    <StatementsShell fill>{(acct) => <TransactionsView acct={acct} />}</StatementsShell>
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
