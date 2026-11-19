import { createMemo, createSignal, For, type JSX, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import {
  type ColumnDef,
  type ColumnFiltersState,
  createSolidTable,
  type FilterFn,
  flexRender,
  getCoreRowModel,
  getFilteredRowModel,
  getPaginationRowModel,
  getSortedRowModel,
  type Row,
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
import { cn } from "~/lib/utils";
import { compactGrids, gridMounted, gridUnmounted, setCompactGrids, setWideGrids, wideGrids } from "~/lib/view";

/** Header filter value meaning "rows where this column is empty". */
export const EMPTY = "\u0000empty";
/** "All" is a page size larger than any list, so paging maths keeps working unchanged. */
const ALL_ROWS = Number.MAX_SAFE_INTEGER;
const PAGE_SIZES = [25, 50, 100, 250, 500, ALL_ROWS];

export const SELECT =
  "h-9 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

export type FilterOption = { value: string; label: string };

/** Per-column settings, in `columnDef.meta`. */
export type GridColumnMeta<T> = {
  /** Tailwind width class for the column. */
  width?: string;
  align?: "right";
  /** Name in the column picker (defaults to the header). */
  label?: string;
  /**
   * A header filter: tick any of these options (built from every row, so ticking one never
   * hides the others); a row matches if its value is any ticked one. `EMPTY` matches blanks;
   * `emptyLabel` adds that choice.
   */
  pick?: { options: (rows: T[]) => FilterOption[]; emptyLabel?: string };
};

const meta = <T,>(m: unknown) => (m ?? {}) as GridColumnMeta<T>;

/** Filter for `pick` columns: the ticked values (`EMPTY` for blanks); none ticked = all rows. */
// `any`: shared by every row type.
export const pickFilter: FilterFn<any> = (row, id, ticked: string[]) => {
  if (!ticked?.length) return true;
  const v = row.getValue<unknown>(id);
  const text = v == null ? "" : String(v);
  return ticked.includes(text === "" ? EMPTY : text);
};

/** `[from, to]` (either may be empty) against a YYYY-MM-DD (or ISO timestamp) column. */
// `any`: shared by every row type.
export const dateRangeFilter: FilterFn<any> = (row, id, [from, to]: [string, string]) => {
  const d = String(row.getValue(id) ?? "").slice(0, 10);
  return (!from || d >= from) && (!to || d <= to);
};

/** Distinct non-empty values of `get`, as options, in `sort` order (alphabetical by default). */
export function distinct<T>(rows: T[], get: (row: T) => string | null | undefined, sort?: (a: string, b: string) => number) {
  const values = [...new Set(rows.map(get).filter((v): v is string => !!v))];
  values.sort(sort ?? ((a, b) => a.localeCompare(b)));
  return values.map((v) => ({ value: v, label: v }));
}

export type GridApi = {
  filterValue: <V>(id: string, empty: V) => V;
  setFilter: (id: string, value: unknown) => void;
};

export function DataGrid<T>(props: {
  rows: T[];
  columns: ColumnDef<T, any>[];
  initialSorting?: SortingState;
  /** Matches a row against the search box (lowercased query, never empty). */
  search: (row: T, query: string) => boolean;
  searchPlaceholder: string;
  /** Remembers the column picker per browser under this key. */
  storageKey: string;
  /** Columns hidden until the user shows them. */
  defaultHidden?: string[];
  /** Extra toolbar controls (e.g. a date range), given access to the filters. */
  toolbar?: (api: GridApi) => JSX.Element;
  /**
   * Extra figures for the status bar, given the rows that pass the filters and whether any
   * filter or search is active (e.g. a sum, shown only when filtering).
   */
  status?: (rows: T[], filtering: boolean) => JSX.Element;
  /** e.g. ["row", "rows"]. */
  noun: [string, string];
}) {
  const [sorting, setSorting] = createSignal<SortingState>(props.initialSorting ?? []);
  const [filters, setFilters] = createSignal<ColumnFiltersState>([]);
  const [query, setQuery] = createSignal("");
  const [visibility, setVisibility] = createSignal<VisibilityState>(initialVisibility(props.storageKey, props.defaultHidden ?? []));
  const [pagination, setPagination] = createSignal({ pageIndex: 0, pageSize: 50 });
  const firstPage = () => setPagination((p) => ({ ...p, pageIndex: 0 }));
  onMount(gridMounted);
  onCleanup(gridUnmounted);
  const compact = compactGrids;

  const table = createSolidTable<T>({
    get data() {
      return props.rows;
    },
    get columns() {
      return props.columns;
    },
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
      firstPage();
    },
    onGlobalFilterChange: (u) => {
      setQuery(u);
      firstPage();
    },
    onColumnVisibilityChange: (u) => {
      const next = typeof u === "function" ? u(visibility()) : u;
      setVisibility(next);
      try {
        localStorage.setItem(props.storageKey, JSON.stringify(next));
      } catch {
        // storage blocked: just don't remember
      }
    },
    onPaginationChange: setPagination,
    globalFilterFn: (row: Row<T>, _id, value: string) => {
      const q = value.trim().toLowerCase();
      return !q || props.search(row.original, q);
    },
    enableMultiSort: true,
    isMultiSortEvent: (e) => (e as MouseEvent).shiftKey,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
  });

  const api: GridApi = {
    filterValue: <V,>(id: string, empty: V) => (filters().find((f) => f.id === id)?.value as V) ?? empty,
    setFilter: (id, value) => table.getColumn(id)?.setFilterValue(value),
  };

  const filtered = () => table.getFilteredRowModel().rows.map((r) => r.original);
  const anyFilter = () => !!query() || filters().length > 0;
  const clearAll = () => {
    setQuery("");
    setFilters([]);
    firstPage();
  };
  const count = (n: number) => `${n} ${n === 1 ? props.noun[0] : props.noun[1]}`;
  const firstShown = () => (filtered().length === 0 ? 0 : pagination().pageIndex * pagination().pageSize + 1);
  const lastShown = () => Math.min(filtered().length, (pagination().pageIndex + 1) * pagination().pageSize);

  return (
    // Fills the space the page leaves it (see `pageHasGrid`); only the table area scrolls.
    <div class="flex min-h-[24rem] flex-1 flex-col gap-3">
      {/* Toolbar */}
      <div class="flex flex-wrap items-end gap-2">
        <div class="relative min-w-0 flex-[2_1_14rem]">
          <Search size={16} class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <Input
            class="h-9 pl-9"
            value={query()}
            onInput={(e) => table.setGlobalFilter(e.currentTarget.value)}
            placeholder={props.searchPlaceholder}
            aria-label="Search"
          />
        </div>
        {props.toolbar?.(api)}
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
                {meta<T>(col.columnDef.meta).label ?? String(col.columnDef.header)}
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

      {/* Table */}
      <div class="min-h-0 flex-1 overflow-auto rounded-xl border border-border">
        <table class={cn("w-full min-w-[44rem] text-sm", compact() && "table-fixed")}>
          {/* Pinned while the rows scroll; opaque so rows don't show through. */}
          <thead class="sticky top-0 z-10 bg-background text-left text-xs text-muted-foreground shadow-[inset_0_-1px_0_var(--color-border)]">
            <For each={table.getHeaderGroups()}>
              {(group) => (
                <tr class="bg-secondary/30">
                  <For each={group.headers}>
                    {(h) => {
                      const m = meta<T>(h.column.columnDef.meta);
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
                              title="Sort (shift-click to sort by more than one column)"
                              class={cn(
                                "inline-flex items-center gap-1 rounded hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                                sorted() && "text-foreground",
                              )}
                            >
                              {flexRender(h.column.columnDef.header, h.getContext()) as JSX.Element}
                              <SortIcon dir={sorted()} index={h.column.getSortIndex()} multi={sorting().length > 1} />
                            </button>
                            <Show when={m.pick}>
                              {(pick) => (
                                <HeaderFilter
                                  name={m.label ?? String(h.column.columnDef.header)}
                                  options={pick().options(props.rows)}
                                  emptyLabel={pick().emptyLabel}
                                  value={api.filterValue<string[]>(h.column.id, [])}
                                  onChange={(v) => api.setFilter(h.column.id, v.length ? v : undefined)}
                                />
                              )}
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
                    Nothing matches.{" "}
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
                      <td
                        class={cn(
                          "px-4 align-top",
                          compact() ? "py-1.5" : "py-2",
                          meta<T>(cell.column.columnDef.meta).align === "right" && "text-right",
                        )}
                        // Compact cuts long text off; hovering shows all of it.
                        title={compact() && typeof cell.getValue() === "string" ? (cell.getValue() as string) : undefined}
                      >
                        <Show when={compact()} fallback={flexRender(cell.column.columnDef.cell, cell.getContext()) as JSX.Element}>
                          <div class="truncate [&_*]:whitespace-nowrap!">
                            {flexRender(cell.column.columnDef.cell, cell.getContext()) as JSX.Element}
                          </div>
                        </Show>
                      </td>
                    )}
                  </For>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      {/* Status bar: what's shown (and its sum when filtering), paging, view switches. It sticks
          to the bottom of the window, so the figures stay in view while scrolling. */}
      <div
        role="status"
        class="sticky bottom-0 z-20 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-border bg-background/95 px-3 py-2 text-xs text-muted-foreground shadow-sm backdrop-blur"
      >
        <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
          <span class="tabular-nums">
            {filtered().length === props.rows.length ? count(props.rows.length) : `${filtered().length} of ${count(props.rows.length)}`}
          </span>
          {props.status?.(filtered(), anyFilter())}
        </div>
        <div class="flex flex-wrap items-center gap-2 sm:ml-auto">
          <label class="flex items-center gap-1.5">
            Per page
            <select class={cn(SELECT, "h-7 px-1.5 text-xs")} value={pagination().pageSize} onChange={(e) => table.setPageSize(Number(e.currentTarget.value))}>
              <For each={PAGE_SIZES}>{(n) => <option value={n}>{n === ALL_ROWS ? "All" : n}</option>}</For>
            </select>
          </label>
          <span class="tabular-nums">
            {firstShown()}–{lastShown()} of {filtered().length}
          </span>
          <Button variant="outline" size="sm" class="h-7 px-2" onClick={() => table.previousPage()} disabled={!table.getCanPreviousPage()} aria-label="Previous page">
            <ChevronLeft size={14} />
          </Button>
          <Button variant="outline" size="sm" class="h-7 px-2" onClick={() => table.nextPage()} disabled={!table.getCanNextPage()} aria-label="Next page">
            <ChevronRight size={14} />
          </Button>
        </div>
        <div class="flex items-center gap-4 sm:border-l sm:border-border sm:pl-4">
          <ToggleSwitch label="Compact" hint="One line per row, all the same height" on={compact()} onChange={setCompactGrids} />
          <ToggleSwitch label="Wide" hint="Use the full width of the window" on={wideGrids()} onChange={setWideGrids} />
        </div>
      </div>
    </div>
  );
}

/**
 * A labelled on/off switch. Not named `Switch`: Solid's compiler turns every `<Switch>` tag into
 * its built-in control-flow component, even when a local component has that name.
 */
function ToggleSwitch(props: { label: string; hint: string; on: boolean; onChange: (on: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.on}
      title={props.hint}
      onClick={() => props.onChange(!props.on)}
      class="inline-flex items-center gap-2 rounded text-xs text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span
        class={cn(
          "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-transparent transition-colors",
          props.on ? "bg-primary" : "bg-input",
        )}
      >
        <span
          class={cn(
            "inline-block h-4 w-4 rounded-full bg-background shadow transition-transform",
            props.on ? "translate-x-4" : "translate-x-0",
          )}
        />
      </span>
      {props.label}
    </button>
  );
}

function initialVisibility(key: string, hidden: string[]): VisibilityState {
  try {
    const saved = JSON.parse(localStorage.getItem(key) ?? "null");
    if (saved && typeof saved === "object") return saved;
  } catch {
    // fall back to the defaults
  }
  return Object.fromEntries(hidden.map((id) => [id, false]));
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

/**
 * Funnel button in a column header: a popover to tick any number of values (or blanks). It
 * stays open while ticking; Escape, Tab or a click outside closes it. "All" clears the ticks.
 * Positioned against the viewport in a portal, so the table's scroll box can't clip it.
 */
function HeaderFilter(props: {
  name: string;
  options: FilterOption[];
  emptyLabel?: string;
  /** Ticked values; empty = no filter. */
  value: string[];
  onChange: (value: string[]) => void;
}) {
  const [pos, setPos] = createSignal<{ top: number; left: number } | null>(null);
  let button!: HTMLButtonElement;
  let panel: HTMLDivElement | undefined;

  const items = () => Array.from(panel?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? []);
  const close = (refocus = false) => {
    setPos(null);
    if (refocus) button.focus();
  };
  const open = () => {
    const r = button.getBoundingClientRect();
    const width = 224;
    setPos({ top: r.bottom + 4, left: Math.max(8, Math.min(r.left, window.innerWidth - width - 8)) });
    queueMicrotask(() => items()[0]?.focus());
  };
  const ticked = (v: string) => props.value.includes(v);
  const toggle = (v: string) => props.onChange(ticked(v) ? props.value.filter((x) => x !== v) : [...props.value, v]);

  const onPointerDown = (e: PointerEvent) => {
    const t = e.target as Node;
    if (pos() && !panel?.contains(t) && !button.contains(t)) close();
  };
  const onViewportChange = (e: Event) => {
    if (pos() && !(e.type === "scroll" && panel?.contains(e.target as Node))) close();
  };
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

  const names = createMemo(() =>
    props.value.map((v) => (v === EMPTY ? props.emptyLabel : props.options.find((o) => o.value === v)?.label) ?? v),
  );
  const label = () =>
    props.value.length === 0
      ? `Filter by ${props.name.toLowerCase()}`
      : `${props.name}: ${props.value.length > 3 ? `${props.value.length} selected` : names().join(", ")}`;

  const Tick = (p: { on: boolean }) => (
    <span
      class={cn(
        "flex h-4 w-4 shrink-0 items-center justify-center rounded border",
        p.on ? "border-primary bg-primary text-primary-foreground" : "border-input",
      )}
    >
      <Show when={p.on}>
        <Check size={12} />
      </Show>
    </span>
  );
  const Choice = (o: { value: string; children: JSX.Element }) => (
    <button type="button" role="menuitemcheckbox" aria-checked={ticked(o.value)} onClick={() => toggle(o.value)} class={menuItemClass}>
      <Tick on={ticked(o.value)} />
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
          "flex h-6 min-w-6 items-center justify-center gap-0.5 rounded px-1 hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          props.value.length > 0 && "bg-accent text-foreground",
        )}
      >
        <Funnel size={12} />
        <Show when={props.value.length > 0}>
          <span class="text-[10px] font-semibold tabular-nums">{props.value.length}</span>
        </Show>
      </button>
      <Show when={pos()}>
        {(p) => (
          <Portal>
            <div
              ref={panel}
              role="menu"
              aria-label={`Filter by ${props.name.toLowerCase()}`}
              onKeyDown={onKeyDown}
              style={{ top: `${p().top}px`, left: `${p().left}px` }}
              class="fixed z-50 max-h-72 w-56 overflow-y-auto rounded-lg border border-border bg-background p-1 text-left text-sm font-normal text-foreground shadow-xl"
            >
              <button
                type="button"
                role="menuitemradio"
                aria-checked={props.value.length === 0}
                onClick={() => props.onChange([])}
                class={menuItemClass}
              >
                <Tick on={props.value.length === 0} />
                <span>All</span>
              </button>
              <Show when={props.emptyLabel}>
                <Choice value={EMPTY}>
                  <span class="text-muted-foreground">{props.emptyLabel}</span>
                </Choice>
              </Show>
              <Show when={props.options.length > 0}>
                <MenuSeparator />
              </Show>
              <For each={props.options}>{(o) => <Choice value={o.value}>{o.label}</Choice>}</For>
            </div>
          </Portal>
        )}
      </Show>
    </>
  );
}
