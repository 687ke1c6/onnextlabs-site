import { createContext, Show, useContext } from "solid-js";
import { Outlet, useParams } from "@tanstack/solid-router";
import type { Org } from "~/lib/orgs";
import { Notice, usePortal } from "~/routes/portal/Layout";

const OrgContext = createContext<Org>();

/** The org whose pages are being shown (from /portal/<slug>/...). */
export function useOrg(): Org {
  const org = useContext(OrgContext);
  if (!org) throw new Error("useOrg() called outside OrgLayout");
  return org;
}

/**
 * Wraps /portal/<slug>/*: resolves the slug against the orgs this user can open. Unknown
 * slugs and orgs they aren't in look the same, so neither reveals that an org exists.
 */
export function OrgLayout() {
  const params = useParams({ from: "/portal/$org" });
  const portal = usePortal();
  const org = () => portal.orgs().find((o) => o.slug === params().org);

  return (
    // keyed: switching orgs remounts the pages, so nothing from the previous org lingers.
    <Show
      when={org()}
      keyed
      fallback={<Notice title="Organisation not found" body="It doesn't exist, or you haven't been added to it." />}
    >
      {(o) => (
        <OrgContext.Provider value={o}>
          <Outlet />
        </OrgContext.Provider>
      )}
    </Show>
  );
}
