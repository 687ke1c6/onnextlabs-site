import { createSignal, onMount, Show } from "solid-js";
import { ParticleField } from "~/components/ParticleField";
import { Badge } from "~/components/ui/badge";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";

// Sibling apps live on subdomains of whatever domain serves this page (www. stripped).
// On localhost/IPs there's no real domain, so fall back to production.
const PROD_DOMAIN = "onnextlabs.xyz";
const baseDomain = () => {
  const host = window.location.hostname.replace(/^www\./, "");
  return host === "localhost" || /^[\d.]+$/.test(host) || !host.includes(".") ? PROD_DOMAIN : host;
};

const links = [
  {
    name: "pdf-flex",
    href: `https://pdf-flex.${baseDomain()}`,
    description: "Sign and edit PDFs entirely in your browser.",
  },
  {
    name: "proxy-rs",
    href: "https://github.com/687ke1c6/proxy-rs",
    description: "Proxying, tunnels and file transfer over iroh.",
  },
];

type Status = { kind: "idle" | "sending" | "done" } | { kind: "error"; message: string };

function SubscribeForm() {
  const [status, setStatus] = createSignal<Status>({ kind: "idle" });
  // Client-side debounce: no new submit until this time (ms), e.g. after a 429.
  let blockedUntil = 0;

  // ALTCHA proof-of-work captcha. The widget is invisible and starts solving when the email
  // field gets focus, so by the time the user hits submit the payload is usually ready.
  let widget: AltchaWidgetElement | undefined;
  let altchaPayload = "";
  let altchaWaiter: ((e: AltchaStateEvent) => void) | undefined;
  onMount(() => void import("altcha"));

  const onAltchaState = (e: AltchaStateEvent) => {
    altchaPayload = e.detail.state === "verified" ? (e.detail.payload ?? "") : "";
    altchaWaiter?.(e);
  };

  const getAltchaPayload = async (): Promise<string> => {
    if (altchaPayload) return altchaPayload;
    await customElements.whenDefined("altcha-widget");
    const w = widget!;
    return new Promise((resolve, reject) => {
      altchaWaiter = (e) => {
        const { state } = e.detail;
        if (state === "verified") resolve(e.detail.payload ?? "");
        else if (state === "error" || state === "expired") reject(new Error(state));
        else return;
        altchaWaiter = undefined;
      };
      if (w.getState() !== "verifying") void w.verify();
    });
  };

  // Each payload is single-use on the server, so start fresh after any attempt.
  const resetAltcha = () => {
    altchaPayload = "";
    widget?.reset();
  };

  const onSubmit = async (e: SubmitEvent) => {
    e.preventDefault();
    if (status().kind === "sending" || status().kind === "done") return;
    if (Date.now() < blockedUntil) {
      setStatus({ kind: "error", message: "Please wait a moment before trying again." });
      return;
    }
    const form = new FormData(e.currentTarget as HTMLFormElement);
    setStatus({ kind: "sending" });
    blockedUntil = Date.now() + 2000;
    let altcha: string;
    try {
      altcha = await getAltchaPayload();
    } catch {
      resetAltcha();
      return setStatus({ kind: "error", message: "Couldn't verify your browser. Please try again." });
    }
    try {
      const res = await fetch("/api/subscribe", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ email: form.get("email"), website: form.get("website"), altcha }),
      });
      resetAltcha();
      if (res.ok) return setStatus({ kind: "done" });
      if (res.status === 429) blockedUntil = Date.now() + Number(res.headers.get("retry-after") ?? 60) * 1000;
      const body = await res.json().catch(() => ({}));
      setStatus({ kind: "error", message: body.error ?? "Something went wrong. Please try again." });
    } catch {
      resetAltcha();
      setStatus({ kind: "error", message: "Couldn't reach the server. Please try again." });
    }
  };

  return (
    <div class="mt-10 w-full max-w-sm">
      <Show
        when={status().kind !== "done"}
        fallback={<p class="text-sm text-emerald-400">Thanks! We'll let you know when it's live.</p>}
      >
        <form class="flex gap-2" onSubmit={onSubmit}>
          <Input name="email" type="email" placeholder="you@example.com" required aria-label="Email address" />
          {/* Honeypot: invisible to people, tempting to bots. */}
          <input name="website" type="text" tabindex="-1" autocomplete="off" aria-hidden="true" class="absolute -left-[9999px] h-px w-px opacity-0" />
          <altcha-widget
            ref={widget}
            attr:challenge="/api/altcha/challenge"
            attr:auto="onfocus"
            attr:display="invisible"
            on:statechange={onAltchaState}
          />
          <Button type="submit" disabled={status().kind === "sending"}>
            {status().kind === "sending" ? "Sending…" : "Notify me"}
          </Button>
        </form>
      </Show>
      <p class="mt-2 min-h-5 text-sm text-red-400" role="status">
        {(() => { const s = status(); return s.kind === "error" ? s.message : ""; })()}
      </p>
    </div>
  );
}

export function Home() {
  return (
    <>
      <header class="relative isolate flex min-h-screen flex-col items-center justify-center overflow-hidden px-4 text-center">
        {/* Background: grid, aurora blobs, particle constellation */}
        <div
          class="absolute inset-0 -z-10 opacity-40 [background-image:linear-gradient(var(--border)_1px,transparent_1px),linear-gradient(90deg,var(--border)_1px,transparent_1px)] [background-size:56px_56px] [mask-image:radial-gradient(ellipse_at_center,black,transparent_70%)]"
          aria-hidden="true"
        />
        <div class="absolute -left-32 -top-32 -z-10 h-[32rem] w-[32rem] animate-float-a rounded-full bg-violet-600/30 blur-3xl" aria-hidden="true" />
        <div class="absolute -bottom-40 -right-32 -z-10 h-[34rem] w-[34rem] animate-float-b rounded-full bg-cyan-500/25 blur-3xl" aria-hidden="true" />
        <div class="absolute inset-0 -z-10"><ParticleField /></div>

        {/* Rotating conic ring behind the heading */}
        <div
          class="absolute -z-10 h-[28rem] w-[28rem] animate-spin-slow rounded-full opacity-30 blur-2xl [background:conic-gradient(from_0deg,#a78bfa,#22d3ee,#f472b6,#a78bfa)] [mask-image:radial-gradient(circle,transparent_55%,black_57%,black_60%,transparent_62%)]"
          aria-hidden="true"
        />

        <Badge class="mb-6 gap-2">
          <span class="relative flex h-2 w-2">
            <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
            <span class="relative inline-flex h-2 w-2 rounded-full bg-emerald-400" />
          </span>
          Building in progress
        </Badge>

        <h1 class="bg-gradient-to-br from-white via-violet-200 to-cyan-300 bg-clip-text pb-2 text-5xl font-semibold tracking-tight text-transparent sm:text-7xl">
          Coming Soon
        </h1>
        <p class="mt-5 max-w-md text-balance text-muted-foreground sm:text-lg">
          OnNext Labs is crafting something new. Leave your email and we'll tell you when it's live.
        </p>

        <SubscribeForm />

        <nav aria-label="Projects" class="mt-8 grid w-full max-w-lg gap-3 text-left sm:grid-cols-2">
          {links.map((l) => (
            <a
              href={l.href}
              target="_blank"
              rel="noreferrer"
              class="group rounded-xl border border-border bg-secondary/30 px-4 py-3 backdrop-blur transition hover:border-ring"
            >
              <span class="flex items-center justify-between text-sm font-medium">
                {l.name}
                <span class="text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5">↗</span>
              </span>
              <span class="mt-1 block text-xs text-muted-foreground">{l.description}</span>
            </a>
          ))}
        </nav>
      </header>

      <footer class="pb-8 text-center text-xs text-muted-foreground">© {new Date().getFullYear()} OnNext Labs</footer>
    </>
  );
}
