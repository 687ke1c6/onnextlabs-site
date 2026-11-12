import { createContext, createSignal, createUniqueId, type JSX, onCleanup, Show, useContext } from "solid-js";
import { cn } from "~/lib/utils";

type MenuApi = { close: () => void };
const MenuContext = createContext<MenuApi>();

/** Inside a <Menu>: `close()` after an item acts (links and buttons call it on click). */
export function useMenu(): MenuApi {
  const menu = useContext(MenuContext);
  if (!menu) throw new Error("useMenu() called outside <Menu>");
  return menu;
}

/** Classes for a menu row; put `role="menuitem"` on the element too. */
export const menuItemClass =
  "flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:text-accent-foreground";

export function MenuLabel(props: { children: JSX.Element }) {
  return <p class="px-2.5 pb-1 pt-2 text-xs font-medium text-muted-foreground">{props.children}</p>;
}

export function MenuSeparator() {
  return <div role="separator" class="my-1 h-px bg-border" />;
}

/**
 * Button that opens a small menu above it (for the bottom of the rail). Closes on Escape,
 * outside click, or when an item calls `useMenu().close()`. Arrow keys move between items.
 */
export function Menu(props: {
  /** Accessible name for the trigger button. */
  label: string;
  trigger: JSX.Element;
  triggerClass?: string;
  title?: string;
  /** Extra classes for the panel (width, placement). */
  class?: string;
  children: JSX.Element;
}) {
  const id = createUniqueId();
  const [open, setOpen] = createSignal(false);
  let root!: HTMLDivElement;
  let button!: HTMLButtonElement;
  let panel: HTMLDivElement | undefined;

  const items = () => Array.from(panel?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []);

  const close = (refocus = false) => {
    setOpen(false);
    if (refocus) button.focus();
  };
  const show = () => {
    setOpen(true);
    queueMicrotask(() => items()[0]?.focus());
  };

  const onPointerDown = (e: PointerEvent) => {
    if (open() && !root.contains(e.target as Node)) close();
  };
  document.addEventListener("pointerdown", onPointerDown);
  onCleanup(() => document.removeEventListener("pointerdown", onPointerDown));

  const onKeyDown = (e: KeyboardEvent) => {
    if (!open()) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close(true);
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
    e.preventDefault();
    const list = items();
    if (!list.length) return;
    const at = list.indexOf(document.activeElement as HTMLElement);
    const next =
      e.key === "Home" ? 0
      : e.key === "End" ? list.length - 1
      : e.key === "ArrowDown" ? (at + 1) % list.length
      : (at - 1 + list.length) % list.length;
    list[next].focus();
  };

  return (
    <div ref={root} class="relative" onKeyDown={onKeyDown} onFocusOut={(e) => {
      // Tabbing out of the menu closes it.
      if (open() && !root.contains(e.relatedTarget as Node | null)) close();
    }}>
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open()}
        aria-controls={open() ? id : undefined}
        aria-label={props.label}
        title={props.title}
        onClick={() => (open() ? close() : show())}
        class={props.triggerClass}
      >
        {props.trigger}
      </button>
      <Show when={open()}>
        <MenuContext.Provider value={{ close: () => close() }}>
          <div
            ref={panel}
            id={id}
            role="menu"
            aria-label={props.label}
            class={cn(
              "absolute bottom-full left-0 z-30 mb-2 w-60 rounded-lg border border-border bg-background p-1 text-foreground shadow-xl",
              props.class,
            )}
          >
            {props.children}
          </div>
        </MenuContext.Provider>
      </Show>
    </div>
  );
}
