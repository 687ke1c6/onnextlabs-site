import { createResource, createSignal, For, Show } from "solid-js";
import Search from "lucide-solid/icons/search";
import { Badge } from "~/components/ui/badge";
import { UserChip } from "~/components/UserChip";
import { isAdmin } from "~/lib/auth";
import { type AdminUser, fetchAdminUsers, setAdmin } from "~/lib/users";
import { cn, settled } from "~/lib/utils";
import { Allowed, PageHeader, usePortal } from "~/routes/portal/Layout";

const errorText = (e: unknown) => (e instanceof Error ? e.message : "Something went wrong.");
const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });

/** On/off switch for the admin flag. */
function AdminSwitch(props: { on: boolean; disabled: boolean; title?: string; label: string; onToggle: () => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.on}
      aria-label={props.label}
      title={props.title}
      disabled={props.disabled}
      onClick={props.onToggle}
      class={cn(
        "relative inline-flex h-6 w-11 shrink-0 items-center rounded-full border border-border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
        props.on ? "bg-foreground" : "bg-secondary",
      )}
    >
      <span
        class={cn(
          "inline-block h-4 w-4 rounded-full bg-background shadow transition-transform",
          props.on ? "translate-x-6" : "translate-x-1",
        )}
      />
    </button>
  );
}

function UserList() {
  const portal = usePortal();
  const [users, { mutate }] = createResource(fetchAdminUsers);
  const [query, setQuery] = createSignal("");
  const [busy, setBusy] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const label = (u: AdminUser) => u.name ?? u.email ?? `User ${u.id}`;
  const shown = () => {
    const q = query().trim().toLowerCase();
    const list = settled(users) ?? [];
    return q
      ? list.filter((u) => (u.name ?? "").toLowerCase().includes(q) || (u.email ?? "").toLowerCase().includes(q))
      : list;
  };
  const adminCount = () => (settled(users) ?? []).filter((u) => u.is_admin || u.env_admin).length;

  const toggle = async (u: AdminUser) => {
    const next = !u.is_admin;
    const question = next
      ? `Make ${label(u)} an admin? They'll be able to do everything, in every organisation.`
      : `Remove admin access from ${label(u)}?`;
    if (!confirm(question)) return;
    setBusy(u.id);
    setError(null);
    try {
      const updated = await setAdmin(u.id, next);
      mutate((list) => list?.map((x) => (x.id === updated.id ? updated : x)));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(null);
    }
  };

  /** Why the switch can't be used, if it can't. */
  const lockedReason = (u: AdminUser) => {
    if (u.env_admin) return "Admin through ADMIN_EMAILS. Remove the address there to change this.";
    if (u.id === portal.user.id && u.is_admin) return "You can't remove your own admin access.";
    return undefined;
  };

  return (
    <>
      <PageHeader
        title="Users"
        description="Everyone who has signed in. Admins can do everything, in every organisation."
      />

      <div class="mt-6 flex flex-wrap items-center gap-3">
        <div class="flex min-w-0 flex-1 items-center rounded-md border border-input bg-background/60 focus-within:ring-2 focus-within:ring-ring sm:max-w-sm">
          <Search size={16} class="ml-3 shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            type="search"
            aria-label="Search people"
            placeholder="Search by name or email"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            class="h-10 min-w-0 flex-1 bg-transparent px-3 text-sm placeholder:text-muted-foreground focus:outline-none"
          />
        </div>
        <Show when={settled(users)}>
          {(list) => (
            <p class="text-sm text-muted-foreground">
              {list().length} {list().length === 1 ? "person" : "people"}, {adminCount()}{" "}
              {adminCount() === 1 ? "admin" : "admins"}
            </p>
          )}
        </Show>
      </div>

      <Show when={users.error}>
        <p class="mt-4 text-sm text-red-500">{errorText(users.error)}</p>
      </Show>
      <Show when={error()}>{(msg) => <p class="mt-4 text-sm text-red-500">{msg()}</p>}</Show>

      <Show when={settled(users)} fallback={<div class="mt-4 h-40 animate-pulse rounded-xl bg-secondary/60" />}>
        <section class="mt-4 overflow-hidden rounded-xl border border-border">
          <table class="w-full table-fixed text-sm">
            <thead class="border-b border-border bg-secondary/30 text-left text-xs text-muted-foreground">
              <tr>
                <th class="px-4 py-2.5 font-medium">Person</th>
                <th class="hidden px-4 py-2.5 font-medium md:table-cell">Organisations</th>
                <th class="hidden w-32 px-4 py-2.5 font-medium sm:table-cell">Last sign-in</th>
                <th class="w-24 px-4 py-2.5 text-right font-medium">Admin</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border">
              <For
                each={shown()}
                fallback={
                  <tr>
                    <td colspan="4" class="px-4 py-8 text-center text-muted-foreground">
                      No one matches that.
                    </td>
                  </tr>
                }
              >
                {(u) => (
                  <tr>
                    <td class="px-4 py-3">
                      <div class="flex min-w-0 items-center gap-2">
                        <div class="min-w-0 flex-1">
                          <UserChip user={u} />
                        </div>
                        <Show when={u.id === portal.user.id}>
                          <Badge class="shrink-0 px-2 py-0.5">You</Badge>
                        </Show>
                      </div>
                    </td>
                    <td class="hidden px-4 py-3 md:table-cell">
                      <div class="flex flex-wrap gap-1.5">
                        <For each={u.orgs} fallback={<span class="text-xs text-muted-foreground">None</span>}>
                          {(m) => (
                            <Badge class="px-2 py-0.5 font-normal">
                              {m.name}
                              <span class="ml-1 text-muted-foreground">· {m.role}</span>
                            </Badge>
                          )}
                        </For>
                      </div>
                    </td>
                    <td class="hidden px-4 py-3 text-muted-foreground sm:table-cell">
                      {dateFormat.format(new Date(u.last_login_at))}
                    </td>
                    <td class="px-4 py-3">
                      <div class="flex flex-col items-end gap-1">
                        <AdminSwitch
                          on={u.is_admin || u.env_admin}
                          disabled={busy() !== null || !!lockedReason(u)}
                          title={lockedReason(u)}
                          label={`${label(u)} is an admin`}
                          onToggle={() => toggle(u)}
                        />
                        <Show when={u.env_admin}>
                          <span class="text-[10px] uppercase tracking-wide text-muted-foreground">env</span>
                        </Show>
                      </div>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </section>
      </Show>
    </>
  );
}

/** /portal/users: every account; admins can make others admins. Admins only. */
export function UsersPage() {
  const portal = usePortal();
  return (
    <Allowed when={isAdmin(portal.user)}>
      <UserList />
    </Allowed>
  );
}
