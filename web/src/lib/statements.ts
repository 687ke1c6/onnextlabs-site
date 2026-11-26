import { type Accessor, createRoot, createSignal, type Signal } from "solid-js";
import { json, send } from "~/lib/orgs";
import type { DeleteImpact } from "~/lib/deleteImpact";

/** A row as parsed (preview) or stored. `txn_id` is the row id: a hash of its date, amount, description and occurrence. */
export type Transaction = {
  txn_id: number;
  date: string;
  amount: string;
  description: string;
  currency: string;
  type: string | null;
  /** YYYY-MM-DD: the date the row counts towards when it isn't `date` (set by hand, e.g. in a Google Sheet). */
  effective_date: string | null;
};

/**
 * A file that states its rows' ids (a Google Sheet export): how those ids compare with the rows.
 * Rows always get new ids computed from them; this checks the import.
 */
export type IdCheck = {
  /** Already the row's id (a sheet this app wrote). */
  current: number;
  /** The old n8n id of the row as written. */
  n8n_matched: number;
  /** The old n8n id of the row with the amount written another way (Sheets reformatted it). */
  n8n_reformatted: number;
  /** Neither: the row was edited after its id was made (1-based file lines). */
  mismatched: { line: number; id: number }[];
  /** Lines with a blank id: rows added to the sheet by hand (imported as they are; deletable). */
  no_id: number[];
};

export type PreviewRow = Transaction & {
  /** Already stored for this org and account: saving updates it. */
  stored: boolean;
  /** The type shown is hand-set (a stored hand-set type stays when saving). */
  type_manual: boolean;
};

/** An account statements are uploaded into. */
export type Account = {
  id: number;
  name: string;
  identifier: string | null;
  /** What statement PDFs show about the account. */
  holder_name: string | null;
  product_name: string | null;
  /** The balance at the start of `opening_balance_date` (both or neither), which statement balances are worked out from. */
  opening_balance_cents: number | null;
  opening_balance_date: string | null;
  /** The statement template it was last printed with. */
  last_template_id: number | null;
};

/** An account's statement details, as typed (blank = none). */
export type AccountDetails = {
  holder_name: string;
  product_name: string;
  /** e.g. `40,957.20` or `-1200` */
  opening_balance: string;
  /** YYYY-MM-DD */
  opening_balance_date: string;
};

/** An account, with what's in it. */
export type AccountSummary = Account & {
  uploads: number;
  rows: number;
  /** When a statement was last saved into it. */
  last_uploaded_at: string | null;
};

/** How the account number a statement states compares with the account's identifier. */
export type AccountCheck = "match" | "mismatch" | "no_identifier" | "not_stated";

/** A line of a statement an upload exclusion leaves out. */
export type ExcludedRow = {
  txn_id: number;
  date: string;
  amount: string;
  description: string;
  currency: string;
  /** Saved before the exclusion existed: it stays until deleted (the exclusion's ▶ lists and deletes them). */
  stored: boolean;
};

export type Preview = {
  filename: string;
  format: string;
  format_name: string;
  /** The account number the file states, if its format states one. */
  account_number: string | null;
  /** The account it would be saved into. */
  account: Account;
  account_check: AccountCheck;
  /** On a mismatch or no_identifier: another account whose identifier is the stated number, if any. */
  suggested_account: Account | null;
  first_date: string | null;
  last_date: string | null;
  rows: number;
  typed: number;
  new: number;
  existing: number;
  transactions: PreviewRow[];
  /** Lines the account's upload exclusions leave out: never saved. */
  excluded: ExcludedRow[];
  id_check: IdCheck | null;
  /** Types the file gives that aren't on the list yet; saving adds them. */
  new_types: string[];
  /** Where the copy of the file is saved in Documents... */
  documents_folder: string;
  /** ...unless this exact file is already there, in which case that document is used. */
  existing_document: { id: number; folder: string; filename: string } | null;
};

export type UploadSummary = {
  id: number;
  format: string;
  /** e.g. "Standard Bank PDF statement (2026-01 layout)". */
  format_name: string;
  /** The account it was saved into. */
  account_id: number;
  account_name: string;
  /** The account number the file itself states ("" if none). */
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

export type UploadDetail = UploadSummary & { transactions: Transaction[] };

/** A statement format uploads can be in. */
export type StatementFormat = { id: string; name: string; pdf: boolean };

export const fetchFormats = async () => json<StatementFormat[]>(await fetch("/api/statement-formats"), "Couldn't load the statement formats.");

/** A type rule; it belongs to an account. */
export type Rule = {
  type: string;
  /** Matches when the description contains any of these, ignoring case. */
  contains: string[];
  min_amount: number | null;
  max_amount: number | null;
};

const base = (org: string) => `/api/orgs/${encodeURIComponent(org)}`;
const accounts = (org: string) => `${base(org)}/statement-accounts`;

function form(file: File, account: number, types?: Record<number, string | null>) {
  const body = new FormData();
  body.append("account", String(account));
  if (types && Object.keys(types).length > 0) body.append("types", JSON.stringify(types));
  body.append("file", file);
  return body;
}

/** Parses a statement for an account without saving anything. */
export const previewStatement = async (org: string, file: File, account: number) =>
  json<Preview>(
    await fetch(`${base(org)}/statements/preview`, { method: "POST", body: form(file, account) }),
    "Couldn't read that statement.",
  );

/**
 * Parses and saves it into the account; the file itself is kept in Documents. `types`: types
 * picked in the preview, by row id (null clears one); they're saved as hand-set.
 */
export const saveStatement = async (org: string, file: File, account: number, types?: Record<number, string | null>) =>
  json<UploadDetail>(
    await fetch(`${base(org)}/statements`, { method: "POST", body: form(file, account, types) }),
    "Couldn't save the statement.",
  );

export const fetchUploads = async (org: string, account: number) =>
  json<UploadSummary[]>(await fetch(`${base(org)}/statements?account=${account}`), "Couldn't load statements.");

// ---- accounts ----

export const fetchAccounts = async (org: string) => json<AccountSummary[]>(await fetch(accounts(org)), "Couldn't load bank accounts.");

export const createAccount = async (org: string, name: string, identifier: string, details?: AccountDetails) =>
  json<Account>(await send("POST", accounts(org), { name, identifier, details }), "Couldn't create the bank account.");

/** Without `details`, the account's details stay as they are. */
export const updateAccount = async (org: string, id: number, name: string, identifier: string, details?: AccountDetails) =>
  json<Account>(await send("PATCH", `${accounts(org)}/${id}`, { name, identifier, details }), "Couldn't save the bank account.");

/** What deleting the account would remove. */
export const fetchAccountDeleteImpact = async (org: string, id: number) =>
  json<DeleteImpact>(await fetch(`${accounts(org)}/${id}/delete-impact`), "Couldn't work out what this deletes.");

/** Deletes the account and everything in it (`confirm` is its name, typed). */
export const deleteAccount = async (org: string, id: number, confirm: string) =>
  json<void>(await send("DELETE", `${accounts(org)}/${id}`, { confirm }), "Couldn't delete the bank account.");

// ---- saved statements ----

export const fetchUpload = async (org: string, id: number) =>
  json<UploadDetail>(await fetch(`${base(org)}/statements/${id}`), "Couldn't load this statement.");

/** What deleting a saved statement would remove; `shared_rows` stay (another statement has them). */
export const fetchUploadDeleteImpact = async (org: string, id: number) =>
  json<DeleteImpact & { shared_rows: number }>(
    await fetch(`${base(org)}/statements/${id}/delete-impact`),
    "Couldn't work out what this deletes.",
  );

export const deleteUpload = async (org: string, id: number) =>
  json<void>(await send("DELETE", `${base(org)}/statements/${id}`), "Couldn't delete the statement.");

// ---- type rules (per account) ----

export const fetchRules = async (org: string, account: number) =>
  json<{ rules: Rule[] }>(await fetch(`${accounts(org)}/${account}/rules`), "Couldn't load the category rules.");

export const saveRules = async (org: string, account: number, rules: Rule[]) =>
  json<{ rules: Rule[] }>(await send("PUT", `${accounts(org)}/${account}/rules`, { rules }), "Couldn't save the category rules.");

// ---- upload exclusions (per account) ----

/** Lines a statement states that are never saved: the description contains any of `contains` (ignoring case). */
export type Exclusion = {
  contains: string[];
  /** Why, for whoever reads the rules later. */
  note: string;
};

export const fetchExclusions = async (org: string, account: number) =>
  json<{ rules: Exclusion[] }>(await fetch(`${accounts(org)}/${account}/exclusions`), "Couldn't load the exclusions.");

export const saveExclusions = async (org: string, account: number, rules: Exclusion[]) =>
  json<{ rules: Exclusion[] }>(await send("PUT", `${accounts(org)}/${account}/exclusions`, { rules }), "Couldn't save the exclusions.");

/** A saved row an exclusion matches. */
export type ExcludedMatch = {
  id: number;
  date: string;
  description: string;
  amount: string;
  currency: string;
  /** Rows in its split (deleted with it), and documents attached (the links go; the documents stay). */
  split_rows: number;
  attachments: number;
};

/** The account's saved rows exclusion `rule` (0-based) matches, oldest first. */
export const fetchExcludedMatches = async (org: string, account: number, rule: number) =>
  json<{ rows: ExcludedMatch[] }>(
    await fetch(`${accounts(org)}/${account}/exclusions/${rule}/matches`),
    "Couldn't find the matching rows.",
  );

/** Deletes those of the rows the exclusion still matches, with their splits. Needs `delete-statement-rows`. */
export const deleteExcludedMatches = async (org: string, account: number, rule: number, ids: number[]) =>
  json<{ deleted: number }>(
    await send("POST", `${accounts(org)}/${account}/exclusions/${rule}/matches/delete`, { ids }),
    "Couldn't delete the rows.",
  );

/** A stored row whose type the saved rules would change. */
export type TypeChange = {
  /** The row's own id. */
  id: number;
  date: string;
  description: string;
  amount: string;
  currency: string;
  from: string | null;
  to: string;
  /** 0-based index of the rule that matches it first. */
  rule: number;
};

/**
 * The stored rows in the account the saved rules (or just rule `rule`, 0-based) would retype.
 * Hand-set types and rows no rule matches are left out, as when saving a statement.
 */
export const previewApplyRules = async (org: string, account: number, rule: number | null) =>
  json<{ changes: TypeChange[] }>(
    await send("POST", `${accounts(org)}/${account}/rules/apply/preview`, { rule }),
    "Couldn't work out which rows would change.",
  );

/** Retype the accepted rows (only those the rules still give the type shown). */
export const applyRules = async (org: string, account: number, rule: number | null, changes: TypeChange[]) =>
  json<{ updated: number }>(
    await send("POST", `${accounts(org)}/${account}/rules/apply`, {
      rule,
      accepted: Object.fromEntries(changes.map((c) => [c.id, c.to])),
    }),
    "Couldn't apply the rules.",
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

/**
 * An amount for display, the same way whatever the file wrote ("-27177.56" and "-27,177.56"
 * both show as "−27,177.56"). Amounts are stored exactly as in the file (row ids are
 * worked out from that text); one that can't be read shows as it is, with a proper minus sign.
 */
export const formatAmount = (amount: string) => {
  const cents = amountCents(amount);
  if (cents !== null) return formatMoney(cents);
  return amount.startsWith("-") ? `−${amount.slice(1)}` : amount;
};

export const isNegative = (amount: string) => amount.trim().startsWith("-");

/** A stored row in the org, with the most recent upload that contained it. */
export type OrgTransaction = Transaction & {
  /** The row's own id (for editing); `txn_id` is the row id from the statement. */
  id: number;
  account_id: number;
  /** The type was picked by hand: saving a statement won't change it. */
  type_manual: boolean;
  /** A split row (added by hand): the original row's `txn_id`. */
  split_of: number | null;
  /** A statement row: how many rows its split has (0 = not split). */
  split_rows: number;
  /** Not from a bank statement (a split row, or a Google Sheet row without an id): can be deleted. */
  hand_added: boolean;
  /** Documents attached to the row. */
  attachments: number;
  /** e.g. "Aug 2024": from `effective_date` if set, else `date`, as in the sheet's `month` column. */
  month: string;
  updated_at: string;
  upload_id: number | null;
  upload_filename: string | null;
};

/** Every stored row in one account, newest first. */
export const fetchOrgTransactions = async (org: string, account: number) =>
  json<OrgTransaction[]>(await fetch(`${base(org)}/statement-transactions?account=${account}`), "Couldn't load transactions.");

/** The amount as a number (the stored string is kept exactly as in the file). NaN if unreadable. */
export const amountValue = (amount: string) => Number(amount.replace(/,/g, "").trim());

// ---- types ----

/** A type on the org's list, with how many rows and rules use it. */
export type TypeInfo = { id: number; name: string; rows: number; rules: number };

const types = (org: string) => `${base(org)}/statement-types`;

export const fetchTypes = async (org: string) => json<TypeInfo[]>(await fetch(types(org)), "Couldn't load the categories.");

export const createType = async (org: string, name: string) =>
  json<TypeInfo[]>(await send("POST", types(org), { name }), "Couldn't add the category.");

/** Renames it on every row and rule too. */
export const renameType = async (org: string, id: number, name: string) =>
  json<TypeInfo[]>(await send("PATCH", `${types(org)}/${id}`, { name }), "Couldn't rename the category.");

/** Refused while a row or rule uses it. */
export const deleteType = async (org: string, id: number) =>
  json<TypeInfo[]>(await send("DELETE", `${types(org)}/${id}`), "Couldn't delete the category.");

/** Sets a row's type by hand (or clears it with null, so saved statements can type it again). */
export const setRowType = async (org: string, row: number, type: string | null) =>
  json<{ type: string | null; type_manual: boolean }>(
    await send("PATCH", `${base(org)}/statement-transactions/${row}`, { type }),
    "Couldn't change the category.",
  );


// ---- one row ----

/** A row of a split, or the original row a split belongs to. */
export type SplitRow = {
  /** The row's own id (for opening it). */
  id: number;
  txn_id: number;
  date: string;
  effective_date: string | null;
  description: string;
  amount: string;
  currency: string;
  type: string | null;
};

/** A split row as sent when saving a split: `id` for an existing one (kept, with its documents). */
export type SplitRowInput = {
  id?: number;
  date: string;
  effective_date: string | null;
  description: string;
  amount: string;
  type: string | null;
};

export type RowDetail = OrgTransaction & {
  /** A split row: the row it splits, if that's still stored. */
  split_parent: SplitRow | null;
  /** A statement row: the rows of its split. */
  splits: SplitRow[];
  account_name: string;
  account_identifier: string | null;
  /** Saved statements containing the row, with their files in Documents. */
  uploads: { id: number; filename: string; uploaded_at: string; document_id: number | null }[];
  /** Documents attached to the row. */
  documents: {
    document_id: number;
    folder: string;
    filename: string;
    content_type: string | null;
    size: number;
    attached_at: string;
    attached_by: string | null;
    /** Its file is gone from storage. */
    missing: boolean;
  }[];
};

const row = (org: string, id: number) => `${base(org)}/statement-transactions/${id}`;

export const fetchRow = async (org: string, id: number) => json<RowDetail>(await fetch(row(org, id)), "Couldn't load the row.");

/**
 * Sets the split of row `id`: its rows become exactly `rows` (which must add up to zero); an
 * empty list removes the split. Returns the row.
 */
export const saveSplit = async (org: string, id: number, rows: SplitRowInput[]) =>
  json<RowDetail>(await send("PUT", `${row(org, id)}/split`, { rows }), "Couldn't save the split.");

/** Deletes a row added by hand (not a split row), with its split's rows if it has a split. */
export const deleteRow = async (org: string, id: number) => json<void>(await send("DELETE", row(org, id)), "Couldn't delete the row.");

/** Sets (YYYY-MM-DD) or clears (null) any row's effective date. Returns the row. */
export const setEffectiveDate = async (org: string, id: number, effective_date: string | null) =>
  json<RowDetail>(await send("PATCH", `${row(org, id)}/effective-date`, { effective_date }), "Couldn't change the effective date.");

/** An amount typed by hand, in cents; null if it isn't one (at most two decimals). */
export function amountCents(amount: string): number | null {
  const plain = amount.replace(/,/g, "").trim();
  if (!/^[+-]?(\d+\.?\d{0,2}|\.\d{1,2})$/.test(plain)) return null;
  return Math.round(Number(plain) * 100);
}

/** Cents as an amount with two decimals: -472616 -> "-4726.16". */
export const formatCents = (cents: number) => `${cents < 0 ? "-" : ""}${(Math.abs(cents) / 100).toFixed(2)}`;

/** Links an existing document to the row. */
export const attachDocument = async (org: string, id: number, documentId: number) =>
  json<RowDetail>(await send("POST", `${row(org, id)}/documents`, { document_id: documentId }), "Couldn't attach the document.");

/** Removes the link; the document itself stays. */
export const detachDocument = async (org: string, id: number, documentId: number) =>
  json<RowDetail>(await send("DELETE", `${row(org, id)}/documents/${documentId}`), "Couldn't remove the document.");

// ---- statement PDFs ----

export type HeaderField = { label: string; value: string };

export type ColumnKey = "date" | "effective_date" | "description" | "type" | "amount" | "debit" | "credit" | "balance";

/** A column of a template's transaction table: what it shows, and its heading. */
export type Column = { key: ColumnKey; label: string };

/** A column a table can have: its usual heading and what it shows. */
export type ColumnChoice = { key: ColumnKey; label: string; hint: string };

export type SummaryKey = "count" | "money_in" | "money_out" | "net" | "opening_balance" | "closing_balance";

/** A figure in a template's summary, with its label (may use variables). */
export type SummaryItem = { key: SummaryKey; label: string };

/** Which totals a template prints, and what they're called (labels may use variables). */
export type TotalsOptions = {
  /** The summary near the top, in order; empty for none. Balances drop out when the account has none. */
  summary: SummaryItem[];
  summary_title: string;
  /** A totals row at the end of the table. */
  table_row: boolean;
  table_row_label: string;
  /** A subtotal after each type, when grouped by type. */
  subtotals: boolean;
  subtotal_label: string;
  /** The table's first row when a running balance prints. */
  opening_row_label: string;
};

/** Which date decides the period, the balances and the running balance's order. */
export type DateBasis = "transaction" | "effective";

/** How statements print: one of the org's templates (any account can print with any of them). */
export type TemplateSettings = {
  name: string;
  /** A built-in layout: `standard` or `bank`. */
  layout: string;
  /** Title, subtitle, footer and header fields can use `{name}` variables. */
  title: string;
  subtitle: string;
  header_fields: HeaderField[];
  footer: string;
  /** A Documents image. */
  logo_document_id: number | null;
  /** A Documents PDF drawn behind the pages. */
  letterhead_document_id: number | null;
  /** The letterhead on every page (its second page, if it has one, behind the later ones), or the first only. */
  letterhead_every_page: boolean;
  /** Space kept clear at the top and bottom of each page, in mm (5 to 150). */
  margin_top_mm: number;
  margin_bottom_mm: number;
  paper: "a4" | "us-letter";
  /** The transaction table's columns, in order, with their headings. */
  columns: Column[];
  totals: TotalsOptions;
  /** The totals by type table (standard layout). */
  show_summary: boolean;
  group_by_type: boolean;
  /** `short` 4 Oct 2026, `padded` 04 Oct 2026. */
  date_style: "short" | "padded";
  /** Before balances and totals, e.g. `R`. */
  currency_symbol: string;
};

export type Template = TemplateSettings & { id: number; updated_at: string };

type DocumentPlace = { folder: string; filename: string };

/** A template, with where its logo and letterhead are in Documents (null if none or gone). */
export type TemplateView = Template & { logo: DocumentPlace | null; letterhead: DocumentPlace | null };

export type Choice = { name: string; label: string };

export type TemplateList = {
  templates: Template[];
  variables: Choice[];
  layouts: Choice[];
  columns: ColumnChoice[];
  summary_figures: { key: SummaryKey; label: string; hint: string }[];
};

const templates = (org: string) => `${base(org)}/statement-templates`;

/** The org's templates (it always has at least one), and the variables and layouts they can use. */
export const fetchTemplates = async (org: string) => json<TemplateList>(await fetch(templates(org)), "Couldn't load the templates.");

export const fetchTemplate = async (org: string, id: number) =>
  json<TemplateView>(await fetch(`${templates(org)}/${id}`), "Couldn't load the template.");

/** A new template from a layout's defaults, or a copy of another. */
export const createTemplate = async (org: string, body: { name: string; layout?: string; copy_of?: number }) =>
  json<TemplateView>(await send("POST", templates(org), body), "Couldn't create the template.");

export const saveTemplate = async (org: string, id: number, settings: TemplateSettings) =>
  json<TemplateView>(await send("PUT", `${templates(org)}/${id}`, settings), "Couldn't save the template.");

export const deleteTemplate = async (org: string, id: number) =>
  json<void>(await send("DELETE", `${templates(org)}/${id}`), "Couldn't delete the template.");

const pdf = (org: string, account: number) => `${accounts(org)}/${account}`;

/** A statement period, YYYY-MM-DD. */
export type Period = { from: string; to: string };

/** What printing would show: the period, the account's balances (from all its rows), and whether a running balance can print. */
export type PrintSummary = {
  from: string | null;
  to: string | null;
  opening_cents: number | null;
  closing_cents: number | null;
  /** How many of the account's rows are dated in the period. */
  period_rows: number;
  /** The printed rows are exactly those. */
  rows_complete: boolean;
};

export const fetchPrintSummary = async (
  org: string,
  account: number,
  body: { rows: number[]; period: Period | null; date_basis: DateBasis },
) => json<PrintSummary>(await send("POST", `${pdf(org, account)}/pdf/summary`, body), "Couldn't work out the balances.");

/** `-123456` -> `−1,234.56` (for showing; [formatCents] is for editing) */
export const formatMoney = (cents: number) => {
  const abs = Math.abs(cents);
  const whole = String(Math.floor(abs / 100)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const text = `${whole}.${String(abs % 100).padStart(2, "0")}`;
  return cents < 0 ? `−${text}` : text;
};

/** The file name from a Content-Disposition header (its UTF-8 form if there is one). */
function dispositionName(header: string | null, fallback: string) {
  const star = header?.match(/filename\*=UTF-8''([^;]+)/i);
  if (star) return decodeURIComponent(star[1]);
  return header?.match(/filename="([^"]+)"/i)?.[1] ?? fallback;
}

/**
 * Prints rows of the account (row ids, in order) as a PDF. `selection` says in words what they
 * are; `save` also keeps a copy in Documents (its id comes back).
 */
export async function printStatementPdf(
  org: string,
  account: number,
  body: {
    rows: number[];
    selection: string[];
    save: boolean;
    template_id: number | null;
    period: Period | null;
    date_basis?: DateBasis;
  },
) {
  const res = await send("POST", `${pdf(org, account)}/pdf`, body);
  if (!res.ok) await json<never>(res, "Couldn't make the PDF.");
  const id = res.headers.get("x-document-id");
  return {
    blob: await res.blob(),
    filename: dispositionName(res.headers.get("content-disposition"), "statement.pdf"),
    documentId: id ? Number(id) : null,
  };
}

/** Starts a download of `blob` as `filename`. */
export function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

/** The day a row counts on, by `basis`: its date, or its effective date where it has one. */
export const dayOf = (r: Pick<OrgTransaction, "date" | "effective_date">, basis: DateBasis) =>
  basis === "effective" ? (r.effective_date ?? r.date) : r.date;

/**
 * The order rows happen in, by `basis`: by day, then the order they were stored (same-day rows
 * have no better order). Running balances run in it, so the grid shows rows in it too.
 */
export const ledgerOrder = (basis: DateBasis) => (a: OrgTransaction, b: OrgTransaction) =>
  dayOf(a, basis).localeCompare(dayOf(b, basis)) || a.id - b.id;

const basisKey = (org: string) => `statement-date-basis:${org}`;
const bases = new Map<string, Signal<DateBasis>>();

/**
 * Which date counts for the org's statement rows in this browser, shared by the grid's quick
 * filters and printing: the transaction date, or the effective date
 * where a row has one. Remembered per org.
 */
export function dateBasis(org: string): [Accessor<DateBasis>, (b: DateBasis) => void] {
  let signal = bases.get(org);
  if (!signal) {
    let start: DateBasis = "transaction";
    try {
      if (localStorage.getItem(basisKey(org)) === "effective") start = "effective";
    } catch {
      // storage blocked: the default
    }
    signal = createRoot(() => createSignal<DateBasis>(start));
    bases.set(org, signal);
  }
  const [get, set] = signal;
  return [
    get,
    (b) => {
      set(b);
      try {
        localStorage.setItem(basisKey(org), b);
      } catch {
        // just don't remember
      }
    },
  ];
}

/**
 * Each row's balance after it, in cents, from the account's opening balance (`cents` at the start
 * of `date`) and *all* its rows: in day order on `basis`, then the order rows were stored (as a
 * printed statement's running balance). Rows before the opening date work backwards from it. Missing for
 * every row when an amount isn't a number, since no balance after it would be right.
 */
export function runningBalances(
  rows: OrgTransaction[],
  opening: { cents: number; date: string },
  basis: DateBasis,
): Map<number, number> {
  const ordered = [...rows].sort(ledgerOrder(basis));
  const cents = ordered.map((r) => Math.round(amountValue(r.amount) * 100));
  if (cents.some((c) => !Number.isFinite(c))) return new Map();
  // The balance after a row = opening + every row up to it − every row before the opening date.
  const before = ordered.reduce((sum, r, i) => (dayOf(r, basis) < opening.date ? sum + cents[i] : sum), 0);
  const balances = new Map<number, number>();
  let sum = 0;
  ordered.forEach((r, i) => {
    sum += cents[i];
    balances.set(r.id, opening.cents + sum - before);
  });
  return balances;
}
