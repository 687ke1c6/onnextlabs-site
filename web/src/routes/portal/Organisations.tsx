import { createResource, createSignal, For, Show } from "solid-js";
import { Link } from "@tanstack/solid-router";
import Building2 from "lucide-solid/icons/building";
import ChevronDown from "lucide-solid/icons/chevron-down";
import Plus from "lucide-solid/icons/plus";
import UserMinus from "lucide-solid/icons/user-minus";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { UserChip } from "~/components/UserChip";
import { UserPicker } from "~/components/UserPicker";
import { isAdmin } from "~/lib/auth";
import {
  addMember,
  changeMemberRole,
  createOrg,
  fetchManageableOrgs,
  fetchMembers,
  type Member,
  type OrgSummary,
  removeMember,
  SLUG_PATTERN,
  slugify,
} from "~/lib/orgs";
import { fetchRoles, type Permission, type Role } from "~/lib/roles";
import { fetchUsers, type UserListItem } from "~/lib/users";
import { cn, settled } from "~/lib/utils";
import { Allowed, PageHeader, usePortal } from "~/routes/portal/Layout";

const errorText = (e: unknown) => (e instanceof Error ? e.message : "Something went wrong.");

function CreateOrgForm(props: { onCreated: () => void }) {
  const [name, setName] = createSignal("");
  const [slug, setSlug] = createSignal("");
  const [slugEdited, setSlugEdited] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const effectiveSlug = () => (slugEdited() ? slug() : slugify(name()));
  const slugError = () => {
    const s = effectiveSlug();
    if (!s) return null;
    if (s.length < 2 || s.length > 63 || !SLUG_PATTERN.test(s)) return "Use lowercase letters, digits and single hyphens.";
    return null;
  };

  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    if (!name().trim() || slugError() || busy()) return;
    setBusy(true);
    setError(null);
    try {
      await createOrg(name().trim(), effectiveSlug());
      setName("");
      setSlug("");
      setSlugEdited(false);
      props.onCreated();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} class="mt-8 rounded-xl border border-border p-4">
      <h2 class="text-sm font-medium">New organisation</h2>
      <div class="mt-3 grid gap-3 sm:grid-cols-2">
        <label class="flex flex-col gap-1.5 text-sm">
          <span class="text-muted-foreground">Name</span>
          <Input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="World Health Organisation" maxLength={100} required />
        </label>
        <label class="flex flex-col gap-1.5 text-sm">
          <span class="text-muted-foreground">URL name</span>
          <div class="flex items-center rounded-md border border-input focus-within:ring-2 focus-within:ring-ring">
            <span class="pl-3 text-sm text-muted-foreground">/portal/</span>
            <input
              value={effectiveSlug()}
              onInput={(e) => {
                setSlugEdited(true);
                setSlug(e.currentTarget.value);
              }}
              placeholder="world-health-organisation"
              maxLength={63}
              class="h-10 min-w-0 flex-1 bg-transparent pr-3 text-sm placeholder:text-muted-foreground focus:outline-none"
              aria-describedby="slug-help"
            />
          </div>
        </label>
      </div>
      <p id="slug-help" class={cn("mt-2 text-xs", slugError() ? "text-red-500" : "text-muted-foreground")}>
        {slugError() ?? "Used in links and as the folder name. It can't be changed later."}
      </p>
      <Show when={error()}>{(msg) => <p class="mt-2 text-sm text-red-500">{msg()}</p>}</Show>
      <div class="mt-3 flex justify-end">
        <Button type="submit" size="sm" disabled={busy() || !name().trim() || !effectiveSlug() || !!slugError()}>
          <Plus size={16} /> {busy() ? "Creating…" : "Create"}
        </Button>
      </div>
    </form>
  );
}

/** Whether every permission of `role` is in `mine`: you can only hand out what you hold. */
const within = (mine: Permission[], role: Role | undefined) => !!role && role.permissions.every((p) => mine.includes(p));

const SELECT =
  "h-9 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50";

/** Native select over the roles; roles beyond the user's own permissions are disabled. */
function RoleSelect(props: {
  roles: Role[];
  mine: Permission[];
  value: number | undefined;
  onChange: (roleId: number) => void;
  disabled?: boolean;
  label: string;
  class?: string;
}) {
  return (
    <select
      aria-label={props.label}
      value={props.value ?? ""}
      disabled={props.disabled}
      onChange={(e) => props.onChange(Number(e.currentTarget.value))}
      class={cn(SELECT, props.class)}
    >
      <For each={props.roles}>
        {(role) => (
          <option value={role.id} selected={role.id === props.value} disabled={!within(props.mine, role)}>
            {role.name}
          </option>
        )}
      </For>
    </select>
  );
}

function Members(props: {
  org: OrgSummary;
  users: UserListItem[] | undefined;
  roles: Role[];
  /** The signed-in user's permissions in this org (all of them for admins). */
  mine: Permission[];
  onChanged: () => void;
}) {
  const [members, { mutate, refetch }] = createResource(() => props.org.slug, fetchMembers);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const memberIds = () => new Set((settled(members) ?? []).map((m) => m.user_id));
  const roleById = (id: number) => props.roles.find((r) => r.id === id);

  // New members get "Member" unless another role is picked (or the first role you may give).
  const defaultRole = () => {
    const allowed = props.roles.filter((r) => within(props.mine, r));
    return (allowed.find((r) => r.name.toLowerCase() === "member") ?? allowed[0])?.id;
  };
  const [newRole, setNewRole] = createSignal<number | undefined>();
  const addRole = () => newRole() ?? defaultRole();

  const run = async (action: () => Promise<Member[] | void>) => {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      const list = await action();
      if (list) mutate(list);
      else void refetch();
      props.onChanged();
    } catch (err) {
      setError(errorText(err));
      void refetch();
    } finally {
      setBusy(false);
    }
  };

  const add = (user: UserListItem) => {
    const role = addRole();
    if (role !== undefined) void run(() => addMember(props.org.slug, user.id, role));
  };

  const remove = (m: Member) => {
    const label = m.name ?? m.email ?? "this member";
    if (confirm(`Remove ${label} from ${props.org.name}?`)) void run(() => removeMember(props.org.slug, m.user_id));
  };

  return (
    <div class="border-t border-border bg-secondary/10 p-4">
      <div class="flex flex-col gap-2 sm:flex-row">
        <div class="min-w-0 flex-1">
          <UserPicker
            users={props.users ?? []}
            exclude={memberIds()}
            onPick={add}
            disabled={busy() || !props.users || !settled(members) || addRole() === undefined}
            label={`Add a member to ${props.org.name}`}
            placeholder={props.users ? "Add a member: search by name or email" : "Loading people…"}
          />
        </div>
        <label class="flex items-center gap-2 text-sm text-muted-foreground">
          as
          <RoleSelect
            roles={props.roles}
            mine={props.mine}
            value={addRole()}
            onChange={setNewRole}
            label="Role for new members"
            class="h-10 min-w-32"
          />
        </label>
      </div>
      <p class="mt-1.5 text-xs text-muted-foreground">
        Anyone who has signed in at least once is listed.
      </p>
      <Show when={error()}>{(msg) => <p class="mt-2 text-sm text-red-500">{msg()}</p>}</Show>

      <Show when={members.error}>
        <p class="mt-3 text-sm text-red-500">{errorText(members.error)}</p>
      </Show>
      <Show when={settled(members)} fallback={<div class="mt-3 h-12 animate-pulse rounded-md bg-secondary/60" />}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="mt-3 text-sm text-muted-foreground">No members yet.</p>}>
            <ul class="mt-3 divide-y divide-border rounded-lg border border-border">
              <For each={list()}>
                {(m) => {
                  // Someone whose role goes beyond your own permissions is out of your reach.
                  const locked = () => !within(props.mine, roleById(m.role_id));
                  const who = () => m.name ?? m.email ?? "member";
                  return (
                    <li class="flex items-center gap-3 px-3 py-2">
                      <div class="min-w-0 flex-1">
                        <UserChip user={m} />
                      </div>
                      <span title={locked() ? "Their role has permissions you don't have here." : undefined}>
                        <RoleSelect
                          roles={props.roles}
                          mine={props.mine}
                          value={m.role_id}
                          disabled={busy() || locked()}
                          onChange={(roleId) => void run(() => changeMemberRole(props.org.slug, m.user_id, roleId))}
                          label={`Role of ${who()}`}
                          class="w-32 sm:w-40"
                        />
                      </span>
                      <button
                        type="button"
                        onClick={() => remove(m)}
                        disabled={busy() || locked()}
                        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40"
                        aria-label={`Remove ${who()}`}
                        title="Remove from organisation"
                      >
                        <UserMinus size={16} />
                      </button>
                    </li>
                  );
                }}
              </For>
            </ul>
          </Show>
        )}
      </Show>
    </div>
  );
}

function ManageOrganisations() {
  const portal = usePortal();
  const admin = isAdmin(portal.user);
  const [orgs, { refetch }] = createResource(fetchManageableOrgs);
  // Loaded once and shared by every org's member panel.
  const [users] = createResource(fetchUsers);
  const [roles] = createResource(fetchRoles);
  const [open, setOpen] = createSignal<string | null>(null);

  // Keep the rail and the /portal list in step with changes made here (your own role may change).
  const changed = () => {
    void refetch();
    portal.refetchOrgs();
  };
  const myPermissions = (slug: string): Permission[] =>
    portal.orgs().find((o) => o.slug === slug)?.permissions ?? [];

  return (
    <>
      <PageHeader
        title="Manage organisations"
        description={
          admin
            ? "Create organisations, choose who belongs to each, and what role they have."
            : "Choose who belongs to the organisations you manage, and what role they have."
        }
      />
      <Show when={admin}>
        <CreateOrgForm onCreated={changed} />
      </Show>

      <Show when={orgs.error || roles.error}>
        <p class="mt-6 text-sm text-red-500">{errorText(orgs.error ?? roles.error)}</p>
      </Show>
      <Show
        when={settled(orgs) && settled(roles) && { list: settled(orgs)!, roles: settled(roles)!.roles }}
        fallback={<div class="mt-6 h-24 animate-pulse rounded-xl bg-secondary/60" />}
      >
        {(data) => (
          <Show when={data().list.length > 0} fallback={<p class="mt-6 text-sm text-muted-foreground">No organisations yet.</p>}>
            <section class="mt-6 overflow-hidden rounded-xl border border-border">
              <For each={data().list}>
                {(org) => {
                  const expanded = () => open() === org.slug;
                  return (
                    <div class="border-b border-border last:border-0">
                      <div class="flex items-center gap-3 px-4 py-3">
                        <Building2 size={18} class="shrink-0 text-muted-foreground" />
                        <div class="min-w-0 flex-1">
                          <Link to="/portal/$org" params={{ org: org.slug }} class="truncate text-sm font-medium hover:underline">
                            {org.name}
                          </Link>
                          <p class="truncate text-xs text-muted-foreground">/{org.slug}</p>
                        </div>
                        <button
                          type="button"
                          onClick={() => setOpen(expanded() ? null : org.slug)}
                          aria-expanded={expanded()}
                          class="inline-flex h-8 items-center gap-1.5 rounded-md px-2 text-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        >
                          {org.member_count} {org.member_count === 1 ? "member" : "members"}
                          <ChevronDown size={16} class={cn("transition-transform", expanded() && "rotate-180")} />
                        </button>
                      </div>
                      <Show when={expanded()}>
                        <Members
                          org={org}
                          users={settled(users)}
                          roles={data().roles}
                          mine={myPermissions(org.slug)}
                          onChanged={changed}
                        />
                      </Show>
                    </div>
                  );
                }}
              </For>
            </section>
          </Show>
        )}
      </Show>
    </>
  );
}

export function OrganisationsPage() {
  const portal = usePortal();
  return (
    <Allowed when={portal.canAnywhere("manage-orgs")}>
      <ManageOrganisations />
    </Allowed>
  );
}
