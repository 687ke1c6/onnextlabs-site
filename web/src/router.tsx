import { createRootRoute, createRoute, createRouter, lazyRouteComponent, Outlet } from "@tanstack/solid-router";
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

// Portal code is split out, so visitors to the public pages never download it.
const PortalLayout = lazyRouteComponent(() => import("~/routes/portal/Layout"), "PortalLayout");
const PortalHomePage = lazyRouteComponent(() => import("~/routes/portal/Home"), "PortalHomePage");
const OrganisationsPage = lazyRouteComponent(() => import("~/routes/portal/Organisations"), "OrganisationsPage");
const AccessPage = lazyRouteComponent(() => import("~/routes/portal/Access"), "AccessPage");
const UsersPage = lazyRouteComponent(() => import("~/routes/portal/Users"), "UsersPage");
const RolesPage = lazyRouteComponent(() => import("~/routes/portal/Roles"), "RolesPage");
const SettingsPage = lazyRouteComponent(() => import("~/routes/portal/Settings"), "SettingsPage");
const OrgLayout = lazyRouteComponent(() => import("~/routes/portal/org/OrgLayout"), "OrgLayout");
const OrgDashboardPage = lazyRouteComponent(() => import("~/routes/portal/org/Dashboard"), "OrgDashboardPage");
const DocumentsPage = lazyRouteComponent(() => import("~/routes/portal/org/Documents"), "DocumentsPage");
const TransactionsPage = lazyRouteComponent(() => import("~/routes/portal/org/Transactions"), "TransactionsPage");
const UploadsPage = lazyRouteComponent(() => import("~/routes/portal/org/Statements"), "UploadsPage");
const StatementDetailPage = lazyRouteComponent(() => import("~/routes/portal/org/StatementDetail"), "StatementDetailPage");
const TypeRulesPage = lazyRouteComponent(() => import("~/routes/portal/org/TypeRules"), "TypeRulesPage");
const AccountsPage = lazyRouteComponent(() => import("~/routes/portal/org/Accounts"), "AccountsPage");
const TemplateEditorPage = lazyRouteComponent(() => import("~/routes/portal/org/TemplateEditor"), "TemplateEditorPage");
const TemplatesPage = lazyRouteComponent(() => import("~/routes/portal/org/Templates"), "TemplatesPage");
const QuickFiltersPage = lazyRouteComponent(() => import("~/routes/portal/org/QuickFilters"), "QuickFiltersPage");

// /portal                      your organisations (PortalLayout: session, org list, rail)
// /portal/settings             personal settings
// /portal/access               your roles and what they allow
// /portal/organisations        members and their roles (manage-orgs); creating orgs (admins)
// /portal/users                every account; make people admins (admins only)
// /portal/roles                roles and their permissions (write-roles)
// /portal/<org>                an org's dashboard (OrgLayout: resolves the slug)
// /portal/<org>/documents      its document store (read-docs; uploading needs write-docs)
// /portal/<org>/statements     every saved transaction (read-statements)
// /portal/<org>/statements/uploads   saved statement files and upload exclusions; uploading needs write-statements
// /portal/<org>/statements/rules     the account's type rules (editing needs write-statements)
// /portal/<org>/statements/accounts  accounts (changes need write-statements)
// /portal/<org>/statements/filters   quick filters for the grid (editing needs write-statements)
// /portal/<org>/statements/templates the org's statement PDF templates (editing needs write-statements)
// /portal/<org>/statements/templates/<id>  designing one
// /portal/<org>/statements/<id>      one saved statement
// Static segments win over $org, and the server reserves those names as slugs.
// What's shown here is cosmetic: the API enforces access.
const portalRoute = createRoute({ getParentRoute: () => rootRoute, path: "/portal", component: PortalLayout });
const portalHomeRoute = createRoute({ getParentRoute: () => portalRoute, path: "/", component: PortalHomePage });
const portalSettingsRoute = createRoute({ getParentRoute: () => portalRoute, path: "settings", component: SettingsPage });
const portalOrganisationsRoute = createRoute({
  getParentRoute: () => portalRoute,
  path: "organisations",
  component: OrganisationsPage,
});
const portalAccessRoute = createRoute({ getParentRoute: () => portalRoute, path: "access", component: AccessPage });
const portalUsersRoute = createRoute({ getParentRoute: () => portalRoute, path: "users", component: UsersPage });
const portalRolesRoute = createRoute({ getParentRoute: () => portalRoute, path: "roles", component: RolesPage });
const orgRoute = createRoute({ getParentRoute: () => portalRoute, path: "$org", component: OrgLayout });
const orgDashboardRoute = createRoute({ getParentRoute: () => orgRoute, path: "/", component: OrgDashboardPage });
const orgDocumentsRoute = createRoute({
  getParentRoute: () => orgRoute,
  path: "documents",
  component: DocumentsPage,
  // ?folder=Brand/Logos — the folder being browsed; absent = top level.
  validateSearch: (s: Record<string, unknown>): { folder?: string } => ({
    folder: typeof s.folder === "string" && s.folder ? s.folder : undefined,
  }),
});

const orgStatementsRoute = createRoute({ getParentRoute: () => orgRoute, path: "statements", component: TransactionsPage });
const orgUploadsRoute = createRoute({ getParentRoute: () => orgRoute, path: "statements/uploads", component: UploadsPage });
const orgTypeRulesRoute = createRoute({ getParentRoute: () => orgRoute, path: "statements/rules", component: TypeRulesPage });
const orgAccountsRoute = createRoute({ getParentRoute: () => orgRoute, path: "statements/accounts", component: AccountsPage });
const orgTemplatesRoute = createRoute({ getParentRoute: () => orgRoute, path: "statements/templates", component: TemplatesPage });
const orgTemplateEditorRoute = createRoute({
  getParentRoute: () => orgRoute,
  path: "statements/templates/$template",
  component: TemplateEditorPage,
});
const orgQuickFiltersRoute = createRoute({ getParentRoute: () => orgRoute, path: "statements/filters", component: QuickFiltersPage });
const orgStatementRoute = createRoute({
  getParentRoute: () => orgRoute,
  path: "statements/$upload",
  component: StatementDetailPage,
});

export const router = createRouter({
  routeTree: rootRoute.addChildren([
    indexRoute,
    loginRoute,
    portalRoute.addChildren([
      portalHomeRoute,
      portalSettingsRoute,
      portalOrganisationsRoute,
      portalRolesRoute,
      portalUsersRoute,
      portalAccessRoute,
      orgRoute.addChildren([
        orgDashboardRoute,
        orgDocumentsRoute,
        orgStatementsRoute,
        orgUploadsRoute,
        orgTypeRulesRoute,
        orgAccountsRoute,
        orgQuickFiltersRoute,
        orgTemplatesRoute,
        orgTemplateEditorRoute,
        orgStatementRoute,
      ]),
    ]),
  ]),
  scrollRestoration: true,
});

declare module "@tanstack/solid-router" {
  interface Register {
    router: typeof router;
  }
}
