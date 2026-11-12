import { Show } from "solid-js";
import type { User } from "~/lib/auth";
import { cn } from "~/lib/utils";

/** Enough of an account to show it. */
type Person = Pick<User, "name" | "email" | "picture">;

export function UserAvatar(props: { user: Person; class?: string }) {
  const label = () => props.user.name ?? props.user.email ?? "Signed in";
  return (
    <Show
      when={props.user.picture}
      fallback={
        <div class={cn("flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-secondary text-sm", props.class)}>
          {label()[0]?.toUpperCase()}
        </div>
      }
    >
      {(src) => <img src={src()} alt="" referrerpolicy="no-referrer" class={cn("h-9 w-9 shrink-0 rounded-full", props.class)} />}
    </Show>
  );
}

export function UserChip(props: { user: Person }) {
  const label = () => props.user.name ?? props.user.email ?? "Signed in";
  return (
    <div class="flex min-w-0 items-center gap-3">
      <UserAvatar user={props.user} />
      <div class="min-w-0 text-left">
        <p class="truncate text-sm font-medium">{label()}</p>
        <Show when={props.user.name && props.user.email}>
          <p class="truncate text-xs text-muted-foreground">{props.user.email}</p>
        </Show>
      </div>
    </div>
  );
}
