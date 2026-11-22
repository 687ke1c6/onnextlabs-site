import { createSignal } from "solid-js";

/** A boolean preference remembered in this browser. */
function remembered(key: string, initial: boolean) {
  let start = initial;
  try {
    const saved = localStorage.getItem(key);
    if (saved !== null) start = saved === "1";
  } catch {
    // storage blocked: use the default
  }
  const [get, set] = createSignal(start);
  const update = (v: boolean) => {
    set(v);
    try {
      localStorage.setItem(key, v ? "1" : "0");
    } catch {
      // just don't remember
    }
  };
  return [get, update] as const;
}

/**
 * Grids: how tightly rows are packed. `compact`: one line per row, all the same height.
 * `condensed`: that, with smaller text and less padding.
 */
export type Density = "normal" | "compact" | "condensed";
export const DENSITIES: Density[] = ["normal", "compact", "condensed"];

/**
 * One grid's density, remembered in this browser under `<storageKey>-density`. Until that grid has
 * its own, it starts from the old setting shared by every grid (or the older on/off one).
 */
export function rememberedDensity(storageKey: string) {
  const key = `${storageKey}-density`;
  let start: Density = "normal";
  try {
    const saved = localStorage.getItem(key) ?? localStorage.getItem("grid-density");
    if (DENSITIES.includes(saved as Density)) start = saved as Density;
    else if (localStorage.getItem("grid-compact") === "1") start = "compact";
  } catch {
    // storage blocked: use the default
  }
  const [get, set] = createSignal<Density>(start);
  const update = (v: Density) => {
    set(v);
    try {
      localStorage.setItem(key, v);
    } catch {
      // just don't remember
    }
  };
  return [get, update] as const;
}

/** Grids: let the page use the full window width. */
export const [wideGrids, setWideGrids] = remembered("grid-wide", false);

// Wide only applies while a grid is on the page, so other portal pages keep their width.
const [gridsShown, setGridsShown] = createSignal(0);
export const gridMounted = () => setGridsShown((n) => n + 1);
export const gridUnmounted = () => setGridsShown((n) => Math.max(0, n - 1));

/**
 * Whether a grid is on the page: the portal's content area is then exactly the window's height
 * and the grid fills what's left, so only its rows scroll.
 */
export const pageHasGrid = () => gridsShown() > 0;

// Only grids with a status bar offer Wide, so only they widen the page.
const [wideGridsShown, setWideGridsShown] = createSignal(0);
export const wideGridMounted = () => setWideGridsShown((n) => n + 1);
export const wideGridUnmounted = () => setWideGridsShown((n) => Math.max(0, n - 1));

/** Whether the portal's content area should drop its max width. */
export const pageIsWide = () => wideGrids() && wideGridsShown() > 0;
