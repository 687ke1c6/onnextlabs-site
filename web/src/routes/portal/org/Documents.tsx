import { createEffect, createResource, createSignal, For, Show } from "solid-js";
import { Link, useNavigate, useSearch } from "@tanstack/solid-router";
import Check from "lucide-solid/icons/check";
import ChevronRight from "lucide-solid/icons/chevron-right";
import Folder from "lucide-solid/icons/folder";
import FolderOpen from "lucide-solid/icons/folder-open";
import Link2 from "lucide-solid/icons/link-2";
import Upload from "lucide-solid/icons/upload";
import { FileTypeIcon } from "~/components/FileTypeIcon";
import { Button } from "~/components/ui/button";
import { type DocumentItem, downloadUrl, fetchFolder, formatBytes } from "~/lib/documents";
import { settled } from "~/lib/utils";
import { can } from "~/lib/orgs";
import { Allowed, PageHeader } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { UploadDialog } from "~/routes/portal/org/UploadDialog";

const DEFAULT_MAX_BYTES = 10 * 1024 * 1024;

const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const formatDate = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : dateFormat.format(d);
};

const join = (parent: string, name: string) => (parent ? `${parent}/${name}` : name);

function Breadcrumbs(props: { folder: string }) {
  const org = useOrg();
  const crumbs = () => {
    const parts = props.folder ? props.folder.split("/") : [];
    return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join("/") }));
  };
  return (
    <nav aria-label="Folder" class="flex min-w-0 flex-wrap items-center gap-1 text-sm">
      <Link
        to="/portal/$org/documents"
        params={{ org: org.slug }}
        search={{}}
        class="rounded px-1 font-medium text-muted-foreground hover:text-foreground"
        activeOptions={{ includeSearch: true }}
      >
        Documents
      </Link>
      <For each={crumbs()}>
        {(c, i) => (
          <>
            <ChevronRight size={14} class="shrink-0 text-muted-foreground" />
            <Show
              when={i() < crumbs().length - 1}
              fallback={<span class="truncate px-1 font-medium" aria-current="page">{c.name}</span>}
            >
              <Link to="/portal/$org/documents"
        params={{ org: org.slug }} search={{ folder: c.path }} class="truncate rounded px-1 text-muted-foreground hover:text-foreground">
                {c.name}
              </Link>
            </Show>
          </>
        )}
      </For>
    </nav>
  );
}

function CopyLinkButton(props: { doc: DocumentItem }) {
  const [copied, setCopied] = createSignal(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(new URL(downloadUrl(props.doc), location.origin).href);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard blocked; nothing useful to do
    }
  };
  return (
    <button
      type="button"
      onClick={copy}
      class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      aria-label={copied() ? "Link copied" : `Copy download link for ${props.doc.filename}`}
      title={copied() ? "Copied" : "Copy download link"}
    >
      <Show when={copied()} fallback={<Link2 size={16} />}>
        <Check size={16} />
      </Show>
    </button>
  );
}

function DocumentsBrowser() {
  const org = useOrg();
  const search = useSearch({ from: "/portal/$org/documents" });
  const navigate = useNavigate();
  const folder = () => search().folder ?? "";

  // Object source: an empty-string folder (top level) would otherwise count as "don't fetch".
  const [listing, { refetch }] = createResource(() => ({ folder: folder() }), (s) => fetchFolder(org.slug, s.folder));
  const [uploadOpen, setUploadOpen] = createSignal(false);
  const [maxBytes, setMaxBytes] = createSignal(DEFAULT_MAX_BYTES);
  createEffect(() => {
    if (listing.state === "ready") setMaxBytes(settled(listing)!.max_upload_bytes);
  });

  const canUpload = () => can(org, "write-docs");
  const isEmpty = () => settled(listing) && settled(listing)!.folders.length === 0 && settled(listing)!.documents.length === 0;

  const onUploaded = (doc: DocumentItem) => {
    setUploadOpen(false);
    if (doc.folder === folder()) void refetch();
    else void navigate({ to: "/portal/$org/documents", params: { org: org.slug }, search: doc.folder ? { folder: doc.folder } : {} });
  };

  return (
    <>
      <PageHeader
        title="Documents"
        description={`Shared documents and brand assets for ${org.name}. Click a document to download it.`}
        actions={
          <Show when={canUpload()}>
            <Button size="sm" onClick={() => setUploadOpen(true)}>
              <Upload size={16} /> Upload
            </Button>
          </Show>
        }
      />

      <div class="mt-6">
        <Breadcrumbs folder={folder()} />
      </div>

      <Show when={listing.error}>
        <div class="mt-4 rounded-xl border border-border p-6 text-sm">
          <p class="text-red-500">{(listing.error as Error)?.message ?? "Couldn't load this folder."}</p>
          <Link to="/portal/$org/documents"
        params={{ org: org.slug }} search={{}} class="mt-2 inline-block text-muted-foreground underline hover:text-foreground">
            Back to all documents
          </Link>
        </div>
      </Show>

      <Show when={!listing.error}>
        <Show
          when={!isEmpty()}
          fallback={
            <section class="mt-4 flex flex-col items-center justify-center rounded-xl border border-dashed border-border px-6 py-16 text-center">
              <FolderOpen size={32} class="text-muted-foreground" />
              <h2 class="mt-4 text-base font-medium">{folder() ? "This folder is empty" : "No documents yet"}</h2>
              <Show when={canUpload()}>
                <p class="mt-1 max-w-sm text-sm text-muted-foreground">Upload a document to get started.</p>
                <Button size="sm" variant="outline" class="mt-4" onClick={() => setUploadOpen(true)}>
                  <Upload size={16} /> Upload
                </Button>
              </Show>
            </section>
          }
        >
          <section class="mt-4 overflow-hidden rounded-xl border border-border">
            <table class="w-full table-fixed text-sm">
              <thead class="border-b border-border bg-secondary/30 text-left text-xs text-muted-foreground">
                <tr>
                  <th class="px-4 py-2.5 font-medium">Name</th>
                  <th class="hidden w-24 px-4 py-2.5 font-medium sm:table-cell">Size</th>
                  <th class="hidden w-40 px-4 py-2.5 font-medium md:table-cell">Uploaded by</th>
                  <th class="hidden w-32 px-4 py-2.5 font-medium sm:table-cell">Date</th>
                  <th class="w-12 px-2 py-2.5"><span class="sr-only">Actions</span></th>
                </tr>
              </thead>
              <tbody class={listing.loading ? "opacity-60 transition-opacity" : "transition-opacity"}>
                <Show when={!settled(listing) && listing.loading}>
                  <For each={[0, 1, 2]}>
                    {() => (
                      <tr class="border-b border-border last:border-0">
                        <td class="px-4 py-3" colspan={5}><div class="h-4 w-1/3 animate-pulse rounded bg-secondary" /></td>
                      </tr>
                    )}
                  </For>
                </Show>
                <For each={settled(listing)?.folders}>
                  {(name) => (
                    <tr class="border-b border-border last:border-0 hover:bg-accent/40">
                      <td class="px-4 py-0" colspan={5}>
                        <Link
                          to="/portal/$org/documents"
        params={{ org: org.slug }}
                          search={{ folder: join(folder(), name) }}
                          class="flex items-center gap-3 py-3 font-medium focus-visible:outline-none focus-visible:underline"
                        >
                          <Folder size={18} class="shrink-0 text-muted-foreground" />
                          <span class="truncate">{name}</span>
                        </Link>
                      </td>
                    </tr>
                  )}
                </For>
                <For each={settled(listing)?.documents}>
                  {(doc) => (
                    <tr class="border-b border-border last:border-0 hover:bg-accent/40">
                      <td class="px-4 py-0">
                        <a
                          href={downloadUrl(doc)}
                          download={doc.filename}
                          class="flex min-w-0 items-center gap-3 py-3 focus-visible:outline-none focus-visible:underline"
                          title={`Download ${doc.filename}`}
                        >
                          <FileTypeIcon filename={doc.filename} contentType={doc.content_type} />
                          <span class="truncate">{doc.filename}</span>
                        </a>
                      </td>
                      <td class="hidden px-4 py-3 tabular-nums text-muted-foreground sm:table-cell">{formatBytes(doc.size)}</td>
                      <td class="hidden truncate px-4 py-3 text-muted-foreground md:table-cell">{doc.uploaded_by ?? "—"}</td>
                      <td class="hidden px-4 py-3 text-muted-foreground sm:table-cell">{formatDate(doc.uploaded_at)}</td>
                      <td class="px-2 py-1.5"><CopyLinkButton doc={doc} /></td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </section>
        </Show>
      </Show>

      <UploadDialog
        open={uploadOpen()}
        org={org.slug}
        initialFolder={folder()}
        maxBytes={maxBytes()}
        onClose={() => setUploadOpen(false)}
        onUploaded={onUploaded}
      />
    </>
  );
}

/** Needs `read-docs` in the org; uploading needs `write-docs`. The API enforces both. */
export function DocumentsPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-docs")}>
      <DocumentsBrowser />
    </Allowed>
  );
}
