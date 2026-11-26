import { createSignal } from "solid-js";
import { amountValue } from "~/lib/statements";

/** Which figure the statement grid's status bar (and the selected-rows panel) shows for Amount. */
export type Stat = "sum" | "average" | "min" | "max";
export const STATS: { id: Stat; label: string }[] = [
  { id: "sum", label: "Sum" },
  { id: "average", label: "Average" },
  { id: "min", label: "Min" },
  { id: "max", label: "Max" },
];
const STAT_KEY = "statements-amount-stat";

function initialStat(): Stat {
  try {
    const saved = localStorage.getItem(STAT_KEY);
    if (STATS.some((s) => s.id === saved)) return saved as Stat;
  } catch {
    // storage blocked: use the default
  }
  return "sum";
}
// Shared by every Transactions grid, like the other view settings.
const [stat, setStat] = createSignal<Stat>(initialStat());
export { stat };
export const chooseStat = (s: Stat) => {
  setStat(s);
  try {
    localStorage.setItem(STAT_KEY, s);
  } catch {
    // just don't remember
  }
};
export const statLabel = () => STATS.find((s) => s.id === stat())!.label;

export type AmountFigures = { sum: number; in: number; out: number; n: number; min: number; max: number };

/** Amount's figures for `rows`, per currency (rows whose amount can't be read are left out). */
export function amountFigures(rows: { amount: string; currency: string }[]): [string, AmountFigures][] {
  const by = new Map<string, AmountFigures>();
  for (const t of rows) {
    const v = amountValue(t.amount);
    if (!Number.isFinite(v)) continue;
    const f = by.get(t.currency) ?? { sum: 0, in: 0, out: 0, n: 0, min: Infinity, max: -Infinity };
    f.sum += v;
    if (v >= 0) f.in += v;
    else f.out += v;
    f.n += 1;
    f.min = Math.min(f.min, v);
    f.max = Math.max(f.max, v);
    by.set(t.currency, f);
  }
  return [...by.entries()];
}

/** The chosen figure (`stat()`) from `f`. */
export const statValue = (f: AmountFigures) =>
  stat() === "sum" ? f.sum : stat() === "average" ? f.sum / f.n : stat() === "min" ? f.min : f.max;

/** "−1,234.50": two decimals, thousands separated, a proper minus sign. */
export const formatFigure = (n: number) =>
  `${n < 0 ? "−" : ""}${Math.abs(n).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
