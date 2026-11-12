import { createSignal, createUniqueId, For, Show } from "solid-js";
import Search from "lucide-solid/icons/search";
import { UserAvatar } from "~/components/UserChip";
import type { UserListItem } from "~/lib/users";
import { cn } from "~/lib/utils";

const MAX_RESULTS = 8;

/**
 * Search-as-you-type picker over existing accounts (by name or email). Accessible combobox:
 * arrow keys move, Enter picks, Escape closes.
 */
export function UserPicker(props: {
  users: UserListItem[];
  /** Accounts to leave out, e.g. people who are already members. */
  exclude?: ReadonlySet<number>;
  onPick: (user: UserListItem) => void;
  disabled?: boolean;
  label: string;
  placeholder?: string;
}) {
  const id = createUniqueId();
  const [query, setQuery] = createSignal("");
  const [open, setOpen] = createSignal(false);
  const [active, setActive] = createSignal(0);

  const available = () => props.users.filter((u) => !props.exclude?.has(u.id));
  const matches = () => {
    const q = query().trim().toLowerCase();
    const list = q
      ? available().filter((u) => (u.name ?? "").toLowerCase().includes(q) || (u.email ?? "").toLowerCase().includes(q))
      : available();
    return list.slice(0, MAX_RESULTS);
  };

  const pick = (user: UserListItem | undefined) => {
    if (!user) return;
    props.onPick(user);
    setQuery("");
    setActive(0);
    setOpen(false);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const n = matches().length;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setOpen(true);
      if (n) setActive((i) => (i + 1) % n);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setOpen(true);
      if (n) setActive((i) => (i - 1 + n) % n);
    } else if (e.key === "Enter") {
      if (open() && n) {
        e.preventDefault();
        pick(matches()[active()]);
      }
    } else if (e.key === "Escape") {
      setOpen(false);
    }
  };

  const optionId = (i: number) => `${id}-opt-${i}`;

  return (
    <div class="relative">
      <div class="flex items-center rounded-md border border-input bg-background/60 focus-within:ring-2 focus-within:ring-ring">
        <Search size={16} class="ml-3 shrink-0 text-muted-foreground" aria-hidden="true" />
        <input
          type="text"
          role="combobox"
          aria-label={props.label}
          aria-autocomplete="list"
          aria-expanded={open()}
          aria-controls={`${id}-list`}
          aria-activedescendant={open() && matches().length ? optionId(active()) : undefined}
          value={query()}
          placeholder={props.placeholder ?? "Search people by name or email"}
          disabled={props.disabled}
          autocomplete="off"
          onInput={(e) => {
            setQuery(e.currentTarget.value);
            setActive(0);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={onKeyDown}
          class="h-10 min-w-0 flex-1 bg-transparent px-3 text-sm placeholder:text-muted-foreground focus:outline-none disabled:opacity-50"
        />
      </div>

      <Show when={open()}>
        <ul
          id={`${id}-list`}
          role="listbox"
          class="absolute z-20 mt-1 max-h-80 w-full overflow-y-auto rounded-lg border border-border bg-background p-1 shadow-lg"
        >
          <For
            each={matches()}
            fallback={
              <li class="px-3 py-2 text-sm text-muted-foreground">
                {available().length === 0 ? "Everyone is already added." : "No one matches that."}
              </li>
            }
          >
            {(user, i) => (
              <li
                id={optionId(i())}
                role="option"
                aria-selected={i() === active()}
                // mousedown, not click: fires before the input's blur closes the list.
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(user);
                }}
                onMouseEnter={() => setActive(i())}
                class={cn(
                  "flex cursor-pointer items-center gap-3 rounded-md px-2 py-1.5",
                  i() === active() && "bg-accent text-accent-foreground",
                )}
              >
                <UserAvatar user={user} class="h-7 w-7 text-xs" />
                <div class="min-w-0">
                  <p class="truncate text-sm">{user.name ?? user.email ?? `User ${user.id}`}</p>
                  <Show when={user.name && user.email}>
                    <p class="truncate text-xs text-muted-foreground">{user.email}</p>
                  </Show>
                </div>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
