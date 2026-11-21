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

/** Grids: one line per row, all the same height. */
export const [compactGrids, setCompactGrids] = remembered("grid-compact", false);

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

/** Whether the portal's content area should drop its max width. */
export const pageIsWide = () => wideGrids() && gridsShown() > 0;
