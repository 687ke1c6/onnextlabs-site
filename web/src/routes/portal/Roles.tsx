import { createResource, createSignal, For, Show } from "solid-js";
import KeyRound from "lucide-solid/icons/key-round";
import Pencil from "lucide-solid/icons/pencil";
import Plus from "lucide-solid/icons/plus";
import Trash from "lucide-solid/icons/trash";
import { Badge } from "~/components/ui/badge";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import {
  createRole,
  deleteRole,
  fetchRoles,
  type Permission,
  type PermissionInfo,
  type Role,
  type RoleInput,
  updateRole,
} from "~/lib/roles";
import { cn, settled } from "~/lib/utils";
import { Allowed, PageHeader, usePortal } from "~/routes/portal/Layout";

const errorText = (e: unknown) => (e instanceof Error ? e.message : "Something went wrong.");

const ICON_BUTTON =
  "flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40";

/** Create or edit form. `role` is undefined for a new role. */
function RoleEditor(props: {
  role?: Role;
  permissions: PermissionInfo[];
  onSave: (input: RoleInput) => Promise<void>;
  onCancel: () => void;
}) {
  const [name, setName] = createSignal(props.role?.name ?? "");
  const [description, setDescription] = createSignal(props.role?.description ?? "");
  const [chosen, setChosen] = createSignal<Set<Permission>>(new Set(props.role?.permissions ?? []));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const toggle = (key: Permission, on: boolean) => {
    const next = new Set(chosen());
    if (on) next.add(key);
    else next.delete(key);
    setChosen(next);
  };

  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    if (!name().trim() || busy()) return;
    setBusy(true);
    setError(null);
    try {
      // Keep the server's display order.
      const permissions = props.permissions.map((p) => p.key).filter((k) => chosen().has(k));
      await props.onSave({ name: name().trim(), description: description().trim(), permissions });
    } catch (err) {
      setError(errorText(err));
      setBusy(false);
    }
  };

  const id = props.role ? `role-${props.role.id}` : "role-new";

  return (
    <form onSubmit={submit} class="rounded-xl border border-ring/60 p-4">
      <h2 class="text-sm font-medium">{props.role ? `Edit ${props.role.name}` : "New role"}</h2>
      <div class="mt-3 grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
        <label class="flex flex-col gap-1.5 text-sm">
          <span class="text-muted-foreground">Name</span>
          <Input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Editor" maxLength={50} required />
        </label>
        <label class="flex flex-col gap-1.5 text-sm">
          <span class="text-muted-foreground">Description (optional)</span>
          <Input
            value={description()}
            onInput={(e) => setDescription(e.currentTarget.value)}
            placeholder="What this role is for"
            maxLength={200}
          />
        </label>
      </div>

      <fieldset class="mt-4">
        <legend class="text-sm text-muted-foreground">Permissions</legend>
        <div class="mt-2 grid gap-2 sm:grid-cols-2">
          <For each={props.permissions}>
            {(p) => (
              <label
                class={cn(
                  "flex cursor-pointer items-start gap-3 rounded-lg border p-3 transition-colors",
                  chosen().has(p.key) ? "border-ring bg-accent/40" : "border-border hover:bg-accent/20",
                )}
              >
                <input
                  type="checkbox"
                  class="mt-0.5 h-4 w-4 shrink-0 accent-current"
                  checked={chosen().has(p.key)}
                  onChange={(e) => toggle(p.key, e.currentTarget.checked)}
                  aria-describedby={`${id}-${p.key}`}
                />
                <span class="min-w-0">
                  <span class="block font-mono text-sm font-medium">{p.key}</span>
                  <span id={`${id}-${p.key}`} class="block text-xs text-muted-foreground">
                    {p.description}
                  </span>
                </span>
              </label>
            )}
          </For>
        </div>
      </fieldset>
      <Show when={chosen().has("write-docs") && !chosen().has("read-docs")}>
        <p class="mt-2 text-xs text-muted-foreground">
          Without <code class="font-mono">read-docs</code>, people with this role can upload but won't see the Documents page.
        </p>
      </Show>

      <Show when={error()}>{(msg) => <p class="mt-3 text-sm text-red-500">{msg()}</p>}</Show>
      <div class="mt-4 flex justify-end gap-2">
        <Button type="button" variant="outline" size="sm" onClick={props.onCancel}>
          Cancel
        </Button>
        <Button type="submit" size="sm" disabled={busy() || !name().trim()}>
          {busy() ? "Saving…" : props.role ? "Save" : "Create role"}
        </Button>
      </div>
    </form>
  );
}

function ManageRoles() {
  const portal = usePortal();
  const [data, { refetch }] = createResource(fetchRoles);
  /** "new", a role id being edited, or null. */
  const [editing, setEditing] = createSignal<"new" | number | null>(null);
  const [error, setError] = createSignal<string | null>(null);


  // Your own permissions may have changed, so the rail and pages refresh too.
  const saved = async () => {
    setEditing(null);
    await refetch();
    portal.refetchOrgs();
  };

  const remove = async (role: Role) => {
    if (!confirm(`Delete the ${role.name} role?`)) return;
    setError(null);
    try {
      await deleteRole(role.id);
      await refetch();
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <>
      <PageHeader
        title="Roles"
        description="A role is a set of permissions. Roles are shared by every organisation; members get one role in each organisation they belong to."
        actions={
          <Button size="sm" onClick={() => setEditing("new")} disabled={editing() === "new"}>
            <Plus size={16} /> New role
          </Button>
        }
      />

      <Show when={data.error}>
        <p class="mt-6 text-sm text-red-500">{errorText(data.error)}</p>
      </Show>
      <Show when={error()}>{(msg) => <p class="mt-6 text-sm text-red-500">{msg()}</p>}</Show>

      <Show when={settled(data)} fallback={<div class="mt-8 h-24 animate-pulse rounded-xl bg-secondary/60" />}>
        {(d) => (
          <>
            <Show when={editing() === "new"}>
              <div class="mt-8">
                <RoleEditor
                  permissions={d().permissions}
                  onSave={async (input) => {
                    await createRole(input);
                    await saved();
                  }}
                  onCancel={() => setEditing(null)}
                />
              </div>
            </Show>

            <Show
              when={d().roles.length > 0}
              fallback={<p class="mt-8 text-sm text-muted-foreground">No roles yet. Create one to start assigning members.</p>}
            >
              <ul class="mt-8 flex flex-col gap-3">
                <For each={d().roles}>
                  {(role) => (
                    <li>
                      <Show
                        when={editing() === role.id}
                        fallback={
                          <div class="flex items-start gap-3 rounded-xl border border-border p-4">
                            <KeyRound size={18} class="mt-0.5 shrink-0 text-muted-foreground" />
                            <div class="min-w-0 flex-1">
                              <div class="flex flex-wrap items-baseline gap-x-2">
                                <h2 class="text-sm font-medium">{role.name}</h2>
                                <span class="text-xs text-muted-foreground">
                                  {role.member_count === 0
                                    ? "Not in use"
                                    : `${role.member_count} ${role.member_count === 1 ? "membership" : "memberships"}`}
                                </span>
                              </div>
                              <Show when={role.description}>
                                <p class="mt-0.5 text-sm text-muted-foreground">{role.description}</p>
                              </Show>
                              <div class="mt-2 flex flex-wrap gap-1.5">
                                <For
                                  each={role.permissions}
                                  fallback={<span class="text-xs text-muted-foreground">No permissions</span>}
                                >
                                  {(p) => <Badge class="font-mono">{p}</Badge>}
                                </For>
                              </div>
                            </div>
                            <button
                              type="button"
                              class={ICON_BUTTON}
                              onClick={() => setEditing(role.id)}
                              aria-label={`Edit ${role.name}`}
                              title="Edit"
                            >
                              <Pencil size={16} />
                            </button>
                            <button
                              type="button"
                              class={cn(ICON_BUTTON, "hover:text-red-500")}
                              onClick={() => remove(role)}
                              disabled={role.member_count > 0}
                              aria-label={`Delete ${role.name}`}
                              title={role.member_count > 0 ? "In use: give its members another role first" : "Delete"}
                            >
                              <Trash size={16} />
                            </button>
                          </div>
                        }
                      >
                        <RoleEditor
                          role={role}
                          permissions={d().permissions}
                                  onSave={async (input) => {
                            await updateRole(role.id, input);
                            await saved();
                          }}
                          onCancel={() => setEditing(null)}
                        />
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </>
        )}
      </Show>
    </>
  );
}

/** /portal/roles: needs `write-roles` in some org (or admin). */
export function RolesPage() {
  const portal = usePortal();
  return (
    <Allowed when={portal.canAnywhere("write-roles")}>
      <ManageRoles />
    </Allowed>
  );
}
