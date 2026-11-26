import { createEffect, createMemo, createResource, createSignal, For, on, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import Eye from "lucide-solid/icons/eye";
import Loader2 from "lucide-solid/icons/loader-circle";
import Printer from "lucide-solid/icons/printer";
import X from "lucide-solid/icons/x";
import { Button } from "~/components/ui/button";
import { can } from "~/lib/orgs";
import {
  type Account,
  dateBasis,
  fetchPrintSummary,
  fetchTemplates,
  formatMoney,
  type OrgTransaction,
  type Period,
  printStatementPdf,
} from "~/lib/statements";
import { selectValue, settled } from "~/lib/utils";
import { useOrg } from "~/routes/portal/org/OrgLayout";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);
const fieldClass =
  "h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60";

/**
 * What's being printed: the grid's rows (in its order), its selection in words, and the period
 * to suggest (from date quick filters; otherwise the rows' first and last dates).
 */
export type PrintJob = { account: Account; rows: OrgTransaction[]; selection: string[]; period: Period | null };

/** The template picked for each account this session (the account's saved choice is only reloaded with the page). */
const picked = new Map<number, number>();

/**
 * Before printing what the grid shows: the template, the period, which date counts, and the
 * account's balances for the period (from all its rows, whatever the filters) with whether a
 * running balance can print. "Preview" opens the PDF in a new tab, where it can be downloaded;
 * a copy can also be saved to Documents.
 */
export function PrintDialog(props: { job: PrintJob | null; onClose: () => void }) {
  const org = useOrg();
  const canSave = can(org, "write-statements");
  let dialog!: HTMLDialogElement;
  const [save, setSave] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [done, setDone] = createSignal<string | null>(null);
  const [templateId, setTemplateId] = createSignal<number | null>(null);
  const [from, setFrom] = createSignal("");
  const [to, setTo] = createSignal("");
  // The grid's Effective dates switch (a preference for the org in this browser).
  const [basis] = dateBasis(org.slug);

  const [templates] = createResource(() => (props.job ? org.slug : undefined), fetchTemplates);
  const list = () => settled(templates)?.templates ?? [];
  const template = () => list().find((t) => t.id === templateId()) ?? list()[0];

  createEffect(
    on(
      () => props.job,
      (job) => {
        setError(null);
        setDone(null);
        if (job) {
          // Suggested by the grid, otherwise the rows' dates on the chosen basis.
          const dates = job.rows.map((r) => (basis() === "effective" ? (r.effective_date ?? r.date) : r.date)).sort();
          setFrom(job.period?.from ?? dates[0] ?? "");
          setTo(job.period?.to ?? dates[dates.length - 1] ?? "");
          setTemplateId(picked.get(job.account.id) ?? job.account.last_template_id);
          dialog.showModal();
        } else if (dialog.open) dialog.close();
      },
    ),
  );

  const period = (): Period | null => (from() && to() ? { from: from(), to: to() } : null);
  const periodError = () => (from() && to() && from() > to() ? "The period ends before it starts." : null);
  // Balances and completeness for the rows and period, from the server (all the account's rows).
  const summaryKey = createMemo(() => {
    const job = props.job;
    if (!job || periodError()) return undefined;
    return { account: job.account.id, rows: job.rows.map((r) => r.id), period: period(), date_basis: basis() };
  });
  const [summary] = createResource(summaryKey, (k) =>
    fetchPrintSummary(org.slug, k.account, { rows: k.rows, period: k.period, date_basis: k.date_basis }),
  );

  const runningNote = () => {
    const t = template();
    const s = settled(summary);
    if (!t?.columns.some((c) => c.key === "balance") || !s) return null;
    if (s.opening_cents === null) return "No running balance: the bank account has no starting balance.";
    if (t.group_by_type) return "No running balance: this template groups rows by category.";
    if (!s.rows_complete) {
      const tip =
        basis() === "transaction"
          ? " Date quick filters go by effective date, so they can leave out rows at the edges: try the grid's Effective dates switch."
          : "";
      return `No running balance: the bank account has ${s.period_rows.toLocaleString()} rows in this period and ${props.job!.rows.length.toLocaleString()} are printed. Clear the filters to print them all.${tip}`;
    }
    return "With a running balance, in date order.";
  };

  const preview = async () => {
    const job = props.job!;
    setBusy(true);
    setError(null);
    setDone(null);
    // Open the tab now: browsers block windows opened after an await.
    const tab = window.open("", "_blank");
    if (tab) {
      tab.document.title = "Making the PDF…";
      tab.document.body.textContent = "Making the PDF…";
    }
    try {
      const t = template();
      const out = await printStatementPdf(org.slug, job.account.id, {
        rows: job.rows.map((r) => r.id),
        selection: job.selection,
        save: canSave && save(),
        template_id: t?.id ?? null,
        period: period(),
        date_basis: basis(),
      });
      if (t) picked.set(job.account.id, t.id);
      const url = URL.createObjectURL(out.blob);
      if (tab) tab.location.href = url;
      else window.open(url, "_blank");
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      if (out.documentId !== null) setDone(`A copy of ${out.filename} is in Documents / Statements / ${job.account.name} / Printed.`);
      else dialog.close();
    } catch (e) {
      tab?.close();
      setError(errorText(e, "Couldn't make the PDF."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <dialog
      ref={dialog}
      onClose={() => props.job && props.onClose()}
      onCancel={(e) => busy() && e.preventDefault()}
      class="m-auto w-[calc(100%-2rem)] max-w-lg rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="print-title"
    >
      <Show when={props.job}>
        {(job) => (
          <div class="flex flex-col gap-5 p-6">
            <div class="flex items-start justify-between gap-4">
              <div>
                <h2 id="print-title" class="flex items-center gap-2 text-lg font-semibold tracking-tight">
                  <Printer size={18} /> Print statement
                </h2>
                <p class="mt-1 text-sm text-muted-foreground">A PDF of exactly the rows the grid shows.</p>
              </div>
              <button
                type="button"
                onClick={() => dialog.close()}
                disabled={busy()}
                class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
                aria-label="Close"
              >
                <X size={18} />
              </button>
            </div>

            <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
              <dt class="text-muted-foreground">Bank account</dt>
              <dd>{job().account.name}</dd>
              <dt class="text-muted-foreground">Rows</dt>
              <dd class="tabular-nums">{job().rows.length.toLocaleString()}</dd>
              <dt class="text-muted-foreground">Selection</dt>
              <dd>
                <Show when={job().selection.length} fallback={<span>All rows in the bank account</span>}>
                  <ul>
                    <For each={job().selection}>{(s) => <li>{s}</li>}</For>
                  </ul>
                </Show>
              </dd>
            </dl>

            <label class="flex flex-col gap-1">
              <span class="text-xs text-muted-foreground">Template</span>
              <select
                class={fieldClass}
                value={selectValue(String(template()?.id ?? ""), list())}
                disabled={busy() || !list().length}
                onChange={(e) => setTemplateId(Number(e.currentTarget.value))}
              >
                <For each={list()}>{(t) => <option value={t.id}>{t.name}</option>}</For>
              </select>
              <Show when={templates.error}>
                <span class="text-xs text-red-500">{errorText(templates.error, "Couldn't load the templates.")}</span>
              </Show>
              <span class="text-xs text-muted-foreground">
                Design them on the{" "}
                <Link to="/portal/$org/statements/templates" params={{ org: org.slug }} class="underline underline-offset-2 hover:text-foreground">
                  Templates
                </Link>{" "}
                tab.
              </span>
            </label>

            <fieldset class="flex flex-col gap-2">
              <legend class="mb-1 text-xs text-muted-foreground">Statement period</legend>
              <div class="flex items-center gap-2">
                <input type="date" class={fieldClass} value={from()} disabled={busy()} onInput={(e) => setFrom(e.currentTarget.value)} aria-label="From" />
                <span class="text-muted-foreground">to</span>
                <input type="date" class={fieldClass} value={to()} disabled={busy()} onInput={(e) => setTo(e.currentTarget.value)} aria-label="To" />
              </div>
              <Show when={periodError()}>
                <span class="text-xs text-red-500">{periodError()}</span>
              </Show>
            </fieldset>

            <div class="rounded-lg border border-border bg-secondary/20 p-3 text-sm">
              <Show when={summary.error}>
                <p class="text-red-500">{errorText(summary.error, "Couldn't work out the balances.")}</p>
              </Show>
              <Show when={!summary.error}>
                <Show when={settled(summary)} fallback={<div class="h-10 animate-pulse rounded bg-secondary/40" />}>
                  {(s) => (
                    <div class="flex flex-col gap-1.5">
                      <Show
                        when={s().opening_cents !== null}
                        fallback={
                          <p class="text-muted-foreground">
                            No balances: give the bank account a starting balance on the{" "}
                            <Link to="/portal/$org/statements/accounts" params={{ org: org.slug }} class="underline underline-offset-2 hover:text-foreground">
                              Bank accounts
                            </Link>{" "}
                            tab.
                          </p>
                        }
                      >
                        <div class="grid grid-cols-2 gap-2">
                          <div>
                            <span class="block text-xs text-muted-foreground">Opening balance</span>
                            <span class="font-mono tabular-nums">{formatMoney(s().opening_cents!)}</span>
                          </div>
                          <div>
                            <span class="block text-xs text-muted-foreground">Closing balance</span>
                            <span class="font-medium font-mono tabular-nums">{formatMoney(s().closing_cents!)}</span>
                          </div>
                        </div>
                        <p class="text-xs text-muted-foreground">From all the bank account's rows, whatever is filtered.</p>
                      </Show>
                      <Show when={runningNote()}>
                        <p class="text-xs text-muted-foreground">{runningNote()}</p>
                      </Show>
                    </div>
                  )}
                </Show>
              </Show>
            </div>

            <Show when={canSave}>
              <label class="flex items-center gap-2 text-sm">
                <input type="checkbox" class="h-4 w-4" checked={save()} disabled={busy()} onChange={(e) => setSave(e.currentTarget.checked)} />
                Also save a copy to Documents (Statements / {job().account.name} / Printed)
              </label>
            </Show>

            <Show when={error()}>
              <p role="alert" class="text-sm text-red-500">{error()}</p>
            </Show>
            <Show when={done()}>
              <p role="status" class="text-sm text-muted-foreground">{done()}</p>
            </Show>

            <div class="flex justify-end gap-2">
              <Button variant="outline" disabled={busy()} onClick={() => dialog.close()}>
                {done() ? "Done" : "Cancel"}
              </Button>
              <Button disabled={busy() || job().rows.length === 0 || !!periodError() || !template()} onClick={() => void preview()}>
                <Show when={busy()} fallback={<Eye size={16} />}>
                  <Loader2 size={16} class="animate-spin" />
                </Show>
                {busy() ? "Making the PDF…" : "Preview"}
              </Button>
            </div>
          </div>
        )}
      </Show>
    </dialog>
  );
}
