import { createSignal } from "solid-js";

// The account picked on the Statements pages, per org: in memory (so every tab follows it at
// once) and in this browser (so it survives reloads).
const key = (org: string) => `statements-account-id:${org}`;
const [picked, setPicked] = createSignal<Record<string, number>>({});

/** The id of the account last picked for this org, if any. Reactive. */
export function pickedAccount(org: string): number | undefined {
  const inMemory = picked()[org];
  if (inMemory !== undefined) return inMemory;
  try {
    const saved = Number(localStorage.getItem(key(org)));
    return Number.isInteger(saved) && saved > 0 ? saved : undefined;
  } catch {
    return undefined;
  }
}

export function pickAccount(org: string, id: number) {
  setPicked((all) => ({ ...all, [org]: id }));
  try {
    localStorage.setItem(key(org), String(id));
  } catch {
    // storage blocked: remembered until the page reloads
  }
}
