import { createEffect, createResource, createSignal, For, on, Show } from "solid-js";
import Loader2 from "lucide-solid/icons/loader-circle";
import Paperclip from "lucide-solid/icons/paperclip";
import Play from "lucide-solid/icons/play";
import Plus from "lucide-solid/icons/plus";
import Split from "lucide-solid/icons/split";
import Trash from "lucide-solid/icons/trash";
import X from "lucide-solid/icons/x";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { can } from "~/lib/orgs";
import {
  type Account,
  deleteExcludedMatches,
  type Exclusion,
  fetchExcludedMatches,
  fetchExclusions,
  formatAmount,
  formatDate,
  isNegative,
  saveExclusions,
} from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { useOrg } from "~/routes/portal/org/OrgLayout";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);

/** Adding one exclusion: the texts to look for (one per line) and why. */
function AddExclusionDialog(props: { open: boolean; onAdd: (rule: Exclusion) => Promise<void>; onClose: () => void }) {
  let dialog!: HTMLDialogElement;
  const [contains, setContains] = createSignal("");
  const [note, setNote] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) {
          setContains("");
          setNote("");
          setError(null);
          dialog.showModal();
        } else if (dialog.open) {
          dialog.close();
        }
      },
    ),
  );

  // Lines are kept as typed (a trailing space can matter), apart from dropping blank ones.
  const texts = () => contains().split("\n").filter((l) => l.trim() !== "");
  const add = async (e: SubmitEvent) => {
    e.preventDefault();
    if (texts().length === 0) return;
    setBusy(true);
    setError(null);
    try {
      await props.onAdd({ contains: texts(), note: note().trim() });
    } catch (err) {
      setError(errorText(err, "Couldn't add the exclusion."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <dialog
      ref={dialog}
      onClose={() => props.open && props.onClose()}
      onCancel={(e) => busy() && e.preventDefault()}
      class="m-auto w-[calc(100%-2rem)] max-w-lg rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="add-exclusion-title"
    >
      <form onSubmit={(e) => void add(e)} class="flex flex-col gap-4 p-5">
        <div class="flex items-center justify-between gap-4">
          <h2 id="add-exclusion-title" class="text-base font-semibold">Add an upload exclusion</h2>
          <button
            type="button"
            onClick={() => props.onClose()}
            aria-label="Close"
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={18} />
          </button>
        </div>
        <label class="flex flex-col gap-1.5">
          <span class="text-xs text-muted-foreground">Leave out lines whose description contains any of (one per line, ignoring case)</span>
          <textarea
            rows={3}
            value={contains()}
            onInput={(e) => setContains(e.currentTarget.value)}
            disabled={busy()}
            class="w-full rounded-md border border-input bg-background/60 px-3 py-2 font-mono text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
            placeholder="OUTSTANDING CARD AUTHORISATION"
          />
        </label>
        <label class="flex flex-col gap-1.5">
          <span class="text-xs text-muted-foreground">Why (optional)</span>
          <Input class="h-9" value={note()} onInput={(e) => setNote(e.currentTarget.value)} disabled={busy()} placeholder="e.g. Not settled yet" />
        </label>
        <Show when={error()}>
          <p role="alert" class="text-sm text-red-500">{error()}</p>
        </Show>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={() => props.onClose()} disabled={busy()}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy() || texts().length === 0}>
            <Show when={busy()}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            Add exclusion
          </Button>
        </div>
      </form>
    </dialog>
  );
}

/**
 * Testing exclusion `rule` (0-based) against the account's saved rows: lists those it matches,
 * which can be deleted from here with `delete-statement-rows`.
 */
function MatchesDialog(props: { rule: number | null; exclusion: Exclusion | undefined; account: Account; onClose: () => void }) {
  const org = useOrg();
  const canDelete = can(org, "delete-statement-rows");
  let dialog!: HTMLDialogElement;
  const [found, { refetch }] = createResource(
    () => (props.rule === null ? undefined : { account: props.account.id, rule: props.rule }),
    (k) => fetchExcludedMatches(org.slug, k.account, k.rule),
  );
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);

  createEffect(
    on(
      () => props.rule,
      (rule) => {
        if (rule !== null) {
          setError(null);
          setNotice(null);
          dialog.showModal();
        } else if (dialog.open) {
          dialog.close();
        }
      },
    ),
  );

  const rows = () => (found.state === "ready" ? found()!.rows : []);
  const remove = async () => {
    const list = rows();
    const splits = list.reduce((n, r) => n + r.split_rows, 0);
    const attached = list.filter((r) => r.attachments > 0).length;
    const extra = [
      splits ? `their splits' ${splits} rows` : "",
      attached ? `the links to documents attached to ${attached} of them (the documents stay)` : "",
    ].filter(Boolean);
    if (
      !confirm(
        `Delete ${list.length} saved ${list.length === 1 ? "row" : "rows"}${extra.length ? `, with ${extra.join(" and ")}` : ""}? ` +
          "This can't be undone. Uploading the statements again won't bring them back while this exclusion matches them.",
      )
    )
      return;
    setBusy(true);
    setError(null);
    try {
      const r = await deleteExcludedMatches(org.slug, props.account.id, props.rule!, list.map((x) => x.id));
      setNotice(`Deleted ${r.deleted.toLocaleString()} ${r.deleted === 1 ? "row" : "rows"}.`);
      await refetch();
    } catch (e) {
      setError(errorText(e, "Couldn't delete the rows."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <dialog
      ref={dialog}
      onClose={() => props.rule !== null && props.onClose()}
      onCancel={(e) => busy() && e.preventDefault()}
      class="m-auto max-h-[calc(100vh-4rem)] w-[calc(100%-2rem)] max-w-4xl overflow-y-auto rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="matches-title"
    >
      <div class="flex flex-col gap-4 p-5">
        <div class="flex items-start justify-between gap-4">
          <div class="min-w-0">
            <h2 id="matches-title" class="text-base font-semibold">Saved rows this exclusion matches</h2>
            <p class="mt-1 flex flex-wrap gap-1.5">
              <For each={props.exclusion?.contains ?? []}>
                {(text) => <span class="whitespace-pre rounded-md border border-border bg-secondary/40 px-1.5 py-0.5 font-mono text-xs">{text}</span>}
              </For>
            </p>
          </div>
          <button
            type="button"
            onClick={() => props.onClose()}
            disabled={busy()}
            aria-label="Close"
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={18} />
          </button>
        </div>

        <Show when={found.loading}>
          <div class="h-24 animate-pulse rounded-xl bg-secondary/40" />
        </Show>
        <Show when={found.error}>
          <p class="text-sm text-red-500">{errorText(found.error, "Couldn't find the matching rows.")}</p>
        </Show>
        <Show when={found.state === "ready"}>
          <Show
            when={rows().length > 0}
            fallback={
              <p class="rounded-xl border border-dashed border-border px-6 py-8 text-center text-sm text-muted-foreground">
                {notice() ?? `No rows saved in ${props.account.name} match.`}
              </p>
            }
          >
            <div class="max-h-[50vh] overflow-y-auto rounded-xl border border-border">
              <table class="w-full min-w-[36rem] text-sm">
                <thead class="sticky top-0 border-b border-border bg-background text-left text-xs text-muted-foreground">
                  <tr>
                    <th class="w-32 px-4 py-2.5 font-medium">Date</th>
                    <th class="px-4 py-2.5 font-medium">Description</th>
                    <th class="w-36 px-4 py-2.5 text-right font-medium">Amount</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={rows()}>
                    {(r) => (
                      <tr class="border-b border-border last:border-0">
                        <td class="whitespace-nowrap px-4 py-2 text-muted-foreground">{formatDate(r.date)}</td>
                        <td class="whitespace-pre-wrap break-words px-4 py-2">
                          <Show when={r.split_rows > 0}>
                            <span class="mr-1.5 inline-flex align-[-2px] text-muted-foreground" title={`Split into ${r.split_rows} rows (deleted with it)`}>
                              <Split size={13} />
                            </span>
                          </Show>
                          <Show when={r.attachments > 0}>
                            <span class="mr-1.5 inline-flex align-[-2px] text-muted-foreground" title={`${r.attachments} documents attached`}>
                              <Paperclip size={13} />
                            </span>
                          </Show>
                          {r.description}
                        </td>
                        <td class={cn("whitespace-nowrap px-4 py-2 text-right font-mono tabular-nums", isNegative(r.amount) ? "" : "text-emerald-600 dark:text-emerald-400")}>
                          {formatAmount(r.amount)} <span class="text-xs text-muted-foreground">{r.currency}</span>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </Show>

        <Show when={error()}>
          <p role="alert" class="text-sm text-red-500">{error()}</p>
        </Show>
        <div class="flex flex-wrap items-center justify-end gap-2">
          <Show when={rows().length > 0 && !canDelete}>
            <p class="mr-auto text-sm text-muted-foreground">Deleting them needs the delete-statement-rows permission.</p>
          </Show>
          <Button variant="ghost" onClick={() => props.onClose()} disabled={busy()}>
            Close
          </Button>
          <Show when={canDelete && rows().length > 0}>
            <Button variant="outline" class="text-red-500 hover:text-red-600" disabled={busy()} onClick={() => void remove()}>
              <Show when={busy()} fallback={<Trash size={14} />}>
                <Loader2 size={14} class="animate-spin" />
              </Show>
              Delete {rows().length} {rows().length === 1 ? "row" : "rows"}
            </Button>
          </Show>
        </div>
      </div>
    </dialog>
  );
}

/**
 * The account's upload exclusions, listed on the Uploads tab: lines whose description contains
 * any of an exclusion's texts are left out of uploads into the account. Adding (in a dialog) and
 * removing need `write-statements`. `onChanged` runs after each change.
 */
export function ExclusionsSection(props: { account: Account; onChanged: () => void }) {
  const org = useOrg();
  const editable = can(org, "write-statements");
  const [rules, { mutate }] = createResource(
    () => props.account.id,
    (id) => fetchExclusions(org.slug, id),
  );
  const [adding, setAdding] = createSignal(false);
  // The exclusion whose matching saved rows are showing.
  const [testing, setTesting] = createSignal<number | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const list = () => settled(rules)?.rules ?? [];

  const save = async (next: Exclusion[]) => {
    mutate(await saveExclusions(org.slug, props.account.id, next));
    props.onChanged();
  };
  const add = async (rule: Exclusion) => {
    await save([...list(), rule]);
    setAdding(false);
  };
  const remove = async (i: number) => {
    setBusy(true);
    setError(null);
    try {
      await save(list().filter((_, j) => j !== i));
    } catch (e) {
      setError(errorText(e, "Couldn't remove the exclusion."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section class="mt-6">
      <div class="flex items-center justify-between gap-2">
        <h2 class="text-sm font-medium">Upload exclusions</h2>
        <Show when={editable}>
          <Button variant="outline" size="sm" class="h-8" onClick={() => setAdding(true)}>
            <Plus size={14} /> Add exclusion
          </Button>
        </Show>
      </div>
      <Show when={rules.error}>
        <p class="mt-2 text-sm text-red-500">{errorText(rules.error, "Couldn't load the exclusions.")}</p>
      </Show>
      <Show when={settled(rules)}>
        <Show
          when={list().length > 0}
          fallback={<p class="mt-2 text-sm text-muted-foreground">None: every line of an upload is saved.</p>}
        >
          <ul class="mt-2 flex flex-col divide-y divide-border rounded-xl border border-border">
            <For each={list()}>
              {(r, i) => (
                <li class="flex items-center gap-3 px-4 py-2 text-sm">
                  <div class="min-w-0 flex-1">
                    <span class="flex flex-wrap gap-1.5">
                      <For each={r.contains}>
                        {(text) => (
                          <span class="whitespace-pre rounded-md border border-border bg-secondary/40 px-1.5 py-0.5 font-mono text-xs">{text}</span>
                        )}
                      </For>
                    </span>
                    <Show when={r.note}>
                      <span class="mt-0.5 block text-xs text-muted-foreground">{r.note}</span>
                    </Show>
                  </div>
                  <button
                    type="button"
                    onClick={() => setTesting(i())}
                    aria-label="Show saved rows this exclusion matches"
                    title="Show saved rows this exclusion matches"
                    class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                  >
                    <Play size={15} />
                  </button>
                  <Show when={editable}>
                    <button
                      type="button"
                      onClick={() => void remove(i())}
                      disabled={busy()}
                      aria-label="Remove exclusion"
                      title="Remove exclusion"
                      class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-500 disabled:opacity-30"
                    >
                      <Trash size={15} />
                    </button>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
      <Show when={error()}>
        <p role="alert" class="mt-2 text-sm text-red-500">{error()}</p>
      </Show>
      <MatchesDialog
        rule={testing()}
        exclusion={testing() === null ? undefined : list()[testing()!]}
        account={props.account}
        onClose={() => setTesting(null)}
      />
      <Show when={editable}>
        <AddExclusionDialog open={adding()} onAdd={add} onClose={() => setAdding(false)} />
      </Show>
    </section>
  );
}
