import { createEffect, createMemo, createResource, createSignal, For, type JSX, on, Show } from "solid-js";
import Landmark from "lucide-solid/icons/landmark";
import Loader2 from "lucide-solid/icons/loader-circle";
import X from "lucide-solid/icons/x";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { can } from "~/lib/orgs";
import { pickAccount, pickedAccount } from "~/lib/statementAccount";
import { type Account, type AccountDetails, createAccount, fetchAccounts, formatCents, updateAccount } from "~/lib/statements";
import { selectValue, settled } from "~/lib/utils";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { StatementsTabs } from "~/routes/portal/org/StatementParts";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);
const NEW_ACCOUNT = "new";

/**
 * The org's accounts and the one the Statements pages show: the one last picked (if it still
 * exists), otherwise the first by name. `account()` is undefined while loading and null when
 * the org has no accounts yet.
 */
export function useStatementAccount() {
  const org = useOrg();
  const [accounts, { refetch }] = createResource(() => org.slug, fetchAccounts);
  const account = createMemo(() => {
    const list = settled(accounts);
    if (!list) return undefined;
    const want = pickedAccount(org.slug);
    return list.find((a) => a.id === want) ?? list[0] ?? null;
  });
  return {
    accounts,
    account,
    pick: (id: number) => pickAccount(org.slug, id),
    /** Reload the list (after creating, renaming or deleting an account, or saving a statement). */
    reload: () => void refetch(),
  };
}

export type AccountState = ReturnType<typeof useStatementAccount>;

/**
 * Name (required) and identifier (optional), for creating or editing an account, and what
 * statement PDFs show about it: holder, product and a starting balance.
 */
function AccountForm(props: {
  account?: Account;
  submitLabel: string;
  onSaved: (account: Account) => void;
  onCancel?: () => void;
}) {
  const org = useOrg();
  const [name, setName] = createSignal(props.account?.name ?? "");
  const [identifier, setIdentifier] = createSignal(props.account?.identifier ?? "");
  const a = props.account;
  const [details, setDetails] = createSignal<AccountDetails>({
    holder_name: a?.holder_name ?? "",
    product_name: a?.product_name ?? "",
    opening_balance: a?.opening_balance_cents != null ? formatCents(a.opening_balance_cents) : "",
    opening_balance_date: a?.opening_balance_date ?? "",
  });
  const setDetail = (key: keyof AccountDetails, value: string) => setDetails({ ...details(), [key]: value });
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    if (!name().trim() || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const saved = props.account
        ? await updateAccount(org.slug, props.account.id, name(), identifier(), details())
        : await createAccount(org.slug, name(), identifier(), details());
      props.onSaved(saved);
    } catch (err) {
      setError(errorText(err, "Couldn't save the bank account."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={(e) => void submit(e)} class="flex flex-col gap-4">
      <div class="grid gap-3 sm:grid-cols-2">
        <label class="flex flex-col gap-1.5 text-sm">
          <span class="text-muted-foreground">Name</span>
          <Input
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            placeholder="e.g. Joint cheque account"
            maxLength={100}
            required
            ref={(el) => queueMicrotask(() => el.focus())}
          />
        </label>
        <label class="flex flex-col gap-1.5 text-sm">
          <span class="text-muted-foreground">
            Identifier <span class="text-xs">(optional)</span>
          </span>
          <Input
            value={identifier()}
            onInput={(e) => setIdentifier(e.currentTarget.value)}
            placeholder="e.g. the account number"
            maxLength={64}
            autocomplete="off"
          />
        </label>
      </div>
      <p class="text-xs text-muted-foreground">
        The identifier is usually the bank account number. When a statement states its account number, it's checked against this
        before saving.
      </p>

      <fieldset class="flex flex-col gap-3 border-t border-border pt-4">
        <legend class="sr-only">Statement details</legend>
        <p class="text-xs text-muted-foreground">
          For printed statements (all optional). Templates show them with variables like <code>{"{account.holder}"}</code>.
        </p>
        <div class="grid gap-3 sm:grid-cols-2">
          <label class="flex flex-col gap-1.5 text-sm">
            <span class="text-muted-foreground">Account holder</span>
            <Input value={details().holder_name} onInput={(e) => setDetail("holder_name", e.currentTarget.value)} maxLength={100} placeholder="e.g. Mr J Smith" />
          </label>
          <label class="flex flex-col gap-1.5 text-sm">
            <span class="text-muted-foreground">Product name</span>
            <Input value={details().product_name} onInput={(e) => setDetail("product_name", e.currentTarget.value)} maxLength={100} placeholder="e.g. Current Account" />
          </label>
          <label class="flex flex-col gap-1.5 text-sm">
            <span class="text-muted-foreground">Starting balance</span>
            <Input
              value={details().opening_balance}
              onInput={(e) => setDetail("opening_balance", e.currentTarget.value)}
              inputMode="decimal"
              autocomplete="off"
              placeholder="e.g. 40957.20"
            />
          </label>
          <label class="flex flex-col gap-1.5 text-sm">
            <span class="text-muted-foreground">At the start of</span>
            <Input type="date" value={details().opening_balance_date} onInput={(e) => setDetail("opening_balance_date", e.currentTarget.value)} />
          </label>
        </div>
        <p class="text-xs text-muted-foreground">
          Statement balances are worked out from this and every row of the bank account, by transaction date, before or after this day.
          Filters never change them.
        </p>
      </fieldset>
      <Show when={error()}>
        <p role="alert" class="text-sm text-red-500">{error()}</p>
      </Show>
      <div class="flex justify-end gap-2">
        <Show when={props.onCancel}>
          <Button type="button" variant="ghost" onClick={() => props.onCancel?.()} disabled={busy()}>
            Cancel
          </Button>
        </Show>
        <Button type="submit" disabled={busy() || !name().trim()}>
          <Show when={busy()}>
            <Loader2 size={16} class="animate-spin" />
          </Show>
          {props.submitLabel}
        </Button>
      </div>
    </form>
  );
}

/** Create or edit an account in a dialog. */
export function AccountDialog(props: {
  open: boolean;
  account?: Account;
  onSaved: (account: Account) => void;
  onClose: () => void;
}) {
  let dialog!: HTMLDialogElement;
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) dialog.showModal();
        else if (dialog.open) dialog.close();
      },
    ),
  );
  return (
    <dialog
      ref={dialog}
      onClose={() => props.open && props.onClose()}
      class="m-auto w-[calc(100%-2rem)] max-w-lg rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="account-title"
    >
      <div class="flex flex-col gap-4 p-6">
        <div class="flex items-start justify-between gap-4">
          <h2 id="account-title" class="text-lg font-semibold tracking-tight">
            {props.account ? `Edit ${props.account.name}` : "New bank account"}
          </h2>
          <button
            type="button"
            onClick={() => props.onClose()}
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            aria-label="Close"
          >
            <X size={18} />
          </button>
        </div>
        {/* Re-created each time it opens, so it starts from the account (or empty). */}
        <Show when={props.open}>
          <AccountForm
            account={props.account}
            submitLabel={props.account ? "Save" : "Create account"}
            onSaved={props.onSaved}
            onCancel={props.onClose}
          />
        </Show>
      </div>
    </dialog>
  );
}

/** "Account [Joint ▾]" for the page header, with "New account…" at the end. */
function AccountPicker(props: { state: AccountState; onNew: () => void }) {
  const org = useOrg();
  const list = () => settled(props.state.accounts) ?? [];
  return (
    <Show when={props.state.account()}>
      {(current) => {
        let select!: HTMLSelectElement;
        // A reload (e.g. after editing an account) re-creates every <option>. While they're
        // swapped the browser selects the first one it can ("New account…"), and the value
        // binding runs before the new options are in. A user effect runs after rendering, so
        // it puts the current account back.
        createEffect(() => {
          list();
          select.value = String(current().id);
        });
        return (
          <label class="flex items-center gap-2 text-sm">
            <span class="text-muted-foreground">Bank account</span>
            <select
              ref={select}
              value={selectValue(String(current().id), list())}
              onChange={(e) => {
                const value = e.currentTarget.value;
                if (value === NEW_ACCOUNT) {
                  e.currentTarget.value = String(current().id);
                  props.onNew();
                } else {
                  props.state.pick(Number(value));
                }
              }}
              class="h-9 min-w-[12rem] rounded-md border border-input bg-background px-3 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              aria-label="Bank account"
            >
              <For each={list()}>{(a) => <option value={String(a.id)}>{a.name}</option>}</For>
              <Show when={can(org, "write-statements")}>
                <option disabled>──────────</option>
                <option value={NEW_ACCOUNT}>New bank account…</option>
              </Show>
            </select>
          </label>
        );
      }}
    </Show>
  );
}

/** Before anything else in Statements: the org's first account. */
function FirstAccount(props: { state: AccountState }) {
  const org = useOrg();
  return (
    <section class="mt-8 max-w-2xl rounded-xl border border-border p-6">
      <div class="flex items-center gap-3">
        <div class="flex h-10 w-10 items-center justify-center rounded-lg bg-secondary text-muted-foreground">
          <Landmark size={20} />
        </div>
        <div>
          <h2 class="text-base font-medium">Create your first bank account</h2>
          <p class="text-sm text-muted-foreground">Statements are uploaded into a bank account, so start with one.</p>
        </div>
      </div>
      <Show
        when={can(org, "write-statements")}
        fallback={<p class="mt-4 text-sm text-muted-foreground">Ask someone who can upload statements to create a bank account.</p>}
      >
        <div class="mt-5">
          <AccountForm
            submitLabel="Create bank account"
            onSaved={(a) => {
              props.state.pick(a.id);
              props.state.reload();
            }}
          />
        </div>
      </Show>
    </section>
  );
}

/**
 * Every Statements page: a header naming the account being worked on (with the picker to
 * switch), the tabs, then the page, given the account state. Until the org has an account,
 * only "Create your first account" is shown.
 */
export function StatementsShell(props: {
  /** Fills the window's height (grid pages). */
  fill?: boolean;
  children: (state: AccountState, account: () => Account) => JSX.Element;
}) {
  const state = useStatementAccount();
  const [creating, setCreating] = createSignal(false);
  const summary = () => {
    const a = state.account();
    const s = a && settled(state.accounts)?.find((x) => x.id === a.id);
    if (!s) return "";
    return `${s.rows.toLocaleString()} ${s.rows === 1 ? "row" : "rows"} from ${s.uploads} ${s.uploads === 1 ? "statement" : "statements"}`;
  };

  return (
    <div class={props.fill ? "flex min-h-0 flex-1 flex-col" : undefined}>
      <header class="flex flex-wrap items-end justify-between gap-4 border-b border-border pb-4">
        <div class="min-w-0">
          <p class="text-xs font-medium uppercase tracking-wide text-muted-foreground">Statements</p>
          <Show when={state.account()} fallback={<h1 class="truncate text-2xl font-semibold tracking-tight">Bank accounts</h1>}>
            {(a) => (
              <>
                <h1 class="truncate text-2xl font-semibold tracking-tight">{a().name}</h1>
                <p class="mt-1 flex flex-wrap items-center gap-x-2 text-sm text-muted-foreground">
                  <Show when={a().identifier} fallback={<span>No identifier</span>}>
                    <span class="rounded-md border border-border bg-secondary/40 px-1.5 py-0.5 font-mono text-xs text-foreground">
                      {a().identifier}
                    </span>
                  </Show>
                  <Show when={summary()}>
                    <span>· {summary()}</span>
                  </Show>
                </p>
              </>
            )}
          </Show>
        </div>
        <AccountPicker state={state} onNew={() => setCreating(true)} />
      </header>

      <Show when={state.accounts.error}>
        <p class="mt-6 text-sm text-red-500">{errorText(state.accounts.error, "Couldn't load accounts.")}</p>
      </Show>
      <Show when={state.account() === undefined && !state.accounts.error}>
        <div class="mt-6 h-32 animate-pulse rounded-xl bg-secondary/40" />
      </Show>
      <Show when={state.account() === null}>
        <FirstAccount state={state} />
      </Show>
      <Show when={state.account()}>
        {(account) => (
          <>
            <StatementsTabs />
            {props.children(state, account)}
          </>
        )}
      </Show>

      <AccountDialog
        open={creating()}
        onSaved={(a) => {
          setCreating(false);
          state.pick(a.id);
          state.reload();
        }}
        onClose={() => setCreating(false)}
      />
    </div>
  );
}
