import { clsx, type ClassValue } from "clsx";
import type { Resource } from "solid-js";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * A resource's value without triggering Suspense: undefined until the first load finishes,
 * then the latest value (kept while refetching). Calling `resource()` directly while it loads
 * makes the router's <Suspense> blank the whole page; use this in portal UI instead.
 */
export function settled<T>(resource: Resource<T>): T | undefined {
  return resource.state === "ready" || resource.state === "refreshing" ? resource.latest : undefined;
}

/**
 * A `<select>`'s value, re-applied whenever its `options` change. A select given a value before
 * the matching <option> exists (e.g. while the options load) shows its first option instead,
 * and Solid doesn't set the value again when the options arrive. Reading `options` here makes
 * it do so: `value={selectValue(row.type ?? "", types())}`.
 */
export function selectValue<T>(value: T, _options: unknown): T {
  return value;
}
