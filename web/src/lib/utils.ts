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
