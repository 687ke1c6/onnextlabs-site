//! Bank statements, per organisation (`/portal/<org>/statements`).
//!
//! A statement file is parsed by `statements-core` (format detection, row ids, type rules) and
//! saved as an *upload*. Its rows are stored once per `(account, row id)` and linked to every
//! upload that contained them, so saving a statement again (or the same rows in another format)
//! updates rows instead of duplicating them. A save never blanks a type: when no rule matches,
//! the stored type is kept. Lines the account's upload exclusions match (`statement_exclusions`)
//! are left out: the preview lists them, and saving never stores them.
//!
//! The row id (`txn_id`) is a hash of the date, amount, normalised description and occurrence
//! (statements-core `id.rs`): it's also what the Google Sheets copy is keyed on
//! (`statements_google`).
//!
//! API: list and view uploads, view type rules (`read-statements`); preview, save and delete
//! uploads, edit type rules (`write-statements`). Saving a copy to Documents also needs
//! `write-docs`.

use crate::db::Db;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

use axum::{
    body::Bytes,
    extract::{
        multipart::MultipartError, DefaultBodyLimit, Multipart, Path as UrlPath, Query, State,
    },
    http::StatusCode,
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use statements_core::{
    categories::{categorise, CategoryRule},
    row::month_label,
    statement::{self, Format, IdCheck, Statement, Transaction},
};

use crate::statement_accounts::{self, Account};
use crate::statement_exclusions::{account_exclusions, take_excluded, Exclusion};

use crate::{
    api::{bad_request, db_error, error, not_found},
    auth::CurrentUser,
    db, documents,
    orgs::Org,
    params,
    rbac::{self, Permission},
    AppState,
};

/// Every saved statement is kept in Documents under `Statements/<account number>`.
const STATEMENTS_FOLDER: &str = "Statements";

/// `Statements/<account>` (or `Statements/No account`). Account numbers are digits in practice;
/// anything that isn't safe as a folder name is replaced.
pub(crate) fn statement_folder(account: &str) -> String {
    let name: String = account
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_start_matches('.').trim();
    if name.is_empty() {
        format!("{STATEMENTS_FOLDER}/No account")
    } else {
        format!(
            "{STATEMENTS_FOLDER}/{}",
            name.chars().take(100).collect::<String>()
        )
    }
}

/// The org's document with exactly this content, if there is one (the oldest).
async fn document_with_sha256(
    db: &Db,
    org_id: i64,
    sha256: &str,
) -> Result<Option<ExistingDocument>, Response> {
    db::query_as(
        "SELECT id, folder, filename FROM documents WHERE org_id = ? AND sha256 = ? ORDER BY id LIMIT 1",
    )
    .bind(org_id)
    .bind(sha256)
    .fetch_optional(db)
    .await
    .map_err(db_error)
}

#[derive(Serialize, Deserialize)]
struct ExistingDocument {
    id: i64,
    folder: String,
    filename: String,
}
/// Room for multipart boundaries and the small fields on top of the file itself.
const MULTIPART_OVERHEAD: usize = 64 * 1024;
const MAX_RULES: usize = 500;
const MAX_NEEDLES_PER_RULE: usize = 50;
const MAX_RULE_TEXT_CHARS: usize = 200;

pub fn routes(max_bytes: u64) -> Router<Arc<AppState>> {
    let body_limit = usize::try_from(max_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(MULTIPART_OVERHEAD);
    Router::new()
        .route(
            "/api/orgs/{org}/statements",
            get(list)
                .post(save)
                .layer(DefaultBodyLimit::max(body_limit)),
        )
        .route(
            "/api/orgs/{org}/statements/preview",
            post(preview).layer(DefaultBodyLimit::max(body_limit)),
        )
        .route("/api/orgs/{org}/statements/{id}", get(show).delete(remove))
        .route("/api/statement-formats", get(formats))
        .route(
            "/api/orgs/{org}/statements/{id}/delete-impact",
            get(delete_preview),
        )
        .route(
            "/api/orgs/{org}/statement-transactions",
            get(all_transactions),
        )
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/rules",
            get(get_rules).put(put_rules),
        )
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/rules/apply/preview",
            post(apply_preview),
        )
        .route(
            "/api/orgs/{org}/statement-accounts/{id}/rules/apply",
            post(apply_rules),
        )
}

// ---- reading and parsing an upload ----

struct UploadForm {
    filename: String,
    bytes: Bytes,
    /// `account`: the account (id) the statement is for.
    account: Option<i64>,
    /// `types`: JSON `{"<row id>": "TYPE" | null}`, the types picked in the preview.
    types: Option<String>,
}

/// Types picked by hand in the preview, by row id: a type, or `None` to clear it.
type Picks = HashMap<i64, Option<String>>;

/// Multipart fields: `file` and `account` (required), `types` (optional, see `UploadForm`).
async fn read_form(mut multipart: Multipart, max_bytes: u64) -> Result<UploadForm, Response> {
    let mut file = None;
    let mut types = None;
    let mut account = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("file") => {
                let name = display_name(field.file_name().unwrap_or_default());
                let bytes = field.bytes().await.map_err(multipart_error)?;
                file = Some((name, bytes));
            }
            Some("types") => types = Some(field.text().await.map_err(multipart_error)?),
            Some("account") => {
                let text = field.text().await.map_err(multipart_error)?;
                account = Some(
                    text.trim()
                        .parse()
                        .map_err(|_| bad_request("Unknown bank account."))?,
                );
            }
            _ => {}
        }
    }
    let (filename, bytes) = file.ok_or_else(|| bad_request("No file was sent."))?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large(max_bytes));
    }
    if bytes.is_empty() {
        return Err(bad_request("The file is empty."));
    }
    let lower = filename.to_ascii_lowercase();
    if !lower.ends_with(".csv") && !lower.ends_with(".pdf") {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Statements are CSV or PDF files.",
        ));
    }
    Ok(UploadForm {
        filename,
        bytes,
        account,
        types,
    })
}

/// The preview's picks, checked: ids must be rows of this statement, and types must be on the
/// org's list (stored with the list's spelling).
async fn read_picks(
    db: &Db,
    org_id: i64,
    raw: Option<&str>,
    st: &Statement,
) -> Result<Picks, Response> {
    let Some(raw) = raw.filter(|r| !r.trim().is_empty()) else {
        return Ok(Picks::new());
    };
    let wanted: HashMap<String, Option<String>> =
        serde_json::from_str(raw).map_err(|_| bad_request("The chosen categories couldn't be read."))?;
    let ids: HashSet<i64> = st.transactions.iter().map(|t| t.id).collect();
    let mut picks = Picks::new();
    for (key, name) in wanted {
        let id: i64 = key
            .parse()
            .ok()
            .filter(|id| ids.contains(id))
            .ok_or_else(|| {
                bad_request("A chosen category is for a row that isn't in this statement.")
            })?;
        let name = match name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            None => None,
            Some(n) => Some(
                crate::statement_types::canonical_name(db, org_id, n)
                    .await
                    .map_err(db_error)?
                    .ok_or_else(|| bad_request(&format!("{n:?} isn't on the categories list.")))?,
            ),
        };
        picks.insert(id, name);
    }
    Ok(picks)
}

/// Types the file itself gives, by row id (`googlesheet_csv`'s `type` column), as written.
type FileTypes = HashMap<i64, String>;

/// Detects the format, parses, assigns the row ids, takes out the lines `exclusions` match
/// (returned, in file order) and applies the type rules. Rules only fill rows the file didn't
/// give a type; those are returned separately, as they're saved as hand-set.
///
/// Row ids are assigned before excluding, so a line's id never depends on what's excluded.
fn parse_statement(
    bytes: &[u8],
    rules: &[CategoryRule],
    exclusions: &[Exclusion],
) -> Result<(Statement, FileTypes, Vec<Transaction>), Response> {
    let mut statement = statement::parse(bytes).map_err(|e| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("This file couldn't be read as a statement: {e:#}"),
        )
    })?;
    if statement.transactions.is_empty() {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "This statement has no transactions.",
        ));
    }
    let excluded = take_excluded(&mut statement, exclusions);
    let file_types = statement
        .transactions
        .iter()
        .filter_map(|t| Some((t.id, t.category.clone()?)))
        .collect();
    categorise(rules, &mut statement);
    Ok((statement, file_types, excluded))
}

/// The file's types as they'd be saved: each in the types list's spelling, except on rows whose
/// stored type is hand-set (that stays, as with any save). With `create`, types not on the list
/// are added to it; without, they keep the file's spelling. Returns the picks and the types not
/// on the list (before any were added), sorted.
async fn file_picks(
    db: &Db,
    org_id: i64,
    file_types: &FileTypes,
    stored: &HashMap<i64, (Option<String>, bool)>,
    create: bool,
) -> Result<(HashMap<i64, String>, Vec<String>), Response> {
    let mut names: HashMap<&str, String> = HashMap::new();
    let mut new_types = BTreeSet::new();
    for raw in file_types.values() {
        if names.contains_key(raw.as_str()) {
            continue;
        }
        let name = match crate::statement_types::canonical_name(db, org_id, raw)
            .await
            .map_err(db_error)?
        {
            Some(name) => name,
            None => {
                new_types.insert(raw.trim().to_string());
                if create {
                    crate::statement_types::ensure_type(db, org_id, raw).await?
                } else {
                    raw.trim().to_string()
                }
            }
        };
        names.insert(raw, name);
    }
    let picks = file_types
        .iter()
        .filter(|(id, _)| !stored.get(id).is_some_and(|(_, manual)| *manual))
        .map(|(id, raw)| (*id, names[raw.as_str()].clone()))
        .collect();
    Ok((picks, new_types.into_iter().collect()))
}

/// The file's name without any directory part or control characters, shortened.
fn display_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let base: String = base.chars().filter(|c| !c.is_control()).take(120).collect();
    if base.is_empty() || base.starts_with('.') {
        "statement.csv".into()
    } else {
        base
    }
}

/// The account number the statement states ('' when its format doesn't state one).
fn stated_account(statement: &Statement) -> String {
    statement.account_number.clone().unwrap_or_default()
}

/// The account the upload is for: required, and in this org.
async fn target_account(db: &Db, org_id: i64, form: &UploadForm) -> Result<Account, Response> {
    let id = form
        .account
        .ok_or_else(|| bad_request("Choose the bank account this statement is for."))?;
    statement_accounts::find(db, org_id, id).await
}

/// How the account number a statement states compares with the account's identifier.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AccountCheck {
    /// The statement's number is the account's identifier.
    Match,
    /// They differ: saving is allowed, but it's probably the wrong account.
    Mismatch,
    /// The account has no identifier to compare with.
    NoIdentifier,
    /// The statement doesn't state an account number.
    NotStated,
}

fn check_account(account: &Account, stated: &str) -> AccountCheck {
    match (stated.trim(), account.identifier.as_deref().map(str::trim)) {
        ("", _) => AccountCheck::NotStated,
        (_, None) => AccountCheck::NoIdentifier,
        (s, Some(i)) if s == i => AccountCheck::Match,
        _ => AccountCheck::Mismatch,
    }
}

// ---- preview ----

#[derive(Serialize)]
struct Preview {
    filename: String,
    format: &'static str,
    format_name: &'static str,
    /// As stated in the file, if the format states one.
    account_number: Option<String>,
    /// The account it would be saved into, and how the stated number compares with it.
    account: Account,
    account_check: AccountCheck,
    /// On a mismatch, or when the account has no identifier: another account whose identifier
    /// is the stated number, if there is one (so the stated number can't be given to this one).
    suggested_account: Option<Account>,
    first_date: Option<String>,
    last_date: Option<String>,
    /// Rows that will be saved (the file's, less the excluded ones).
    rows: usize,
    /// Rows a type rule matched.
    typed: usize,
    /// Rows not stored yet, and rows already stored (they'll be updated).
    new: usize,
    existing: usize,
    transactions: Vec<PreviewRow>,
    /// Lines the account's upload exclusions leave out, in file order: never saved.
    excluded: Vec<ExcludedRow>,
    /// Formats that state their rows' ids (`googlesheet_csv`): how they compare with the ids
    /// computed from the rows.
    id_check: Option<IdCheck>,
    /// Types the file gives that aren't on the org's list yet; saving adds them.
    new_types: Vec<String>,
    /// Where the copy of the file goes in Documents...
    documents_folder: String,
    /// ...unless this exact file is already there, in which case that one is used.
    existing_document: Option<ExistingDocument>,
}

/// A line of the file an upload exclusion leaves out.
#[derive(Serialize)]
struct ExcludedRow {
    txn_id: i64,
    date: String,
    amount: String,
    description: String,
    currency: String,
    /// Saved before the exclusion existed: it stays until deleted (from Uploads › Upload exclusions).
    stored: bool,
}

#[derive(Serialize)]
struct PreviewRow {
    txn_id: i64,
    date: String,
    amount: String,
    description: String,
    currency: String,
    /// The type the row will have once saved: a stored hand-set type stays; otherwise the
    /// rules' type, or the stored one when no rule matches (a blank never replaces a type).
    #[serde(rename = "type")]
    category: Option<String>,
    /// That type is hand-set (saving won't change it).
    type_manual: bool,
    /// Already stored for this org and account.
    stored: bool,
    effective_date: Option<String>,
}

/// POST /api/orgs/{org}/statements/preview — parse without saving anything.
async fn preview(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    multipart: Multipart,
) -> Result<Json<Preview>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let form = read_form(multipart, state.max_upload_bytes).await?;
    let account = target_account(&state.db, org.id, &form).await?;
    let rules = account_rules(&state.db, account.id).await?;
    let exclusions = account_exclusions(&state.db, account.id).await?;
    let (st, file_types, excluded) = parse_statement(&form.bytes, &rules, &exclusions)?;
    let stored = stored_types(&state.db, account.id).await?;
    let (file_picks, new_types) =
        file_picks(&state.db, org.id, &file_types, &stored, false).await?;
    let stated = stated_account(&st);
    let account_check = check_account(&account, &stated);
    let suggested_account = if matches!(
        account_check,
        AccountCheck::Mismatch | AccountCheck::NoIdentifier
    ) {
        statement_accounts::with_identifier(&state.db, org.id, &stated).await?
    } else {
        None
    };
    let txs = &st.transactions;
    // (type, hand-set) as saving would leave it.
    let effective = |t: &Transaction| match (stored.get(&t.id), file_picks.get(&t.id)) {
        (Some((typ, true)), _) => (typ.clone(), true),
        (_, Some(typ)) => (Some(typ.clone()), true),
        (Some((typ, false)), None) => (t.category.clone().or_else(|| typ.clone()), false),
        (None, None) => (t.category.clone(), false),
    };
    let existing = txs.iter().filter(|t| stored.contains_key(&t.id)).count();
    let sha256 = hex::encode(Sha256::digest(&form.bytes));
    let existing_document = document_with_sha256(&state.db, org.id, &sha256).await?;
    Ok(Json(Preview {
        filename: form.filename,
        format: st.format.id(),
        format_name: st.format.description(),
        account_number: st.account_number.clone(),
        // The period the file covers, excluded lines included.
        first_date: txs.iter().chain(&excluded).map(|t| &t.date).min().cloned(),
        last_date: txs.iter().chain(&excluded).map(|t| &t.date).max().cloned(),
        rows: txs.len(),
        typed: txs.iter().filter(|t| effective(t).0.is_some()).count(),
        new: txs.len() - existing,
        existing,
        transactions: txs
            .iter()
            .map(|t| {
                let (category, type_manual) = effective(t);
                PreviewRow {
                    txn_id: t.id,
                    date: t.date.clone(),
                    amount: t.amount.clone(),
                    description: t.description.clone(),
                    currency: t.currency.clone(),
                    category,
                    type_manual,
                    stored: stored.contains_key(&t.id),
                    effective_date: t.effective_date.clone(),
                }
            })
            .collect(),
        excluded: excluded
            .iter()
            .map(|t| ExcludedRow {
                txn_id: t.id,
                date: t.date.clone(),
                amount: t.amount.clone(),
                description: t.description.clone(),
                currency: t.currency.clone(),
                stored: stored.contains_key(&t.id),
            })
            .collect(),
        id_check: st.id_check.clone(),
        new_types,
        documents_folder: statement_folder(&account.name),
        existing_document,
        account,
        account_check,
        suggested_account,
    }))
}

/// The rows already stored in this account: row id -> (type, hand-set).
async fn stored_types(
    db: &Db,
    account_id: i64,
) -> Result<HashMap<i64, (Option<String>, bool)>, Response> {
    let rows: Vec<(i64, Option<String>, bool)> = db::query_as(
        "SELECT txn_id, type, type_manual FROM statement_transactions WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    Ok(rows
        .into_iter()
        .map(|(id, typ, manual)| (id, (typ, manual)))
        .collect())
}

// ---- save ----

/// POST /api/orgs/{org}/statements — parse again and save: the file in Documents (on the
/// uploader's behalf, private; an identical file already there is reused), then the upload and
/// every row (new rows inserted, stored rows updated).
async fn save(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<UploadDetail>), Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let form = read_form(multipart, state.max_upload_bytes).await?;
    // A mismatch with the account's identifier is shown in the preview, never refused here.
    let account = target_account(&state.db, org.id, &form).await?;
    let rules = account_rules(&state.db, account.id).await?;
    let exclusions = account_exclusions(&state.db, account.id).await?;
    let (st, file_types, excluded) = parse_statement(&form.bytes, &rules, &exclusions)?;
    if st.transactions.is_empty() {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Every line of this statement is excluded by the bank account's upload exclusions, so there's nothing to save.",
        ));
    }
    let user_picks = read_picks(&state.db, org.id, form.types.as_deref(), &st).await?;
    let picks = save_picks(&state.db, org.id, account.id, &file_types, user_picks).await?;
    let sha256 = hex::encode(Sha256::digest(&form.bytes));

    // The file first: storage is the likelier failure, and nothing is in the database yet.
    // Saved on the uploader's behalf (no `write-docs` needed) and private.
    let document_id = match document_with_sha256(&state.db, org.id, &sha256).await? {
        Some(doc) => doc.id,
        None => {
            documents::save_bytes(
                &state,
                &org,
                user.id,
                &statement_folder(&account.name),
                &form.filename,
                Some(
                    if st.format.is_pdf() {
                        "application/pdf"
                    } else {
                        "text/csv"
                    }
                    .into(),
                ),
                form.bytes.clone(),
                false,
            )
            .await?
            .0
        }
    };

    let upload_id = store_with(
        &state.db,
        &org,
        account.id,
        user.id,
        &form.filename,
        &sha256,
        &st,
        Some(document_id),
        &picks,
    )
    .await
    .map_err(db_error)?;
    let detail = upload_detail(&state.db, org.id, upload_id).await?;
    tracing::info!(
        upload = upload_id,
        org = %org.slug,
        format = st.format.id(),
        rows = detail.upload.rows_parsed,
        new = detail.upload.rows_new,
        updated = detail.upload.rows_updated,
        excluded = excluded.len(),
        user = user.id,
        "statement saved"
    );
    Ok((StatusCode::CREATED, Json(detail)))
}

/// The types a save stores as hand-set: the file's own (added to the list if missing; a stored
/// hand-set type stays), then the ones picked in the preview, which win.
async fn save_picks(
    db: &Db,
    org_id: i64,
    account_id: i64,
    file_types: &FileTypes,
    user_picks: Picks,
) -> Result<Picks, Response> {
    let stored = stored_types(db, account_id).await?;
    let (file_picks, _) = file_picks(db, org_id, file_types, &stored, true).await?;
    let mut picks: Picks = file_picks
        .into_iter()
        .map(|(id, t)| (id, Some(t)))
        .collect();
    picks.extend(user_picks);
    Ok(picks)
}

/// Tests: the account whose identifier is `stated` (created if needed; '' = "No account
/// stated", with no identifier), as the migration does for existing data.
#[cfg(test)]
async fn test_account(db: &Db, org_id: i64, stated: &str) -> i64 {
    let identifier = Some(stated).filter(|s| !s.is_empty());
    let name = identifier.unwrap_or("No account stated");
    db::query(
        "INSERT OR IGNORE INTO statement_accounts (org_id, name, identifier) VALUES (?, ?, ?)",
    )
    .bind(org_id)
    .bind(name)
    .bind(identifier)
    .execute(db)
    .await
    .unwrap();
    db::query_scalar("SELECT id FROM statement_accounts WHERE org_id = ? AND name = ?")
        .bind(org_id)
        .bind(name)
        .fetch_one(db)
        .await
        .unwrap()
}

/// `store_with` with no types picked by hand, into the account whose identifier is the
/// statement's stated number (created if needed, as the migration did).
#[cfg(test)]
async fn store(
    db: &Db,
    org: &Org,
    user_id: i64,
    filename: &str,
    sha256: &str,
    st: &Statement,
    document_id: Option<i64>,
) -> db::Result<i64> {
    let account = test_account(db, org.id, &stated_account(st)).await;
    store_with(
        db,
        org,
        account,
        user_id,
        filename,
        sha256,
        st,
        document_id,
        &Picks::new(),
    )
    .await
}

/// One database transaction: the upload row, then each statement row upserted on
/// `(account, row id)` and linked to the upload in file order.
///
/// An effective date in the file is stored; a row without one keeps any it has. A PDF never
/// changes the description or amount text of a row already saved (see `keep_text`). A row stays
/// hand-added (deletable) only while no statement that isn't hand-added contains it.
///
/// Types: a type picked in the preview (`picks`) is stored as hand-set; clearing one there
/// stores the row blank and not hand-set. Otherwise a stored hand-set type is kept, and a
/// blank from the rules never replaces a type.
#[allow(clippy::too_many_arguments)]
async fn store_with(
    db: &Db,
    org: &Org,
    account_id: i64,
    user_id: i64,
    filename: &str,
    sha256: &str,
    st: &Statement,
    document_id: Option<i64>,
    picks: &Picks,
) -> db::Result<i64> {
    // One batch, decided up front. New/updated counts follow the file: a row whose id was
    // stored before is "updated"; one that wasn't is "new" the first time it appears and
    // "updated" for any repeat. So new = distinct ids not yet stored; updated = the rest.
    let ids: BTreeSet<i64> = st.transactions.iter().map(|t| t.id).collect();
    let ids_json = serde_json::to_string(&ids).expect("ids serialise");
    let parsed = st.transactions.len() as i64;

    // A PDF's text is rebuilt to give the same ids as the bank's CSV export, but it can still read a little
    // differently (runs of spaces, `-3.50` for `-3.5`): rows already saved keep their own text.
    let keep_text = st.format.is_pdf();

    let mut batch = db::Batch::new();
    let upload = batch.push(
        "INSERT INTO statement_uploads
             (org_id, account_id, format, account_number, filename, sha256, rows_parsed, rows_new,
              rows_updated, document_id, uploaded_by)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, n.new, ?7 - n.new, ?8, ?9
         FROM (SELECT count(*) AS new FROM json_each(?10) j
               WHERE NOT EXISTS (SELECT 1 FROM statement_transactions
                                 WHERE account_id = ?2 AND txn_id = j.value)) n
         RETURNING id",
        params![
            org.id,
            account_id,
            st.format.id(),
            stated_account(st),
            filename,
            sha256,
            parsed,
            document_id,
            user_id,
            ids_json,
        ],
    );

    for (position, t) in st.transactions.iter().enumerate() {
        match picks.get(&t.id) {
            // Picked in the preview: stored as hand-set (or blank and not hand-set).
            Some(chosen) => batch.push(
                "INSERT INTO statement_transactions
                     (org_id, account_id, txn_id, date, amount, description, currency, type,
                      type_manual, effective_date, occurrence, hand_added)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (account_id, txn_id) DO UPDATE SET
                     date = excluded.date,
                     amount = CASE WHEN ? THEN amount ELSE excluded.amount END,
                     description = CASE WHEN ? THEN description ELSE excluded.description END,
                     currency = excluded.currency,
                     type = excluded.type, type_manual = excluded.type_manual,
                     effective_date = coalesce(excluded.effective_date, effective_date),
                     hand_added = min(hand_added, excluded.hand_added),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![
                    org.id,
                    account_id,
                    t.id,
                    &t.date,
                    &t.amount,
                    &t.description,
                    &t.currency,
                    chosen.clone(),
                    chosen.is_some(),
                    t.effective_date.clone(),
                    t.occurrence,
                    t.hand_added,
                    keep_text,
                    keep_text,
                ],
            ),
            // From the rules: a hand-set type is never changed, and a blank never replaces one.
            None => batch.push(
                "INSERT INTO statement_transactions
                     (org_id, account_id, txn_id, date, amount, description, currency, type,
                      type_manual, effective_date, occurrence, hand_added)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)
                 ON CONFLICT (account_id, txn_id) DO UPDATE SET
                     date = excluded.date,
                     amount = CASE WHEN ? THEN amount ELSE excluded.amount END,
                     description = CASE WHEN ? THEN description ELSE excluded.description END,
                     currency = excluded.currency,
                     type = CASE WHEN type_manual = 1 THEN type ELSE coalesce(excluded.type, type) END,
                     effective_date = coalesce(excluded.effective_date, effective_date),
                     hand_added = min(hand_added, excluded.hand_added),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![
                    org.id,
                    account_id,
                    t.id,
                    &t.date,
                    &t.amount,
                    &t.description,
                    &t.currency,
                    t.category.clone(),
                    t.effective_date.clone(),
                    t.occurrence,
                    t.hand_added,
                    keep_text,
                    keep_text,
                ],
            ),
        };
        // The upload is the newest (AUTOINCREMENT, and a batch is the only writer while it
        // runs). OR IGNORE: two rows of one file with the same id (on different days) share a
        // row.
        batch.push(
            "INSERT OR IGNORE INTO statement_upload_transactions (upload_id, transaction_id, position)
             VALUES ((SELECT max(id) FROM statement_uploads),
                     (SELECT id FROM statement_transactions WHERE account_id = ? AND txn_id = ?),
                     ?)",
            params![account_id, t.id, position],
        );
    }

    let mut results = db::run(db, batch).await?;
    results
        .swap_remove(upload)
        .into_first()
        .and_then(|v| v.as_i64())
        .ok_or_else(|| db::Error::new(db_protocol::ErrorKind::Decode, "no upload id returned"))
}

// ---- listing, viewing, deleting ----

#[derive(Serialize, Deserialize)]
pub struct UploadSummary {
    pub id: i64,
    pub format: String,
    /// The format's name, e.g. "Standard Bank PDF statement (2026-01 layout)" (not stored: from `format`).
    #[serde(default)]
    pub format_name: String,
    /// The account it was saved into.
    pub account_id: i64,
    pub account_name: String,
    /// The account number the file itself states ('' if none).
    pub account_number: String,
    pub filename: String,
    pub rows_parsed: i64,
    pub rows_new: i64,
    pub rows_updated: i64,
    /// The copy in Documents, if one was saved and still exists.
    pub document_id: Option<i64>,
    pub uploaded_at: String,
    /// Uploader's name (or email), if the account still exists.
    pub uploaded_by: Option<String>,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
}

macro_rules! select_upload {
    ($tail:literal) => {
        concat!(
            "SELECT u.id, u.format, u.account_id, a.name AS account_name, u.account_number,
                    u.filename, u.rows_parsed, u.rows_new,
                    u.rows_updated, u.document_id, u.uploaded_at,
                    coalesce(us.name, us.email) AS uploaded_by,
                    (SELECT min(t.date) FROM statement_upload_transactions l
                         JOIN statement_transactions t ON t.id = l.transaction_id
                         WHERE l.upload_id = u.id) AS first_date,
                    (SELECT max(t.date) FROM statement_upload_transactions l
                         JOIN statement_transactions t ON t.id = l.transaction_id
                         WHERE l.upload_id = u.id) AS last_date
             FROM statement_uploads u
             JOIN statement_accounts a ON a.id = u.account_id
             LEFT JOIN users us ON us.id = u.uploaded_by ",
            $tail
        )
    };
}

/// A stored row, as linked to an upload.
#[derive(Serialize, Deserialize, Clone)]
pub struct StoredTransaction {
    pub txn_id: i64,
    pub date: String,
    pub amount: String,
    pub description: String,
    pub currency: String,
    #[serde(rename = "type")]
    pub category: Option<String>,
    pub effective_date: Option<String>,
    pub occurrence: u32,
    /// A split row: the original row's `txn_id`.
    pub split_of: Option<i64>,
}

#[derive(Serialize)]
pub struct UploadDetail {
    #[serde(flatten)]
    pub upload: UploadSummary,
    pub transactions: Vec<StoredTransaction>,
}

/// `?account=<id>`: only that account. Absent = all of the org's.
#[derive(Deserialize)]
struct AccountQuery {
    account: Option<i64>,
}

/// GET /api/orgs/{org}/statements?account= — the org's uploads, newest first.
async fn list(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Query(q): Query<AccountQuery>,
) -> Result<Json<Vec<UploadSummary>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    let uploads = db::query_as(select_upload!(
        "WHERE u.org_id = ?1 AND (?2 IS NULL OR u.account_id = ?2)
         ORDER BY u.uploaded_at DESC, u.id DESC"
    ))
    .bind(org.id)
    .bind(q.account)
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;
    Ok(Json(uploads.into_iter().map(named).collect()))
}

/// The upload with its format's name filled in.
fn named(mut u: UploadSummary) -> UploadSummary {
    u.format_name = Format::from_id(&u.format)
        .map_or("Unknown format", Format::description)
        .to_string();
    u
}

#[derive(Serialize)]
struct FormatInfo {
    id: &'static str,
    name: &'static str,
    pdf: bool,
}

/// GET /api/statement-formats — the statement formats uploads can be in.
async fn formats(_user: CurrentUser) -> Json<Vec<FormatInfo>> {
    Json(
        Format::ALL
            .into_iter()
            .map(|f| FormatInfo {
                id: f.id(),
                name: f.description(),
                pdf: f.is_pdf(),
            })
            .collect(),
    )
}

/// GET /api/orgs/{org}/statements/{id} — one upload with every row it contained.
async fn show(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<Json<UploadDetail>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(upload_detail(&state.db, org.id, id).await?))
}

pub async fn upload_detail(db: &Db, org_id: i64, id: i64) -> Result<UploadDetail, Response> {
    let upload: UploadSummary = db::query_as(select_upload!("WHERE u.id = ? AND u.org_id = ?"))
        .bind(id)
        .bind(org_id)
        .fetch_optional(db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Statement not found."))?;
    let transactions = db::query_as(
        "SELECT t.txn_id, t.date, t.amount, t.description, t.currency, t.type, t.effective_date,
                t.occurrence, t.split_of
         FROM statement_upload_transactions l
         JOIN statement_transactions t ON t.id = l.transaction_id
         WHERE l.upload_id = ? ORDER BY l.position",
    )
    .bind(id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    Ok(UploadDetail {
        upload: named(upload),
        transactions,
    })
}

/// The split rows of the rows `parents` (their `txn_id`s) in the account, each split's rows in the
/// order added.
pub async fn split_rows_of(
    db: &Db,
    account_id: i64,
    parents: &[i64],
) -> db::Result<Vec<StoredTransaction>> {
    db::query_as(
        "SELECT txn_id, date, amount, description, currency, type, effective_date, occurrence,
                split_of
         FROM statement_transactions
         WHERE account_id = ? AND split_of IN (SELECT value FROM json_each(?))
         ORDER BY split_of, id",
    )
    .bind(account_id)
    .bind(serde_json::to_string(parents).expect("ids serialise"))
    .fetch_all(db)
    .await
}

/// A stored row with where it came from, for the org-wide transactions grid.
#[derive(Serialize, Deserialize)]
struct OrgTransaction {
    /// The row's own id (for editing it); `txn_id` is the row id from the statement.
    id: i64,
    txn_id: i64,
    account_id: i64,
    date: String,
    /// e.g. "Aug 2024", derived from `date` exactly as the sheet's `month` column is.
    #[serde(skip_deserializing)]
    month: String,
    amount: String,
    description: String,
    currency: String,
    #[serde(rename = "type")]
    category: Option<String>,
    /// The type was picked by hand: saving a statement won't change it.
    type_manual: bool,
    effective_date: Option<String>,
    /// A split row: the original row's `txn_id`.
    split_of: Option<i64>,
    /// A statement row: how many rows its split has (0 = not split).
    split_rows: i64,
    /// Not from a bank statement (a split row, or a Google Sheet row without an id).
    hand_added: bool,
    /// Documents attached to the row.
    attachments: i64,
    updated_at: String,
    /// The most recent upload containing it (rows always belong to at least one).
    upload_id: Option<i64>,
    upload_filename: Option<String>,
}

/// GET /api/orgs/{org}/statement-transactions?account= — every stored row in the org (or
/// account), newest first. The page sorts, filters and pages them itself.
async fn all_transactions(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Query(q): Query<AccountQuery>,
) -> Result<Json<Vec<OrgTransaction>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(
        org_transactions(&state.db, org.id, q.account)
            .await
            .map_err(db_error)?,
    ))
}

async fn org_transactions(
    db: &Db,
    org_id: i64,
    account: Option<i64>,
) -> db::Result<Vec<OrgTransaction>> {
    db::query_as(
        "SELECT t.id, t.txn_id, t.account_id, t.date, t.amount, t.description, t.currency, t.type,
                t.type_manual, t.effective_date, t.split_of, t.hand_added,
                (SELECT count(*) FROM statement_transactions s
                 WHERE s.account_id = t.account_id AND s.split_of = t.txn_id) AS split_rows,
                (SELECT count(*) FROM statement_transaction_documents d WHERE d.transaction_id = t.id)
                    AS attachments,
                t.updated_at, u.id AS upload_id, u.filename AS upload_filename
         FROM statement_transactions t
         LEFT JOIN statement_uploads u ON u.id = (
             SELECT max(l.upload_id) FROM statement_upload_transactions l
             WHERE l.transaction_id = t.id)
         WHERE t.org_id = ?1 AND (?2 IS NULL OR t.account_id = ?2)
         ORDER BY t.date DESC, t.id DESC",
    )
    .bind(org_id)
    .bind(account)
    .fetch_all(db)
    .await
    .map(|mut rows: Vec<OrgTransaction>| {
        for r in &mut rows {
            r.month = month_label(&r.date);
        }
        rows
    })
}

/// DELETE /api/orgs/{org}/statements/{id} — removes the upload and the rows no other upload
/// contains. A copy saved to Documents stays.
async fn remove(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<StatusCode, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let removed = delete_upload(&state.db, org.id, id)
        .await
        .map_err(db_error)?;
    let Some(rows_deleted) = removed else {
        return Err(not_found("Statement not found."));
    };
    tracing::info!(upload = id, org = %org.slug, rows_deleted, user = user.id, "statement deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// The rows of an upload that no other saved statement contains: deleting the upload deletes
/// these. (Rows another statement also contains stay, or that statement would lose them.)
async fn rows_only_in(db: &Db, upload_id: i64) -> db::Result<Vec<i64>> {
    db::query_scalar(
        "SELECT l.transaction_id FROM statement_upload_transactions l
         WHERE l.upload_id = ?1 AND NOT EXISTS (
             SELECT 1 FROM statement_upload_transactions o
             WHERE o.transaction_id = l.transaction_id AND o.upload_id <> ?1)",
    )
    .bind(upload_id)
    .fetch_all(db)
    .await
}

#[derive(Serialize)]
struct UploadDeletePreview {
    #[serde(flatten)]
    impact: crate::delete_impact::Impact,
    /// Rows of this statement that another saved statement also contains: they stay.
    shared_rows: i64,
}

/// GET /api/orgs/{org}/statements/{id}/delete-impact — what deleting it would remove.
async fn delete_preview(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<Json<UploadDeletePreview>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    upload_detail(&state.db, org.id, id).await?; // 404 unless it's this org's
    let only_here = rows_only_in(&state.db, id).await.map_err(db_error)?;
    let linked: i64 =
        db::query_scalar("SELECT count(*) FROM statement_upload_transactions WHERE upload_id = ?")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .map_err(db_error)?;
    let impact = crate::delete_impact::impact(
        &state.db,
        &[
            ("statement_uploads", vec![id]),
            ("statement_transactions", only_here.clone()),
        ],
    )
    .await
    .map_err(db_error)?;
    Ok(Json(UploadDeletePreview {
        impact,
        shared_rows: linked - only_here.len() as i64,
    }))
}

/// `None` if there's no such upload in the org; otherwise how many rows were deleted with it.
/// Deletes the upload, its rows that no other statement contains, and (by cascade) their
/// links and attached-document links. Documents themselves stay.
async fn delete_upload(db: &Db, org_id: i64, id: i64) -> db::Result<Option<u64>> {
    // One batch: first the rows only this upload contains (their links and attached-document
    // links go by cascade), then the upload and its remaining links. With no such upload in
    // the org, neither statement matches anything.
    let mut batch = db::Batch::new();
    let rows = batch.push(
        "DELETE FROM statement_transactions
         WHERE id IN (SELECT l.transaction_id FROM statement_upload_transactions l
                      JOIN statement_uploads u ON u.id = l.upload_id
                      WHERE l.upload_id = ?1 AND u.org_id = ?2)
           AND NOT EXISTS (SELECT 1 FROM statement_upload_transactions o
                           WHERE o.transaction_id = statement_transactions.id
                             AND o.upload_id <> ?1)",
        params![id, org_id],
    );
    let upload = batch.push(
        "DELETE FROM statement_uploads WHERE id = ? AND org_id = ?",
        params![id, org_id],
    );
    let results = db::run(db, batch).await?;
    Ok((results[upload].rows_affected > 0).then_some(results[rows].rows_affected))
}

// ---- type rules ----

/// A type rule as the API and the database hold it: `contains` as typed (matching ignores
/// case). Rules belong to an account.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RuleJson {
    #[serde(rename = "type")]
    pub category: String,
    pub contains: Vec<String>,
    #[serde(default)]
    pub min_amount: Option<f64>,
    #[serde(default)]
    pub max_amount: Option<f64>,
}

impl RuleJson {
    fn to_rule(&self) -> Result<CategoryRule, String> {
        if self.category.chars().count() > MAX_RULE_TEXT_CHARS {
            return Err("the category is too long".into());
        }
        if self.contains.len() > MAX_NEEDLES_PER_RULE {
            return Err(format!("at most {MAX_NEEDLES_PER_RULE} texts to look for"));
        }
        if self
            .contains
            .iter()
            .any(|c| c.chars().count() > MAX_RULE_TEXT_CHARS)
        {
            return Err("a text to look for is too long".into());
        }
        CategoryRule::new(
            &self.category,
            &self.contains,
            self.min_amount,
            self.max_amount,
            vec![],
        )
        .map_err(|e| e.to_string())
    }
}

/// Checked rules, or a 400 naming the first bad one.
fn validate_rules(rules: &[RuleJson]) -> Result<Vec<CategoryRule>, Response> {
    if rules.len() > MAX_RULES {
        return Err(bad_request(&format!("At most {MAX_RULES} rules.")));
    }
    rules
        .iter()
        .enumerate()
        .map(|(i, r)| {
            r.to_rule()
                .map_err(|e| bad_request(&format!("Rule {}: {e}.", i + 1)))
        })
        .collect()
}

#[derive(Deserialize)]
struct RuleRow {
    #[serde(rename = "type")]
    category: String,
    contains: String,
    min_amount: Option<f64>,
    max_amount: Option<f64>,
}

async fn load_rules(db: &Db, account_id: i64) -> Result<Vec<RuleJson>, Response> {
    let rows: Vec<RuleRow> = db::query_as(
        "SELECT type, contains, min_amount, max_amount FROM statement_type_rules
         WHERE account_id = ? ORDER BY position, id",
    )
    .bind(account_id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    Ok(rows
        .into_iter()
        .map(|r| RuleJson {
            category: r.category,
            contains: serde_json::from_str(&r.contains).unwrap_or_default(),
            min_amount: r.min_amount,
            max_amount: r.max_amount,
        })
        .collect())
}

/// The account's rules, ready to apply. Stored rules were checked when saved; any that no
/// longer pass are skipped.
pub async fn account_rules(db: &Db, account_id: i64) -> Result<Vec<CategoryRule>, Response> {
    Ok(load_rules(db, account_id)
        .await?
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r.to_rule() {
            Ok(rule) => Some(rule),
            Err(e) => {
                tracing::warn!(account_id, rule = i + 1, error = %e, "skipping invalid type rule");
                None
            }
        })
        .collect())
}

#[derive(Serialize)]
struct RulesResponse {
    rules: Vec<RuleJson>,
}

/// GET /api/orgs/{org}/statement-accounts/{id}/rules
async fn get_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id)): UrlPath<(String, i64)>,
) -> Result<Json<RulesResponse>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    Ok(Json(RulesResponse {
        rules: load_rules(&state.db, account_id).await?,
    }))
}

#[derive(Deserialize)]
struct RulesBody {
    rules: Vec<RuleJson>,
}

/// PUT /api/orgs/{org}/statement-accounts/{id}/rules — replaces the account's ordered list.
async fn put_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id)): UrlPath<(String, i64)>,
    Json(body): Json<RulesBody>,
) -> Result<Json<RulesResponse>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    validate_rules(&body.rules)?;
    let mut body = body;
    for (i, r) in body.rules.iter_mut().enumerate() {
        r.category = crate::statement_types::canonical_name(&state.db, org.id, &r.category)
            .await
            .map_err(db_error)?
            .ok_or_else(|| {
                bad_request(&format!(
                    "Rule {}: {:?} isn't on the categories list. Add it first.",
                    i + 1,
                    r.category.trim()
                ))
            })?;
    }
    let mut batch = db::Batch::new();
    batch.push(
        "DELETE FROM statement_type_rules WHERE account_id = ?",
        params![account_id],
    );
    for (position, r) in body.rules.iter().enumerate() {
        batch.push(
            "INSERT INTO statement_type_rules
                 (org_id, account_id, position, type, contains, min_amount, max_amount)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                org.id,
                account_id,
                position,
                r.category.trim(),
                serde_json::to_string(&r.contains).unwrap_or_else(|_| "[]".into()),
                r.min_amount,
                r.max_amount,
            ],
        );
    }
    db::run(&state.db, batch).await.map_err(db_error)?;
    tracing::info!(org = %org.slug, account = account_id, rules = body.rules.len(), user = user.id, "type rules saved");
    Ok(Json(RulesResponse {
        rules: load_rules(&state.db, account_id).await?,
    }))
}

// ---- applying saved rules to stored rows ----

#[derive(Deserialize)]
struct ApplyScope {
    /// 0-based index of one saved rule; `None` = all of them.
    #[serde(default)]
    rule: Option<usize>,
}

/// A stored row whose type the saved rules would change.
#[derive(Serialize, Debug, PartialEq)]
struct TypeChange {
    /// The row's own id.
    id: i64,
    date: String,
    description: String,
    amount: String,
    currency: String,
    /// Its type now, and the type the rule gives it.
    from: Option<String>,
    to: String,
    /// 0-based index of the rule that matches it first.
    rule: usize,
}

#[derive(Deserialize)]
struct StoredRow {
    id: i64,
    date: String,
    description: String,
    amount: String,
    currency: String,
    #[serde(rename = "type")]
    category: Option<String>,
    type_manual: bool,
}

/// The rows in the account the saved rules would retype, as saving a statement would:
/// the first matching rule wins, a hand-set type is never changed, and a row no rule
/// matches keeps its type. With `only`, just the rows that rule is the first match for.
async fn type_changes(
    db: &Db,
    account_id: i64,
    only: Option<usize>,
) -> Result<Vec<TypeChange>, Response> {
    let rules = account_rules(db, account_id).await?;
    if only.is_some_and(|i| i >= rules.len()) {
        return Err(bad_request(
            "That rule isn't saved. Save the rules and try again.",
        ));
    }
    let rows: Vec<StoredRow> = db::query_as(
        "SELECT id, date, description, amount, currency, type, type_manual
         FROM statement_transactions WHERE account_id = ? AND type_manual = 0
         ORDER BY date DESC, id DESC",
    )
    .bind(account_id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    Ok(changes_for(&rules, rows, only))
}

fn changes_for(
    rules: &[CategoryRule],
    rows: Vec<StoredRow>,
    only: Option<usize>,
) -> Vec<TypeChange> {
    // Rules aren't limited by format any more; any format matches the same way.
    let format = Format::ALL[0];
    rows.into_iter()
        .filter(|r| !r.type_manual)
        .filter_map(|r| {
            let t = Transaction {
                id: 0,
                occurrence: 0,
                date: r.date,
                description: r.description,
                amount: r.amount,
                currency: r.currency,
                category: None,
                effective_date: None,
                hand_added: false,
            };
            let rule = rules.iter().position(|rule| rule.matches(&t, format))?;
            if only.is_some_and(|i| i != rule) {
                return None;
            }
            let to = rules[rule].category.clone();
            (r.category.as_deref() != Some(to.as_str())).then_some(TypeChange {
                id: r.id,
                date: t.date,
                description: t.description,
                amount: t.amount,
                currency: t.currency,
                from: r.category,
                to,
                rule,
            })
        })
        .collect()
}

#[derive(Serialize)]
struct ApplyPreview {
    changes: Vec<TypeChange>,
}

/// POST /api/orgs/{org}/statement-accounts/{id}/rules/apply/preview — the stored rows the
/// saved rules (or one of them, `{"rule": i}`) would retype. Changes nothing.
async fn apply_preview(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id)): UrlPath<(String, i64)>,
    Json(scope): Json<ApplyScope>,
) -> Result<Json<ApplyPreview>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    Ok(Json(ApplyPreview {
        changes: type_changes(&state.db, account_id, scope.rule).await?,
    }))
}

#[derive(Deserialize)]
struct ApplyBody {
    #[serde(default)]
    rule: Option<usize>,
    /// The changes the user accepted, row id -> the type they were shown.
    accepted: HashMap<i64, String>,
}

#[derive(Serialize)]
struct Applied {
    updated: u64,
}

/// POST /api/orgs/{org}/statement-accounts/{id}/rules/apply — retype the accepted rows.
/// Worked out again first: a row is only changed if the rules still give it the type the
/// user saw (so rules or rows changed in between can't apply something unseen).
async fn apply_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, account_id)): UrlPath<(String, i64)>,
    Json(body): Json<ApplyBody>,
) -> Result<Json<Applied>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    statement_accounts::find(&state.db, org.id, account_id).await?;
    let updated = apply_changes(&state.db, account_id, body.rule, &body.accepted).await?;
    tracing::info!(org = %org.slug, account = account_id, rule = ?body.rule, updated, user = user.id, "type rules applied");
    Ok(Json(Applied { updated }))
}

/// Sets the type of each accepted row the saved rules still give that type; how many changed.
async fn apply_changes(
    db: &Db,
    account_id: i64,
    only: Option<usize>,
    accepted: &HashMap<i64, String>,
) -> Result<u64, Response> {
    let mut by_type: HashMap<String, Vec<i64>> = HashMap::new();
    for c in type_changes(db, account_id, only).await? {
        if accepted.get(&c.id) == Some(&c.to) {
            by_type.entry(c.to).or_default().push(c.id);
        }
    }
    let mut batch = db::Batch::new();
    for (typ, ids) in &by_type {
        batch.push(
            "UPDATE statement_transactions
             SET type = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE account_id = ? AND type_manual = 0
               AND id IN (SELECT value FROM json_each(?))",
            params![
                typ,
                account_id,
                serde_json::to_string(ids).expect("ids serialise"),
            ],
        );
    }
    Ok(if by_type.is_empty() {
        0
    } else {
        db::run(db, batch)
            .await
            .map_err(db_error)?
            .iter()
            .map(|r| r.rows_affected)
            .sum()
    })
}

// ---- errors ----

fn too_large(max_bytes: u64) -> Response {
    let mb = max_bytes as f64 / (1024.0 * 1024.0);
    error(
        StatusCode::PAYLOAD_TOO_LARGE,
        &format!("Files can be at most {mb:.0} MB."),
    )
}

fn multipart_error(e: MultipartError) -> Response {
    let status = e.status();
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return error(status, "That file is too large.");
    }
    tracing::debug!(error = %e, "bad multipart upload");
    error(status, "The upload was malformed or interrupted.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stated_account_numbers_are_checked_against_the_identifier() {
        let account = |identifier: Option<&str>| Account {
            id: 1,
            name: "Joint".into(),
            identifier: identifier.map(Into::into),
            ..Account::default()
        };
        assert_eq!(
            check_account(&account(Some("123")), "123"),
            AccountCheck::Match
        );
        assert_eq!(
            check_account(&account(Some(" 123 ")), "123"),
            AccountCheck::Match
        );
        assert_eq!(
            check_account(&account(Some("123")), "999"),
            AccountCheck::Mismatch
        );
        assert_eq!(
            check_account(&account(None), "123"),
            AccountCheck::NoIdentifier
        );
        assert_eq!(
            check_account(&account(Some("123")), ""),
            AccountCheck::NotStated
        );
    }

    #[test]
    fn statement_folders_are_safe_and_per_account() {
        assert_eq!(statement_folder("123456789"), "Statements/123456789");
        assert_eq!(statement_folder(""), "Statements/No account");
        assert_eq!(statement_folder("  "), "Statements/No account");
        assert_eq!(statement_folder("12/34"), "Statements/12-34");
        assert_eq!(statement_folder("..x"), "Statements/x");
    }

    #[test]
    fn display_names_drop_paths_and_control_characters() {
        assert_eq!(
            display_name("C:\\Users\\me\\statement.csv"),
            "statement.csv"
        );
        assert_eq!(display_name("dir/x\u{7}.csv"), "x.csv");
        assert_eq!(display_name("  "), "statement.csv");
        assert_eq!(display_name(".hidden.csv"), "statement.csv");
    }

    fn rule(category: &str, contains: &[&str]) -> RuleJson {
        RuleJson {
            category: category.into(),
            contains: contains.iter().map(|s| s.to_string()).collect(),
            min_amount: None,
            max_amount: None,
        }
    }

    /// A fresh in-memory database with every migration, one org and one user.
    async fn test_db() -> (Db, Org, i64) {
        let db = db::test_db().await;
        let org_id: i64 = db::query_scalar(
            "INSERT INTO organisations (slug, name) VALUES ('acme', 'Acme') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let user_id: i64 = db::query_scalar(
            "INSERT INTO users (issuer, subject) VALUES ('test', '1') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let org = Org {
            id: org_id,
            slug: "acme".into(),
            name: "Acme".into(),
        };
        (db, org, user_id)
    }

    const SYNTHETIC: &[u8] =
        include_bytes!("../../statements-core/tests/fixtures/sb_csv_synthetic.csv");

    fn parsed(rules: &[RuleJson]) -> Statement {
        parse_statement(SYNTHETIC, &validate_rules(rules).unwrap(), &[])
            .unwrap()
            .0
    }

    #[test]
    fn excluded_lines_are_taken_out_and_other_ids_dont_change() {
        let all = parsed(&[]);
        let exclusion: crate::statement_exclusions::RuleJson = serde_json::from_value(
            serde_json::json!({ "contains": ["monthly fee", "service fee"] }),
        )
        .unwrap();
        let rules = vec![crate::statement_exclusions::tests_check(&exclusion)];
        let (kept, _, excluded) = parse_statement(SYNTHETIC, &[], &rules).unwrap();
        assert_eq!(excluded.len(), 2);
        assert!(excluded.iter().all(|t| t.description.contains("FEE")));
        assert_eq!(kept.transactions.len() + 2, all.transactions.len());
        let kept_ids: Vec<i64> = kept.transactions.iter().map(|t| t.id).collect();
        let expected: Vec<i64> = all
            .transactions
            .iter()
            .filter(|t| {
                !t.description.contains("FIXED MONTHLY FEE")
                    && !t.description.contains("OVERDRAFT SERVICE FEE")
            })
            .map(|t| t.id)
            .collect();
        assert_eq!(kept_ids, expected);
    }

    async fn count(db: &Db, sql: &'static str) -> i64 {
        db::query_scalar(sql).fetch_one(db).await.unwrap()
    }

    #[tokio::test]
    async fn saving_again_updates_rows_and_never_duplicates() {
        let (db, org, user) = test_db().await;
        let st = parsed(&[]);
        let n = st.transactions.len() as i64;
        let first = store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        let d1 = upload_detail(&db, org.id, first).await.unwrap();
        assert_eq!((d1.upload.rows_new, d1.upload.rows_updated), (n, 0));

        let second = store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        let d2 = upload_detail(&db, org.id, second).await.unwrap();
        assert_eq!((d2.upload.rows_new, d2.upload.rows_updated), (0, n));
        assert_eq!(
            count(&db, "SELECT count(*) FROM statement_transactions").await,
            n
        );
        // Both uploads list every row, in file order, with the prod ids.
        assert_eq!(d2.transactions.len() as i64, n);
        let ids: Vec<i64> = st.transactions.iter().map(|t| t.id).collect();
        let stored: Vec<i64> = d2.transactions.iter().map(|t| t.txn_id).collect();
        assert_eq!(stored, ids);
    }

    #[tokio::test]
    async fn a_repeated_id_in_one_file_counts_as_new_once_then_updated() {
        let (db, org, user) = test_db().await;
        let mut st = parsed(&[]);
        let n = st.transactions.len() as i64;
        st.transactions.push(st.transactions[0].clone());
        let id = store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        let d = upload_detail(&db, org.id, id).await.unwrap();
        assert_eq!(
            (
                d.upload.rows_parsed,
                d.upload.rows_new,
                d.upload.rows_updated
            ),
            (n + 1, n, 1)
        );
        assert_eq!(
            count(&db, "SELECT count(*) FROM statement_transactions").await,
            n
        );
        // The repeat shares the first one's link (and position).
        assert_eq!(d.transactions.len() as i64, n);
    }

    #[tokio::test]
    async fn rule_types_overwrite_but_blanks_never_do() {
        let (db, org, user) = test_db().await;
        let typed = parsed(&[rule("PAYMENT", &["PAYMENT"])]);
        let first = store(&db, &org, user, "a.csv", "x", &typed, None)
            .await
            .unwrap();
        let before = upload_detail(&db, org.id, first)
            .await
            .unwrap()
            .transactions;
        assert!(before
            .iter()
            .any(|t| t.category.as_deref() == Some("PAYMENT")));

        // No rules: types stay.
        store(&db, &org, user, "a.csv", "x", &parsed(&[]), None)
            .await
            .unwrap();
        let kept = upload_detail(&db, org.id, first)
            .await
            .unwrap()
            .transactions;
        assert_eq!(
            kept.iter().map(|t| t.category.clone()).collect::<Vec<_>>(),
            before
                .iter()
                .map(|t| t.category.clone())
                .collect::<Vec<_>>()
        );

        // A different rule's type replaces it.
        store(
            &db,
            &org,
            user,
            "a.csv",
            "x",
            &parsed(&[rule("PAID", &["PAYMENT"])]),
            None,
        )
        .await
        .unwrap();
        let after = upload_detail(&db, org.id, first)
            .await
            .unwrap()
            .transactions;
        assert!(after
            .iter()
            .all(|t| t.category.as_deref() != Some("PAYMENT")));
        assert!(after.iter().any(|t| t.category.as_deref() == Some("PAID")));
    }

    #[tokio::test]
    async fn hand_set_types_survive_saves_and_cleared_ones_are_typed_again() {
        let (db, org, user) = test_db().await;
        let typed = parsed(&[rule("PAYMENT", &["PAYMENT"])]);
        let first = store(&db, &org, user, "a.csv", "x", &typed, None)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        let paid = rows
            .iter()
            .find(|r| r.category.as_deref() == Some("PAYMENT"))
            .unwrap();
        let other = rows.iter().find(|r| r.category.is_none()).unwrap();

        // By hand: one gets a different type, the other one too.
        assert!(
            crate::statement_types::set_type(&db, org.id, paid.id, Some("MINE"))
                .await
                .unwrap()
        );
        assert!(
            crate::statement_types::set_type(&db, org.id, other.id, Some("ALSO MINE"))
                .await
                .unwrap()
        );
        store(&db, &org, user, "a.csv", "x", &typed, None)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        let by_id = |id| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by_id(paid.id).category.as_deref(), Some("MINE"));
        assert!(by_id(paid.id).type_manual);
        assert_eq!(by_id(other.id).category.as_deref(), Some("ALSO MINE"));

        // Cleared by hand: no longer hand-set, so the next save types it from the rules.
        crate::statement_types::set_type(&db, org.id, paid.id, None)
            .await
            .unwrap();
        store(&db, &org, user, "a.csv", "x", &typed, None)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        let row = rows.iter().find(|r| r.id == paid.id).unwrap();
        assert_eq!(row.category.as_deref(), Some("PAYMENT"));
        assert!(!row.type_manual);
        // Another org's row can't be touched.
        assert!(
            !crate::statement_types::set_type(&db, org.id + 1, paid.id, Some("X"))
                .await
                .unwrap()
        );
        let _ = first;
    }

    #[tokio::test]
    async fn types_picked_in_the_preview_are_hand_set() {
        let (db, org, user) = test_db().await;
        let typed = parsed(&[rule("PAYMENT", &["PAYMENT"])]);
        let paid = typed
            .transactions
            .iter()
            .find(|t| t.category.is_some())
            .unwrap()
            .id;
        let plain = typed
            .transactions
            .iter()
            .find(|t| t.category.is_none())
            .unwrap()
            .id;
        let row = |rows: &[OrgTransaction], id: i64| {
            let r = rows.iter().find(|r| r.txn_id == id).unwrap();
            (r.category.clone(), r.type_manual)
        };

        // New rows: a pick is hand-set; a cleared rule type is stored blank, not hand-set.
        let picks = Picks::from([(plain, Some("MINE".to_string())), (paid, None)]);
        let acc = test_account(&db, org.id, "123456789").await;
        store_with(&db, &org, acc, user, "a.csv", "x", &typed, None, &picks)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        assert_eq!(row(&rows, plain), (Some("MINE".into()), true));
        assert_eq!(row(&rows, paid), (None, false));

        // Saved again without picks: the hand-set one stays; the cleared one is typed again.
        store(&db, &org, user, "a.csv", "x", &typed, None)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        assert_eq!(row(&rows, plain), (Some("MINE".into()), true));
        assert_eq!(row(&rows, paid), (Some("PAYMENT".into()), false));

        // A pick also replaces an existing hand-set type.
        let picks = Picks::from([(plain, Some("OTHER".to_string()))]);
        store_with(&db, &org, acc, user, "a.csv", "x", &typed, None, &picks)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        assert_eq!(row(&rows, plain), (Some("OTHER".into()), true));
    }

    #[tokio::test]
    async fn saved_rules_can_be_applied_to_stored_rows() {
        let (db, org, user) = test_db().await;
        store(&db, &org, user, "a.csv", "x", &parsed(&[]), None)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        let account = rows[0].account_id;
        let cards: Vec<i64> = rows
            .iter()
            .filter(|r| r.description.to_uppercase().contains("DEBIT CARD"))
            .map(|r| r.id)
            .collect();
        assert!(cards.len() >= 2, "the sample has card purchases");
        // One is hand-set: applying rules never touches it.
        crate::statement_types::set_type(&db, org.id, cards[0], Some("MINE"))
            .await
            .unwrap();
        for (position, (typ, contains)) in [("CARD", "DEBIT CARD"), ("OTHER", "CARD")]
            .into_iter()
            .enumerate()
        {
            db::query(
                "INSERT INTO statement_type_rules (org_id, account_id, position, type, contains)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(org.id)
            .bind(account)
            .bind(position as i64)
            .bind(typ)
            .bind(serde_json::to_string(&[contains]).unwrap())
            .execute(&db)
            .await
            .unwrap();
        }

        // Rule 1 wins; rule 2 never comes first, so applying it alone changes nothing.
        let all = type_changes(&db, account, None).await.unwrap();
        assert_eq!(
            all.iter().map(|c| c.id).collect::<BTreeSet<_>>(),
            cards[1..].iter().copied().collect()
        );
        assert!(all
            .iter()
            .all(|c| c.to == "CARD" && c.rule == 0 && c.from.is_none()));
        assert!(type_changes(&db, account, Some(1))
            .await
            .unwrap()
            .is_empty());
        assert!(type_changes(&db, account, Some(2)).await.is_err());

        // Only accepted rows, with the type the user saw, are changed.
        let mut accepted: HashMap<i64, String> = all.iter().map(|c| (c.id, c.to.clone())).collect();
        let skipped = all[0].id;
        accepted.remove(&skipped);
        accepted.insert(cards[0], "CARD".into()); // hand-set: not a change on offer
        if let Some(c) = all.get(1) {
            accepted.insert(c.id, "STALE".into()); // not what the rules give now
        }
        let stale = all.get(1).map(|c| c.id);
        let updated = apply_changes(&db, account, None, &accepted).await.unwrap();
        assert_eq!(
            updated as usize,
            all.len() - 1 - usize::from(stale.is_some())
        );

        let rows = org_transactions(&db, org.id, None).await.unwrap();
        let typ = |id| rows.iter().find(|r| r.id == id).unwrap().category.clone();
        assert_eq!(typ(cards[0]).as_deref(), Some("MINE"));
        assert_eq!(typ(skipped), None);
        if let Some(id) = stale {
            assert_eq!(typ(id), None);
        }
        // What's left on offer is exactly what wasn't applied.
        let left: BTreeSet<i64> = type_changes(&db, account, None)
            .await
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(left, [Some(skipped), stale].into_iter().flatten().collect());
    }

    #[tokio::test]
    async fn same_ids_in_another_account_or_org_are_separate_rows() {
        let (db, org, user) = test_db().await;
        let st = parsed(&[]);
        let n = st.transactions.len() as i64;
        store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        let mut other_account = st.clone();
        other_account.account_number = Some("999".into());
        store(&db, &org, user, "b.csv", "x", &other_account, None)
            .await
            .unwrap();
        let other_org_id: i64 = db::query_scalar(
            "INSERT INTO organisations (slug, name) VALUES ('other', 'Other') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let other_org = Org {
            id: other_org_id,
            slug: "other".into(),
            name: "Other".into(),
        };
        store(&db, &other_org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        assert_eq!(
            count(&db, "SELECT count(*) FROM statement_transactions").await,
            3 * n
        );
    }

    #[tokio::test]
    async fn org_rows_list_each_row_once_with_its_latest_upload() {
        let (db, org, user) = test_db().await;
        let st = parsed(&[]);
        store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        let second = store(&db, &org, user, "b.csv", "x", &st, None)
            .await
            .unwrap();
        let rows = org_transactions(&db, org.id, None).await.unwrap();
        assert_eq!(rows.len(), st.transactions.len());
        assert!(rows.iter().all(|r| r.upload_id == Some(second)));
        let first = rows.iter().find(|r| r.date == "2027-12-29").unwrap();
        assert_eq!(first.month, "Dec 2027");
        assert!(rows.windows(2).all(|w| w[0].date >= w[1].date));
        assert!(org_transactions(&db, org.id + 1, None)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn rows_and_accounts_by_account() {
        let (db, org, user) = test_db().await;
        let st = parsed(&[]);
        let n = st.transactions.len();
        store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        let mut other = st.clone();
        other.account_number = Some("999".into());
        other.transactions.truncate(3);
        store(&db, &org, user, "b.csv", "x", &other, None)
            .await
            .unwrap();

        assert_eq!(
            org_transactions(&db, org.id, None).await.unwrap().len(),
            n + 3
        );
        let main = test_account(&db, org.id, "123456789").await;
        let other_account = test_account(&db, org.id, "999").await;
        let mine = org_transactions(&db, org.id, Some(main)).await.unwrap();
        assert_eq!(mine.len(), n);
        assert!(mine.iter().all(|r| r.account_id == main));
        assert_eq!(
            org_transactions(&db, org.id, Some(other_account))
                .await
                .unwrap()
                .len(),
            3
        );
        assert!(org_transactions(&db, org.id, Some(other_account + 100))
            .await
            .unwrap()
            .is_empty());

        let accounts = statement_accounts::org_accounts(&db, org.id).await.unwrap();
        let summary: Vec<(&str, i64, i64)> = accounts
            .iter()
            .map(|a| (a.account.name.as_str(), a.uploads, a.rows))
            .collect();
        // By name.
        assert_eq!(summary, [("123456789", 1, n as i64), ("999", 1, 3)]);
    }

    #[tokio::test]
    async fn deleting_keeps_rows_other_uploads_contain() {
        let (db, org, user) = test_db().await;
        let st = parsed(&[]);
        let n = st.transactions.len() as i64;
        let first = store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        let mut shorter = st.clone();
        shorter.transactions.truncate(2);
        let second = store(&db, &org, user, "b.csv", "x", &shorter, None)
            .await
            .unwrap();

        assert_eq!(
            delete_upload(&db, org.id, first).await.unwrap(),
            Some(n as u64 - 2)
        );
        assert_eq!(
            count(&db, "SELECT count(*) FROM statement_transactions").await,
            2
        );
        assert_eq!(delete_upload(&db, org.id, second).await.unwrap(), Some(2));
        assert_eq!(
            count(&db, "SELECT count(*) FROM statement_transactions").await,
            0
        );
        // Unknown, or another org's: not found.
        assert_eq!(delete_upload(&db, org.id, second).await.unwrap(), None);
        let third = store(&db, &org, user, "a.csv", "x", &st, None)
            .await
            .unwrap();
        assert_eq!(delete_upload(&db, org.id + 1, third).await.unwrap(), None);
    }

    #[test]
    fn rules_are_validated_with_their_position() {
        assert!(validate_rules(&[rule("FEES", &["fee"])]).is_ok());
        assert!(rule("X", &[]).to_rule().is_err());
        assert!(rule(&"x".repeat(201), &["a"]).to_rule().is_err());
        assert!(validate_rules(&vec![rule("X", &["a"]); MAX_RULES + 1]).is_err());
    }

    const SHEET: &[u8] =
        include_bytes!("../../statements-core/tests/fixtures/googlesheet_csv_synthetic.csv");

    /// Parses the Google Sheet fixture with `rules` and saves it into `account` as the save
    /// handler does, with `user_picks` as picked in the preview.
    async fn save_sheet(
        db: &Db,
        org: &Org,
        user: i64,
        account: i64,
        rules: &[RuleJson],
        user_picks: Picks,
    ) -> Statement {
        let (st, file_types, _) =
            parse_statement(SHEET, &validate_rules(rules).unwrap(), &[]).unwrap();
        let picks = save_picks(db, org.id, account, &file_types, user_picks)
            .await
            .unwrap();
        store_with(db, org, account, user, "sheet.csv", "s", &st, None, &picks)
            .await
            .unwrap();
        st
    }

    #[tokio::test]
    async fn google_sheet_types_are_hand_set_and_join_the_list() {
        let (db, org, user) = test_db().await;
        let account = test_account(&db, org.id, "").await;
        // "Fees" is already on the list: the sheet's "FEES" takes its spelling.
        db::query("INSERT INTO statement_types (org_id, name) VALUES (?, 'Fees')")
            .bind(org.id)
            .execute(&db)
            .await
            .unwrap();
        // A rule only types rows the sheet left blank.
        let st = save_sheet(
            &db,
            &org,
            user,
            account,
            &[rule("MUNI", &["MUNICIPALITY", "FEE"])],
            Picks::new(),
        )
        .await;
        assert_eq!(st.format, Format::GoogleSheetCsv);

        let rows = org_transactions(&db, org.id, Some(account)).await.unwrap();
        let by_desc = |d: &str| rows.iter().find(|r| r.description == d).unwrap();
        let fee_at = |amount: &str| {
            rows.iter()
                .find(|r| r.description == "FIXED MONTHLY FEE " && r.amount == amount)
                .unwrap()
        };
        let fee = fee_at("-120.00");
        assert_eq!(
            (fee.category.as_deref(), fee.type_manual),
            (Some("Fees"), true)
        );
        let muni = by_desc("IB PAYMENT TO EXAMPLE MUNICIPALITY ACC 001");
        assert_eq!(
            (muni.category.as_deref(), muni.type_manual),
            (Some("MUNI"), false)
        );
        // The zeroed duplicate (its n8n `_` removed) has no type in the sheet, so the rule types it.
        assert_eq!(fee_at("0.00").category.as_deref(), Some("MUNI"));

        let bond = by_desc("REGISTERED DC DEBIT HOME LOAN DC   0000001       280201");
        assert_eq!(bond.effective_date.as_deref(), Some("2028-01-31"));
        assert_eq!(bond.category.as_deref(), Some("BOND REPAYMENT"));
        assert_eq!(rows.len(), 9);

        let names: Vec<String> =
            db::query_scalar("SELECT name FROM statement_types WHERE org_id = ? ORDER BY name")
                .bind(org.id)
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(names, ["BOND REPAYMENT", "Fees", "RENT"]);
    }

    #[tokio::test]
    async fn google_sheet_types_never_replace_a_hand_set_one_but_preview_picks_do() {
        let (db, org, user) = test_db().await;
        let account = test_account(&db, org.id, "").await;
        save_sheet(&db, &org, user, account, &[], Picks::new()).await;
        let rows = org_transactions(&db, org.id, Some(account)).await.unwrap();
        let fee = rows
            .iter()
            .find(|r| r.description == "FIXED MONTHLY FEE " && r.amount == "-120.00")
            .unwrap();
        let rent = rows.iter().find(|r| r.description == "Split rent").unwrap();
        crate::statement_types::set_type(&db, org.id, fee.id, Some("RENT"))
            .await
            .unwrap();

        // Saving the sheet again keeps the type set in the app; a preview pick still wins.
        let picks = Picks::from([(rent.txn_id, Some("FEES".to_string()))]);
        save_sheet(&db, &org, user, account, &[], picks).await;
        let rows = org_transactions(&db, org.id, Some(account)).await.unwrap();
        let typ = |id: i64| rows.iter().find(|r| r.id == id).unwrap().category.clone();
        assert_eq!(typ(fee.id).as_deref(), Some("RENT"));
        assert_eq!(typ(rent.id).as_deref(), Some("FEES"));
    }

    #[tokio::test]
    async fn a_statement_without_an_effective_date_keeps_the_stored_one() {
        let (db, org, user) = test_db().await;
        let account = test_account(&db, org.id, "").await;
        let st = save_sheet(&db, &org, user, account, &[], Picks::new()).await;

        // The same rows again, as a bank statement would give them: no effective dates.
        let mut bank = st.clone();
        bank.format = Format::SbCsv;
        for t in &mut bank.transactions {
            t.effective_date = None;
            t.category = None;
        }
        store_with(
            &db,
            &org,
            account,
            user,
            "bank.csv",
            "b",
            &bank,
            None,
            &Picks::new(),
        )
        .await
        .unwrap();
        let rows = org_transactions(&db, org.id, Some(account)).await.unwrap();
        let dated: Vec<_> = rows
            .iter()
            .filter_map(|r| r.effective_date.as_deref())
            .collect();
        assert_eq!(dated, ["2028-02-29", "2028-01-31"]);
    }

    #[tokio::test]
    async fn a_pdf_statement_keeps_the_saved_text_of_rows_already_saved() {
        let (db, org, user) = test_db().await;
        let account = test_account(&db, org.id, "").await;
        let st = save_sheet(&db, &org, user, account, &[], Picks::new()).await;
        let before = org_transactions(&db, org.id, Some(account)).await.unwrap();

        // The same rows (same ids) as a PDF reads them: other text, one new row.
        let mut pdf = st.clone();
        pdf.format = Format::SbPdf2026_01;
        for t in &mut pdf.transactions {
            t.description = format!("{}  (as the PDF reads it)", t.description);
            t.amount = format!("{}0", t.amount);
        }
        let mut new = pdf.transactions[0].clone();
        new.id = 999;
        new.description = "NEW FROM THE PDF".into();
        pdf.transactions.push(new);
        store_with(
            &db,
            &org,
            account,
            user,
            "s.pdf",
            "p",
            &pdf,
            None,
            &Picks::new(),
        )
        .await
        .unwrap();

        let after = org_transactions(&db, org.id, Some(account)).await.unwrap();
        assert_eq!(after.len(), before.len() + 1);
        for b in &before {
            let a = after.iter().find(|r| r.id == b.id).unwrap();
            assert_eq!((&a.description, &a.amount), (&b.description, &b.amount));
        }
        assert!(after.iter().any(|r| r.description == "NEW FROM THE PDF"));
    }

    #[tokio::test]
    async fn a_bank_statement_makes_a_hand_added_row_its_own() {
        let (db, org, user) = test_db().await;
        let account = test_account(&db, org.id, "").await;
        let st = save_sheet(&db, &org, user, account, &[], Picks::new()).await;
        let hand = |rows: &[OrgTransaction]| {
            rows.iter()
                .filter(|r| r.hand_added)
                .map(|r| r.description.clone())
                .collect::<Vec<_>>()
        };
        let rows = org_transactions(&db, org.id, Some(account)).await.unwrap();
        assert_eq!(hand(&rows), ["Split rent", "Split rent"]);

        // A bank statement with the first of them.
        let mut bank = st.clone();
        bank.format = Format::SbCsv;
        bank.transactions.retain(|t| t.hand_added);
        bank.transactions.truncate(1);
        bank.transactions[0].hand_added = false;
        store_with(
            &db,
            &org,
            account,
            user,
            "bank.csv",
            "b",
            &bank,
            None,
            &Picks::new(),
        )
        .await
        .unwrap();
        let rows = org_transactions(&db, org.id, Some(account)).await.unwrap();
        assert_eq!(hand(&rows), ["Split rent"]);

        // Saving the sheet again doesn't make it hand-added again.
        save_sheet(&db, &org, user, account, &[], Picks::new()).await;
        let rows = org_transactions(&db, org.id, Some(account)).await.unwrap();
        assert_eq!(hand(&rows).len(), 1);
    }
}
