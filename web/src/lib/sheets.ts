import { send } from "~/lib/orgs";

export type SheetsStatus = { available: boolean; connected: boolean; email: string | null };
export type Spreadsheet = { id: string; name: string; modified_time: string };
export type SheetTab = { sheet_id: number; title: string };
export type SpreadsheetDetails = { id: string; title: string; tabs: SheetTab[] };
export type SendResult = {
  spreadsheet_title: string;
  tab: string;
  url: string;
  parsed: number;
  existing: number;
  appended: number;
  updated: number;
  wrote_header: boolean;
  unreadable_ids: number;
};

/** Thrown when the Google connection is missing or has expired, so the page can offer to reconnect. */
export class NotConnected extends Error {}

async function json<T>(res: Response, fallback: string): Promise<T> {
  if (res.ok) return res.status === 204 ? (undefined as T) : res.json();
  let msg = fallback;
  try {
    const body = await res.json();
    if (typeof body?.error === "string") msg = body.error;
  } catch {
    // not JSON
  }
  throw res.status === 401 ? new NotConnected(msg) : new Error(msg);
}

const base = "/api/statements/google";

export const fetchSheetsStatus = async () => json<SheetsStatus>(await fetch(base), "Couldn't check Google Sheets.");

/** Where the browser goes to connect; Google sends it back to `returnTo`. */
export const connectUrl = (returnTo: string) => `${base}/connect?return_to=${encodeURIComponent(returnTo)}`;

export const disconnectSheets = async () => json<void>(await send("POST", `${base}/disconnect`), "Couldn't disconnect.");

export const fetchSpreadsheets = async () =>
  json<Spreadsheet[]>(await fetch(`${base}/spreadsheets`), "Couldn't list your spreadsheets.");

export const fetchSpreadsheet = async (id: string) =>
  json<SpreadsheetDetails>(await fetch(`${base}/spreadsheets/${encodeURIComponent(id)}`), "Couldn't open that spreadsheet.");

export const createSpreadsheet = async (name: string, tab: string) =>
  json<SpreadsheetDetails>(await send("POST", `${base}/spreadsheets`, { name, tab }), "Couldn't create the spreadsheet.");

export const createTab = async (id: string, title: string) =>
  json<SheetTab>(await send("POST", `${base}/spreadsheets/${encodeURIComponent(id)}/tabs`, { title }), "Couldn't add the tab.");

export const sendToSheet = async (org: string, upload: number, spreadsheetId: string, tab: string) =>
  json<SendResult>(
    await send("POST", `/api/orgs/${encodeURIComponent(org)}/statements/${upload}/sheet`, { spreadsheet_id: spreadsheetId, tab }),
    "Couldn't send the statement to Google Sheets.",
  );

const connectErrors: Record<string, string> = {
  denied: "Google Sheets wasn't connected: the consent screen was cancelled.",
  scopes: "Google Sheets wasn't connected: tick both permissions on Google's consent screen.",
  expired: "That took too long. Try connecting again.",
  google: "Google didn't accept the connection. Try again.",
};

export const connectErrorText = (code: string) => connectErrors[code] ?? "Google Sheets wasn't connected. Try again.";

// The last spreadsheet and tab used, per browser: most people always send to the same place.
const LAST_KEY = "statements-sheet-target";

export function lastTarget(): { spreadsheetId: string; tab: string } | null {
  try {
    const v = JSON.parse(localStorage.getItem(LAST_KEY) ?? "null");
    return v && typeof v.spreadsheetId === "string" && typeof v.tab === "string" ? v : null;
  } catch {
    return null;
  }
}

export function rememberTarget(spreadsheetId: string, tab: string) {
  try {
    localStorage.setItem(LAST_KEY, JSON.stringify({ spreadsheetId, tab }));
  } catch {
    // storage blocked: just don't remember
  }
}
