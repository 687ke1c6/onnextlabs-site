import { createSignal } from "solid-js";

// The account picked on the Statements pages, per org: in memory (so both tabs follow it at
// once) and in this browser (so it survives reloads).
const key = (org: string) => `statements-account:${org}`;
const [picked, setPicked] = createSignal<Record<string, string>>({});

/** The account last picked for this org, if any. Reactive. */
export function pickedAccount(org: string): string | undefined {
  const inMemory = picked()[org];
  if (inMemory !== undefined) return inMemory;
  try {
    return localStorage.getItem(key(org)) ?? undefined;
  } catch {
    return undefined;
  }
}

export function pickAccount(org: string, account: string) {
  setPicked((all) => ({ ...all, [org]: account }));
  try {
    localStorage.setItem(key(org), account);
  } catch {
    // storage blocked: remembered until the page reloads
  }
}
