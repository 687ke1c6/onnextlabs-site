import { createEffect, createResource, createSignal, For, on, Show } from "solid-js";
import { Link, useNavigate } from "@tanstack/solid-router";
import Copy from "lucide-solid/icons/copy";
import Loader2 from "lucide-solid/icons/loader-circle";
import Pencil from "lucide-solid/icons/pencil";
import Plus from "lucide-solid/icons/plus";
import Trash from "lucide-solid/icons/trash";
import X from "lucide-solid/icons/x";
import { DeleteDialog } from "~/components/DeleteDialog";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { can } from "~/lib/orgs";
import { createTemplate, deleteTemplate, fetchTemplates, formatDate, type Template } from "~/lib/statements";
import { cn, settled } from "~/lib/utils";
import { Allowed } from "~/routes/portal/Layout";
import { useOrg } from "~/routes/portal/org/OrgLayout";
import { type AccountState, StatementsShell } from "~/routes/portal/org/StatementsShell";

const errorText = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);
const ICON_BUTTON =
  "flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

/** A name (and a layout, for a new one) for a new template or a copy. */
function NewTemplateDialog(props: {
  /** null = closed; "new" = from a layout; a template = a copy of it. */
  from: Template | "new" | null;
  layouts: { name: string; label: string }[];
  onCreated: (id: number) => void;
  onClose: () => void;
}) {
  const org = useOrg();
  let dialog!: HTMLDialogElement;
  const [name, setName] = createSignal("");
  const [layout, setLayout] = createSignal("standard");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  createEffect(
    on(
      () => props.from,
      (from) => {
        setError(null);
        setName(from && from !== "new" ? `${from.name} (copy)` : "");
        setLayout("standard");
        if (from) dialog.showModal();
        else if (dialog.open) dialog.close();
      },
    ),
  );
  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    if (!name().trim() || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const from = props.from;
      const t = await createTemplate(org.slug, from === "new" ? { name: name(), layout: layout() } : { name: name(), copy_of: from!.id });
      props.onCreated(t.id);
    } catch (err) {
      setError(errorText(err, "Couldn't create the template."));
    } finally {
      setBusy(false);
    }
  };
  return (
    <dialog
      ref={dialog}
      onClose={() => props.from && props.onClose()}
      class="m-auto w-[calc(100%-2rem)] max-w-md rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="new-template-title"
    >
      <form onSubmit={(e) => void submit(e)} class="flex flex-col gap-4 p-6">
        <div class="flex items-start justify-between gap-4">
          <h2 id="new-template-title" class="text-lg font-semibold tracking-tight">
            {props.from === "new" ? "New template" : `Copy ${props.from ? (props.from as Template).name : ""}`}
          </h2>
          <button
            type="button"
            onClick={() => dialog.close()}
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            aria-label="Close"
          >
            <X size={18} />
          </button>
        </div>
        <label class="flex flex-col gap-1.5 text-sm">
          <span class="text-muted-foreground">Name</span>
          <Input value={name()} onInput={(e) => setName(e.currentTarget.value)} maxLength={100} required placeholder="e.g. Bank style" />
        </label>
        <Show when={props.from === "new"}>
          <label class="flex flex-col gap-1.5 text-sm">
            <span class="text-muted-foreground">Layout</span>
            <select
              value={layout()}
              onChange={(e) => setLayout(e.currentTarget.value)}
              class="h-9 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <For each={props.layouts}>{(l) => <option value={l.name}>{l.label}</option>}</For>
            </select>
            <span class="text-xs text-muted-foreground">It starts with that layout's usual header fields and options.</span>
          </label>
        </Show>
        <Show when={error()}>
          <p role="alert" class="text-sm text-red-500">{error()}</p>
        </Show>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={() => dialog.close()} disabled={busy()}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy() || !name().trim()}>
            <Show when={busy()}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            Create
          </Button>
        </div>
      </form>
    </dialog>
  );
}

function TemplatesList(props: { state: AccountState }) {
  const org = useOrg();
  const editable = can(org, "write-statements");
  const navigate = useNavigate();
  const [list, { refetch }] = createResource(() => org.slug, fetchTemplates);
  const open = (id: number) => void navigate({ to: "/portal/$org/statements/templates/$template", params: { org: org.slug, template: String(id) } });
  const [creating, setCreating] = createSignal<Template | "new" | null>(null);
  const [deleting, setDeleting] = createSignal<Template | null>(null);
  const templates = () => settled(list)?.templates ?? [];
  const layoutLabel = (name: string) => settled(list)?.layouts.find((l) => l.name === name)?.label ?? name;

  return (
    <section class="mt-6 flex flex-col gap-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <p class="max-w-2xl text-sm text-muted-foreground">
          How statements print as PDF. Templates belong to the organisation: print any bank account with any of them, picking one when you
          print from the Statement grid. Each bank account remembers the last one used.
        </p>
        <Show when={editable}>
          <Button size="sm" onClick={() => setCreating("new")}>
            <Plus size={16} /> New template
          </Button>
        </Show>
      </div>

      <Show when={list.error}>
        <p class="text-sm text-red-500">{errorText(list.error, "Couldn't load the templates.")}</p>
      </Show>

      <div class="overflow-x-auto rounded-xl border border-border">
        <table class="w-full min-w-[36rem] text-sm">
          <thead class="border-b border-border bg-secondary/30 text-left text-xs text-muted-foreground">
            <tr>
              <th class="px-4 py-2.5 font-medium">Name</th>
              <th class="px-4 py-2.5 font-medium">Layout</th>
              <th class="px-4 py-2.5 font-medium">Changed</th>
              <th class="w-32 px-2 py-2.5">
                <span class="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <For each={templates()}>
              {(t) => (
                <tr class="border-b border-border last:border-0 hover:bg-accent/30">
                  <td class="px-4 py-2.5">
                    <Link
                      to="/portal/$org/statements/templates/$template"
                      params={{ org: org.slug, template: String(t.id) }}
                      class="font-medium underline-offset-2 hover:underline"
                    >
                      {t.name}
                    </Link>
                  </td>
                  <td class="px-4 py-2.5">{layoutLabel(t.layout)}</td>
                  <td class="px-4 py-2.5 text-muted-foreground">{formatDate(t.updated_at)}</td>
                  <td class="px-2 py-1.5">
                    <div class="flex justify-end gap-1">
                      <button
                        type="button"
                        class={cn(ICON_BUTTON, "hover:text-accent-foreground")}
                        onClick={() => open(t.id)}
                        aria-label={`${editable ? "Edit" : "View"} ${t.name}`}
                        title={editable ? "Edit" : "View"}
                      >
                        <Pencil size={14} />
                      </button>
                      <Show when={editable}>
                        <button
                          type="button"
                          class={cn(ICON_BUTTON, "hover:text-accent-foreground")}
                          onClick={() => setCreating(t)}
                          aria-label={`Copy ${t.name}`}
                          title="Copy"
                        >
                          <Copy size={14} />
                        </button>
                        <button
                          type="button"
                          class={cn(ICON_BUTTON, "hover:text-red-500")}
                          onClick={() => setDeleting(t)}
                          aria-label={`Delete ${t.name}`}
                          title="Delete"
                        >
                          <Trash size={14} />
                        </button>
                      </Show>
                    </div>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      <NewTemplateDialog
        from={creating()}
        layouts={settled(list)?.layouts ?? []}
        onCreated={(id) => {
          setCreating(null);
          open(id);
        }}
        onClose={() => setCreating(null)}
      />

      <DeleteDialog
        open={deleting() !== null}
        title={`Delete ${deleting()?.name ?? "template"}?`}
        note={<>Statements already printed with it aren't affected. If it's the last template, a new "Standard" one takes its place.</>}
        load={async () => {
          const id = deleting()!.id;
          const count = (settled(props.state.accounts) ?? []).filter((a) => a.last_template_id === id).length;
          return {
            blocked: false,
            effects: count ? [{ table: "statement_accounts", label: "bank accounts that last printed with it", action: "unlink" as const, count }] : [],
          };
        }}
        onConfirm={async () => {
          await deleteTemplate(org.slug, deleting()!.id);
          setDeleting(null);
          void refetch();
          props.state.reload();
        }}
        onClose={() => setDeleting(null)}
      />
    </section>
  );
}

/** /portal/<org>/statements/templates. Viewing needs `read-statements`; changes `write-statements`. */
export function TemplatesPage() {
  const org = useOrg();
  return (
    <Allowed when={can(org, "read-statements")}>
      <StatementsShell>{(state) => <TemplatesList state={state} />}</StatementsShell>
    </Allowed>
  );
}
