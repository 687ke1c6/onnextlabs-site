import { createEffect, createResource, createSignal, For, type JSX, on, Show } from "solid-js";
import Ban from "lucide-solid/icons/ban";
import Link2Off from "lucide-solid/icons/link-2-off";
import Loader2 from "lucide-solid/icons/loader-circle";
import Trash from "lucide-solid/icons/trash";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import type { DeleteImpact } from "~/lib/deleteImpact";


const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);

/**
 * Confirms a delete after showing what it would remove: rows deleted with it, rows kept with
 * their link cleared, and anything blocking it. With `confirmText`, the user types it first.
 */
export function DeleteDialog(props: {
  open: boolean;
  title: string;
  /** Extra explanation above the list. */
  note?: JSX.Element;
  load: () => Promise<DeleteImpact>;
  /** Must be typed exactly to enable Delete (e.g. the organisation's name). */
  confirmText?: string;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  let dialog!: HTMLDialogElement;
  const [impact, { refetch }] = createResource(() => (props.open ? true : undefined), () => props.load());
  const [typed, setTyped] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) {
          setTyped("");
          setError(null);
          void refetch();
          dialog.showModal();
        } else if (dialog.open) {
          dialog.close();
        }
      },
    ),
  );

  const confirmed = () => !props.confirmText || typed().trim() === props.confirmText;
  const ready = () => impact.state === "ready" && !impact()!.blocked;
  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      await props.onConfirm();
    } catch (e) {
      setError(errorText(e, "Couldn't delete it."));
    } finally {
      setBusy(false);
    }
  };
  const group = (action: "delete" | "unlink" | "block") => (impact.state === "ready" ? impact()!.effects.filter((e) => e.action === action) : []);

  return (
    <dialog
      ref={dialog}
      onClose={() => props.open && props.onClose()}
      onCancel={(e) => busy() && e.preventDefault()}
      class="m-auto w-[calc(100%-2rem)] max-w-lg rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="delete-title"
    >
      <div class="flex flex-col gap-4 p-6">
        <h2 id="delete-title" class="text-lg font-semibold tracking-tight">{props.title}</h2>
        <Show when={props.note}>
          <div class="text-sm text-muted-foreground">{props.note}</div>
        </Show>

        <Show when={impact.loading}>
          <p class="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 size={14} class="animate-spin" /> Working out what this deletes…
          </p>
        </Show>
        <Show when={impact.error}>
          <p class="text-sm text-red-500">{errorText(impact.error, "Couldn't work out what this deletes.")}</p>
        </Show>
        <Show when={impact.state === "ready"}>
          <div class="flex flex-col gap-3 text-sm">
            <ImpactGroup icon={<Trash size={14} />} title="Deleted" items={group("delete")} class="text-red-600 dark:text-red-400" />
            <ImpactGroup icon={<Link2Off size={14} />} title="Kept, with their link to it removed" items={group("unlink")} />
            <ImpactGroup icon={<Ban size={14} />} title="Blocking the delete" items={group("block")} class="text-red-600 dark:text-red-400" />
          </div>
        </Show>

        <Show when={props.confirmText && ready()}>
          <label class="flex flex-col gap-1.5 text-sm">
            <span class="text-muted-foreground">
              Type <span class="font-medium text-foreground">{props.confirmText}</span> to confirm
            </span>
            <Input value={typed()} onInput={(e) => setTyped(e.currentTarget.value)} disabled={busy()} autocomplete="off" />
          </label>
        </Show>
        <Show when={error()}>
          <p role="alert" class="text-sm text-red-500">{error()}</p>
        </Show>

        <div class="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => props.onClose()} disabled={busy()}>
            Cancel
          </Button>
          <Button
            onClick={() => void go()}
            disabled={busy() || !ready() || !confirmed()}
            class="bg-red-600 text-white hover:bg-red-600/90"
          >
            <Show when={busy()} fallback={<Trash size={16} />}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            Delete
          </Button>
        </div>
      </div>
    </dialog>
  );
}

function ImpactGroup(props: {
  icon: JSX.Element;
  title: string;
  items: DeleteImpact["effects"];
  class?: string;
}) {
  return (
    <Show when={props.items.length > 0}>
      <div>
        <p class={`flex items-center gap-1.5 text-xs font-medium ${props.class ?? "text-muted-foreground"}`}>
          {props.icon} {props.title}
        </p>
        <ul class="mt-1 flex flex-col gap-0.5 pl-5">
          <For each={props.items}>
            {(e) => (
              <li class="flex gap-2">
                <span class="w-12 shrink-0 text-right tabular-nums font-medium">{e.count.toLocaleString()}</span>
                <span class="text-muted-foreground">{e.label}</span>
              </li>
            )}
          </For>
        </ul>
      </div>
    </Show>
  );
}
