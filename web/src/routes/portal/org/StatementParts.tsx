import { For, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import { formatAmount, formatDate, isNegative, type Transaction } from "~/lib/statements";
import { cn } from "~/lib/utils";
import { useOrg } from "~/routes/portal/org/OrgLayout";

const TAB = "rounded-md px-3 py-1.5 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

/** Transactions | Uploads | Type rules, under the page header. */
export function StatementsTabs() {
  const org = useOrg();
  return (
    <nav aria-label="Statements" class="mt-6 flex gap-1">
      <Link
        to="/portal/$org/statements"
        params={{ org: org.slug }}
        activeOptions={{ exact: true }}
        class={TAB}
        activeProps={{ class: "bg-secondary text-foreground" }}
        inactiveProps={{ class: "text-muted-foreground hover:bg-accent hover:text-accent-foreground" }}
      >
        Transactions
      </Link>
      <Link
        to="/portal/$org/statements/uploads"
        params={{ org: org.slug }}
        class={TAB}
        activeProps={{ class: "bg-secondary text-foreground" }}
        inactiveProps={{ class: "text-muted-foreground hover:bg-accent hover:text-accent-foreground" }}
      >
        Uploads
      </Link>
      <Link
        to="/portal/$org/statements/rules"
        params={{ org: org.slug }}
        class={TAB}
        activeProps={{ class: "bg-secondary text-foreground" }}
        inactiveProps={{ class: "text-muted-foreground hover:bg-accent hover:text-accent-foreground" }}
      >
        Type rules
      </Link>
    </nav>
  );
}

/** Rows of a statement, in file order. `stored` (preview only) marks rows already saved. */
export function TransactionsTable(props: { rows: (Transaction & { stored?: boolean })[]; showStored?: boolean }) {
  return (
    <div class="overflow-x-auto rounded-xl border border-border">
      <table class="w-full min-w-[40rem] text-sm">
        <thead class="border-b border-border bg-secondary/30 text-left text-xs text-muted-foreground">
          <tr>
            <th class="w-32 px-4 py-2.5 font-medium">Date</th>
            <th class="px-4 py-2.5 font-medium">Description</th>
            <th class="w-32 px-4 py-2.5 text-right font-medium">Amount</th>
            <th class="w-40 px-4 py-2.5 font-medium">Type</th>
            <Show when={props.showStored}>
              <th class="w-28 px-4 py-2.5 font-medium">
                <span class="sr-only">Status</span>
              </th>
            </Show>
          </tr>
        </thead>
        <tbody>
          <For each={props.rows}>
            {(t) => (
              <tr class="border-b border-border last:border-0" title={`id ${t.txn_id}`}>
                <td class="whitespace-nowrap px-4 py-2 text-muted-foreground">{formatDate(t.date)}</td>
                {/* Kept exactly as stored (trailing spaces and "_" suffixes are part of the id). */}
                <td class="whitespace-pre-wrap break-words px-4 py-2">{t.description}</td>
                <td class={cn("whitespace-nowrap px-4 py-2 text-right tabular-nums", isNegative(t.amount) ? "" : "text-emerald-600 dark:text-emerald-400")}>
                  {formatAmount(t.amount)} <span class="text-xs text-muted-foreground">{t.currency}</span>
                </td>
                <td class="px-4 py-2">
                  <Show when={t.type} fallback={<span class="text-muted-foreground">—</span>}>
                    <span class="rounded-full border border-border bg-secondary/40 px-2 py-0.5 text-xs font-medium">{t.type}</span>
                  </Show>
                </td>
                <Show when={props.showStored}>
                  <td class="px-4 py-2 text-xs text-muted-foreground">{t.stored ? "Already saved" : "New"}</td>
                </Show>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}

/** A small label/value pair for statement summaries. */
export function Fact(props: { label: string; value: string | number }) {
  return (
    <div class="min-w-0">
      <dt class="text-xs text-muted-foreground">{props.label}</dt>
      <dd class="mt-0.5 truncate text-sm font-medium">{props.value}</dd>
    </div>
  );
}
