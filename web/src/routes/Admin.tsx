import { createResource, Match, Show, Switch } from "solid-js";
import { useNavigate } from "@tanstack/solid-router";
import { AuthShell } from "~/components/AuthShell";
import { Badge } from "~/components/ui/badge";
import { Button } from "~/components/ui/button";
import { fetchMe, logout, type User } from "~/lib/auth";

type Overview = { subscribers: number; users: number };

function UserChip(props: { user: User }) {
  const label = () => props.user.name ?? props.user.email ?? "Signed in";
  return (
    <div class="flex items-center gap-3">
      <Show
        when={props.user.picture}
        fallback={<div class="flex h-9 w-9 items-center justify-center rounded-full bg-secondary text-sm">{label()[0]?.toUpperCase()}</div>}
      >
        {(src) => <img src={src()} alt="" referrerpolicy="no-referrer" class="h-9 w-9 rounded-full" />}
      </Show>
      <div class="min-w-0 text-left">
        <p class="truncate text-sm font-medium">{label()}</p>
        <Show when={props.user.name && props.user.email}>
          <p class="truncate text-xs text-muted-foreground">{props.user.email}</p>
        </Show>
      </div>
    </div>
  );
}

function Stat(props: { label: string; value?: number }) {
  return (
    <div class="rounded-xl border border-border bg-background/40 p-4">
      <p class="text-xs text-muted-foreground">{props.label}</p>
      <p class="mt-1 text-2xl font-semibold tabular-nums">{props.value ?? "–"}</p>
    </div>
  );
}

function Dashboard(props: { user: User; onSignOut: () => void }) {
  const [overview] = createResource<Overview>(async () => {
    const res = await fetch("/api/admin/overview");
    if (!res.ok) throw new Error(`overview: ${res.status}`);
    return res.json();
  });

  return (
    <main class="mx-auto min-h-screen w-full max-w-4xl px-4 py-10">
      <header class="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-6">
        <div class="flex items-center gap-3">
          <h1 class="text-xl font-semibold tracking-tight">Admin</h1>
          <Badge>admin</Badge>
        </div>
        <div class="flex items-center gap-4">
          <UserChip user={props.user} />
          <Button variant="outline" size="sm" onClick={props.onSignOut}>Sign out</Button>
        </div>
      </header>

      <section class="mt-8 grid gap-4 sm:grid-cols-2">
        <Stat label="Waitlist subscribers" value={overview()?.subscribers} />
        <Stat label="Accounts" value={overview()?.users} />
      </section>
      <Show when={overview.error}>
        <p class="mt-4 text-sm text-red-400">Couldn't load the overview.</p>
      </Show>
    </main>
  );
}

export function AdminPage() {
  const navigate = useNavigate();
  const [me] = createResource(async () => {
    const user = await fetchMe();
    if (!user) void navigate({ to: "/login", search: { return_to: "/admin" }, replace: true });
    return user;
  });

  const signOut = async () => {
    await logout();
    void navigate({ to: "/login", replace: true });
  };

  return (
    <Switch fallback={<AuthShell><div class="h-24 animate-pulse rounded-md bg-secondary/60" /></AuthShell>}>
      <Match when={me.error}>
        <AuthShell>
          <p class="text-sm text-red-400">Couldn't check your session. Please refresh.</p>
        </AuthShell>
      </Match>
      <Match when={me()?.roles.includes("admin") && me()}>
        {(user) => <Dashboard user={user()} onSignOut={signOut} />}
      </Match>
      <Match when={me()}>
        {(user) => (
          <AuthShell>
            <h1 class="text-2xl font-semibold tracking-tight">Access pending</h1>
            <p class="mt-2 text-sm text-muted-foreground">
              You're signed in, but this account hasn't been granted access yet.
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
