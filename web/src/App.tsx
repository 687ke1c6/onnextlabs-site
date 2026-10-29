import { ParticleField } from "~/components/ParticleField";
import { Countdown } from "~/components/Countdown";
import { Badge } from "~/components/ui/badge";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";

export default function App() {
  return (
    <main class="relative isolate flex min-h-screen flex-col items-center justify-center overflow-hidden px-4 text-center">
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

      <h1 class="bg-gradient-to-br from-white via-violet-200 to-cyan-300 bg-clip-text text-5xl font-semibold tracking-tight text-transparent sm:text-7xl">
        Coming Soon
      </h1>
      <p class="mt-5 max-w-md text-balance text-muted-foreground sm:text-lg">
        OnNext Labs is crafting something new. Leave your email and we'll tell you when it's live.
      </p>

      <div class="mt-10"><Countdown /></div>

      {/* TODO: wire this form up to a backend endpoint (e.g. POST /api/subscribe in src/main.rs). */}
      <form class="mt-10 flex w-full max-w-sm gap-2" onSubmit={(e) => e.preventDefault()}>
        <Input type="email" placeholder="you@example.com" required aria-label="Email address" />
        <Button type="submit">Notify me</Button>
      </form>

      <footer class="absolute bottom-6 text-xs text-muted-foreground">© {new Date().getFullYear()} OnNext Labs</footer>
    </main>
  );
}
