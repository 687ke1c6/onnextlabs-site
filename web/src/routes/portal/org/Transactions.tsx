import { createMemo, createResource, createSignal, For, type JSX, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Link } from "@tanstack/solid-router";
import {
  type ColumnDef,
  type ColumnFiltersState,
  createSolidTable,
  type FilterFn,
  flexRender,
  getCoreRowModel,
  getFacetedUniqueValues,
  getFilteredRowModel,
  getPaginationRowModel,
  getSortedRowModel,
  type SortingState,
  type VisibilityState,
} from "@tanstack/solid-table";
import ArrowDown from "lucide-solid/icons/arrow-down";
import ArrowUp from "lucide-solid/icons/arrow-up";
import ArrowUpDown from "lucide-solid/icons/arrow-up-down";
import Check from "lucide-solid/icons/check";
import ChevronLeft from "lucide-solid/icons/chevron-left";
import ChevronRight from "lucide-solid/icons/chevron-right";
import Columns3 from "lucide-solid/icons/columns-3";
import Funnel from "lucide-solid/icons/funnel";
import Search from "lucide-solid/icons/search";
import X from "lucide-solid/icons/x";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { Menu, menuItemClass, MenuSeparator } from "~/components/ui/menu";
import { can } from "~/lib/orgs";
import {
  amountValue,
  fetchOrgTransactions,
  formatAmount,
  formatDate,
  isNegative,
  type OrgTransaction,
} from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed, PageHeader } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { StatementsTabs } from "~/routes/portal/org/StatementParts";

/** Filter value for "rows without a type". */
const NO_TYPE = "\u0000none";
const PAGE_SIZES = [25, 50, 100, 250];
const VISIBILITY_KEY = "statements-grid-columns";

const SELECT =
  "h-9 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

const equals: FilterFn<OrgTransaction> = (row, id, value: string) => !value || row.getValue(id) === value;
const typeIs: FilterFn<OrgTransaction> = (row, _id, value: string) =>
  !value || (value === NO_TYPE ? !row.original.type : row.original.type === value);
/** `[from, to]`, either may be empty; compares YYYY-MM-DD strings. */
const between: FilterFn<OrgTransaction> = (row, id, [from, to]: [string, string]) => {
  const d = row.getValue<string>(id);
  return (!from || d >= from) && (!to || d <= to);
};
/** Search: description, type, account and amount, ignoring case. */
const search: FilterFn<OrgTransaction> = (row, _id, value: string) => {
  const q = value.trim().toLowerCase();
  if (!q) return true;
  const t = row.original;
  return [t.description, t.type ?? "", t.account_number, t.amount].some((v) => v.toLowerCase().includes(q));
};

const columns: ColumnDef<OrgTransaction>[] = [
  {
    accessorKey: "date",
    header: "Date",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{formatDate(c.getValue<string>())}</span>,
    filterFn: between,
    meta: { width: "w-32" },
  },
  {
    accessorKey: "account_number",
    header: "Account",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{c.getValue<string>() || "—"}</span>,
    filterFn: equals,
    meta: { width: "w-32" },
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
    meta: { width: "w-32", align: "right" },
  },
  {
    accessorKey: "currency",
    header: "Currency",
    cell: (c) => <span class="text-muted-foreground">{c.getValue<string>()}</span>,
    meta: { width: "w-24" },
  },
  {
    id: "type",
    accessorFn: (t) => t.type ?? "",
    header: "Type",
    cell: (c) => (
      <Show when={c.row.original.type} fallback={<span class="text-muted-foreground">—</span>}>
        <span class="whitespace-nowrap rounded-full border border-border bg-secondary/40 px-2 py-0.5 text-xs font-medium">
          {c.row.original.type}
        </span>
      </Show>
    ),
    filterFn: typeIs,
    meta: { width: "w-40" },
  },
  {
    accessorKey: "txn_id",
    header: "Id",
    cell: (c) => <span class="font-mono text-xs text-muted-foreground">{c.getValue<number>()}</span>,
    meta: { width: "w-32" },
  },
  {
    accessorKey: "updated_at",
    header: "Last saved",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{formatDate(c.getValue<string>())}</span>,
    meta: { width: "w-32" },
  },
];

type ColumnMeta = { width?: string; align?: "right" };
const meta = (m: unknown) => (m ?? {}) as ColumnMeta;

function initialVisibility(): VisibilityState {
  try {
    const saved = JSON.parse(localStorage.getItem(VISIBILITY_KEY) ?? "null");
    if (saved && typeof saved === "object") return saved;
  } catch {
    // fall back to the defaults
  }
  return { currency: false, txn_id: false, updated_at: false };
}

/** Money in, money out and net of the rows shown, per currency. */
function Totals(props: { rows: OrgTransaction[] }) {
  const totals = createMemo(() => {
    const by = new Map<string, { in: number; out: number }>();
    for (const t of props.rows) {
      const v = amountValue(t.amount);
      if (!Number.isFinite(v)) continue;
      const cur = by.get(t.currency) ?? { in: 0, out: 0 };
      if (v >= 0) cur.in += v;
      else cur.out += v;
      by.set(t.currency, cur);
    }
    return [...by.entries()];
  });
  const fmt = (n: number) => n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return (
    <For each={totals()}>
      {([currency, t]) => (
        <span class="whitespace-nowrap">
          {currency}: <span class="text-emerald-600 dark:text-emerald-400">+{fmt(t.in)}</span> /{" "}
          <span>−{fmt(Math.abs(t.out))}</span> · net <span class="font-medium text-foreground">{fmt(t.in + t.out)}</span>
        </span>
      )}
    </For>
  );
}

function Grid(props: { rows: OrgTransaction[] }) {
  const [sorting, setSorting] = createSignal<SortingState>([{ id: "date", desc: true }]);
  const [filters, setFilters] = createSignal<ColumnFiltersState>([]);
  const [query, setQuery] = createSignal("");
  const [visibility, setVisibility] = createSignal<VisibilityState>(initialVisibility());
  const [pagination, setPagination] = createSignal({ pageIndex: 0, pageSize: 50 });

  const table = createSolidTable({
    get data() {
      return props.rows;
    },
    columns,
    state: {
      get sorting() {
        return sorting();
      },
      get columnFilters() {
        return filters();
      },
      get globalFilter() {
        return query();
      },
      get columnVisibility() {
        return visibility();
      },
      get pagination() {
        return pagination();
      },
    },
    onSortingChange: setSorting,
    onColumnFiltersChange: (u) => {
      setFilters(u);
      setPagination((p) => ({ ...p, pageIndex: 0 }));
    },
    onGlobalFilterChange: (u) => {
      setQuery(u);
      setPagination((p) => ({ ...p, pageIndex: 0 }));
    },
    onColumnVisibilityChange: (u) => {
      const next = typeof u === "function" ? u(visibility()) : u;
      setVisibility(next);
      try {
        localStorage.setItem(VISIBILITY_KEY, JSON.stringify(next));
      } catch {
        // storage blocked: just don't remember
      }
    },
    onPaginationChange: setPagination,
    globalFilterFn: search,
    enableMultiSort: true,
    isMultiSortEvent: (e) => (e as MouseEvent).shiftKey,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    getFacetedUniqueValues: getFacetedUniqueValues(),
  });

  const filterValue = <T,>(id: string, empty: T): T => (filters().find((f) => f.id === id)?.value as T) ?? empty;
  const setFilter = (id: string, value: unknown) => table.getColumn(id)?.setFilterValue(value);
  const dateRange = () => filterValue<[string, string]>("date", ["", ""]);

  // Options come from all rows, so choosing one never hides the others.
  const accounts = createMemo(() => [...new Set(props.rows.map((r) => r.account_number))].sort());
  const types = createMemo(() => [...new Set(props.rows.map((r) => r.type).filter((t): t is string => !!t))].sort());

  const filtered = () => table.getFilteredRowModel().rows.map((r) => r.original);
  const anyFilter = () => !!query() || filters().length > 0;
  const clearAll = () => {
    setQuery("");
    setFilters([]);
    setPagination((p) => ({ ...p, pageIndex: 0 }));
  };

  const firstShown = () => (filtered().length === 0 ? 0 : pagination().pageIndex * pagination().pageSize + 1);
  const lastShown = () => Math.min(filtered().length, (pagination().pageIndex + 1) * pagination().pageSize);

  return (
    <div class="mt-6 flex flex-col gap-3">
      {/* Toolbar */}
      <div class="flex flex-wrap items-end gap-2">
        <div class="relative min-w-0 flex-[2_1_14rem]">
          <Search size={16} class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <Input
            class="h-9 pl-9"
            value={query()}
            onInput={(e) => table.setGlobalFilter(e.currentTarget.value)}
            placeholder="Search description, type, account, amount"
            aria-label="Search transactions"
          />
        </div>
        <Show when={accounts().length > 1}>
          <select
            class={cn(SELECT, "min-w-0 flex-[1_1_9rem]")}
            value={filterValue("account_number", "")}
            onChange={(e) => setFilter("account_number", e.currentTarget.value || undefined)}
            aria-label="Account"
          >
            <option value="">All accounts</option>
            <For each={accounts()}>{(a) => <option value={a}>{a || "No account"}</option>}</For>
          </select>
        </Show>
        <div class="flex min-w-0 flex-[2_1_16rem] items-center gap-1">
          <Input
            type="date"
            class="h-9 min-w-0"
            value={dateRange()[0]}
            onInput={(e) => setFilter("date", [e.currentTarget.value, dateRange()[1]])}
            aria-label="From date"
          />
          <span class="text-muted-foreground">–</span>
          <Input
            type="date"
            class="h-9 min-w-0"
            value={dateRange()[1]}
            onInput={(e) => setFilter("date", [dateRange()[0], e.currentTarget.value])}
            aria-label="To date"
          />
        </div>
        <Menu
          label="Columns"
          title="Show or hide columns"
          trigger={
            <>
              <Columns3 size={16} /> Columns
            </>
          }
          triggerClass="inline-flex h-9 items-center gap-2 rounded-md border border-input px-3 text-sm font-medium hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          class="bottom-auto left-auto right-0 top-full mb-0 mt-1 w-48"
        >
          <For each={table.getAllLeafColumns().filter((c) => c.getCanHide())}>
            {(col) => (
              <label class="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent">
                <input type="checkbox" checked={col.getIsVisible()} onChange={col.getToggleVisibilityHandler()} class="h-4 w-4" />
                {String(col.columnDef.header)}
              </label>
            )}
          </For>
        </Menu>
        <Show when={anyFilter()}>
          <Button variant="ghost" size="sm" onClick={clearAll}>
            <X size={16} /> Clear
          </Button>
        </Show>
      </div>

      {/* Summary of what's shown */}
      <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span>
          {filtered().length === props.rows.length
            ? `${props.rows.length} rows`
            : `${filtered().length} of ${props.rows.length} rows`}
        </span>
        <Totals rows={filtered()} />
        <span class="hidden sm:inline">Shift-click headers to sort by more than one column.</span>
      </div>

      {/* Table */}
      <div class="overflow-x-auto rounded-xl border border-border">
        <table class="w-full min-w-[44rem] text-sm">
          <thead class="border-b border-border bg-secondary/30 text-left text-xs text-muted-foreground">
            <For each={table.getHeaderGroups()}>
              {(group) => (
                <tr>
                  <For each={group.headers}>
                    {(h) => {
                      const m = meta(h.column.columnDef.meta);
                      const sorted = () => h.column.getIsSorted();
                      return (
                        <th
                          class={cn("px-4 py-2 font-medium", m.width, m.align === "right" && "text-right")}
                          aria-sort={sorted() === "asc" ? "ascending" : sorted() === "desc" ? "descending" : "none"}
                        >
                          <div class={cn("inline-flex items-center gap-1", m.align === "right" && "flex-row-reverse")}>
                            <button
                              type="button"
                              onClick={h.column.getToggleSortingHandler()}
                              class={cn(
                                "inline-flex items-center gap-1 rounded hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                                sorted() && "text-foreground",
                              )}
                            >
                              {flexRender(h.column.columnDef.header, h.getContext()) as JSX.Element}
                              <SortIcon dir={sorted()} index={h.column.getSortIndex()} multi={sorting().length > 1} />
                            </button>
                            <Show when={h.column.id === "type"}>
                              <TypeFilter
                                types={types()}
                                value={filterValue("type", "")}
                                onChange={(v) => setFilter("type", v || undefined)}
                              />
                            </Show>
                          </div>
                        </th>
                      );
                    }}
                  </For>
                </tr>
              )}
            </For>
          </thead>
          <tbody>
            <For
              each={table.getRowModel().rows}
              fallback={
                <tr>
                  <td colspan={table.getVisibleLeafColumns().length} class="px-4 py-10 text-center text-muted-foreground">
                    No rows match.{" "}
                    <button type="button" class="underline hover:text-foreground" onClick={clearAll}>
                      Clear the filters
                    </button>
                  </td>
                </tr>
              }
            >
              {(row) => (
                <tr class="border-b border-border last:border-0 hover:bg-accent/30">
                  <For each={row.getVisibleCells()}>
                    {(cell) => (
                      <td class={cn("px-4 py-2 align-top", meta(cell.column.columnDef.meta).align === "right" && "text-right")}>
                        {flexRender(cell.column.columnDef.cell, cell.getContext()) as JSX.Element}
                      </td>
                    )}
                  </For>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      {/* Pagination */}
      <div class="flex flex-wrap items-center justify-between gap-3 text-sm">
        <label class="flex items-center gap-2 text-muted-foreground">
          Rows per page
          <select
            class={cn(SELECT, "h-8 px-2")}
            value={pagination().pageSize}
            onChange={(e) => table.setPageSize(Number(e.currentTarget.value))}
          >
            <For each={PAGE_SIZES}>{(n) => <option value={n}>{n}</option>}</For>
          </select>
        </label>
        <div class="flex items-center gap-2">
          <span class="tabular-nums text-muted-foreground">
            {firstShown()}–{lastShown()} of {filtered().length}
          </span>
          <Button variant="outline" size="sm" onClick={() => table.previousPage()} disabled={!table.getCanPreviousPage()} aria-label="Previous page">
            <ChevronLeft size={16} />
          </Button>
          <Button variant="outline" size="sm" onClick={() => table.nextPage()} disabled={!table.getCanNextPage()} aria-label="Next page">
            <ChevronRight size={16} />
          </Button>
        </div>
      </div>
    </div>
  );
}

/**
 * Filter button in the Type header: a popover listing every type, "No type", and "All types".
 * Positioned against the viewport (in a portal), so the table's scroll box can't clip it.
 */
function TypeFilter(props: { types: string[]; value: string; onChange: (value: string) => void }) {
  const [pos, setPos] = createSignal<{ top: number; left: number } | null>(null);
  let button!: HTMLButtonElement;
  let panel: HTMLDivElement | undefined;

  const items = () => Array.from(panel?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? []);
  const close = (refocus = false) => {
    setPos(null);
    if (refocus) button.focus();
  };
  const open = () => {
    const r = button.getBoundingClientRect();
    const width = 224;
    setPos({ top: r.bottom + 4, left: Math.max(8, Math.min(r.left, window.innerWidth - width - 8)) });
    queueMicrotask(() => (items().find((el) => el.getAttribute("aria-checked") === "true") ?? items()[0])?.focus());
  };
  const choose = (v: string) => {
    props.onChange(v);
    close(true);
  };

  const onPointerDown = (e: PointerEvent) => {
    const t = e.target as Node;
    if (pos() && !panel?.contains(t) && !button.contains(t)) close();
  };
  const onViewportChange = () => pos() && close();
  document.addEventListener("pointerdown", onPointerDown);
  window.addEventListener("resize", onViewportChange);
  window.addEventListener("scroll", onViewportChange, true);
  onCleanup(() => {
    document.removeEventListener("pointerdown", onPointerDown);
    window.removeEventListener("resize", onViewportChange);
    window.removeEventListener("scroll", onViewportChange, true);
  });

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      close(true);
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
    e.preventDefault();
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    const next =
      e.key === "Home" ? 0
      : e.key === "End" ? list.length - 1
      : e.key === "ArrowDown" ? (at + 1) % list.length
      : (at - 1 + list.length) % list.length;
    list[next]?.focus();
  };

  const label = () =>
    !props.value ? "Filter by type" : props.value === NO_TYPE ? "Filtered: no type" : `Filtered: ${props.value}`;
  const Option = (o: { value: string; children: JSX.Element }) => (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={props.value === o.value}
      onClick={() => choose(o.value)}
      class={menuItemClass}
    >
      <Check size={14} class={cn("shrink-0", props.value !== o.value && "invisible")} />
      <span class="truncate">{o.children}</span>
    </button>
  );

  return (
    <>
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={!!pos()}
        aria-label={label()}
        title={label()}
        onClick={() => (pos() ? close() : open())}
        class={cn(
          "flex h-6 w-6 items-center justify-center rounded hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          props.value && "bg-accent text-foreground",
        )}
      >
        <Funnel size={12} />
      </button>
      <Show when={pos()}>
        {(p) => (
          <Portal>
            <div
              ref={panel}
              role="menu"
              aria-label="Filter by type"
              onKeyDown={onKeyDown}
              style={{ top: `${p().top}px`, left: `${p().left}px` }}
              class="fixed z-50 max-h-72 w-56 overflow-y-auto rounded-lg border border-border bg-background p-1 text-foreground shadow-xl"
            >
              <Option value="">All types</Option>
              <Option value={NO_TYPE}>
                <span class="text-muted-foreground">No type</span>
              </Option>
              <Show when={props.types.length > 0}>
                <MenuSeparator />
              </Show>
              <For each={props.types}>{(t) => <Option value={t}>{t}</Option>}</For>
            </div>
          </Portal>
        )}
      </Show>
    </>
  );
}

function SortIcon(props: { dir: false | "asc" | "desc"; index: number; multi: boolean }) {
  return (
    <span class="inline-flex items-center">
      <Show when={props.dir} fallback={<ArrowUpDown size={12} class="opacity-40" />}>
        <Show when={props.dir === "asc"} fallback={<ArrowDown size={12} />}>
          <ArrowUp size={12} />
        </Show>
        <Show when={props.multi && props.index >= 0}>
          <span class="ml-0.5 text-[10px]">{props.index + 1}</span>
        </Show>
      </Show>
    </span>
  );
}

function TransactionsView() {
  const org = useOrg();
  const [rows] = createResource(() => org.slug, fetchOrgTransactions);
  return (
    <>
      <Show when={rows.error}>
        <p class="mt-6 text-sm text-red-500">{rows.error instanceof Error ? rows.error.message : "Couldn't load transactions."}</p>
      </Show>
      <Show when={!settled(rows) && rows.loading}>
        <div class="mt-6 h-64 animate-pulse rounded-xl bg-secondary/40" />
      </Show>
      <Show when={settled(rows)}>
        {(r) => (
          <Show
            when={r().length > 0}
            fallback={
              <section class="mt-6 rounded-xl border border-dashed border-border px-6 py-14 text-center">
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
            }
          >
            <Grid rows={r()} />
          </Show>
        )}
      </Show>
    </>
  );
}

/** /portal/<org>/statements: every saved row in the org. Needs `read-statements`. */
export function TransactionsPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <PageHeader title="Statements" description={`Every transaction saved for ${org.name}, from all statements.`} />
      <StatementsTabs />
      <TransactionsView />
    </Allowed>
  );
}
