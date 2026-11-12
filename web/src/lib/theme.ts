import { createSignal } from "solid-js";

export type Theme = "dark" | "light";

const KEY = "portal-theme";

function load(): Theme {
  try {
    return localStorage.getItem(KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

const [theme, setThemeSignal] = createSignal<Theme>(load());

/** Portal colour theme, remembered per browser. Dark is the default. */
export { theme };

export function setTheme(next: Theme) {
  setThemeSignal(next);
  try {
    localStorage.setItem(KEY, next);
  } catch {
    // Storage blocked (private mode etc.): the choice just lasts for this visit.
  }
}
