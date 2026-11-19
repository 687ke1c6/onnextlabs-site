//! One statement row: its details, where it came from, its type, and the documents attached to
//! it (receipts, invoices...).
//!
//! Attached documents are links to the org's existing Documents, many to many. Removing a link
//! never touches the document; a link whose file is gone from storage is shown as missing.
//! Attaching or removing needs `write-statements` and `read-docs` (you pick from Documents);
//! viewing needs `read-statements`, and opening a private file still needs `read-docs`.

use std::sync::Arc;

use axum::{
    extract::{Path as UrlPath, State},
    response::Response,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use statements_core::row::month_label;

use crate::{
    api::{bad_request, db_error, not_found},
    auth::CurrentUser,
    documents,
    orgs::Org,
    rbac::{self, Permission},
    statement_types, AppState,
};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/orgs/{org}/statement-transactions/{id}",
            get(show).patch(statement_types::set_row_type),
        )
        .route(
            "/api/orgs/{org}/statement-transactions/{id}/documents",
            post(attach),
        )
        .route(
            "/api/orgs/{org}/statement-transactions/{id}/documents/{document_id}",
            delete(detach),
        )
}

#[derive(Serialize, sqlx::FromRow)]
struct RowFields {
    id: i64,
    txn_id: i64,
    account_number: String,
    date: String,
    amount: String,
    description: String,
    currency: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    category: Option<String>,
    type_manual: bool,
    updated_at: String,
}

/// A saved statement that contains the row, with its file in Documents.
#[derive(Serialize, sqlx::FromRow)]
struct SourceUpload {
    id: i64,
    filename: String,
    uploaded_at: String,
    document_id: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct AttachedDocument {
    document_id: i64,
    folder: String,
    filename: String,
    content_type: Option<String>,
    size: i64,
    attached_at: String,
    /// Who attached it, if the account still exists.
    attached_by: Option<String>,
    /// The file isn't in storage any more.
    #[sqlx(skip)]
    missing: bool,
}

#[derive(Serialize)]
struct RowDetail {
    #[serde(flatten)]
    row: RowFields,
    month: String,
    uploads: Vec<SourceUpload>,
    documents: Vec<AttachedDocument>,
}

async fn detail(state: &AppState, org: &Org, id: i64) -> Result<RowDetail, Response> {
    let row: RowFields = sqlx::query_as(
        "SELECT id, txn_id, account_number, date, amount, description, currency, type, type_manual,
                updated_at
         FROM statement_transactions WHERE id = ? AND org_id = ?",
    )
    .bind(id)
    .bind(org.id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_error)?
    .ok_or_else(|| not_found("Row not found."))?;
    let uploads = sqlx::query_as(
        "SELECT u.id, u.filename, u.uploaded_at, u.document_id
         FROM statement_upload_transactions l JOIN statement_uploads u ON u.id = l.upload_id
         WHERE l.transaction_id = ? ORDER BY u.uploaded_at, u.id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;
    let mut documents = attached(&state.db, id).await.map_err(db_error)?;
    for d in &mut documents {
        d.missing = !documents::file_exists(state, org, &d.folder, &d.filename).await;
    }
    Ok(RowDetail {
        month: month_label(&row.date),
        row,
        uploads,
        documents,
    })
}

async fn attached(db: &SqlitePool, row_id: i64) -> sqlx::Result<Vec<AttachedDocument>> {
    sqlx::query_as(
        "SELECT d.id AS document_id, d.folder, d.filename, d.content_type, d.size, l.attached_at,
                coalesce(u.name, u.email) AS attached_by
         FROM statement_transaction_documents l
         JOIN documents d ON d.id = l.document_id
         LEFT JOIN users u ON u.id = l.attached_by
         WHERE l.transaction_id = ? ORDER BY l.attached_at, d.id",
    )
    .bind(row_id)
    .fetch_all(db)
    .await
}

/// GET /api/orgs/{org}/statement-transactions/{id}
async fn show(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<Json<RowDetail>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(detail(&state, &org, id).await?))
}

/// The org, if the user may attach and remove documents there.
async fn may_attach(state: &AppState, user: &CurrentUser, slug: &str) -> Result<Org, Response> {
    let (org, perms) = rbac::org_access(&state.db, user, slug).await?;
    perms.require(Permission::WriteStatements)?;
    perms.require(Permission::ReadDocs)?;
    Ok(org)
}

#[derive(Deserialize)]
struct AttachBody {
    document_id: i64,
}

/// POST /api/orgs/{org}/statement-transactions/{id}/documents — link an existing document.
async fn attach(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<AttachBody>,
) -> Result<Json<RowDetail>, Response> {
    let org = may_attach(&state, &user, &slug).await?;
    match link(&state.db, org.id, id, body.document_id, user.id)
        .await
        .map_err(db_error)?
    {
        Link::Done => {}
        Link::NoRow => return Err(not_found("Row not found.")),
        Link::NoDocument => return Err(bad_request("That document isn't in this organisation.")),
    }
    tracing::info!(org = %org.slug, row = id, document = body.document_id, user = user.id, "document attached to row");
    Ok(Json(detail(&state, &org, id).await?))
}

#[derive(Debug, PartialEq)]
enum Link {
    Done,
    NoRow,
    NoDocument,
}

/// Links a document to a row, both in `org_id` (already linked is fine).
async fn link(
    db: &SqlitePool,
    org_id: i64,
    row_id: i64,
    document_id: i64,
    user_id: i64,
) -> sqlx::Result<Link> {
    let row: Option<i64> =
        sqlx::query_scalar("SELECT id FROM statement_transactions WHERE id = ? AND org_id = ?")
            .bind(row_id)
            .bind(org_id)
            .fetch_optional(db)
            .await?;
    if row.is_none() {
        return Ok(Link::NoRow);
    }
    let doc: Option<i64> =
        sqlx::query_scalar("SELECT id FROM documents WHERE id = ? AND org_id = ?")
            .bind(document_id)
            .bind(org_id)
            .fetch_optional(db)
            .await?;
    if doc.is_none() {
        return Ok(Link::NoDocument);
    }
    sqlx::query(
        "INSERT OR IGNORE INTO statement_transaction_documents (transaction_id, document_id, attached_by)
         VALUES (?, ?, ?)",
    )
    .bind(row_id)
    .bind(document_id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(Link::Done)
}

/// DELETE /api/orgs/{org}/statement-transactions/{id}/documents/{document_id} — remove the link
/// (the document itself stays).
async fn detach(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id, document_id)): UrlPath<(String, i64, i64)>,
) -> Result<Json<RowDetail>, Response> {
    let org = may_attach(&state, &user, &slug).await?;
    let removed = unlink(&state.db, org.id, id, document_id)
        .await
        .map_err(db_error)?;
    if !removed {
        return Err(not_found("That document isn't attached to this row."));
    }
    tracing::info!(org = %org.slug, row = id, document = document_id, user = user.id, "document detached from row");
    Ok(Json(detail(&state, &org, id).await?))
}

async fn unlink(db: &SqlitePool, org_id: i64, row_id: i64, document_id: i64) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "DELETE FROM statement_transaction_documents
         WHERE transaction_id = ?1 AND document_id = ?2
           AND EXISTS (SELECT 1 FROM statement_transactions WHERE id = ?1 AND org_id = ?3)",
    )
    .bind(row_id)
    .bind(document_id)
    .bind(org_id)
    .execute(db)
    .await?
    .rows_affected()
        > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (SqlitePool, i64, i64, i64, i64) {
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
        let q = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);
        let org = q("INSERT INTO organisations (slug, name) VALUES ('a', 'A') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
        let other = q("INSERT INTO organisations (slug, name) VALUES ('b', 'B') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
        let user = q("INSERT INTO users (issuer, subject) VALUES ('t', '1') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
        let row = sqlx::query_scalar(
            "INSERT INTO statement_transactions (org_id, txn_id, date, amount, description, currency)
             VALUES (?, 1, '2027-01-01', '-1', 'X', 'ZAR') RETURNING id",
        )
        .bind(org)
        .fetch_one(&db)
        .await
        .unwrap();
        (db, org, other, user, row)
    }

    async fn doc(db: &SqlitePool, org: i64, name: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO documents (org_id, folder, filename, size) VALUES (?, '', ?, 1) RETURNING id",
        )
        .bind(org)
        .bind(name)
        .fetch_one(db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn links_stay_within_the_org_and_never_touch_documents() {
        let (db, org, other, user, row) = db().await;
        let receipt = doc(&db, org, "receipt.pdf").await;
        let theirs = doc(&db, other, "theirs.pdf").await;

        assert_eq!(
            link(&db, org, row, receipt, user).await.unwrap(),
            Link::Done
        );
        // Twice is fine.
        assert_eq!(
            link(&db, org, row, receipt, user).await.unwrap(),
            Link::Done
        );
        assert_eq!(
            link(&db, org, row, theirs, user).await.unwrap(),
            Link::NoDocument
        );
        assert_eq!(
            link(&db, other, row, theirs, user).await.unwrap(),
            Link::NoRow
        );
        assert_eq!(attached(&db, row).await.unwrap().len(), 1);

        // Another org can't remove it; removing keeps the document.
        assert!(!unlink(&db, other, row, receipt).await.unwrap());
        assert!(unlink(&db, org, row, receipt).await.unwrap());
        assert!(!unlink(&db, org, row, receipt).await.unwrap());
        let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM documents WHERE id = ?")
            .bind(receipt)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(kept, 1);

        // Deleting the document's row (e.g. a missing file deleted in Documents) drops the link.
        link(&db, org, row, receipt, user).await.unwrap();
        sqlx::query("DELETE FROM documents WHERE id = ?")
            .bind(receipt)
            .execute(&db)
            .await
            .unwrap();
        assert!(attached(&db, row).await.unwrap().is_empty());
    }
}
