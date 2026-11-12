import { createContext, createResource, createSignal, For, type JSX, Match, Show, Switch, useContext } from "solid-js";
import { Link, Outlet, useLocation, useNavigate, useParams } from "@tanstack/solid-router";
import Building2 from "lucide-solid/icons/building";
import FileText from "lucide-solid/icons/file-text";
import LayoutDashboard from "lucide-solid/icons/layout-dashboard";
import Check from "lucide-solid/icons/check";
import ChevronRight from "lucide-solid/icons/chevron-right";
import LogOut from "lucide-solid/icons/log-out";
import PanelLeftClose from "lucide-solid/icons/panel-left-close";
import PanelLeftOpen from "lucide-solid/icons/panel-left-open";
import Settings from "lucide-solid/icons/settings";
import SlidersHorizontal from "lucide-solid/icons/sliders-horizontal";
import ShieldX from "lucide-solid/icons/shield-x";
import KeyRound from "lucide-solid/icons/key-round";
import UsersIcon from "lucide-solid/icons/users";
import ShieldCheck from "lucide-solid/icons/shield-check";
import { AuthShell } from "~/components/AuthShell";
import { Button } from "~/components/ui/button";
import { Menu, MenuLabel, menuItemClass, MenuSeparator, useMenu } from "~/components/ui/menu";
import { UserAvatar, UserChip } from "~/components/UserChip";
import { fetchMe, isAdmin, logout, type User } from "~/lib/auth";
import { can, fetchMyOrgs, type Org } from "~/lib/orgs";
import type { Permission } from "~/lib/roles";
import { theme } from "~/lib/theme";
import { cn, settled } from "~/lib/utils";

type Portal = {
  user: User;
  /** Orgs the user can open (every org for admins). */
  orgs: () => Org[];
  /** Reload the org list, e.g. after an admin creates one. */
  refetchOrgs: () => void;
  /** Whether the user holds `permission` in at least one org (always true for admins). */
  canAnywhere: (permission: Permission) => boolean;
};

const PortalContext = createContext<Portal>();

/** Session and org list, for any page rendered inside the portal layout. */
export function usePortal(): Portal {
  const portal = useContext(PortalContext);
  if (!portal) throw new Error("usePortal() called outside PortalLayout");
  return portal;
}

export const usePortalUser = () => usePortal().user;

/** Page title block shared by every portal page. */
export function PageHeader(props: { title: string; description?: string; actions?: JSX.Element }) {
  return (
    <header class="flex flex-wrap items-end justify-between gap-4 border-b border-border pb-6">
      <div class="min-w-0">
        <h1 class="truncate text-2xl font-semibold tracking-tight">{props.title}</h1>
        <Show when={props.description}>
          <p class="mt-1 text-sm text-muted-foreground">{props.description}</p>
        </Show>
      </div>
      {props.actions}
    </header>
  );
}

/** Full-width message for pages the user can't see (no access, not found). */
export function Notice(props: { title: string; body: string }) {
  return (
    <section class="flex flex-col items-center justify-center rounded-xl border border-dashed border-border px-6 py-16 text-center">
      <ShieldX size={32} class="text-muted-foreground" />
      <h1 class="mt-4 text-base font-medium">{props.title}</h1>
      <p class="mt-1 max-w-sm text-sm text-muted-foreground">{props.body}</p>
      <Link to="/portal" class="mt-4 text-sm text-muted-foreground underline hover:text-foreground">
        Back to your organisations
      </Link>
    </section>
  );
}

/** Renders its children when `when` holds; otherwise a "no access" notice. */
export function Allowed(props: { when: boolean; children: JSX.Element }) {
  return (
    <Show
      when={props.when}
      fallback={<Notice title="You don't have access to this page" body="Ask an administrator if you need it." />}
    >
      {props.children}
    </Show>
  );
}

const anywhere = (user: User, orgs: Org[], permission: Permission) =>
  isAdmin(user) || orgs.some((o) => can(o, permission));

const RAIL_KEY = "portal-rail-collapsed";

function initialCollapsed(): boolean {
  try {
    const saved = localStorage.getItem(RAIL_KEY);
    if (saved !== null) return saved === "1";
  } catch {
    // fall through to the width-based default
  }
  return window.innerWidth < 768;
}

const ITEM =
  "flex h-10 items-center gap-3 rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
const ACTIVE = { class: "bg-accent text-accent-foreground" };
/** Rail menus open to the right of their button (the ">" on the button says so). */
const SIDE_MENU = "bottom-auto top-0 left-full mb-0 ml-2";
const INACTIVE = { class: "text-muted-foreground hover:bg-accent/60 hover:text-foreground" };

function Rail(props: { user: User; orgs: Org[]; onSignOut: () => void }) {
  const [collapsed, setCollapsed] = createSignal(initialCollapsed());
  const toggle = () => {
    const next = !collapsed();
    setCollapsed(next);
    try {
      localStorage.setItem(RAIL_KEY, next ? "1" : "0");
    } catch {
      // not persisted; fine
    }
  };

  // The org in the URL (/portal/<slug>/...), if any.
  const params = useParams({ strict: false });
  const org = () => props.orgs.find((o) => o.slug === (params() as { org?: string }).org);
  const location = useLocation();
  const onManagePage = () => /^\/portal\/(organisations|roles|users|settings)(\/|$)/.test(location().pathname);

  const item = () => cn(ITEM, collapsed() ? "justify-center" : "px-3");
  const label = () => cn("truncate", collapsed() && "sr-only");
  const tip = (text: string) => (collapsed() ? text : undefined);
  const userLabel = () => props.user.name ?? props.user.email ?? "Account";

  return (
    <aside
      class={cn(
        "sticky top-0 z-20 flex h-screen shrink-0 flex-col border-r border-border bg-secondary/30 transition-[width] duration-200",
        collapsed() ? "w-16" : "w-60",
      )}
    >
      <div class={cn("flex h-16 items-center border-b border-border px-3", collapsed() ? "justify-center" : "justify-between")}>
        <Show when={!collapsed()}>
          <Link to="/portal" class="truncate pl-2 text-sm font-semibold tracking-tight">
            OnNext Labs <span class="font-normal text-muted-foreground">portal</span>
          </Link>
        </Show>
        <button
          type="button"
          onClick={toggle}
          class="flex h-9 w-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={collapsed() ? "Expand navigation" : "Collapse navigation"}
          aria-expanded={!collapsed()}
          title={collapsed() ? "Expand" : "Collapse"}
        >
          <Show when={collapsed()} fallback={<PanelLeftClose size={18} />}>
            <PanelLeftOpen size={18} />
          </Show>
        </button>
      </div>

      {/* The open org's sections (none outside an org), then Manage. Not a scroll container:
          that would clip the Manage menu, which opens to the side. */}
      <nav class="flex flex-col gap-1 p-3" aria-label="Portal">
        <Show when={org()} keyed>
          {(o) => (
            <>
              <p
                class={cn("truncate px-3 pb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground", collapsed() && "sr-only")}
                title={o.name}
              >
                {o.name}
              </p>
              <Link
                to="/portal/$org"
                params={{ org: o.slug }}
                activeOptions={{ exact: true }}
                title={tip(`${o.name} dashboard`)}
                class={item()}
                activeProps={ACTIVE}
                inactiveProps={INACTIVE}
              >
                <LayoutDashboard size={18} class="shrink-0" />
                <span class={label()}>Dashboard</span>
              </Link>
              <Show when={can(o, "read-docs")}>
                <Link
                  to="/portal/$org/documents"
                  params={{ org: o.slug }}
                  title={tip("Documents")}
                  class={item()}
                  activeProps={ACTIVE}
                  inactiveProps={INACTIVE}
                >
                  <FileText size={18} class="shrink-0" />
                  <span class={label()}>Documents</span>
                </Link>
              </Show>
            </>
          )}
        </Show>

        {/* Manage: configuration pages (orgs and roles, if permitted; personal settings). */}
        <div class={cn(org() && "mt-2 border-t border-border pt-3")}>
          <Menu
            label="Manage"
            title={tip("Manage")}
            trigger={
              <>
                <Settings size={18} class="shrink-0" />
                <span class={cn(label(), "flex-1 text-left")}>Manage</span>
                <Show when={!collapsed()}>
                  <ChevronRight size={16} class="shrink-0 opacity-70" />
                </Show>
              </>
            }
            triggerClass={cn(item(), "w-full", onManagePage() ? ACTIVE.class : INACTIVE.class)}
            class={SIDE_MENU}
          >
            <ManageMenuItems
              admin={isAdmin(props.user)}
              manageOrgs={anywhere(props.user, props.orgs, "manage-orgs")}
              writeRoles={anywhere(props.user, props.orgs, "write-roles")}
            />
          </Menu>
        </div>
      </nav>

      <div class="flex-1" />

      <div class="border-t border-border p-3">
        {/* Account: switch org, sign out. */}
        <Menu
          label={`Account: ${userLabel()}`}
          title={tip(userLabel())}
          trigger={
            <Show when={!collapsed()} fallback={<UserAvatar user={props.user} class="h-8 w-8" />}>
              <div class="min-w-0 flex-1">
                <UserChip user={props.user} />
              </div>
              <ChevronRight size={16} class="shrink-0 text-muted-foreground" />
            </Show>
          }
          triggerClass={cn(
            "flex w-full items-center gap-2 rounded-md text-left transition-colors hover:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            collapsed() ? "justify-center py-1" : "px-2 py-2",
          )}
          class={cn(SIDE_MENU, "top-auto bottom-0")}
        >
          <AccountMenuItems user={props.user} orgs={props.orgs} current={org()} onSignOut={props.onSignOut} />
        </Menu>
      </div>
    </aside>
  );
}

function ManageMenuItems(props: { admin: boolean; manageOrgs: boolean; writeRoles: boolean }) {
  const menu = useMenu();
  return (
    <>
      <Show when={props.manageOrgs}>
        <Link to="/portal/organisations" role="menuitem" class={menuItemClass} onClick={menu.close}>
          <Building2 size={16} class="shrink-0 text-muted-foreground" /> Organisations
        </Link>
      </Show>
      <Show when={props.writeRoles}>
        <Link to="/portal/roles" role="menuitem" class={menuItemClass} onClick={menu.close}>
          <KeyRound size={16} class="shrink-0 text-muted-foreground" /> Roles
        </Link>
      </Show>
      <Show when={props.admin}>
        <Link to="/portal/users" role="menuitem" class={menuItemClass} onClick={menu.close}>
          <UsersIcon size={16} class="shrink-0 text-muted-foreground" /> Users
        </Link>
      </Show>
      <Link to="/portal/settings" role="menuitem" class={menuItemClass} onClick={menu.close}>
        <SlidersHorizontal size={16} class="shrink-0 text-muted-foreground" /> Settings
      </Link>
    </>
  );
}

function AccountMenuItems(props: { user: User; orgs: Org[]; current?: Org; onSignOut: () => void }) {
  const menu = useMenu();
  // "Admin-org in World Health" / "Admin": who you are where you're standing.
  const standing = () => {
    const role = props.current?.role;
    if (role && props.current) return `${role.name} in ${props.current.name}`;
    return isAdmin(props.user) ? "Admin (every organisation)" : undefined;
  };
  return (
    <>
      <Show when={standing()}>{(text) => <MenuLabel>{text()}</MenuLabel>}</Show>
      <Link to="/portal/access" role="menuitem" class={menuItemClass} onClick={menu.close}>
        <ShieldCheck size={16} class="shrink-0 text-muted-foreground" /> Your access
      </Link>
      <MenuSeparator />
      {/* Only worth offering when there's somewhere else to go. */}
      <Show when={props.orgs.length > 1}>
        <MenuLabel>Switch organisation</MenuLabel>
        <div class="max-h-64 overflow-y-auto">
          <For each={props.orgs}>
            {(o) => (
              <Link
                to="/portal/$org"
                params={{ org: o.slug }}
                role="menuitem"
                aria-current={o.slug === props.current?.slug ? "true" : undefined}
                class={menuItemClass}
                onClick={menu.close}
              >
                <Building2 size={16} class="shrink-0 text-muted-foreground" />
                <span class="min-w-0 flex-1 truncate">{o.name}</span>
                <Show when={o.slug === props.current?.slug}>
                  <Check size={16} class="shrink-0" />
                </Show>
              </Link>
            )}
          </For>
        </div>
        <MenuSeparator />
      </Show>
      <button
        type="button"
        role="menuitem"
        class={menuItemClass}
        onClick={() => {
          menu.close();
          props.onSignOut();
        }}
      >
        <LogOut size={16} class="shrink-0 text-muted-foreground" /> Sign out
      </button>
    </>
  );
}

/**
 * Wraps every /portal/* route: checks the session, loads the user's orgs, shows the rail.
 * Admins always get in; anyone else needs to belong to at least one org. This only
 * decides what to show: data is protected by the API.
 */
export function PortalLayout() {
  const navigate = useNavigate();
  const [me] = createResource(async () => {
    const user = await fetchMe();
    if (!user) {
      void navigate({ to: "/login", search: { return_to: window.location.pathname }, replace: true });
    }
    return user;
  });
  const [orgs, { refetch: refetchOrgs }] = createResource(() => settled(me) ?? undefined, fetchMyOrgs);

  const signOut = async () => {
    await logout();
    void navigate({ to: "/login", replace: true });
  };

  const ready = () => {
    const user = settled(me);
    const list = settled(orgs);
    return user && list ? { user, orgs: list } : undefined;
  };
  const allowed = () => {
    const r = ready();
    return r && (isAdmin(r.user) || r.orgs.length > 0) ? r.user : undefined;
  };

  return (
    <Switch fallback={<AuthShell><div class="h-24 animate-pulse rounded-md bg-secondary/60" /></AuthShell>}>
      <Match when={me.error || orgs.error}>
        <AuthShell>
          <p class="text-sm text-red-400">Couldn't load your account. Please refresh.</p>
        </AuthShell>
      </Match>
      <Match when={allowed()}>
        {(user) => (
          <div
            class={cn("flex min-h-screen bg-background text-foreground", theme() === "light" && "light")}
            style={{ "color-scheme": theme() }}
          >
            <PortalContext.Provider
              value={{
                user: user(),
                orgs: () => settled(orgs) ?? [],
                refetchOrgs: () => void refetchOrgs(),
                canAnywhere: (p) => anywhere(user(), settled(orgs) ?? [], p),
              }}
            >
              <Rail user={user()} orgs={settled(orgs) ?? []} onSignOut={signOut} />
              <main class="min-w-0 flex-1 px-4 py-8 sm:px-8">
                <div class="mx-auto w-full max-w-5xl">
                  <Outlet />
                </div>
              </main>
            </PortalContext.Provider>
          </div>
        )}
      </Match>
      <Match when={ready()?.user}>
        {(user) => (
          <AuthShell>
            <h1 class="text-2xl font-semibold tracking-tight">Access pending</h1>
            <p class="mt-2 text-sm text-muted-foreground">
              You're signed in, but you haven't been added to an organisation yet.
            </p>
            <div class="mt-6 rounded-xl border border-border bg-background/40 p-4">
              <UserChip user={user()} />
            </div>
            <Button variant="outline" class="mt-6 w-full" onClick={signOut}>Sign out</Button>
          </AuthShell>
        )}
      </Match>
    </Switch>
  );
}
