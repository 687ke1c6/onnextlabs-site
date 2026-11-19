import { createMemo, createResource, For, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import type { ColumnDef } from "@tanstack/solid-table";
import { DataGrid, dateRangeFilter, distinct, type GridColumnMeta, pickFilter } from "~/components/DataGrid";
import { Input } from "~/components/ui/input";
import { can } from "~/lib/orgs";
import {
  accountLabel,
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

/**
 * The sum of Amount over the filtered rows, per currency (money in and out in its tooltip).
 * Only while a filter or search is on: unfiltered, it would just be the account's whole history.
 */
function FilteredSum(props: { rows: OrgTransaction[] }) {
  const sums = createMemo(() => {
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
  const signed = (n: number) => (n < 0 ? `−${fmt(-n)}` : fmt(n));
  return (
    <For each={sums()}>
      {([currency, t]) => (
        <span class="whitespace-nowrap" title={`In ${fmt(t.in)} · out ${fmt(-t.out)}`}>
          Sum{" "}
          <span class={cn("font-semibold tabular-nums", t.in + t.out < 0 ? "text-foreground" : "text-emerald-600 dark:text-emerald-400")}>
            {signed(t.in + t.out)}
          </span>{" "}
          {currency}
        </span>
      )}
    </For>
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
  const [rows] = createResource(
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

  return (
    <div class="mt-6">
      <Show when={error()}>
        {(e) => <p class="text-sm text-red-500">{e() instanceof Error ? (e() as Error).message : "Couldn't load transactions."}</p>}
      </Show>
      <Show when={props.acct.account() === null}>
        <NoTransactions />
      </Show>
      <Show when={props.acct.account() && !current() && !error()}>
        <div class="h-64 animate-pulse rounded-xl bg-secondary/40" />
      </Show>
      {/* Keyed: a new account starts with fresh filters and paging. */}
      <Show when={current()} keyed>
        {(data) => (
          <Show when={data.rows.length > 0} fallback={<NoTransactions />}>
            <DataGrid
              rows={data.rows}
              columns={columns}
              initialSorting={[{ id: "date", desc: false }]}
              search={(t, q) => [t.description, t.type ?? "", t.amount, t.month].some((v) => v.toLowerCase().includes(q))}
              searchPlaceholder="Search description, type, amount, month"
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
              status={(shown, filtering) => (
                <Show when={filtering && shown.length > 0}>
                  <FilteredSum rows={shown} />
                </Show>
              )}
            />
          </Show>
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
    <>
      <PageHeader title="Statements" description={description()} actions={<AccountPicker state={acct} />} />
      <StatementsTabs />
      <TransactionsView acct={acct} />
    </>
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
