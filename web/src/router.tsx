import { createRootRoute, createRoute, createRouter, Outlet } from "@tanstack/solid-router";
import { AdminPage } from "~/routes/Admin";
import { Home } from "~/routes/Home";
import { LoginPage } from "~/routes/Login";

const rootRoute = createRootRoute({ component: () => <Outlet /> });

const indexRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: Home });

type LoginSearch = { return_to?: string; error?: string };

export const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  component: LoginPage,
  validateSearch: (s: Record<string, unknown>): LoginSearch => ({
    // Same-origin paths only (the server re-checks this too).
    return_to: typeof s.return_to === "string" && /^\/(?![/\\])/.test(s.return_to) ? s.return_to : undefined,
    error: typeof s.error === "string" ? s.error : undefined,
  }),
});

// Access is enforced by the API (`/api/admin/*` requires the admin role); the page only
// decides what to show.
const adminRoute = createRoute({ getParentRoute: () => rootRoute, path: "/admin", component: AdminPage });

export const router = createRouter({
  routeTree: rootRoute.addChildren([indexRoute, loginRoute, adminRoute]),
  scrollRestoration: true,
});

declare module "@tanstack/solid-router" {
  interface Register {
    router: typeof router;
  }
}
