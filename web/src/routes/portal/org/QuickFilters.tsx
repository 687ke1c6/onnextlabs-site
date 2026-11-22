import { createEffect, createResource, createSignal, For, Show, untrack } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import ArrowDown from "lucide-solid/icons/arrow-down";
import ArrowUp from "lucide-solid/icons/arrow-up";
import CalendarRange from "lucide-solid/icons/calendar-range";
import Loader2 from "lucide-solid/icons/loader-circle";
import Plus from "lucide-solid/icons/plus";
import X from "lucide-solid/icons/x";
import { Button } from "~/components/ui/button";
import { can } from "~/lib/orgs";
import { type FilterGroup, fetchFilterGroups, type QuickFilter, saveFilterGroups } from "~/lib/quickFilters";
import { fetchTypes } from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { StatementsShell } from "~/routes/portal/org/StatementsShell";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);
const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** A filter as edited: amounts and texts as typed. */
type DraftFilter = {
  key: number;
  name: string;
  date_from: string;
  date_to: string;
  types: string[];
  /** One per line. */
  contains: string;
  min: string;
  max: string;
};
type DraftGroup = {
  key: number;
  name: string;
  kind: FilterGroup["kind"];
  start_month: number;
  start_day: number;
  filters: DraftFilter[];
};

let nextKey = 0;
const blankFilter = (): DraftFilter => ({ key: ++nextKey, name: "", date_from: "", date_to: "", types: [], contains: "", min: "", max: "" });
const toDraft = (g: FilterGroup): DraftGroup => ({
  key: ++nextKey,
  name: g.name,
  kind: g.kind,
  start_month: g.start_month ?? 3,
  start_day: g.start_day ?? 1,
  filters: g.filters.map((f) => ({
    key: ++nextKey,
    name: f.name,
    date_from: f.date_from ?? "",
    date_to: f.date_to ?? "",
    types: [...f.types],
    contains: f.contains.join("\n"),
    min: f.min_amount === null ? "" : String(f.min_amount),
    max: f.max_amount === null ? "" : String(f.max_amount),
  })),
});
const amount = (s: string) => (s.trim() === "" ? null : Number(s.replace(/,/g, "")));
const toGroup = (d: DraftGroup): FilterGroup => ({
  name: d.name,
  kind: d.kind,
  start_month: d.kind === "yearly" ? d.start_month : null,
  start_day: d.kind === "yearly" ? d.start_day : null,
  filters:
    d.kind === "yearly"
      ? []
      : d.filters.map(
          (f): QuickFilter => ({
            name: f.name,
            date_from: f.date_from || null,
            date_to: f.date_to || null,
            types: f.types,
            contains: f.contains.split("\n").filter((l) => l.trim() !== ""),
            min_amount: amount(f.min),
            max_amount: amount(f.max),
          }),
        ),
});

const fieldClass =
  "h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60";

/** Moves item `i` of `list` by `by` places. */
const moved = <T,>(list: T[], i: number, by: number) => {
  const j = i + by;
  if (j < 0 || j >= list.length) return list;
  const next = [...list];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
};

function MoveButtons(props: { index: number; count: number; disabled: boolean; label: string; onMove: (by: number) => void }) {
  const btn =
    "flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-30";
  return (
    <>
      <button type="button" class={btn} disabled={props.disabled || props.index === 0} onClick={() => props.onMove(-1)} aria-label={`Move ${props.label} up`}>
        <ArrowUp size={14} />
      </button>
      <button
        type="button"
        class={btn}
        disabled={props.disabled || props.index === props.count - 1}
        onClick={() => props.onMove(1)}
        aria-label={`Move ${props.label} down`}
      >
        <ArrowDown size={14} />
      </button>
    </>
  );
}

function FiltersEditor() {
  const org = useOrg();
  const editable = () => can(org, "write-statements");
  const [loaded] = createResource(() => org.slug, fetchFilterGroups);
  const [types] = createResource(() => org.slug, fetchTypes);
  const typeNames = () => (settled(types) ?? []).map((t) => t.name);

  // A store, so typing changes fields in place (inputs keep focus).
  const [groups, setGroups] = createStore<DraftGroup[]>([]);
  const [saved, setSaved] = createSignal("[]");
  const reset = (list: FilterGroup[]) => {
    setGroups(reconcile(list.map(toDraft)));
    setSaved(JSON.stringify(list.map(toDraft).map(toGroup)));
  };
  // The saved groups, once: later refreshes don't overwrite edits.
  const [ready, setReady] = createSignal(false);
  createEffect(() => {
    const l = settled(loaded);
    if (l && !untrack(ready)) {
      reset(l);
      setReady(true);
    }
  });
  // Read through the store (not `unwrap`), so every field is tracked and edits re-check this.
  const current = () => JSON.stringify(groups.map(toGroup));
  const dirty = () => current() !== saved();

  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);

  const save = async () => {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      reset(await saveFilterGroups(org.slug, groups.map(toGroup)));
      setNotice("Saved.");
    } catch (e) {
      setError(errorText(e, "Couldn't save the quick filters."));
    } finally {
      setSaving(false);
    }
  };

  const addGroup = (g: Partial<DraftGroup> = {}) =>
    setGroups(groups.length, { key: ++nextKey, name: "", kind: "filters", start_month: 3, start_day: 1, filters: [blankFilter()], ...g });

  return (
    <div class="mt-6 flex flex-col gap-4">
      <p class="max-w-3xl text-sm text-muted-foreground">
        Quick filters appear as toggles above the statement grid, for every account. Toggled filters in the same group show rows matching
        any of them; toggled filters in different groups show rows matching all the groups. Dates are compared with a row's effective
        date, or its date when it has none.
      </p>

      <Show when={loaded.error}>
        <p class="text-sm text-red-500">{errorText(loaded.error, "Couldn't load the quick filters.")}</p>
      </Show>
      <Show when={!ready() && !loaded.error}>
        <div class="h-32 animate-pulse rounded-xl bg-secondary/40" />
      </Show>

      <Show when={ready()}>
        <Show
          when={groups.length > 0}
          fallback={
            <div class="rounded-xl border border-dashed border-border px-6 py-10 text-center">
              <p class="text-sm text-muted-foreground">No quick filters yet.</p>
              <Show when={editable()}>
                <div class="mt-4 flex flex-wrap justify-center gap-2">
                  <Button variant="outline" size="sm" onClick={() => addGroup({ name: "Tax year", kind: "yearly", start_month: 3, start_day: 1, filters: [] })}>
                    <CalendarRange size={16} /> Tax years (from 1 March)
                  </Button>
                  <Button variant="outline" size="sm" onClick={() => addGroup()}>
                    <Plus size={16} /> A group of filters
                  </Button>
                </div>
              </Show>
            </div>
          }
        >
          <ol class="flex flex-col gap-4">
            <For each={groups}>
              {(g, gi) => (
                <li class="rounded-xl border border-border p-4">
                  <div class="flex flex-wrap items-end gap-3">
                    <label class="flex min-w-48 flex-1 flex-col gap-1">
                      <span class="text-xs text-muted-foreground">Group</span>
                      <input
                        class={fieldClass}
                        placeholder="e.g. Tax year, Household"
                        value={g.name}
                        disabled={!editable()}
                        onInput={(e) => setGroups(gi(), "name", e.currentTarget.value)}
                      />
                    </label>
                    <label class="flex w-48 flex-col gap-1">
                      <span class="text-xs text-muted-foreground">Kind</span>
                      <select
                        class={fieldClass}
                        value={g.kind}
                        disabled={!editable()}
                        onChange={(e) => {
                          const kind = e.currentTarget.value as DraftGroup["kind"];
                          setGroups(gi(), { kind, filters: kind === "filters" && g.filters.length === 0 ? [blankFilter()] : g.filters });
                        }}
                      >
                        <option value="filters">Filters</option>
                        <option value="yearly">Yearly periods</option>
                      </select>
                    </label>
                    <Show when={editable()}>
                      <div class="flex items-center">
                        <MoveButtons
                          index={gi()}
                          count={groups.length}
                          disabled={saving()}
                          label="group"
                          onMove={(by) => setGroups((all) => moved(all, gi(), by))}
                        />
                        <button
                          type="button"
                          onClick={() => setGroups((all) => all.filter((x) => x.key !== g.key))}
                          class="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-500"
                          aria-label="Remove group"
                          title="Remove group"
                        >
                          <X size={14} />
                        </button>
                      </div>
                    </Show>
                  </div>

                  <Show when={g.kind === "yearly"}>
                    <div class="mt-3 flex flex-wrap items-end gap-3">
                      <label class="flex w-24 flex-col gap-1">
                        <span class="text-xs text-muted-foreground">Starts on day</span>
                        <input
                          type="number"
                          min="1"
                          max="28"
                          class={fieldClass}
                          value={g.start_day}
                          disabled={!editable()}
                          onInput={(e) => setGroups(gi(), "start_day", Number(e.currentTarget.value))}
                        />
                      </label>
                      <label class="flex w-40 flex-col gap-1">
                        <span class="text-xs text-muted-foreground">of</span>
                        <select
                          class={fieldClass}
                          value={g.start_month}
                          disabled={!editable()}
                          onChange={(e) => setGroups(gi(), "start_month", Number(e.currentTarget.value))}
                        >
                          <For each={MONTHS}>{(m, i) => <option value={i() + 1}>{m}</option>}</For>
                        </select>
                      </label>
                      <p class="pb-2 text-sm text-muted-foreground">One toggle per year that has rows, e.g. {yearExample(g)}.</p>
                    </div>
                  </Show>

                  <Show when={g.kind === "filters"}>
                    <ol class="mt-3 flex flex-col gap-3">
                      <For each={g.filters}>
                        {(f, fi) => (
                          <FilterCard
                            filter={f}
                            index={fi()}
                            count={g.filters.length}
                            types={typeNames()}
                            editable={editable()}
                            disabled={saving()}
                            onChange={(patch) => setGroups(gi(), "filters", fi(), patch)}
                            onMove={(by) => setGroups(gi(), "filters", (all) => moved(all, fi(), by))}
                            onRemove={() => setGroups(gi(), "filters", (all) => all.filter((x) => x.key !== f.key))}
                          />
                        )}
                      </For>
                    </ol>
                    <Show when={editable()}>
                      <Button
                        variant="ghost"
                        size="sm"
                        class="mt-2"
                        onClick={() => setGroups(gi(), "filters", g.filters.length, blankFilter())}
                        disabled={g.filters.length >= 50}
                      >
                        <Plus size={14} /> Add filter
                      </Button>
                    </Show>
                  </Show>
                </li>
              )}
            </For>
          </ol>
        </Show>

        <Show when={editable()}>
          <div class="sticky bottom-0 -mx-1 flex flex-wrap items-center gap-2 border-t border-border bg-background/95 px-1 py-3 backdrop-blur">
            <Show when={groups.length > 0}>
              <Button variant="outline" size="sm" onClick={() => addGroup()} disabled={groups.length >= 20}>
                <Plus size={16} /> Add group
              </Button>
            </Show>
            <div class="flex-1" />
            <Show when={error()}>
              <p role="alert" class="text-sm text-red-500">{error()}</p>
            </Show>
            <Show when={notice() && !dirty()}>
              <p role="status" class="text-sm text-muted-foreground">{notice()}</p>
            </Show>
            <Show when={dirty()}>
              <Button variant="ghost" size="sm" onClick={() => reset(JSON.parse(saved()))} disabled={saving()}>
                Discard changes
              </Button>
            </Show>
            <Button size="sm" onClick={save} disabled={!dirty() || saving()}>
              <Show when={saving()}>
                <Loader2 size={16} class="animate-spin" />
              </Show>
              Save quick filters
            </Button>
          </div>
        </Show>
      </Show>
    </div>
  );
}

/** e.g. "2024/25 (1 Mar 2024 – 28 Feb 2025)". */
function yearExample(g: DraftGroup) {
  const short = MONTHS.map((m) => m.slice(0, 3));
  const day = Math.min(Math.max(Math.trunc(g.start_day) || 1, 1), 28);
  const month = g.start_month;
  if (day === 1 && month === 1) return "2025 (1 Jan 2025 – 31 Dec 2025)";
  const end = new Date(Date.UTC(2025, month - 1, day - 1));
  return `2024/25 (${day} ${short[month - 1]} 2024 – ${end.getUTCDate()} ${short[end.getUTCMonth()]} ${end.getUTCFullYear()})`;
}

function FilterCard(props: {
  filter: DraftFilter;
  index: number;
  count: number;
  types: string[];
  editable: boolean;
  disabled: boolean;
  onChange: (patch: Partial<DraftFilter>) => void;
  onMove: (by: number) => void;
  onRemove: () => void;
}) {
  const f = () => props.filter;
  const ro = () => !props.editable;
  const badAmount = (s: string) => s.trim() !== "" && Number.isNaN(Number(s.replace(/,/g, "")));
  return (
    <li class="rounded-lg bg-secondary/20 p-3">
      <div class="flex items-end gap-2">
        <label class="flex min-w-0 flex-1 flex-col gap-1">
          <span class="text-xs text-muted-foreground">Filter</span>
          <input class={fieldClass} placeholder="e.g. Rent & utilities" value={f().name} disabled={ro()} onInput={(e) => props.onChange({ name: e.currentTarget.value })} />
        </label>
        <Show when={props.editable}>
          <div class="flex items-center pb-1">
            <MoveButtons index={props.index} count={props.count} disabled={props.disabled} label="filter" onMove={props.onMove} />
            <button
              type="button"
              onClick={props.onRemove}
              class="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-500"
              aria-label="Remove filter"
              title="Remove filter"
            >
              <X size={14} />
            </button>
          </div>
        </Show>
      </div>

      <p class="mt-3 text-xs text-muted-foreground">Rows matching every condition you fill in:</p>
      <div class="mt-1 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <label class="flex flex-col gap-1">
          <span class="text-xs text-muted-foreground">From (date)</span>
          <input type="date" class={fieldClass} value={f().date_from} disabled={ro()} onInput={(e) => props.onChange({ date_from: e.currentTarget.value })} />
        </label>
        <label class="flex flex-col gap-1">
          <span class="text-xs text-muted-foreground">To (date, inclusive)</span>
          <input type="date" class={fieldClass} value={f().date_to} disabled={ro()} onInput={(e) => props.onChange({ date_to: e.currentTarget.value })} />
        </label>
        <label class="flex flex-col gap-1">
          <span class="text-xs text-muted-foreground">Amount from</span>
          <input
            class={cn(fieldClass, "tabular-nums", badAmount(f().min) && "border-red-500")}
            inputmode="decimal"
            placeholder="e.g. -5000"
            value={f().min}
            disabled={ro()}
            onInput={(e) => props.onChange({ min: e.currentTarget.value })}
          />
        </label>
        <label class="flex flex-col gap-1">
          <span class="text-xs text-muted-foreground">Amount to</span>
          <input
            class={cn(fieldClass, "tabular-nums", badAmount(f().max) && "border-red-500")}
            inputmode="decimal"
            placeholder="e.g. 0"
            value={f().max}
            disabled={ro()}
            onInput={(e) => props.onChange({ max: e.currentTarget.value })}
          />
        </label>
      </div>

      <div class="mt-3 grid gap-3 sm:grid-cols-2">
        <div class="flex flex-col gap-1">
          <span class="text-xs text-muted-foreground">Type is any of</span>
          <div class="flex min-h-9 flex-wrap items-center gap-1.5 rounded-md border border-input bg-background px-2 py-1">
            <For each={f().types}>
              {(t) => (
                <span class="inline-flex items-center gap-1 rounded-full border border-border bg-secondary/40 px-2 py-0.5 text-xs font-medium">
                  {t}
                  <Show when={props.editable}>
                    <button
                      type="button"
                      onClick={() => props.onChange({ types: f().types.filter((x) => x !== t) })}
                      class="text-muted-foreground hover:text-red-500"
                      aria-label={`Remove ${t}`}
                    >
                      <X size={11} />
                    </button>
                  </Show>
                </span>
              )}
            </For>
            <Show when={props.editable}>
              <select
                class="h-7 min-w-0 flex-1 bg-transparent text-xs text-muted-foreground focus:outline-none"
                value=""
                aria-label="Add a type"
                onChange={(e) => {
                  const t = e.currentTarget.value;
                  e.currentTarget.value = "";
                  if (t && !f().types.includes(t)) props.onChange({ types: [...f().types, t] });
                }}
              >
                <option value="">{f().types.length ? "Add another…" : "Any type (add one…)"}</option>
                <For each={props.types.filter((t) => !f().types.includes(t))}>{(t) => <option value={t}>{t}</option>}</For>
              </select>
            </Show>
          </div>
        </div>
        <label class="flex flex-col gap-1">
          <span class="text-xs text-muted-foreground">Description contains any of (one per line, ignoring case)</span>
          <textarea
            class={cn(fieldClass, "h-auto py-1.5")}
            rows={Math.min(4, Math.max(1, f().contains.split("\n").length))}
            value={f().contains}
            disabled={ro()}
            onInput={(e) => props.onChange({ contains: e.currentTarget.value })}
          />
        </label>
      </div>
    </li>
  );
}

/** /portal/<org>/statements/filters. Viewing needs `read-statements`; editing `write-statements`. */
export function QuickFiltersPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <StatementsShell>{() => <FiltersEditor />}</StatementsShell>
    </Allowed>
  );
}
