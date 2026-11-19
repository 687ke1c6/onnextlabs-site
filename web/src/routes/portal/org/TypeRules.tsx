import { createEffect, createResource, createSignal, For, Index, type JSX, on, Show } from "solid-js";
import ArrowDown from "lucide-solid/icons/arrow-down";
import ArrowUp from "lucide-solid/icons/arrow-up";
import Loader2 from "lucide-solid/icons/loader-circle";
import Pencil from "lucide-solid/icons/pencil";
import Plus from "lucide-solid/icons/plus";
import Trash from "lucide-solid/icons/trash";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { can } from "~/lib/orgs";
import {
  createType,
  deleteType,
  fetchRules,
  fetchTypes,
  type FormatInfo,
  renameType,
  type Rule,
  saveRules,
  testRules,
  type TypeInfo,
} from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed, PageHeader } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { StatementsTabs } from "~/routes/portal/org/StatementParts";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);

/** A rule as edited: texts one per line, amounts as typed. */
type Draft = { type: string; contains: string; min: string; max: string; format: string };

const toDraft = (r: Rule): Draft => ({
  type: r.type,
  contains: r.contains.join("\n"),
  min: r.min_amount?.toString() ?? "",
  max: r.max_amount?.toString() ?? "",
  format: r.formats[0] ?? "",
});

const amount = (s: string) => (s.trim() === "" ? null : Number(s.trim()));

// Lines are kept as typed (a trailing space can matter), apart from dropping blank ones.
const toRule = (d: Draft): Rule => ({
  type: d.type,
  contains: d.contains.split("\n").filter((l) => l.trim() !== ""),
  min_amount: amount(d.min),
  max_amount: amount(d.max),
  formats: d.format ? [d.format] : [],
});

const blank = (): Draft => ({ type: "", contains: "", min: "", max: "", format: "" });

const FIELD = "h-9";

function RuleCard(props: {
  index: number;
  draft: Draft;
  count: number;
  formats: FormatInfo[];
  /** Type names on the org's list. */
  types: string[];
  editable: boolean;
  matched: boolean;
  onChange: (d: Draft) => void;
  onMove: (by: number) => void;
  onRemove: () => void;
}) {
  const set = (patch: Partial<Draft>) => props.onChange({ ...props.draft, ...patch });
  const id = (f: string) => `rule-${props.index}-${f}`;
  return (
    <li
      class={cn(
        "rounded-xl border p-4 transition-colors",
        props.matched ? "border-emerald-500/60 bg-emerald-500/5" : "border-border",
      )}
    >
      <div class="flex items-center justify-between gap-2">
        <span class="text-xs font-medium text-muted-foreground">
          Rule {props.index + 1}
          <Show when={props.matched}>
            <span class="ml-2 text-emerald-600 dark:text-emerald-400">matches the test</span>
          </Show>
        </span>
        <Show when={props.editable}>
          <div class="flex gap-1">
            <IconButton label="Move up" disabled={props.index === 0} onClick={() => props.onMove(-1)}>
              <ArrowUp size={16} />
            </IconButton>
            <IconButton label="Move down" disabled={props.index === props.count - 1} onClick={() => props.onMove(1)}>
              <ArrowDown size={16} />
            </IconButton>
            <IconButton label="Delete rule" onClick={props.onRemove} danger>
              <Trash size={16} />
            </IconButton>
          </div>
        </Show>
      </div>
      <div class="mt-3 grid gap-3 md:grid-cols-[minmax(0,12rem)_minmax(0,1fr)]">
        <div class="flex flex-col gap-1.5">
          <label for={id("type")} class="text-xs text-muted-foreground">Type</label>
          <select
            id={id("type")}
            value={props.draft.type}
            onChange={(e) => set({ type: e.currentTarget.value })}
            disabled={!props.editable}
            class="h-9 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
          >
            <option value="" disabled>
              Choose a type
            </option>
            {/* A name that isn't on the list any more still shows, so it can be changed. */}
            <Show when={props.draft.type && !props.types.includes(props.draft.type)}>
              <option value={props.draft.type}>{props.draft.type} (not on the list)</option>
            </Show>
            <For each={props.types}>{(t) => <option value={t}>{t}</option>}</For>
          </select>
        </div>
        <div class="flex flex-col gap-1.5">
          <label for={id("contains")} class="text-xs text-muted-foreground">
            Description contains any of (one per line, ignoring case)
          </label>
          <textarea
            id={id("contains")}
            rows={Math.min(6, Math.max(2, props.draft.contains.split("\n").length))}
            value={props.draft.contains}
            onInput={(e) => set({ contains: e.currentTarget.value })}
            disabled={!props.editable}
            class="w-full rounded-md border border-input bg-background/60 px-3 py-2 font-mono text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
            placeholder={"MONTHLY FEE\nSERVICE FEE"}
          />
        </div>
      </div>
      <div class="mt-3 grid grid-cols-2 gap-3 md:grid-cols-3">
        <div class="flex flex-col gap-1.5">
          <label for={id("min")} class="text-xs text-muted-foreground">Amount at least</label>
          <Input
            id={id("min")}
            class={FIELD}
            inputMode="decimal"
            value={props.draft.min}
            onInput={(e) => set({ min: e.currentTarget.value })}
            disabled={!props.editable}
            placeholder="any"
          />
        </div>
        <div class="flex flex-col gap-1.5">
          <label for={id("max")} class="text-xs text-muted-foreground">Amount below</label>
          <Input
            id={id("max")}
            class={FIELD}
            inputMode="decimal"
            value={props.draft.max}
            onInput={(e) => set({ max: e.currentTarget.value })}
            disabled={!props.editable}
            placeholder="any"
          />
        </div>
        <div class="col-span-2 flex flex-col gap-1.5 md:col-span-1">
          <label for={id("format")} class="text-xs text-muted-foreground">Statement format</label>
          <select
            id={id("format")}
            value={props.draft.format}
            onChange={(e) => set({ format: e.currentTarget.value })}
            disabled={!props.editable}
            class="h-9 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
          >
            <option value="">Any format</option>
            <For each={props.formats}>{(f) => <option value={f.id}>{f.name}</option>}</For>
          </select>
        </div>
      </div>
    </li>
  );
}

function IconButton(props: { label: string; onClick: () => void; disabled?: boolean; danger?: boolean; children: JSX.Element }) {
  return (
    <button
      type="button"
      onClick={props.onClick}
      disabled={props.disabled}
      aria-label={props.label}
      title={props.label}
      class={cn(
        "flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-30",
        props.danger ? "hover:text-red-500" : "hover:text-accent-foreground",
      )}
    >
      {props.children}
    </button>
  );
}

/** Try a description and amount against the rules as currently edited. */
function TestBox(props: { rules: () => Rule[]; onMatch: (index: number | null) => void }) {
  const org = useOrg();
  const [description, setDescription] = createSignal("");
  const [value, setValue] = createSignal("");
  const [result, setResult] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const run = async (e: SubmitEvent) => {
    e.preventDefault();
    setError(null);
    try {
      const r = await testRules(org.slug, props.rules(), description(), value() || "0");
      props.onMatch(r.rule);
      setResult(r.rule === null ? "No rule matches: the type is left blank." : `Rule ${r.rule + 1} matches: type ${r.type}.`);
    } catch (err) {
      props.onMatch(null);
      setResult(null);
      setError(errorText(err, "Couldn't test the rules."));
    }
  };

  return (
    <form onSubmit={run} class="rounded-xl border border-border bg-secondary/20 p-4">
      <h2 class="text-sm font-medium">Test the rules</h2>
      <p class="mt-1 text-xs text-muted-foreground">Uses the rules as they are on this page, saved or not.</p>
      <div class="mt-3 flex flex-wrap gap-2">
        <Input
          class="h-9 min-w-0 flex-[3_1_16rem]"
          value={description()}
          onInput={(e) => setDescription(e.currentTarget.value)}
          placeholder="Description, e.g. MONTHLY MANAGEMENT FEE"
          aria-label="Description"
        />
        <Input
          class="h-9 min-w-0 flex-[1_1_7rem]"
          inputMode="decimal"
          value={value()}
          onInput={(e) => setValue(e.currentTarget.value)}
          placeholder="Amount, e.g. -57.50"
          aria-label="Amount"
        />
        <Button type="submit" size="sm" variant="outline" disabled={!description().trim()}>
          Test
        </Button>
      </div>
      <Show when={result()}>
        <p class="mt-3 text-sm" role="status">{result()}</p>
      </Show>
      <Show when={error()}>
        <p class="mt-3 text-sm text-red-500" role="alert">{error()}</p>
      </Show>
    </form>
  );
}

/** The org's types: add, rename (everywhere), delete (only when unused). */
function TypesList(props: {
  types: TypeInfo[] | undefined;
  error: unknown;
  editable: boolean;
  onChange: (types: TypeInfo[]) => void;
  onRenamed: (from: string, to: string) => void;
}) {
  const org = useOrg();
  const [adding, setAdding] = createSignal("");
  const [editing, setEditing] = createSignal<number | null>(null);
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const run = async (fn: () => Promise<TypeInfo[]>, fallback: string) => {
    setBusy(true);
    setError(null);
    try {
      props.onChange(await fn());
      return true;
    } catch (e) {
      setError(errorText(e, fallback));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const add = async (e: SubmitEvent) => {
    e.preventDefault();
    if (adding().trim() && (await run(() => createType(org.slug, adding()), "Couldn't add the type."))) setAdding("");
  };
  const rename = async (e: SubmitEvent, t: TypeInfo) => {
    e.preventDefault();
    const to = name().trim();
    if (!to || to === t.name) return setEditing(null);
    if (await run(() => renameType(org.slug, t.id, to), "Couldn't rename the type.")) {
      props.onRenamed(t.name, to);
      setEditing(null);
    }
  };
  const remove = (t: TypeInfo) => run(() => deleteType(org.slug, t.id), "Couldn't delete the type.");
  const uses = (t: TypeInfo) =>
    [t.rows && `${t.rows} ${t.rows === 1 ? "row" : "rows"}`, t.rules && `${t.rules} ${t.rules === 1 ? "rule" : "rules"}`]
      .filter(Boolean)
      .join(", ") || "unused";

  return (
    <section class="rounded-xl border border-border p-4">
      <h2 class="text-sm font-medium">Types</h2>
      <p class="mt-1 text-xs text-muted-foreground">
        The types rows can have. Rules and the Transactions grid choose from this list. Renaming a type renames it on every row and rule.
      </p>
      <Show when={props.error}>
        <p class="mt-3 text-sm text-red-500">{errorText(props.error, "Couldn't load the types.")}</p>
      </Show>
      <Show when={props.types}>
        {(list) => (
          <ul class="mt-3 flex flex-wrap gap-2">
            <For each={list()} fallback={<li class="text-sm text-muted-foreground">No types yet.</li>}>
              {(t) => (
                <li class="flex items-center gap-1 rounded-full border border-border bg-secondary/30 py-0.5 pl-3 pr-1 text-sm">
                  <Show
                    when={editing() === t.id}
                    fallback={
                      <>
                        <span class="font-medium">{t.name}</span>
                        <span class="text-xs text-muted-foreground">· {uses(t)}</span>
                        <Show when={props.editable}>
                          <IconButton
                            label={`Rename ${t.name}`}
                            onClick={() => {
                              setName(t.name);
                              setEditing(t.id);
                            }}
                          >
                            <Pencil size={14} />
                          </IconButton>
                          <IconButton
                            label={t.rows || t.rules ? `${t.name} is in use, so it can't be deleted` : `Delete ${t.name}`}
                            onClick={() => void remove(t)}
                            disabled={busy() || t.rows + t.rules > 0}
                            danger
                          >
                            <Trash size={14} />
                          </IconButton>
                        </Show>
                      </>
                    }
                  >
                    <form onSubmit={(e) => void rename(e, t)} class="flex items-center gap-1">
                      <Input
                        class="h-7 w-40"
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        onKeyDown={(e) => e.key === "Escape" && setEditing(null)}
                        maxLength={100}
                        aria-label={`New name for ${t.name}`}
                        ref={(el) => queueMicrotask(() => el.select())}
                        disabled={busy()}
                      />
                      <Button type="submit" size="sm" class="h-7" disabled={busy()}>
                        Save
                      </Button>
                      <Button type="button" variant="ghost" size="sm" class="h-7" onClick={() => setEditing(null)}>
                        Cancel
                      </Button>
                    </form>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        )}
      </Show>
      <Show when={props.editable}>
        <form onSubmit={(e) => void add(e)} class="mt-3 flex max-w-sm gap-2">
          <Input
            class="h-9"
            value={adding()}
            onInput={(e) => setAdding(e.currentTarget.value)}
            placeholder="New type, e.g. FEES"
            maxLength={100}
            aria-label="New type"
            disabled={busy()}
          />
          <Button type="submit" size="sm" variant="outline" disabled={busy() || !adding().trim()}>
            <Plus size={16} /> Add
          </Button>
        </form>
      </Show>
      <Show when={error()}>
        <p role="alert" class="mt-2 text-sm text-red-500">{error()}</p>
      </Show>
    </section>
  );
}

function RulesEditor() {
  const org = useOrg();
  const editable = () => can(org, "write-statements");
  const [loaded, { mutate }] = createResource(() => org.slug, fetchRules);
  const [drafts, setDrafts] = createSignal<Draft[]>([]);
  const [saved, setSaved] = createSignal<string>("[]");
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [matched, setMatched] = createSignal<number | null>(null);
  const [types, { mutate: setTypes }] = createResource(() => org.slug, fetchTypes);
  const typeNames = () => (settled(types) ?? []).map((t) => t.name);
  // A rename also renames the type in rules (saved and unsaved), as the server does for saved ones.
  const renamed = (from: string, to: string) => {
    const swap = (list: Draft[]) => list.map((d) => (d.type === from ? { ...d, type: to } : d));
    setDrafts(swap);
    setSaved((json) => JSON.stringify(swap(JSON.parse(json) as Draft[])));
  };

  createEffect(
    on(
      () => settled(loaded),
      (data) => {
        if (!data) return;
        setDrafts(data.rules.map(toDraft));
        setSaved(JSON.stringify(data.rules.map(toDraft)));
      },
    ),
  );

  const dirty = () => JSON.stringify(drafts()) !== saved();
  const rules = () => drafts().map(toRule);
  const missingType = () => drafts().findIndex((d) => !d.type);
  const invalidAmount = () =>
    drafts().findIndex((d) => [d.min, d.max].some((v) => v.trim() !== "" && !Number.isFinite(Number(v.trim()))));

  const update = (i: number, d: Draft) => {
    setNotice(null);
    setMatched(null);
    setDrafts((all) => all.map((x, j) => (j === i ? d : x)));
  };
  const move = (i: number, by: number) => {
    setMatched(null);
    setDrafts((all) => {
      const next = [...all];
      const [r] = next.splice(i, 1);
      next.splice(i + by, 0, r);
      return next;
    });
  };
  const remove = (i: number) => {
    setMatched(null);
    setDrafts((all) => all.filter((_, j) => j !== i));
  };

  const save = async () => {
    setError(null);
    setNotice(null);
    const untyped = missingType();
    if (untyped >= 0) {
      setError(`Rule ${untyped + 1}: choose a type.`);
      return;
    }
    const bad = invalidAmount();
    if (bad >= 0) {
      setError(`Rule ${bad + 1}: amounts must be numbers.`);
      return;
    }
    setSaving(true);
    try {
      const data = await saveRules(org.slug, rules());
      mutate(data);
      setNotice("Saved. New and re-saved statements use these rules.");
    } catch (e) {
      setError(errorText(e, "Couldn't save the type rules."));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div class="mt-6 flex flex-col gap-6">
      <TypesList
        types={settled(types)}
        error={types.error}
        editable={editable()}
        onChange={(list) => setTypes(list)}
        onRenamed={renamed}
      />

      <p class="max-w-3xl text-sm text-muted-foreground">
        Rules set each row's <span class="font-medium text-foreground">type</span> when a statement is saved. They're tried in order and
        the first match wins. A row no rule matches keeps the type it already had. To apply changed rules to rows already saved, save
        their statement again.
      </p>

      <Show when={loaded.error}>
        <p class="text-sm text-red-500">{errorText(loaded.error, "Couldn't load the type rules.")}</p>
      </Show>

      <Show when={settled(loaded)}>
        {(data) => (
          <>
            <TestBox rules={rules} onMatch={setMatched} />

            <Show
              when={drafts().length > 0}
              fallback={
                <p class="rounded-xl border border-dashed border-border px-6 py-10 text-center text-sm text-muted-foreground">
                  No rules yet: every type is left blank.
                </p>
              }
            >
              <ol class="flex flex-col gap-3">
                <Index each={drafts()}>
                  {(d, i) => (
                    <RuleCard
                      index={i}
                      draft={d()}
                      count={drafts().length}
                      formats={data().formats}
                      types={typeNames()}
                      editable={editable()}
                      matched={matched() === i}
                      onChange={(nd) => update(i, nd)}
                      onMove={(by) => move(i, by)}
                      onRemove={() => remove(i)}
                    />
                  )}
                </Index>
              </ol>
            </Show>

            <Show when={editable()}>
              <div class="sticky bottom-0 -mx-1 flex flex-wrap items-center gap-2 border-t border-border bg-background/95 px-1 py-3 backdrop-blur">
                <Button variant="outline" size="sm" onClick={() => setDrafts((all) => [...all, blank()])}>
                  <Plus size={16} /> Add rule
                </Button>
                <div class="flex-1" />
                <Show when={error()}>
                  <p role="alert" class="text-sm text-red-500">{error()}</p>
                </Show>
                <Show when={notice() && !dirty()}>
                  <p role="status" class="text-sm text-muted-foreground">{notice()}</p>
                </Show>
                <Show when={dirty()}>
                  <Button variant="ghost" size="sm" onClick={() => setDrafts(JSON.parse(saved()))} disabled={saving()}>
                    Discard changes
                  </Button>
                </Show>
                <Button size="sm" onClick={save} disabled={!dirty() || saving()}>
                  <Show when={saving()}>
                    <Loader2 size={16} class="animate-spin" />
                  </Show>
                  Save rules
                </Button>
              </div>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}

/** /portal/<org>/statements/rules. Viewing needs `read-statements`; editing `write-statements`. */
export function TypeRulesPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <PageHeader title="Statements" description={`Bank statements for ${org.name}.`} />
      <StatementsTabs />
      <RulesEditor />
    </Allowed>
  );
}
