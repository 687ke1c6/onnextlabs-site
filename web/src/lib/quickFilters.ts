import { json, send } from "~/lib/orgs";
import { amountValue, type OrgTransaction } from "~/lib/statements";

/**
 * Quick filters: named filters shown as toggles above the statement grid, per organisation.
 * Toggled filters in one group combine with OR, groups with AND. A yearly group offers one filter
 * per year (e.g. tax years from 1 March), worked out from the rows.
 */
export type FilterGroup = {
  name: string;
  kind: "filters" | "yearly";
  /** Yearly: the month (1-12) and day (1-28) each year starts on. */
  start_month: number | null;
  start_day: number | null;
  /** `filters` groups only. */
  filters: QuickFilter[];
};

/** Matches a row when every condition it has holds. */
export type QuickFilter = {
  name: string;
  /** YYYY-MM-DD, inclusive; compared with the effective date (or the date). */
  date_from: string | null;
  date_to: string | null;
  /** Any of these types. */
  types: string[];
  /** The description contains any of these, ignoring case. */
  contains: string[];
  /** Inclusive. */
  min_amount: number | null;
  max_amount: number | null;
};

const url = (org: string) => `/api/orgs/${encodeURIComponent(org)}/statement-filters`;

export const fetchFilterGroups = async (org: string) => json<FilterGroup[]>(await fetch(url(org)), "Couldn't load the quick filters.");

/** Replaces every group; returns them as saved (names trimmed, types in the list's spelling). */
export const saveFilterGroups = async (org: string, groups: FilterGroup[]) =>
  json<FilterGroup[]>(await send("PUT", url(org), { groups }), "Couldn't save the quick filters.");

/** The date a row counts towards: its effective date, or its date. */
export const rowDate = (r: Pick<OrgTransaction, "date" | "effective_date">) => r.effective_date ?? r.date;

export function matches(f: QuickFilter, r: OrgTransaction): boolean {
  const date = rowDate(r);
  if (f.date_from && date < f.date_from) return false;
  if (f.date_to && date > f.date_to) return false;
  if (f.types.length) {
    const t = (r.type ?? "").toLowerCase();
    if (!f.types.some((x) => x.toLowerCase() === t)) return false;
  }
  if (f.contains.length) {
    const d = r.description.toLowerCase();
    if (!f.contains.some((c) => d.includes(c.toLowerCase()))) return false;
  }
  if (f.min_amount !== null || f.max_amount !== null) {
    const a = amountValue(r.amount);
    if (Number.isNaN(a)) return false;
    if (f.min_amount !== null && a < f.min_amount) return false;
    if (f.max_amount !== null && a > f.max_amount) return false;
  }
  return true;
}

/** A toggle above the grid: a group's own filter, or one year of a yearly group. */
export type Chip = {
  /** Stable across visits (group and filter names, or the year's first day). */
  id: string;
  label: string;
  title: string;
  filter: QuickFilter;
};

const pad = (n: number) => String(n).padStart(2, "0");
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const longDate = (iso: string) => `${Number(iso.slice(8, 10))} ${MONTHS[Number(iso.slice(5, 7)) - 1]} ${iso.slice(0, 4)}`;

/** The day before `iso` (YYYY-MM-DD). */
function dayBefore(iso: string) {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() - 1);
  return d.toISOString().slice(0, 10);
}

/** One chip per year with rows, newest first: "2024/25" (or "2024" for years from 1 January). */
export function yearlyChips(group: FilterGroup, rows: OrgTransaction[]): Chip[] {
  const md = `${pad(group.start_month ?? 1)}-${pad(group.start_day ?? 1)}`;
  const years = new Set<number>();
  for (const r of rows) {
    const date = rowDate(r);
    const y = Number(date.slice(0, 4));
    years.add(date.slice(5) >= md ? y : y - 1);
  }
  return [...years]
    .sort((a, b) => b - a)
    .map((y) => {
      const from = `${y}-${md}`;
      const to = dayBefore(`${y + 1}-${md}`);
      return {
        id: `${group.name}\u001f${from}`,
        label: md === "01-01" ? String(y) : `${y}/${pad((y + 1) % 100)}`,
        title: `${longDate(from)} – ${longDate(to)}`,
        filter: { name: String(y), date_from: from, date_to: to, types: [], contains: [], min_amount: null, max_amount: null },
      };
    });
}

export function groupChips(group: FilterGroup, rows: OrgTransaction[]): Chip[] {
  if (group.kind === "yearly") return yearlyChips(group, rows);
  return group.filters.map((f) => ({ id: `${group.name}\u001f${f.name}`, label: f.name, title: describe(f), filter: f }));
}

/** A filter's conditions in words, for tooltips. */
export function describe(f: QuickFilter): string {
  const parts: string[] = [];
  if (f.date_from && f.date_to) parts.push(`${longDate(f.date_from)} – ${longDate(f.date_to)}`);
  else if (f.date_from) parts.push(`from ${longDate(f.date_from)}`);
  else if (f.date_to) parts.push(`to ${longDate(f.date_to)}`);
  if (f.types.length) parts.push(`type ${f.types.join(" or ")}`);
  if (f.contains.length) parts.push(`description has ${f.contains.map((c) => `"${c}"`).join(" or ")}`);
  if (f.min_amount !== null && f.max_amount !== null) parts.push(`amount ${f.min_amount} to ${f.max_amount}`);
  else if (f.min_amount !== null) parts.push(`amount ≥ ${f.min_amount}`);
  else if (f.max_amount !== null) parts.push(`amount ≤ ${f.max_amount}`);
  return parts.join(", ");
}

/**
 * The rows the toggled chips let through: within a group any toggled chip (OR), across groups
 * all of them (AND). Groups with nothing toggled don't filter.
 */
export function applyChips(rows: OrgTransaction[], groups: { chips: Chip[] }[], active: Set<string>): OrgTransaction[] {
  const filters = groups.map((g) => g.chips.filter((c) => active.has(c.id)).map((c) => c.filter)).filter((fs) => fs.length > 0);
  if (!filters.length) return rows;
  return rows.filter((r) => filters.every((fs) => fs.some((f) => matches(f, r))));
}
