import { createEffect, createMemo, createResource, createSignal, For, type JSX, Show, untrack } from "solid-js";
import { createStore, produce, reconcile, unwrap } from "solid-js/store";
import { Link, useParams } from "@tanstack/solid-router";
import ArrowDown from "lucide-solid/icons/arrow-down";
import ArrowLeft from "lucide-solid/icons/arrow-left";
import ArrowUp from "lucide-solid/icons/arrow-up";
import Eye from "lucide-solid/icons/eye";
import Loader2 from "lucide-solid/icons/loader-circle";
import Plus from "lucide-solid/icons/plus";
import X from "lucide-solid/icons/x";
import { Button } from "~/components/ui/button";
import { fetchAllDocuments } from "~/lib/documents";
import { can } from "~/lib/orgs";
import {
  type ColumnKey,
  type SummaryKey,
  fetchOrgTransactions,
  fetchTemplate,
  fetchTemplates,
  printStatementPdf,
  saveTemplate,
  type TemplateSettings,
} from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { type AccountState, StatementsShell } from "~/routes/portal/org/StatementsShell";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);
const IMAGE = /\.(png|jpe?g|gif|svg|webp)$/i;
const PDF = /\.pdf$/i;
const place = (d: { folder: string; filename: string }) => [d.folder, d.filename].filter(Boolean).join(" / ");
/** Rows in the preview: about the account's latest ones (whole days, so a running balance can show). */
const PREVIEW_ROWS = 30;

const fieldClass =
  "h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60";
const iconButton =
  "flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-40";

const BLANK: TemplateSettings = {
  name: "",
  layout: "standard",
  title: "Statement",
  subtitle: "",
  header_fields: [],
  footer: "",
  logo_document_id: null,
  letterhead_document_id: null,
  letterhead_every_page: true,
  margin_top_mm: 18,
  margin_bottom_mm: 22,
  paper: "a4",
  columns: [],
  totals: {
    summary: [],
    summary_title: "",
    table_row: true,
    table_row_label: "Totals",
    subtotals: true,
    subtotal_label: "Subtotal",
    opening_row_label: "Balance brought forward",
  },
  show_summary: true,
  group_by_type: false,
  date_style: "short",
  currency_symbol: "",
};

/** A copy without the store's proxies, to compare and send. */
const plain = (s: TemplateSettings): TemplateSettings => structuredClone(unwrap(s));

function Section(props: { title: string; hint?: JSX.Element; children: JSX.Element }) {
  return (
    <section class="flex flex-col gap-3 rounded-xl border border-border p-4">
      <div>
        <h2 class="text-sm font-semibold">{props.title}</h2>
        <Show when={props.hint}>
          <p class="mt-0.5 text-xs text-muted-foreground">{props.hint}</p>
        </Show>
      </div>
      {props.children}
    </section>
  );
}

/**
 * Designs one of the org's statement templates. Text can use `{name}` variables: clicking one in
 * the list inserts it where the cursor was. "Save and preview" prints the latest rows of the
 * account the Statements pages show, with it, into a new tab.
 */
function Editor(props: { id: number; state: AccountState }) {
  const org = useOrg();
  const editable = can(org, "write-statements");
  // Picking a logo or letterhead lists Documents.
  const canPick = editable && can(org, "read-docs");

  const [loaded, { mutate }] = createResource(() => props.id, (id) => fetchTemplate(org.slug, id));
  const [choices] = createResource(() => org.slug, fetchTemplates);
  const [docs] = createResource(() => (canPick ? org.slug : undefined), fetchAllDocuments);
  const images = () => settled(docs)?.filter((d) => IMAGE.test(d.filename));
  const pdfs = () => settled(docs)?.filter((d) => PDF.test(d.filename));
  const variables = () => settled(choices)?.variables ?? [];
  const layouts = () => settled(choices)?.layouts ?? [];
  const columnChoices = () => settled(choices)?.columns ?? [];
  const figureChoices = () => settled(choices)?.summary_figures ?? [];

  const [s, setS] = createStore<TemplateSettings>({ ...BLANK });
  // What was last loaded or saved, to tell when there are changes.
  const [saved, setSaved] = createSignal<string>("");
  const [busy, setBusy] = createSignal<"saving" | "preview" | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);

  createEffect(() => {
    const l = settled(loaded);
    if (!l) return;
    const { logo: _l, letterhead: _h, id: _id, updated_at: _u, ...settings } = l;
    setS(reconcile(settings));
    // Untracked: reading the store here would re-run this on every edit, so nothing would ever
    // count as a change (and Save would stay disabled).
    setSaved(untrack(() => JSON.stringify(plain(s))));
  });
  const dirty = createMemo(() => !!saved() && JSON.stringify(plain(s)) !== saved());
  const isStandard = () => s.layout !== "bank";
  const logoName = () => {
    const l = settled(loaded)?.logo;
    return l && settled(loaded)?.logo_document_id === s.logo_document_id ? place(l) : null;
  };
  const letterheadName = () => {
    const l = settled(loaded)?.letterhead;
    return l && settled(loaded)?.letterhead_document_id === s.letterhead_document_id ? place(l) : null;
  };

  // ---- variables: inserted where the cursor was in the last text field used ----
  let target: { el: HTMLInputElement; set: (v: string) => void } | null = null;
  const track = (set: (v: string) => void) => (e: FocusEvent) => {
    target = { el: e.currentTarget as HTMLInputElement, set };
  };
  const insert = (name: string) => {
    if (!target || !editable) return;
    const { el, set } = target;
    const token = `{${name}}`;
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? start;
    set(el.value.slice(0, start) + token + el.value.slice(end));
    queueMicrotask(() => {
      el.focus();
      el.setSelectionRange(start + token.length, start + token.length);
    });
  };

  /** Moves item `i` of a list by `by` places. */
  const move = <T,>(list: T[], i: number, by: number) => {
    const j = i + by;
    if (j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
  };
  const unusedColumns = () => columnChoices().filter((c) => !s.columns.some((x) => x.key === c.key));
  const columnInfo = (key: ColumnKey) => columnChoices().find((c) => c.key === key);
  const unusedFigures = () => figureChoices().filter((c) => !s.totals.summary.some((x) => x.key === c.key));
  const figureInfo = (key: SummaryKey) => figureChoices().find((c) => c.key === key);
  const hasBalance = () => s.columns.some((c) => c.key === "balance");

  const save = async () => {
    // An empty or partly typed number is NaN, which would reach the server as null.
    if (![s.margin_top_mm, s.margin_bottom_mm].every((mm) => Number.isInteger(mm) && mm >= 5 && mm <= 150)) {
      throw new Error("The margins are whole numbers of mm, from 5 to 150.");
    }
    const view = await saveTemplate(org.slug, props.id, plain(s));
    mutate(view);
  };

  const run = async (kind: "saving" | "preview") => {
    setBusy(kind);
    setError(null);
    setNotice(null);
    // Open the tab now: browsers block windows opened after an await.
    const tab = kind === "preview" ? window.open("", "_blank") : null;
    try {
      await save();
      if (kind === "saving") {
        setNotice("Saved.");
        return;
      }
      const account = props.state.account()!;
      // Newest first. Every row from the day of the 30th newest on, oldest first.
      const all = await fetchOrgTransactions(org.slug, account.id);
      const from = all[Math.min(PREVIEW_ROWS, all.length) - 1]?.date;
      const rows = all.filter((r) => from && r.date >= from).reverse();
      const { blob } = await printStatementPdf(org.slug, account.id, {
        rows: rows.map((r) => r.id),
        selection: [`Preview: the latest ${rows.length} rows`],
        save: false,
        template_id: props.id,
        period: null,
      });
      const url = URL.createObjectURL(blob);
      if (tab) tab.location.href = url;
      else window.open(url, "_blank");
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
    } catch (e) {
      tab?.close();
      setError(errorText(e, kind === "preview" ? "Couldn't make the preview." : "Couldn't save the template."));
    } finally {
      setBusy(null);
    }
  };

  const Check = (p: { label: string; hint?: string; value: boolean; onChange: (v: boolean) => void }) => (
    <label class="flex items-start gap-2 text-sm">
      <input type="checkbox" class="mt-0.5 h-4 w-4" checked={p.value} disabled={!editable} onChange={(e) => p.onChange(e.currentTarget.checked)} />
      <span>
        {p.label}
        <Show when={p.hint}>
          <span class="block text-xs text-muted-foreground">{p.hint}</span>
        </Show>
      </span>
    </label>
  );

  /** A text input that variables can be inserted into. */
  const VarInput = (p: { value: string; onInput: (v: string) => void; placeholder?: string; label?: string }) => (
    <input
      class={fieldClass}
      value={p.value}
      placeholder={p.placeholder}
      aria-label={p.label}
      disabled={!editable}
      onFocus={track(p.onInput)}
      onInput={(e) => p.onInput(e.currentTarget.value)}
    />
  );

  const MoveButtons = (p: { index: number; count: number; onMove: (by: number) => void; onRemove: () => void; what: string }) => (
    <Show when={editable}>
      <button type="button" class={iconButton} disabled={p.index === 0} onClick={() => p.onMove(-1)} aria-label={`Move ${p.what} up`}>
        <ArrowUp size={14} />
      </button>
      <button type="button" class={iconButton} disabled={p.index === p.count - 1} onClick={() => p.onMove(1)} aria-label={`Move ${p.what} down`}>
        <ArrowDown size={14} />
      </button>
      <button type="button" class={iconButton} onClick={() => p.onRemove()} aria-label={`Remove ${p.what}`}>
        <X size={14} />
      </button>
    </Show>
  );

  return (
    <div class="mt-6 flex flex-col gap-4">
      <div class="sticky top-0 z-10 -mx-1 flex flex-wrap items-center gap-2 border-b border-border bg-background/95 px-1 py-3 backdrop-blur">
        <Link
          to="/portal/$org/statements/templates"
          params={{ org: org.slug }}
          class="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft size={16} /> Templates
        </Link>
        <span class="text-muted-foreground">/</span>
        <h1 class="text-base font-semibold">{settled(loaded)?.name ?? "…"}</h1>
        <Show when={dirty()}>
          <span class="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-400">Unsaved changes</span>
        </Show>
        <Show when={notice() && !dirty()}>
          <span role="status" class="text-sm text-muted-foreground">
            {notice()}
          </span>
        </Show>
        <span class="flex-1" />
        <Show when={editable}>
          <Button
            variant="outline"
            size="sm"
            disabled={!!busy() || !settled(loaded) || !props.state.account()}
            onClick={() => void run("preview")}
            title={
              props.state.account()
                ? `Saves, then prints about the latest ${PREVIEW_ROWS} rows of ${props.state.account()!.name} in a new tab`
                : "Pick a bank account first"
            }
          >
            <Show when={busy() === "preview"} fallback={<Eye size={16} />}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            Save and preview
          </Button>
          <Button size="sm" disabled={!!busy() || !settled(loaded) || !dirty()} onClick={() => void run("saving")}>
            <Show when={busy() === "saving"}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            Save
          </Button>
        </Show>
      </div>

      <Show when={error()}>
        <p role="alert" class="text-sm text-red-500">{error()}</p>
      </Show>
      <Show when={loaded.error}>
        <p class="text-sm text-red-500">{errorText(loaded.error, "Couldn't load the template.")}</p>
      </Show>
      <Show when={!settled(loaded) && !loaded.error}>
        <div class="h-64 animate-pulse rounded-xl bg-secondary/40" />
      </Show>

      <Show when={settled(loaded)}>
        <div class="grid gap-6 lg:grid-cols-[minmax(0,1fr)_16rem]">
          <div class="flex min-w-0 flex-col gap-4">
            <Section title="Template">
              <div class="grid gap-4 sm:grid-cols-2">
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Name</span>
                  <input class={fieldClass} value={s.name} disabled={!editable} onInput={(e) => setS("name", e.currentTarget.value)} />
                </label>
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Layout</span>
                  <select class={fieldClass} value={s.layout} disabled={!editable} onChange={(e) => setS("layout", e.currentTarget.value)}>
                    <For each={layouts()}>{(l) => <option value={l.name}>{l.label}</option>}</For>
                  </select>
                </label>
              </div>
            </Section>

            <Section title="Text" hint="Variables work in all of these.">
              <div class="grid gap-4 sm:grid-cols-2">
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Title (optional)</span>
                  <VarInput value={s.title} onInput={(v) => setS("title", v)} placeholder="None, e.g. when the letterhead has one" />
                </label>
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Subtitle (optional)</span>
                  <VarInput value={s.subtitle} onInput={(v) => setS("subtitle", v)} placeholder="e.g. {account.name}" />
                </label>
                <label class="flex flex-col gap-1 sm:col-span-2">
                  <span class="text-xs text-muted-foreground">Footer text (optional)</span>
                  <VarInput value={s.footer} onInput={(v) => setS("footer", v)} placeholder="e.g. Prepared for SARS" />
                </label>
              </div>
            </Section>

            <Section
              title="Header fields"
              hint="The block at the top, label and value. A field whose value comes out empty (e.g. a bank account without a holder) is left out."
            >
              <div class="flex flex-col gap-2">
                <For each={s.header_fields}>
                  {(f, i) => (
                    <div class="flex items-center gap-2">
                      <div class="w-2/5">
                        <VarInput value={f.label} onInput={(v) => setS("header_fields", i(), "label", v)} label={`Label ${i() + 1}`} placeholder="Label" />
                      </div>
                      <div class="flex-1">
                        <VarInput value={f.value} onInput={(v) => setS("header_fields", i(), "value", v)} label={`Value ${i() + 1}`} placeholder="Value" />
                      </div>
                      <MoveButtons
                        index={i()}
                        count={s.header_fields.length}
                        what="field"
                        onMove={(by) => setS("header_fields", produce((list) => move(list, i(), by)))}
                        onRemove={() => setS("header_fields", (list) => list.filter((_, j) => j !== i()))}
                      />
                    </div>
                  )}
                </For>
                <Show when={editable && s.header_fields.length < 20}>
                  <div>
                    <Button size="sm" variant="outline" onClick={() => setS("header_fields", (list) => [...list, { label: "", value: "" }])}>
                      <Plus size={14} /> Add field
                    </Button>
                  </div>
                </Show>
              </div>
            </Section>

            <Section
              title="Table columns"
              hint="Which columns the transactions table has, in order, and their headings. Amount is signed; Debit and Credit split it into money out and money in."
            >
              <div class="flex flex-col gap-2">
                <For each={s.columns}>
                  {(c, i) => (
                    <div class="flex flex-col gap-1">
                      <div class="flex items-center gap-2">
                        <span class="w-32 shrink-0 text-sm" title={columnInfo(c.key)?.hint}>
                          {columnInfo(c.key)?.label ?? c.key}
                        </span>
                        <input
                          class={fieldClass}
                          value={c.label}
                          placeholder={columnInfo(c.key)?.label}
                          aria-label={`Heading for ${columnInfo(c.key)?.label ?? c.key}`}
                          disabled={!editable}
                          onInput={(e) => setS("columns", i(), "label", e.currentTarget.value)}
                        />
                        <MoveButtons
                          index={i()}
                          count={s.columns.length}
                          what="column"
                          onMove={(by) => setS("columns", produce((list) => move(list, i(), by)))}
                          onRemove={() => setS("columns", (list) => list.filter((_, j) => j !== i()))}
                        />
                      </div>
                      <Show when={c.key === "balance"}>
                        <span class="pl-[8.5rem] text-xs text-muted-foreground">
                          Prints only when the bank account has a starting balance and every row in the period is printed (not grouped by category).
                        </span>
                      </Show>
                    </div>
                  )}
                </For>
                <Show when={editable && unusedColumns().length > 0}>
                  <label class="flex items-center gap-2">
                    <Plus size={14} class="text-muted-foreground" />
                    <select
                      class={cn(fieldClass, "w-auto")}
                      value=""
                      onChange={(e) => {
                        const key = e.currentTarget.value as ColumnKey;
                        e.currentTarget.value = "";
                        const info = columnInfo(key);
                        if (info) setS("columns", (list) => [...list, { key, label: info.label }]);
                      }}
                      aria-label="Add a column"
                    >
                      <option value="">Add a column…</option>
                      <For each={unusedColumns()}>{(c) => <option value={c.key}>{`${c.label}: ${c.hint}`}</option>}</For>
                    </select>
                  </label>
                </Show>
              </div>
              <Check
                label="Group rows by category"
                hint="A heading for each category, then its rows (no Category or Balance column then)"
                value={s.group_by_type}
                onChange={(v) => setS("group_by_type", v)}
              />
            </Section>

            <Section title="Totals" hint="Which totals the statement has, and what they're called. Labels can use variables.">
              <div class="flex flex-col gap-2">
                <h3 class="text-xs font-medium text-muted-foreground">Summary</h3>
                <p class="-mt-1 text-xs text-muted-foreground">
                  Figures near the top: boxes in the standard layout, a panel beside the header fields in the bank style. Remove them all for
                  no summary. Balances show only when the bank account has a starting balance.
                </p>
                <label class="flex items-center gap-2">
                  <span class="w-32 shrink-0 text-sm">Heading</span>
                  <VarInput value={s.totals.summary_title} onInput={(v) => setS("totals", "summary_title", v)} placeholder="None" />
                </label>
                <For each={s.totals.summary}>
                  {(f, i) => (
                    <div class="flex items-center gap-2">
                      <span class="w-32 shrink-0 text-sm" title={figureInfo(f.key)?.hint}>
                        {figureInfo(f.key)?.label ?? f.key}
                      </span>
                      <VarInput
                        value={f.label}
                        onInput={(v) => setS("totals", "summary", i(), "label", v)}
                        label={`Label for ${figureInfo(f.key)?.label ?? f.key}`}
                        placeholder={figureInfo(f.key)?.label}
                      />
                      <MoveButtons
                        index={i()}
                        count={s.totals.summary.length}
                        what="figure"
                        onMove={(by) => setS("totals", "summary", produce((list) => move(list, i(), by)))}
                        onRemove={() => setS("totals", "summary", (list) => list.filter((_, j) => j !== i()))}
                      />
                    </div>
                  )}
                </For>
                <Show when={editable && unusedFigures().length > 0}>
                  <label class="flex items-center gap-2">
                    <Plus size={14} class="text-muted-foreground" />
                    <select
                      class={cn(fieldClass, "w-auto")}
                      value=""
                      onChange={(e) => {
                        const key = e.currentTarget.value as SummaryKey;
                        e.currentTarget.value = "";
                        const info = figureInfo(key);
                        if (info) setS("totals", "summary", (list) => [...list, { key, label: info.label }]);
                      }}
                      aria-label="Add a summary figure"
                    >
                      <option value="">Add a figure…</option>
                      <For each={unusedFigures()}>{(c) => <option value={c.key}>{`${c.label}: ${c.hint}`}</option>}</For>
                    </select>
                  </label>
                </Show>
              </div>

              <div class="flex flex-col gap-2 border-t border-border pt-3">
                <h3 class="text-xs font-medium text-muted-foreground">In the table</h3>
                <div class="flex flex-wrap items-center gap-2">
                  <label class="flex w-56 shrink-0 items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      class="h-4 w-4"
                      checked={s.totals.table_row}
                      disabled={!editable}
                      onChange={(e) => setS("totals", "table_row", e.currentTarget.checked)}
                    />
                    Totals row at the end
                  </label>
                  <div class="min-w-48 flex-1">
                    <Show when={s.totals.table_row}>
                      <VarInput value={s.totals.table_row_label} onInput={(v) => setS("totals", "table_row_label", v)} label="Totals row label" />
                    </Show>
                  </div>
                </div>
                <div class="flex flex-wrap items-center gap-2">
                  <label class="flex w-56 shrink-0 items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      class="h-4 w-4"
                      checked={s.totals.subtotals}
                      disabled={!editable}
                      onChange={(e) => setS("totals", "subtotals", e.currentTarget.checked)}
                    />
                    Subtotal after each category
                  </label>
                  <div class="min-w-48 flex-1">
                    <Show when={s.totals.subtotals}>
                      <VarInput value={s.totals.subtotal_label} onInput={(v) => setS("totals", "subtotal_label", v)} label="Subtotal label" />
                    </Show>
                  </div>
                </div>
                <Show when={s.totals.subtotals && !s.group_by_type}>
                  <p class="text-xs text-muted-foreground">Subtotals print only when rows are grouped by category (Table columns).</p>
                </Show>
                <Show when={hasBalance()}>
                  <label class="flex flex-wrap items-center gap-2">
                    <span class="w-56 shrink-0 text-sm">Opening balance row</span>
                    <div class="min-w-48 flex-1">
                      <VarInput
                        value={s.totals.opening_row_label}
                        onInput={(v) => setS("totals", "opening_row_label", v)}
                        label="Opening balance row label"
                      />
                    </div>
                  </label>
                  <p class="text-xs text-muted-foreground">The table's first row, with the opening balance, when the Balance column prints.</p>
                </Show>
                <Show when={isStandard()}>
                  <div class="border-t border-border pt-3">
                    <Check
                      label="Totals by category"
                      hint="A table of rows, money in, out and net for each category, before the transactions"
                      value={s.show_summary}
                      onChange={(v) => setS("show_summary", v)}
                    />
                  </div>
                </Show>
              </div>
            </Section>

            <Section title="Logo and letterhead">
              <label class="flex flex-col gap-1">
                <span class="text-xs text-muted-foreground">Logo (an image in Documents)</span>
                <Show when={canPick} fallback={<span class="text-sm">{logoName() ?? <span class="text-muted-foreground">None</span>}</span>}>
                  <select
                    class={fieldClass}
                    value={s.logo_document_id ?? ""}
                    onChange={(e) => setS("logo_document_id", e.currentTarget.value ? Number(e.currentTarget.value) : null)}
                  >
                    <option value="">No logo</option>
                    <For each={images() ?? []}>{(d) => <option value={d.id}>{place(d)}</option>}</For>
                  </select>
                  <Show when={docs.error}>
                    <span class="text-xs text-red-500">Couldn't list Documents.</span>
                  </Show>
                  <Show when={images()?.length === 0}>
                    <span class="text-xs text-muted-foreground">No images in Documents yet: upload a PNG, JPEG or SVG there first.</span>
                  </Show>
                </Show>
              </label>

              <label class="flex flex-col gap-1">
                <span class="text-xs text-muted-foreground">Letterhead (a PDF in Documents, drawn behind the pages)</span>
                <Show when={canPick} fallback={<span class="text-sm">{letterheadName() ?? <span class="text-muted-foreground">None</span>}</span>}>
                  <select
                    class={fieldClass}
                    value={s.letterhead_document_id ?? ""}
                    onChange={(e) => setS("letterhead_document_id", e.currentTarget.value ? Number(e.currentTarget.value) : null)}
                  >
                    <option value="">No letterhead</option>
                    <For each={pdfs() ?? []}>{(d) => <option value={d.id}>{place(d)}</option>}</For>
                  </select>
                  <Show when={pdfs()?.length === 0}>
                    <span class="text-xs text-muted-foreground">No PDFs in Documents yet: upload your letterhead there first.</span>
                  </Show>
                </Show>
              </label>
              <Show when={s.letterhead_document_id !== null}>
                <span class="text-xs text-muted-foreground">
                  Its first page goes behind the statement's first page. Make it the same paper size, and set the margins so the statement
                  clears it.
                </span>
                <div class="flex flex-wrap gap-x-4 gap-y-1 text-sm">
                  <label class="flex items-center gap-2">
                    <input
                      type="radio"
                      name="letterhead-pages"
                      class="h-4 w-4"
                      checked={s.letterhead_every_page}
                      disabled={!editable}
                      onChange={() => setS("letterhead_every_page", true)}
                    />
                    Every page <span class="text-xs text-muted-foreground">(its 2nd page, if it has one, on the rest)</span>
                  </label>
                  <label class="flex items-center gap-2">
                    <input
                      type="radio"
                      name="letterhead-pages"
                      class="h-4 w-4"
                      checked={!s.letterhead_every_page}
                      disabled={!editable}
                      onChange={() => setS("letterhead_every_page", false)}
                    />
                    First page only
                  </label>
                </div>
              </Show>
            </Section>

            <Section title="Page and formats">
              <div class="grid gap-4 sm:grid-cols-2">
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Paper</span>
                  <select
                    class={fieldClass}
                    value={s.paper}
                    disabled={!editable}
                    onChange={(e) => setS("paper", e.currentTarget.value as TemplateSettings["paper"])}
                  >
                    <option value="a4">A4</option>
                    <option value="us-letter">US Letter</option>
                  </select>
                </label>
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Dates</span>
                  <select
                    class={fieldClass}
                    value={s.date_style}
                    disabled={!editable}
                    onChange={(e) => setS("date_style", e.currentTarget.value as TemplateSettings["date_style"])}
                  >
                    <option value="short">4 Oct 2026</option>
                    <option value="padded">04 Oct 2026</option>
                  </select>
                </label>
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Top margin (mm)</span>
                  <input
                    type="number"
                    min="5"
                    max="150"
                    step="1"
                    class={fieldClass}
                    value={s.margin_top_mm}
                    disabled={!editable}
                    onInput={(e) => setS("margin_top_mm", e.currentTarget.valueAsNumber)}
                  />
                </label>
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Bottom margin (mm, the footer sits in it)</span>
                  <input
                    type="number"
                    min="5"
                    max="150"
                    step="1"
                    class={fieldClass}
                    value={s.margin_bottom_mm}
                    disabled={!editable}
                    onInput={(e) => setS("margin_bottom_mm", e.currentTarget.valueAsNumber)}
                  />
                </label>
                <label class="flex flex-col gap-1">
                  <span class="text-xs text-muted-foreground">Currency symbol (balances and totals)</span>
                  <input
                    class={fieldClass}
                    value={s.currency_symbol}
                    maxLength={8}
                    placeholder="e.g. R"
                    disabled={!editable}
                    onInput={(e) => setS("currency_symbol", e.currentTarget.value)}
                  />
                </label>
              </div>
              <p class="text-xs text-muted-foreground">
                Every statement also ends with "Prepared by {org.name} from imported bank data" and the date, and marks rows added by hand.
              </p>
            </Section>
          </div>

          <aside class="flex flex-col gap-2 self-start lg:sticky lg:top-16">
            <h2 class="text-sm font-semibold">Variables</h2>
            <p class="text-xs text-muted-foreground">
              Click one to put it where the cursor is in a text field. Type <code>{"{{"}</code> for a brace.
            </p>
            <ul class="flex flex-col gap-1">
              <For each={variables()}>
                {(v) => (
                  <li>
                    <button
                      type="button"
                      disabled={!editable}
                      // Keep the text field's cursor: don't take focus on mousedown.
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => insert(v.name)}
                      title={v.label}
                      class="w-full rounded-md px-2 py-1 text-left hover:bg-accent disabled:cursor-default disabled:hover:bg-transparent"
                    >
                      <code class="text-xs">{`{${v.name}}`}</code>
                      <span class="block text-xs text-muted-foreground">{v.label}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </aside>
        </div>
      </Show>
    </div>
  );
}

/** /portal/<org>/statements/templates/<id>. Viewing needs `read-statements`; changes `write-statements`. */
export function TemplateEditorPage() {
  const org = useOrg();
  const params = useParams({ from: "/portal/$org/statements/templates/$template" });
  return (
    <Allowed when={can(org, "read-statements")}>
      <StatementsShell>
        {(state) => (
          <Show when={Number(params().template)} keyed>
            {(id) => <Editor id={id} state={state} />}
          </Show>
        )}
      </StatementsShell>
    </Allowed>
  );
}
