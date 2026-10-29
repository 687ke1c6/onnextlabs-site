import { createSignal, For, onCleanup } from "solid-js";

// TODO: set the real launch date.
const LAUNCH = new Date("2027-01-01T00:00:00Z").getTime();

export function Countdown() {
  const [now, setNow] = createSignal(Date.now());
  const id = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(id));

  const parts = () => {
    const s = Math.max(0, Math.floor((LAUNCH - now()) / 1000));
    return [
      ["Days", Math.floor(s / 86400)],
      ["Hours", Math.floor(s / 3600) % 24],
      ["Min", Math.floor(s / 60) % 60],
      ["Sec", s % 60],
    ] as const;
  };

  return (
    <div class="flex gap-3 sm:gap-4">
      <For each={parts()}>
        {([label, value]) => (
          <div class="flex w-16 flex-col items-center rounded-xl border border-border bg-white/5 py-3 backdrop-blur sm:w-20">
            <span class="font-mono text-2xl tabular-nums sm:text-3xl">{String(value).padStart(2, "0")}</span>
            <span class="mt-1 text-[10px] uppercase tracking-widest text-muted-foreground">{label}</span>
          </div>
        )}
      </For>
    </div>
  );
}
