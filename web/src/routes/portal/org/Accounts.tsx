import { createSignal, For, Show } from "solid-js";
import Pencil from "lucide-solid/icons/pencil";
import Plus from "lucide-solid/icons/plus";
import Trash from "lucide-solid/icons/trash";
import { DeleteDialog } from "~/components/DeleteDialog";
import { Button } from "~/components/ui/button";
import { can } from "~/lib/orgs";
import { type Account, deleteAccount, fetchAccountDeleteImpact, formatDate, formatMoney } from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { AccountDialog, type AccountState, StatementsShell } from "~/routes/portal/org/StatementsShell";

const ICON_BUTTON =
  "flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function AccountsList(props: { state: AccountState }) {
  const org = useOrg();
  const editable = can(org, "write-statements");
  // The dialog: null = closed, "new" = create, an account = edit.
  const [editing, setEditing] = createSignal<Account | "new" | null>(null);
  const [deleting, setDeleting] = createSignal<Account | null>(null);
  const list = () => settled(props.state.accounts) ?? [];

  return (
    <section class="mt-6 flex flex-col gap-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <p class="max-w-2xl text-sm text-muted-foreground">
          Statements are uploaded into a bank account. Its identifier (usually the bank account number) is checked against the account
          number a statement states. Each bank account has its own category rules; the categories are shared. Holder, product and starting
          balance are for printed statements.
        </p>
        <Show when={editable}>
          <Button size="sm" onClick={() => setEditing("new")}>
            <Plus size={16} /> New bank account
          </Button>
        </Show>
      </div>

      <div class="overflow-x-auto rounded-xl border border-border">
        <table class="w-full min-w-[52rem] text-sm">
          <thead class="border-b border-border bg-secondary/30 text-left text-xs text-muted-foreground">
            <tr>
              <th class="px-4 py-2.5 font-medium">Name</th>
              <th class="px-4 py-2.5 font-medium">Identifier</th>
              <th class="px-4 py-2.5 font-medium">Holder</th>
              <th class="px-4 py-2.5 text-right font-medium">Starting balance</th>
              <th class="px-4 py-2.5 text-right font-medium">Statements</th>
              <th class="px-4 py-2.5 text-right font-medium">Rows</th>
              <th class="px-4 py-2.5 font-medium">Last upload</th>
              <th class="w-32 px-2 py-2.5">
                <span class="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <For each={list()}>
              {(a) => {
                const current = () => props.state.account()?.id === a.id;
                return (
                  <tr class={cn("border-b border-border last:border-0", current() ? "bg-accent/40" : "hover:bg-accent/30")}>
                    <td class="px-4 py-2.5">
                      <button
                        type="button"
                        onClick={() => props.state.pick(a.id)}
                        class="text-left font-medium underline-offset-2 hover:underline"
                        title={current() ? "The bank account you're working in" : "Work in this bank account"}
                      >
                        {a.name}
                      </button>
                      <Show when={current()}>
                        <span class="ml-2 text-xs text-muted-foreground">(selected)</span>
                      </Show>
                    </td>
                    <td class="px-4 py-2.5">
                      <Show when={a.identifier} fallback={<span class="text-muted-foreground">—</span>}>
                        <span class="font-mono text-xs">{a.identifier}</span>
                      </Show>
                    </td>
                    <td class="px-4 py-2.5">{a.holder_name ?? <span class="text-muted-foreground">—</span>}</td>
                    <td class="px-4 py-2.5 text-right font-mono tabular-nums">
                      <Show when={a.opening_balance_cents !== null} fallback={<span class="text-muted-foreground">—</span>}>
                        {formatMoney(a.opening_balance_cents!)}
                        <span class="block text-xs text-muted-foreground">at the start of {formatDate(a.opening_balance_date!)}</span>
                      </Show>
                    </td>
                    <td class="px-4 py-2.5 text-right tabular-nums">{a.uploads}</td>
                    <td class="px-4 py-2.5 text-right tabular-nums">{a.rows.toLocaleString()}</td>
                    <td class="px-4 py-2.5 text-muted-foreground">{a.last_uploaded_at ? formatDate(a.last_uploaded_at) : "—"}</td>
                    <td class="px-2 py-1.5">
                      <div class="flex justify-end gap-1">
                        <Show when={editable}>
                          <button type="button" class={cn(ICON_BUTTON, "hover:text-accent-foreground")} onClick={() => setEditing(a)} aria-label={`Edit ${a.name}`} title="Edit">
                            <Pencil size={14} />
                          </button>
                          <button type="button" class={cn(ICON_BUTTON, "hover:text-red-500")} onClick={() => setDeleting(a)} aria-label={`Delete ${a.name}`} title="Delete">
                            <Trash size={14} />
                          </button>
                        </Show>
                      </div>
                    </td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>

      <AccountDialog
        open={editing() !== null}
        account={editing() === "new" ? undefined : (editing() as Account | null) ?? undefined}
        onSaved={(a) => {
          const created = editing() === "new";
          setEditing(null);
          if (created) props.state.pick(a.id);
          props.state.reload();
        }}
        onClose={() => setEditing(null)}
      />

      <DeleteDialog
        open={deleting() !== null}
        title={`Delete ${deleting()?.name ?? "bank account"}?`}
        note={
          <>
            This deletes the bank account with its saved statements, rows and category rules. Documents (including the statements' own files)
            stay in Documents. This can't be undone.
          </>
        }
        load={() => fetchAccountDeleteImpact(org.slug, deleting()!.id)}
        confirmText={deleting()?.name}
        onConfirm={async () => {
          const a = deleting()!;
          await deleteAccount(org.slug, a.id, a.name);
          setDeleting(null);
          props.state.reload();
        }}
        onClose={() => setDeleting(null)}
      />
    </section>
  );
}

/** /portal/<org>/statements/accounts. Viewing needs `read-statements`; changes `write-statements`. */
export function AccountsPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <StatementsShell>{(state) => <AccountsList state={state} />}</StatementsShell>
    </Allowed>
  );
}
