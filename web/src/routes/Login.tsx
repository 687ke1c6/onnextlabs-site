import { createResource, For, Show } from "solid-js";
import { useNavigate } from "@tanstack/solid-router";
import { AuthShell } from "~/components/AuthShell";
import { buttonVariants } from "~/components/ui/button";
import { fetchMe, fetchProviders, loginUrl } from "~/lib/auth";
import { cn } from "~/lib/utils";
import { loginRoute } from "~/router";

const ERRORS: Record<string, string> = {
  denied: "Sign-in was cancelled.",
  expired: "That sign-in took too long or was already used. Please try again.",
  state: "Sign-in couldn't be verified. Please try again.",
  provider: "Couldn't complete sign-in with the provider. Please try again.",
  unavailable: "That sign-in option isn't available.",
  busy: "Too many sign-in attempts right now. Please try again shortly.",
  server: "Something went wrong on our side. Please try again.",
};

const PROVIDER_LABELS: Record<string, string> = { google: "Continue with Google" };

function GoogleIcon() {
  return (
    <svg viewBox="0 0 24 24" class="h-4 w-4" aria-hidden="true">
      <path fill="#4285F4" d="M23.5 12.3c0-.8-.1-1.6-.2-2.3H12v4.4h6.5a5.6 5.6 0 0 1-2.4 3.6v3h3.9c2.3-2.1 3.5-5.2 3.5-8.7z" />
      <path fill="#34A853" d="M12 24c3.2 0 6-1.1 8-2.9l-3.9-3c-1.1.7-2.5 1.2-4.1 1.2-3.1 0-5.8-2.1-6.7-5H1.3v3.1A12 12 0 0 0 12 24z" />
      <path fill="#FBBC05" d="M5.3 14.3a7.2 7.2 0 0 1 0-4.6V6.6H1.3a12 12 0 0 0 0 10.8l4-3.1z" />
      <path fill="#EA4335" d="M12 4.8c1.8 0 3.3.6 4.6 1.8l3.4-3.4A12 12 0 0 0 1.3 6.6l4 3.1c.9-2.9 3.6-4.9 6.7-4.9z" />
    </svg>
  );
}

export function LoginPage() {
  const search = loginRoute.useSearch();
  const navigate = useNavigate();
  const returnTo = () => search().return_to ?? "/admin";
  const [providers] = createResource(fetchProviders);

  // Already signed in: skip straight to where they were going.
  createResource(async () => {
    const me = await fetchMe().catch(() => null);
    if (me) void navigate({ to: returnTo(), replace: true });
  });

  return (
    <AuthShell>
      <h1 class="text-2xl font-semibold tracking-tight">Sign in</h1>
      <p class="mt-2 text-sm text-muted-foreground">Members area for OnNext Labs.</p>

      <Show when={search().error}>
        {(code) => (
          <p class="mt-6 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300" role="alert">
            {ERRORS[code()] ?? "Sign-in failed. Please try again."}
          </p>
        )}
      </Show>

      <div class="mt-8 flex flex-col gap-3">
        <Show when={!providers.loading} fallback={<div class="h-10 animate-pulse rounded-md bg-secondary" />}>
          <For each={providers()} fallback={<p class="text-sm text-muted-foreground">Sign-in isn't configured yet.</p>}>
            {(p) => (
              <a href={loginUrl(p, returnTo())} class={cn(buttonVariants({ variant: "outline" }), "w-full")}>
                {p === "google" && <GoogleIcon />}
                {PROVIDER_LABELS[p] ?? `Continue with ${p}`}
              </a>
            )}
          </For>
        </Show>
      </div>
    </AuthShell>
  );
}
