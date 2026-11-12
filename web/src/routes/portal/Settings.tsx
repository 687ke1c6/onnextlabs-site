import { For } from "solid-js";
import Moon from "lucide-solid/icons/moon";
import Sun from "lucide-solid/icons/sun";
import { setTheme, theme, type Theme } from "~/lib/theme";
import { cn } from "~/lib/utils";
import { PageHeader } from "~/routes/portal/Layout";

const THEMES = [
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
] as const satisfies readonly { value: Theme; label: string; icon: unknown }[];

export function SettingsPage() {
  return (
    <>
      <PageHeader title="Settings" description="Your portal preferences." />

      <section class="mt-8 rounded-xl border border-border">
        <div class="flex flex-wrap items-center justify-between gap-4 p-4">
          <div>
            <h2 id="appearance-label" class="text-sm font-medium">Appearance</h2>
            <p class="mt-0.5 text-sm text-muted-foreground">Colour theme for the portal. Saved in this browser.</p>
          </div>
          <div
            role="radiogroup"
            aria-labelledby="appearance-label"
            class="inline-flex rounded-lg border border-border bg-secondary/40 p-1"
          >
            <For each={THEMES}>
              {(t) => (
                <button
                  type="button"
                  role="radio"
                  aria-checked={theme() === t.value}
                  onClick={() => setTheme(t.value)}
                  class={cn(
                    "inline-flex h-8 items-center gap-2 rounded-md px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    theme() === t.value
                      ? "bg-background text-foreground shadow-sm"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <t.icon size={16} /> {t.label}
                </button>
              )}
            </For>
          </div>
        </div>
      </section>
    </>
  );
}
