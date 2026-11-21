import { createEffect, createResource, createSignal, For, on, onMount, Show } from "solid-js";
import ExternalLink from "lucide-solid/icons/external-link";
import Loader2 from "lucide-solid/icons/loader-circle";
import Plus from "lucide-solid/icons/plus";
import Sheet from "lucide-solid/icons/sheet";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import {
  connectErrorText,
  connectUrl,
  createSpreadsheet,
  createTab,
  disconnectSheets,
  fetchSheetsStatus,
  fetchSpreadsheet,
  fetchSpreadsheets,
  lastTarget,
  NotConnected,
  rememberTarget,
  type SendResult,
  sendToSheet,
  type SpreadsheetDetails,
} from "~/lib/sheets";
import { settled } from "~/lib/utils";

const SELECT =
  "h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50";

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Sends a saved statement to the user's own Google Sheet. */
export function GoogleSheetPanel(props: { org: string; upload: number }) {
  const [status, { refetch: recheck }] = createResource(fetchSheetsStatus);
  const [message, setMessage] = createSignal<{ ok: boolean; text: string } | null>(null);

  // Back from Google's consent screen: say how it went, then tidy the URL.
  onMount(() => {
    const params = new URLSearchParams(location.search);
    const ok = params.get("google");
    const err = params.get("google_error");
    if (ok === "connected") setMessage({ ok: true, text: "Google Sheets connected." });
    else if (err) setMessage({ ok: false, text: connectErrorText(err) });
    if (ok || err) {
      params.delete("google");
      params.delete("google_error");
      const q = params.toString();
      history.replaceState(history.state, "", location.pathname + (q ? `?${q}` : ""));
    }
  });

  const disconnect = async () => {
    await disconnectSheets().catch(() => undefined);
    setMessage(null);
    void recheck();
  };

  return (
    <section class="mt-8 rounded-xl border border-border p-5">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <Sheet size={18} class="text-muted-foreground" />
          <h2 class="text-base font-medium">Google Sheets</h2>
        </div>
        <Show when={settled(status)?.connected}>
          <p class="text-xs text-muted-foreground">
            Connected as {settled(status)!.email ?? "your Google account"} ·{" "}
            <button type="button" class="underline hover:text-foreground" onClick={disconnect}>
              Disconnect
            </button>
          </p>
        </Show>
      </div>
      <p class="mt-1 text-sm text-muted-foreground">
        Sends every row of this statement to a tab in your own spreadsheet. Rows already in the tab (same id) get their type updated;
        the rest are added.
      </p>

      <Show when={message()}>
        {(m) => (
          <p role="status" class={m().ok ? "mt-3 text-sm text-emerald-600 dark:text-emerald-400" : "mt-3 text-sm text-red-500"}>
            {m().text}
          </p>
        )}
      </Show>

      <Show when={status.error}>
        <p class="mt-3 text-sm text-red-500">Couldn't check the Google Sheets connection.</p>
      </Show>

      <Show when={settled(status)}>
        {(s) => (
          <Show
            when={s().available}
            fallback={<p class="mt-3 text-sm text-muted-foreground">Google Sheets isn't set up on this site.</p>}
          >
            <Show
              when={s().connected}
              fallback={
                <div class="mt-4">
                  <a
                    href={connectUrl(location.pathname)}
                    class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                  >
                    Connect Google Sheets
                  </a>
                  <p class="mt-2 text-xs text-muted-foreground">
                    Google asks to let this site see your spreadsheets' names and edit spreadsheets. The connection lasts about an hour
                    and is never stored.
                  </p>
                </div>
              }
            >
              <Pickers
                org={props.org}
                upload={props.upload}
                onDisconnected={() => {
                  setMessage({ ok: false, text: "The Google connection has expired. Connect again to continue." });
                  void recheck();
                }}
              />
            </Show>
          </Show>
        )}
      </Show>
    </section>
  );
}

function Pickers(props: { org: string; upload: number; onDisconnected: () => void }) {
  const remembered = lastTarget();
  const [spreadsheets, { mutate: setSpreadsheets }] = createResource(fetchSpreadsheets);
  const [spreadsheetId, setSpreadsheetId] = createSignal(remembered?.spreadsheetId ?? "");
  const [details, { mutate: setDetails }] = createResource(
    () => spreadsheetId() || undefined,
    fetchSpreadsheet,
  );
  const [tab, setTab] = createSignal(remembered?.tab ?? "");
  const [creating, setCreating] = createSignal<"spreadsheet" | "tab" | null>(null);
  const [newName, setNewName] = createSignal("");
  const [newTab, setNewTab] = createSignal("Transactions");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [result, setResult] = createSignal<SendResult | null>(null);

  const fail = (e: unknown, fallback: string) => {
    if (e instanceof NotConnected) return props.onDisconnected();
    setError(e instanceof Error ? e.message : fallback);
  };

  createEffect(() => {
    for (const r of [spreadsheets, details]) if (r.error) fail(r.error, "Google didn't answer.");
  });

  // A remembered spreadsheet that's gone (or the user can no longer see) is just forgotten.
  createEffect(() => {
    const list = settled(spreadsheets);
    if (list && spreadsheetId() && !list.some((s) => s.id === spreadsheetId())) setSpreadsheetId("");
  });
  // Keep the chosen tab only if the spreadsheet has it; otherwise take its first tab.
  createEffect(
    on(
      () => settled(details),
      (d) => {
        if (d && !d.tabs.some((t) => t.title === tab())) setTab(d.tabs[0]?.title ?? "");
      },
    ),
  );

  const run = async (fn: () => Promise<void>, fallback: string) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      fail(e, fallback);
    } finally {
      setBusy(false);
    }
  };

  const addSpreadsheet = (e: SubmitEvent) => {
    e.preventDefault();
    void run(async () => {
      const d: SpreadsheetDetails = await createSpreadsheet(newName(), newTab());
      setSpreadsheets((list) => [{ id: d.id, name: d.title, modified_time: new Date().toISOString() }, ...(list ?? [])]);
      setSpreadsheetId(d.id);
      setDetails(d);
      setTab(d.tabs[0]?.title ?? "");
      setCreating(null);
      setNewName("");
    }, "Couldn't create the spreadsheet.");
  };

  const addTab = (e: SubmitEvent) => {
    e.preventDefault();
    void run(async () => {
      const t = await createTab(spreadsheetId(), newTab());
      setDetails((d) => (d ? { ...d, tabs: [...d.tabs, t] } : d));
      setTab(t.title);
      setCreating(null);
    }, "Couldn't add the tab.");
  };

  const sendIt = () =>
    void run(async () => {
      setResult(null);
      const r = await sendToSheet(props.org, props.upload, spreadsheetId(), tab());
      rememberTarget(spreadsheetId(), tab());
      setResult(r);
    }, "Couldn't send the statement.");

  return (
    <div class="mt-4 flex flex-col gap-3">
      <div class="flex flex-col gap-1.5">
        <label for="sheet-spreadsheet" class="text-xs text-muted-foreground">Spreadsheet</label>
        <div class="flex gap-2">
          <select
            id="sheet-spreadsheet"
            class={SELECT}
            value={(settled(spreadsheets), spreadsheetId())}
            onChange={(e) => {
              setResult(null);
              setSpreadsheetId(e.currentTarget.value);
            }}
            disabled={busy() || spreadsheets.loading}
          >
            <option value="">{spreadsheets.loading ? "Loading your spreadsheets…" : "Choose a spreadsheet"}</option>
            <For each={settled(spreadsheets)}>{(s) => <option value={s.id}>{s.name}</option>}</For>
          </select>
          <Button variant="outline" size="sm" onClick={() => setCreating("spreadsheet")} disabled={busy()}>
            <Plus size={16} /> New
          </Button>
        </div>
      </div>

      <Show when={creating() === "spreadsheet"}>
        <form onSubmit={addSpreadsheet} class="flex flex-wrap gap-2 rounded-lg bg-secondary/20 p-3">
          <Input class="h-9 min-w-0 flex-[2_1_12rem]" value={newName()} onInput={(e) => setNewName(e.currentTarget.value)} placeholder="Spreadsheet name" aria-label="New spreadsheet name" />
          <Input class="h-9 min-w-0 flex-[1_1_8rem]" value={newTab()} onInput={(e) => setNewTab(e.currentTarget.value)} placeholder="First tab" aria-label="First tab name" />
          <Button type="submit" size="sm" disabled={busy() || !newName().trim() || !newTab().trim()}>Create</Button>
          <Button type="button" variant="ghost" size="sm" onClick={() => setCreating(null)}>Cancel</Button>
        </form>
      </Show>

      <Show when={spreadsheetId()}>
        <div class="flex flex-col gap-1.5">
          <label for="sheet-tab" class="text-xs text-muted-foreground">Tab</label>
          <div class="flex gap-2">
            <select
              id="sheet-tab"
              class={SELECT}
              value={(settled(details), tab())}
              onChange={(e) => {
                setResult(null);
                setTab(e.currentTarget.value);
              }}
              disabled={busy() || details.loading}
            >
              <Show when={details.loading}>
                <option value="">Loading tabs…</option>
              </Show>
              <For each={settled(details)?.tabs}>{(t) => <option value={t.title}>{t.title}</option>}</For>
            </select>
            <Button variant="outline" size="sm" onClick={() => setCreating("tab")} disabled={busy() || !settled(details)}>
              <Plus size={16} /> New
            </Button>
          </div>
        </div>
      </Show>

      <Show when={creating() === "tab"}>
        <form onSubmit={addTab} class="flex flex-wrap gap-2 rounded-lg bg-secondary/20 p-3">
          <Input class="h-9 min-w-0 flex-[1_1_12rem]" value={newTab()} onInput={(e) => setNewTab(e.currentTarget.value)} placeholder="Tab name" aria-label="New tab name" />
          <Button type="submit" size="sm" disabled={busy() || !newTab().trim()}>Add tab</Button>
          <Button type="button" variant="ghost" size="sm" onClick={() => setCreating(null)}>Cancel</Button>
        </form>
      </Show>

      <div class="flex flex-wrap items-center gap-3">
        <Button onClick={sendIt} disabled={busy() || !spreadsheetId() || !tab() || details.loading}>
          <Show when={busy()}>
            <Loader2 size={16} class="animate-spin" />
          </Show>
          Send to sheet
        </Button>
        <Show when={error()}>
          <p role="alert" class="text-sm text-red-500">{error()}</p>
        </Show>
      </div>

      <Show when={result()}>
        {(r) => (
          <div role="status" class="rounded-lg bg-emerald-500/10 p-3 text-sm">
            <p>
              Sent {plural(r().parsed, "row", "rows")} to <span class="font-medium">{r().spreadsheet_title}</span> › {r().tab}:{" "}
              {plural(r().appended, "added", "added")}
              {", "}
              {plural(r().updated, "type updated", "types updated")}
              <Show when={r().existing - r().updated > 0}>
                {", "}
                {r().existing - r().updated} left as they were
              </Show>
              .
              <Show when={r().wrote_header}> The header row was written first.</Show>
            </p>
            <Show when={r().unreadable_ids > 0}>
              <p class="mt-1 text-xs text-muted-foreground">
                {plural(r().unreadable_ids, "cell", "cells")} in the id column aren't ids and were ignored.
              </p>
            </Show>
            <a href={r().url} target="_blank" rel="noopener noreferrer" class="mt-2 inline-flex items-center gap-1 text-sm underline">
              Open in Google Sheets <ExternalLink size={14} />
            </a>
          </div>
        )}
      </Show>
    </div>
  );
}
