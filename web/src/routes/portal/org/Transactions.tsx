import { createMemo, createResource, For, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import type { ColumnDef } from "@tanstack/solid-table";
import { DataGrid, dateRangeFilter, distinct, type GridColumnMeta, pickFilter } from "~/components/DataGrid";
import { Input } from "~/components/ui/input";
import { can } from "~/lib/orgs";
import { amountValue, fetchOrgTransactions, formatAmount, formatDate, isNegative, type OrgTransaction } from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed, PageHeader } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { StatementsTabs } from "~/routes/portal/org/StatementParts";

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
    filterFn: dateRangeFilter,
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
    accessorKey: "account_number",
    header: "Account",
    cell: (c) => <span class="whitespace-nowrap text-muted-foreground">{c.getValue<string>() || "—"}</span>,
    filterFn: pickFilter,
    meta: {
      width: "w-36",
      pick: { options: (rows) => distinct(rows, (r) => r.account_number), emptyLabel: "No account" },
    } satisfies Meta,
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
    cell: (c) => (
      <Show when={c.row.original.type} fallback={<span class="text-muted-foreground">—</span>}>
        <span class="whitespace-nowrap rounded-full border border-border bg-secondary/40 px-2 py-0.5 text-xs font-medium">
          {c.row.original.type}
        </span>
      </Show>
    ),
    filterFn: pickFilter,
    meta: { width: "w-40", pick: { options: (rows) => distinct(rows, (r) => r.type), emptyLabel: "No type" } } satisfies Meta,
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
          {currency}: <span class="text-emerald-600 dark:text-emerald-400">+{fmt(t.in)}</span> / <span>−{fmt(Math.abs(t.out))}</span> · net{" "}
          <span class="font-medium text-foreground">{fmt(t.in + t.out)}</span>
        </span>
      )}
    </For>
  );
}

function TransactionsView() {
  const org = useOrg();
  const [rows] = createResource(() => org.slug, fetchOrgTransactions);
  return (
    <div class="mt-6">
      <Show when={rows.error}>
        <p class="text-sm text-red-500">{rows.error instanceof Error ? rows.error.message : "Couldn't load transactions."}</p>
      </Show>
      <Show when={!settled(rows) && rows.loading}>
        <div class="h-64 animate-pulse rounded-xl bg-secondary/40" />
      </Show>
      <Show when={settled(rows)}>
        {(r) => (
          <Show
            when={r().length > 0}
            fallback={
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
            }
          >
            <DataGrid
              rows={r()}
              columns={columns}
              initialSorting={[{ id: "date", desc: true }]}
              search={(t, q) => [t.description, t.type ?? "", t.account_number, t.amount, t.month].some((v) => v.toLowerCase().includes(q))}
              searchPlaceholder="Search description, type, account, amount, month"
              storageKey="statements-grid-columns"
              defaultHidden={["currency", "txn_id", "updated_at"]}
              noun={["row", "rows"]}
              toolbar={(grid) => {
                const range = () => grid.filterValue<[string, string]>("date", ["", ""]);
                const set = (from: string, to: string) => grid.setFilter("date", from || to ? [from, to] : undefined);
                return (
                  <div class="flex min-w-0 flex-[2_1_16rem] items-center gap-1">
                    <Input type="date" class="h-9 min-w-0" value={range()[0]} onInput={(e) => set(e.currentTarget.value, range()[1])} aria-label="From date" />
                    <span class="text-muted-foreground">–</span>
                    <Input type="date" class="h-9 min-w-0" value={range()[1]} onInput={(e) => set(range()[0], e.currentTarget.value)} aria-label="To date" />
                  </div>
                );
              }}
              summary={(shown) => <Totals rows={shown} />}
            />
          </Show>
        )}
      </Show>
    </div>
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
