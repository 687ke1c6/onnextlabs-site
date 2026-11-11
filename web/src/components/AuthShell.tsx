import type { JSX } from "solid-js";
import { Link } from "@tanstack/solid-router";

/** Centered card on the same grid/aurora backdrop as the home page. */
export function AuthShell(props: { children: JSX.Element }) {
  return (
    <main class="relative isolate flex min-h-screen flex-col items-center justify-center overflow-hidden px-4 py-16">
      <div
        class="absolute inset-0 -z-10 opacity-30 [background-image:linear-gradient(var(--border)_1px,transparent_1px),linear-gradient(90deg,var(--border)_1px,transparent_1px)] [background-size:56px_56px] [mask-image:radial-gradient(ellipse_at_center,black,transparent_70%)]"
        aria-hidden="true"
      />
      <div class="absolute -left-32 -top-32 -z-10 h-[28rem] w-[28rem] rounded-full bg-violet-600/20 blur-3xl" aria-hidden="true" />
      <div class="absolute -bottom-40 -right-32 -z-10 h-[30rem] w-[30rem] rounded-full bg-cyan-500/15 blur-3xl" aria-hidden="true" />
      <div class="w-full max-w-sm rounded-2xl border border-border bg-secondary/30 p-8 backdrop-blur">{props.children}</div>
      <Link to="/" class="mt-6 text-sm text-muted-foreground hover:text-foreground">← Back to home</Link>
    </main>
  );
}
