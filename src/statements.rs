//! Bank statements, per organisation (`/portal/<org>/statements`).
//!
//! A statement file is parsed by `statements-core` (format detection, the prod n8n row ids,
//! type rules) and saved as an *upload*. Its rows are stored once per
//! `(org, account, n8n id)` and linked to every upload that contained them, so saving a
//! statement again updates rows instead of duplicating them. A save never blanks a type: when
//! no rule matches, the stored type is kept.
//!
//! The n8n id is the prod hash of date, description and amount, unchanged: it's what the
//! Google Sheets copy is keyed on (`statements_google`).
//!
//! API: list and view uploads, view type rules (`read-statements`); preview, save and delete
//! uploads, edit type rules (`write-statements`). Saving a copy to Documents also needs
//! `write-docs`.

use std::{collections::HashSet, sync::Arc};

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
use sqlx::SqlitePool;
use statements_core::{
    categories::{categorise, CategoryRule},
    hash::DateFormat,
    row::month_label,
    statement::{self, Format, Statement, Transaction},
};

use crate::{
    api::{bad_request, db_error, error, not_found},
    auth::CurrentUser,
    documents,
    orgs::Org,
    rbac::{self, Permission},
    AppState,
};

/// Where "Save to Documents" puts statements unless the uploader picks another folder.
pub const DEFAULT_DOCUMENTS_FOLDER: &str = "Statements";
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
        .route("/api/orgs/{org}/statement-accounts", get(accounts))
        .route(
            "/api/orgs/{org}/statement-transactions",
            get(all_transactions),
        )
        .route(
            "/api/orgs/{org}/statement-rules",
            get(get_rules).put(put_rules),
        )
        .route("/api/orgs/{org}/statement-rules/test", post(test_rules))
}

// ---- reading and parsing an upload ----

struct UploadForm {
    filename: String,
    bytes: Bytes,
    save_to_documents: bool,
    folder: Option<String>,
}

/// Multipart fields: `file` (required), `save_to_documents` ("true"), `folder`.
async fn read_form(mut multipart: Multipart, max_bytes: u64) -> Result<UploadForm, Response> {
    let mut file = None;
    let mut save_to_documents = false;
    let mut folder = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("file") => {
                let name = display_name(field.file_name().unwrap_or_default());
                let bytes = field.bytes().await.map_err(multipart_error)?;
                file = Some((name, bytes));
            }
            Some("save_to_documents") => {
                save_to_documents = field.text().await.map_err(multipart_error)? == "true";
            }
            Some("folder") => {
                let text = field.text().await.map_err(multipart_error)?;
                folder = Some(text).filter(|f| !f.trim().is_empty());
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
    if !filename.to_ascii_lowercase().ends_with(".csv") {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Only CSV statements are supported for now.",
        ));
    }
    Ok(UploadForm {
        filename,
        bytes,
        save_to_documents,
        folder,
    })
}

/// Detects the format, parses, assigns the n8n ids and applies the type rules.
fn parse_statement(bytes: &[u8], rules: &[CategoryRule]) -> Result<Statement, Response> {
    let mut statement = statement::parse(bytes, DateFormat::UtcTimestamp).map_err(|e| {
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
    categorise(rules, &mut statement);
    Ok(statement)
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

fn account_of(statement: &Statement) -> String {
    statement.account_number.clone().unwrap_or_default()
}

// ---- preview ----

#[derive(Serialize)]
struct Preview {
    filename: String,
    format: &'static str,
    format_name: &'static str,
    /// As stated in the file, if the format states one.
    account_number: Option<String>,
    first_date: Option<String>,
    last_date: Option<String>,
    /// Rows in the file.
    rows: usize,
    /// Rows a type rule matched.
    typed: usize,
    /// Rows not stored yet, and rows already stored (they'll be updated).
    new: usize,
    existing: usize,
    transactions: Vec<PreviewRow>,
    /// The default "Save to Documents" folder.
    documents_folder: &'static str,
}

#[derive(Serialize)]
struct PreviewRow {
    txn_id: i32,
    date: String,
    amount: String,
    description: String,
    currency: String,
    #[serde(rename = "type")]
    category: Option<String>,
    /// Already stored for this org and account.
    stored: bool,
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
    let rules = org_rules(&state.db, org.id).await?;
    let st = parse_statement(&form.bytes, &rules)?;
    let stored = stored_ids(&state.db, org.id, &account_of(&st)).await?;

    let txs = &st.transactions;
    let existing = txs.iter().filter(|t| stored.contains(&t.id)).count();
    Ok(Json(Preview {
        filename: form.filename,
        format: st.format.id(),
        format_name: st.format.description(),
        account_number: st.account_number.clone(),
        first_date: txs.iter().map(|t| &t.date).min().cloned(),
        last_date: txs.iter().map(|t| &t.date).max().cloned(),
        rows: txs.len(),
        typed: txs.iter().filter(|t| t.category.is_some()).count(),
        new: txs.len() - existing,
        existing,
        transactions: txs
            .iter()
            .map(|t| PreviewRow {
                txn_id: t.id,
                date: t.date.clone(),
                amount: t.amount.clone(),
                description: t.description.clone(),
                currency: t.currency.clone(),
                category: t.category.clone(),
                stored: stored.contains(&t.id),
            })
            .collect(),
        documents_folder: DEFAULT_DOCUMENTS_FOLDER,
    }))
}

/// The n8n ids already stored for this org and account.
async fn stored_ids(db: &SqlitePool, org_id: i64, account: &str) -> Result<HashSet<i32>, Response> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT txn_id FROM statement_transactions WHERE org_id = ? AND account_number = ?",
    )
    .bind(org_id)
    .bind(account)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    Ok(ids
        .into_iter()
        .filter_map(|id| i32::try_from(id).ok())
        .collect())
}

// ---- save ----

/// POST /api/orgs/{org}/statements — parse again and save: the upload, every row (new rows
/// inserted, stored rows updated), and optionally a copy of the file in Documents.
async fn save(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<UploadDetail>), Response> {
    let (org, perms) = rbac::org_access(&state.db, &user, &slug).await?;
    perms.require(Permission::WriteStatements)?;
    let form = read_form(multipart, state.max_upload_bytes).await?;
    if form.save_to_documents {
        perms.require(Permission::WriteDocs)?;
    }
    let rules = org_rules(&state.db, org.id).await?;
    let st = parse_statement(&form.bytes, &rules)?;
    let sha256 = hex::encode(Sha256::digest(&form.bytes));

    // The file first: storage is the likelier failure, and nothing is in the database yet.
    let document_id = if form.save_to_documents {
        let folder = form.folder.as_deref().unwrap_or(DEFAULT_DOCUMENTS_FOLDER);
        let (id, _, _) = documents::save_bytes(
            &state,
            &org,
            user.id,
            folder,
            &form.filename,
            Some("text/csv".into()),
            form.bytes.clone(),
        )
        .await?;
        Some(id)
    } else {
        None
    };

    let upload_id = store(
        &state.db,
        &org,
        user.id,
        &form.filename,
        &sha256,
        &st,
        document_id,
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
        user = user.id,
        "statement saved"
    );
    Ok((StatusCode::CREATED, Json(detail)))
}

/// One database transaction: the upload row, then each statement row upserted on
/// `(org, account, n8n id)` and linked to the upload in file order.
async fn store(
    db: &SqlitePool,
    org: &Org,
    user_id: i64,
    filename: &str,
    sha256: &str,
    st: &Statement,
    document_id: Option<i64>,
) -> sqlx::Result<i64> {
    let account = account_of(st);
    let mut tx = db.begin().await?;
    let upload_id: i64 = sqlx::query_scalar(
        "INSERT INTO statement_uploads
             (org_id, format, account_number, filename, sha256, rows_parsed, rows_new,
              rows_updated, document_id, uploaded_by)
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?) RETURNING id",
    )
    .bind(org.id)
    .bind(st.format.id())
    .bind(&account)
    .bind(filename)
    .bind(sha256)
    .bind(st.transactions.len() as i64)
    .bind(document_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;

    let (mut new, mut updated) = (0i64, 0i64);
    for (position, t) in st.transactions.iter().enumerate() {
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM statement_transactions
             WHERE org_id = ? AND account_number = ? AND txn_id = ?",
        )
        .bind(org.id)
        .bind(&account)
        .bind(t.id)
        .fetch_optional(&mut *tx)
        .await?;
        let row_id = match existing {
            Some(id) => {
                // A blank type never replaces a stored one.
                sqlx::query(
                    "UPDATE statement_transactions
                     SET date = ?, amount = ?, description = ?, currency = ?,
                         type = coalesce(?, type),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE id = ?",
                )
                .bind(&t.date)
                .bind(&t.amount)
                .bind(&t.description)
                .bind(&t.currency)
                .bind(&t.category)
                .bind(id)
                .execute(&mut *tx)
                .await?;
                updated += 1;
                id
            }
            None => {
                new += 1;
                sqlx::query_scalar(
                    "INSERT INTO statement_transactions
                         (org_id, account_number, txn_id, date, amount, description, currency, type)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
                )
                .bind(org.id)
                .bind(&account)
                .bind(t.id)
                .bind(&t.date)
                .bind(&t.amount)
                .bind(&t.description)
                .bind(&t.currency)
                .bind(&t.category)
                .fetch_one(&mut *tx)
                .await?
            }
        };
        // OR IGNORE: two rows of one file with the same id (on different days) share a row.
        sqlx::query(
            "INSERT OR IGNORE INTO statement_upload_transactions (upload_id, transaction_id, position)
             VALUES (?, ?, ?)",
        )
        .bind(upload_id)
        .bind(row_id)
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE statement_uploads SET rows_new = ?, rows_updated = ? WHERE id = ?")
        .bind(new)
        .bind(updated)
        .bind(upload_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(upload_id)
}

// ---- listing, viewing, deleting ----

#[derive(Serialize, sqlx::FromRow)]
pub struct UploadSummary {
    pub id: i64,
    pub format: String,
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
            "SELECT u.id, u.format, u.account_number, u.filename, u.rows_parsed, u.rows_new,
                    u.rows_updated, u.document_id, u.uploaded_at,
                    coalesce(us.name, us.email) AS uploaded_by,
                    (SELECT min(t.date) FROM statement_upload_transactions l
                         JOIN statement_transactions t ON t.id = l.transaction_id
                         WHERE l.upload_id = u.id) AS first_date,
                    (SELECT max(t.date) FROM statement_upload_transactions l
                         JOIN statement_transactions t ON t.id = l.transaction_id
                         WHERE l.upload_id = u.id) AS last_date
             FROM statement_uploads u LEFT JOIN users us ON us.id = u.uploaded_by ",
            $tail
        )
    };
}

/// A stored row, as linked to an upload.
#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct StoredTransaction {
    pub txn_id: i64,
    pub date: String,
    pub amount: String,
    pub description: String,
    pub currency: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub category: Option<String>,
}

#[derive(Serialize)]
pub struct UploadDetail {
    #[serde(flatten)]
    pub upload: UploadSummary,
    pub format_name: &'static str,
    pub transactions: Vec<StoredTransaction>,
}

/// `?account=<number>`: only that account ('' = statements that state none). Absent = all.
#[derive(Deserialize)]
struct AccountQuery {
    account: Option<String>,
}

/// An account the org has statements for.
#[derive(Serialize, sqlx::FromRow)]
struct AccountSummary {
    /// '' when the statements don't state one.
    account_number: String,
    uploads: i64,
    rows: i64,
    last_uploaded_at: String,
}

/// GET /api/orgs/{org}/statement-accounts — accounts with statements, most recently uploaded
/// first.
async fn accounts(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<Vec<AccountSummary>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(
        org_accounts(&state.db, org.id).await.map_err(db_error)?,
    ))
}

async fn org_accounts(db: &SqlitePool, org_id: i64) -> sqlx::Result<Vec<AccountSummary>> {
    sqlx::query_as(
        "SELECT u.account_number, count(*) AS uploads, max(u.uploaded_at) AS last_uploaded_at,
                (SELECT count(*) FROM statement_transactions t
                 WHERE t.org_id = u.org_id AND t.account_number = u.account_number) AS rows
         FROM statement_uploads u WHERE u.org_id = ?
         GROUP BY u.account_number ORDER BY last_uploaded_at DESC, max(u.id) DESC",
    )
    .bind(org_id)
    .fetch_all(db)
    .await
}

/// GET /api/orgs/{org}/statements?account= — the org's uploads, newest first.
async fn list(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Query(q): Query<AccountQuery>,
) -> Result<Json<Vec<UploadSummary>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    let uploads = sqlx::query_as(select_upload!(
        "WHERE u.org_id = ?1 AND (?2 IS NULL OR u.account_number = ?2)
         ORDER BY u.uploaded_at DESC, u.id DESC"
    ))
    .bind(org.id)
    .bind(q.account)
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;
    Ok(Json(uploads))
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

pub async fn upload_detail(
    db: &SqlitePool,
    org_id: i64,
    id: i64,
) -> Result<UploadDetail, Response> {
    let upload: UploadSummary = sqlx::query_as(select_upload!("WHERE u.id = ? AND u.org_id = ?"))
        .bind(id)
        .bind(org_id)
        .fetch_optional(db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Statement not found."))?;
    let transactions = sqlx::query_as(
        "SELECT t.txn_id, t.date, t.amount, t.description, t.currency, t.type
         FROM statement_upload_transactions l
         JOIN statement_transactions t ON t.id = l.transaction_id
         WHERE l.upload_id = ? ORDER BY l.position",
    )
    .bind(id)
    .fetch_all(db)
    .await
    .map_err(db_error)?;
    let format_name = Format::from_id(&upload.format).map_or("Unknown format", Format::description);
    Ok(UploadDetail {
        upload,
        format_name,
        transactions,
    })
}

/// A stored row with where it came from, for the org-wide transactions grid.
#[derive(Serialize, sqlx::FromRow)]
struct OrgTransaction {
    txn_id: i64,
    account_number: String,
    date: String,
    /// e.g. "Aug 2024", derived from `date` exactly as the sheet's `month` column is.
    #[sqlx(skip)]
    month: String,
    amount: String,
    description: String,
    currency: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    category: Option<String>,
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
        org_transactions(&state.db, org.id, q.account.as_deref())
            .await
            .map_err(db_error)?,
    ))
}

async fn org_transactions(
    db: &SqlitePool,
    org_id: i64,
    account: Option<&str>,
) -> sqlx::Result<Vec<OrgTransaction>> {
    sqlx::query_as(
        "SELECT t.txn_id, t.account_number, t.date, t.amount, t.description, t.currency, t.type,
                t.updated_at, u.id AS upload_id, u.filename AS upload_filename
         FROM statement_transactions t
         LEFT JOIN statement_uploads u ON u.id = (
             SELECT max(l.upload_id) FROM statement_upload_transactions l
             WHERE l.transaction_id = t.id)
         WHERE t.org_id = ?1 AND (?2 IS NULL OR t.account_number = ?2)
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

/// `None` if there's no such upload in the org; otherwise how many rows were deleted with it.
async fn delete_upload(db: &SqlitePool, org_id: i64, id: i64) -> sqlx::Result<Option<u64>> {
    let mut tx = db.begin().await?;
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM statement_uploads WHERE id = ? AND org_id = ?")
            .bind(id)
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Ok(None);
    }
    let linked: Vec<i64> = sqlx::query_scalar(
        "DELETE FROM statement_upload_transactions WHERE upload_id = ? RETURNING transaction_id",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM statement_uploads WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let mut deleted = 0;
    for row in linked {
        deleted += sqlx::query(
            "DELETE FROM statement_transactions WHERE id = ?1
             AND NOT EXISTS (SELECT 1 FROM statement_upload_transactions WHERE transaction_id = ?1)",
        )
        .bind(row)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(Some(deleted))
}

// ---- type rules ----

/// A type rule as the API and the database hold it: `contains` as typed (matching ignores
/// case), formats by id.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RuleJson {
    #[serde(rename = "type")]
    pub category: String,
    pub contains: Vec<String>,
    #[serde(default)]
    pub min_amount: Option<f64>,
    #[serde(default)]
    pub max_amount: Option<f64>,
    #[serde(default)]
    pub formats: Vec<String>,
}

impl RuleJson {
    fn to_rule(&self) -> Result<CategoryRule, String> {
        if self.category.chars().count() > MAX_RULE_TEXT_CHARS {
            return Err("the type is too long".into());
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
        let formats = self
            .formats
            .iter()
            .map(|f| Format::from_id(f).ok_or_else(|| format!("unknown format {f:?}")))
            .collect::<Result<Vec<_>, _>>()?;
        CategoryRule::new(
            &self.category,
            &self.contains,
            self.min_amount,
            self.max_amount,
            formats,
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

#[derive(sqlx::FromRow)]
struct RuleRow {
    #[sqlx(rename = "type")]
    category: String,
    contains: String,
    min_amount: Option<f64>,
    max_amount: Option<f64>,
    formats: String,
}

async fn load_rules(db: &SqlitePool, org_id: i64) -> Result<Vec<RuleJson>, Response> {
    let rows: Vec<RuleRow> = sqlx::query_as(
        "SELECT type, contains, min_amount, max_amount, formats FROM statement_type_rules
         WHERE org_id = ? ORDER BY position, id",
    )
    .bind(org_id)
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
            formats: serde_json::from_str(&r.formats).unwrap_or_default(),
        })
        .collect())
}

/// The org's rules, ready to apply. Stored rules were checked when saved; any that no longer
/// pass (e.g. a format that was removed) are skipped.
pub async fn org_rules(db: &SqlitePool, org_id: i64) -> Result<Vec<CategoryRule>, Response> {
    Ok(load_rules(db, org_id)
        .await?
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r.to_rule() {
            Ok(rule) => Some(rule),
            Err(e) => {
                tracing::warn!(org_id, rule = i + 1, error = %e, "skipping invalid type rule");
                None
            }
        })
        .collect())
}

#[derive(Serialize)]
struct FormatInfo {
    id: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct RulesResponse {
    rules: Vec<RuleJson>,
    /// Formats a rule can be limited to.
    formats: Vec<FormatInfo>,
}

fn formats() -> Vec<FormatInfo> {
    Format::ALL
        .into_iter()
        .map(|f| FormatInfo {
            id: f.id(),
            name: f.description(),
        })
        .collect()
}

/// GET /api/orgs/{org}/statement-rules
async fn get_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<RulesResponse>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(RulesResponse {
        rules: load_rules(&state.db, org.id).await?,
        formats: formats(),
    }))
}

#[derive(Deserialize)]
struct RulesBody {
    rules: Vec<RuleJson>,
}

/// PUT /api/orgs/{org}/statement-rules — replaces the whole ordered list.
async fn put_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Json(body): Json<RulesBody>,
) -> Result<Json<RulesResponse>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    validate_rules(&body.rules)?;
    let save = async {
        let mut tx = state.db.begin().await?;
        sqlx::query("DELETE FROM statement_type_rules WHERE org_id = ?")
            .bind(org.id)
            .execute(&mut *tx)
            .await?;
        for (position, r) in body.rules.iter().enumerate() {
            sqlx::query(
                "INSERT INTO statement_type_rules
                     (org_id, position, type, contains, min_amount, max_amount, formats)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(org.id)
            .bind(position as i64)
            .bind(r.category.trim())
            .bind(serde_json::to_string(&r.contains).unwrap_or_else(|_| "[]".into()))
            .bind(r.min_amount)
            .bind(r.max_amount)
            .bind(serde_json::to_string(&r.formats).unwrap_or_else(|_| "[]".into()))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    };
    save.await.map_err(db_error)?;
    tracing::info!(org = %org.slug, rules = body.rules.len(), user = user.id, "type rules saved");
    Ok(Json(RulesResponse {
        rules: load_rules(&state.db, org.id).await?,
        formats: formats(),
    }))
}

#[derive(Deserialize)]
struct TestBody {
    /// The rules being edited (not necessarily saved).
    rules: Vec<RuleJson>,
    description: String,
    amount: String,
    /// Defaults to the first known format.
    #[serde(default)]
    format: Option<String>,
}

#[derive(Serialize)]
struct TestResult {
    /// 0-based index of the first matching rule.
    rule: Option<usize>,
    #[serde(rename = "type")]
    category: Option<String>,
}

/// POST /api/orgs/{org}/statement-rules/test — which rule would set the type of a
/// description and amount. Uses the same matching as saving a statement.
async fn test_rules(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Json(body): Json<TestBody>,
) -> Result<Json<TestResult>, Response> {
    rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    let rules = validate_rules(&body.rules)?;
    let format = match body.format.as_deref() {
        None => Format::ALL[0],
        Some(id) => Format::from_id(id).ok_or_else(|| bad_request("Unknown format."))?,
    };
    let t = Transaction {
        id: 0,
        date: String::new(),
        description: body.description,
        amount: body.amount.trim().to_string(),
        currency: String::new(),
        category: None,
    };
    let rule = rules.iter().position(|r| r.matches(&t, format));
    Ok(Json(TestResult {
        rule,
        category: rule.map(|i| rules[i].category.clone()),
    }))
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
            formats: vec![],
        }
    }

    /// A fresh in-memory database with every migration, one org and one user.
    async fn test_db() -> (SqlitePool, Org, i64) {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&db)
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        let org_id: i64 = sqlx::query_scalar(
            "INSERT INTO organisations (slug, name) VALUES ('acme', 'Acme') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let user_id: i64 = sqlx::query_scalar(
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
        include_bytes!("../crates/statements-core/tests/fixtures/sb_csv_synthetic.csv");

    fn parsed(rules: &[RuleJson]) -> Statement {
        parse_statement(SYNTHETIC, &validate_rules(rules).unwrap()).unwrap()
    }

    async fn count(db: &SqlitePool, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(db).await.unwrap()
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
        let ids: Vec<i64> = st.transactions.iter().map(|t| i64::from(t.id)).collect();
        let stored: Vec<i64> = d2.transactions.iter().map(|t| t.txn_id).collect();
        assert_eq!(stored, ids);
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
        let other_org_id: i64 = sqlx::query_scalar(
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
        let mine = org_transactions(&db, org.id, Some("123456789"))
            .await
            .unwrap();
        assert_eq!(mine.len(), n);
        assert!(mine.iter().all(|r| r.account_number == "123456789"));
        assert_eq!(
            org_transactions(&db, org.id, Some("999"))
                .await
                .unwrap()
                .len(),
            3
        );
        assert!(org_transactions(&db, org.id, Some(""))
            .await
            .unwrap()
            .is_empty());

        let accounts = org_accounts(&db, org.id).await.unwrap();
        let summary: Vec<(&str, i64, i64)> = accounts
            .iter()
            .map(|a| (a.account_number.as_str(), a.uploads, a.rows))
            .collect();
        // Most recently uploaded first.
        assert_eq!(summary, [("999", 1, 3), ("123456789", 1, n as i64)]);
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
        let mut bad_format = rule("X", &["a"]);
        bad_format.formats = vec!["pdf".into()];
        assert!(rule("X", &[]).to_rule().is_err());
        assert!(bad_format.to_rule().unwrap_err().contains("unknown format"));
        assert!(rule(&"x".repeat(201), &["a"]).to_rule().is_err());
        assert!(validate_rules(&vec![rule("X", &["a"]); MAX_RULES + 1]).is_err());
    }
}
