import { json, send } from "~/lib/orgs";

/** A row as parsed (preview) or stored. `txn_id` is the prod n8n id. */
export type Transaction = {
  txn_id: number;
  date: string;
  amount: string;
  description: string;
  currency: string;
  type: string | null;
};

export type PreviewRow = Transaction & {
  /** Already stored for this org and account: saving updates it. */
  stored: boolean;
};

export type Preview = {
  filename: string;
  format: string;
  format_name: string;
  account_number: string | null;
  first_date: string | null;
  last_date: string | null;
  rows: number;
  typed: number;
  new: number;
  existing: number;
  transactions: PreviewRow[];
  /** Where the copy of the file is saved in Documents... */
  documents_folder: string;
  /** ...unless this exact file is already there, in which case that document is used. */
  existing_document: { id: number; folder: string; filename: string } | null;
};

export type UploadSummary = {
  id: number;
  format: string;
  /** "" when the format doesn't state one. */
  account_number: string;
  filename: string;
  rows_parsed: number;
  rows_new: number;
  rows_updated: number;
  document_id: number | null;
  uploaded_at: string;
  uploaded_by: string | null;
  first_date: string | null;
  last_date: string | null;
};

export type UploadDetail = UploadSummary & { format_name: string; transactions: Transaction[] };

export type Rule = {
  type: string;
  /** Matches when the description contains any of these, ignoring case. */
  contains: string[];
  min_amount: number | null;
  max_amount: number | null;
  /** Format ids; empty = every format. */
  formats: string[];
};

export type FormatInfo = { id: string; name: string };

const base = (org: string) => `/api/orgs/${encodeURIComponent(org)}`;

function form(file: File) {
  const body = new FormData();
  body.append("file", file);
  return body;
}

/** Parses a statement without saving anything. */
export const previewStatement = async (org: string, file: File) =>
  json<Preview>(
    await fetch(`${base(org)}/statements/preview`, { method: "POST", body: form(file) }),
    "Couldn't read that statement.",
  );

/** Parses and saves it; the file itself is kept in Documents (Statements/<account>). */
export const saveStatement = async (org: string, file: File) =>
  json<UploadDetail>(
    await fetch(`${base(org)}/statements`, { method: "POST", body: form(file) }),
    "Couldn't save the statement.",
  );

/** `?account=` for list endpoints; "" (statements stating no account) is a real value. */
const accountQuery = (account: string) => `?account=${encodeURIComponent(account)}`;

export const fetchUploads = async (org: string, account: string) =>
  json<UploadSummary[]>(await fetch(`${base(org)}/statements${accountQuery(account)}`), "Couldn't load statements.");

/** An account the org has statements for. */
export type AccountSummary = {
  /** "" when the statements don't state one. */
  account_number: string;
  uploads: number;
  rows: number;
  last_uploaded_at: string;
};

/** Accounts with statements, most recently uploaded first. */
export const fetchAccounts = async (org: string) =>
  json<AccountSummary[]>(await fetch(`${base(org)}/statement-accounts`), "Couldn't load accounts.");

export const accountLabel = (account: string) => account || "No account stated";

export const fetchUpload = async (org: string, id: number) =>
  json<UploadDetail>(await fetch(`${base(org)}/statements/${id}`), "Couldn't load this statement.");

export const deleteUpload = async (org: string, id: number) =>
  json<void>(await send("DELETE", `${base(org)}/statements/${id}`), "Couldn't delete the statement.");

export const fetchRules = async (org: string) =>
  json<{ rules: Rule[]; formats: FormatInfo[] }>(await fetch(`${base(org)}/statement-rules`), "Couldn't load the type rules.");

export const saveRules = async (org: string, rules: Rule[]) =>
  json<{ rules: Rule[]; formats: FormatInfo[] }>(
    await send("PUT", `${base(org)}/statement-rules`, { rules }),
    "Couldn't save the type rules.",
  );

/** Which of `rules` (saved or not) would set the type of this description and amount. */
export const testRules = async (org: string, rules: Rule[], description: string, amount: string) =>
  json<{ rule: number | null; type: string | null }>(
    await send("POST", `${base(org)}/statement-rules/test`, { rules, description, amount }),
    "Couldn't test the rules.",
  );

// ---- display helpers ----

const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });

export function formatDate(iso: string | null): string {
  if (!iso) return "—";
  // A bare YYYY-MM-DD is a calendar date: read it as local, or west of UTC it shows a day early.
  const day = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  const d = day ? new Date(+day[1], +day[2] - 1, +day[3]) : new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : dateFormat.format(d);
}

/** "1 Mar 2027 – 31 May 2027" for a statement's first and last transaction. */
export const formatPeriod = (first: string | null, last: string | null) =>
  first && last ? (first === last ? formatDate(first) : `${formatDate(first)} – ${formatDate(last)}`) : "—";

/** Amounts are kept exactly as in the file; negatives get a proper minus sign. */
export const formatAmount = (amount: string) => (amount.startsWith("-") ? `−${amount.slice(1)}` : amount);

export const isNegative = (amount: string) => amount.trim().startsWith("-");

/** A stored row in the org, with the most recent upload that contained it. */
export type OrgTransaction = Transaction & {
  /** The row's own id (for editing); `txn_id` is the n8n id. */
  id: number;
  /** The type was picked by hand: saving a statement won't change it. */
  type_manual: boolean;
  /** Documents attached to the row. */
  attachments: number;
  /** e.g. "Aug 2024": derived from `date`, as in the sheet's `month` column. */
  month: string;
  account_number: string;
  updated_at: string;
  upload_id: number | null;
  upload_filename: string | null;
};

/** Every stored row for one account in the org, newest first. */
export const fetchOrgTransactions = async (org: string, account: string) =>
  json<OrgTransaction[]>(await fetch(`${base(org)}/statement-transactions${accountQuery(account)}`), "Couldn't load transactions.");

/** The amount as a number (the stored string is kept exactly as in the file). NaN if unreadable. */
export const amountValue = (amount: string) => Number(amount.replace(/,/g, "").trim());

// ---- types ----

/** A type on the org's list, with how many rows and rules use it. */
export type TypeInfo = { id: number; name: string; rows: number; rules: number };

const types = (org: string) => `${base(org)}/statement-types`;

export const fetchTypes = async (org: string) => json<TypeInfo[]>(await fetch(types(org)), "Couldn't load the types.");

export const createType = async (org: string, name: string) =>
  json<TypeInfo[]>(await send("POST", types(org), { name }), "Couldn't add the type.");

/** Renames it on every row and rule too. */
export const renameType = async (org: string, id: number, name: string) =>
  json<TypeInfo[]>(await send("PATCH", `${types(org)}/${id}`, { name }), "Couldn't rename the type.");

/** Refused while a row or rule uses it. */
export const deleteType = async (org: string, id: number) =>
  json<TypeInfo[]>(await send("DELETE", `${types(org)}/${id}`), "Couldn't delete the type.");

/** Sets a row's type by hand (or clears it with null, so saved statements can type it again). */
export const setRowType = async (org: string, row: number, type: string | null) =>
  json<{ type: string | null; type_manual: boolean }>(
    await send("PATCH", `${base(org)}/statement-transactions/${row}`, { type }),
    "Couldn't change the type.",
  );

