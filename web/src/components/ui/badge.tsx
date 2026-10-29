import type { JSX } from "solid-js";
import { splitProps } from "solid-js";
import { cn } from "~/lib/utils";

export function Badge(props: JSX.HTMLAttributes<HTMLDivElement>) {
  const [local, rest] = splitProps(props, ["class"]);
  return (
    <div
      class={cn(
        "inline-flex items-center rounded-full border border-border bg-secondary/40 px-3 py-1 text-xs font-medium text-secondary-foreground backdrop-blur",
        local.class,
      )}
      {...rest}
    />
  );
}
