//! Per-organisation document store. An org's files live in storage under
//! `orgs/<slug>/documents/` (folders are real folders there); each file's metadata is a
//! row in `documents`. Stored files without a row are ignored.
//!
//! API: list a folder and download private documents (`read-docs`), upload (`write-docs`).
//! Downloads are public unless the document's `public` flag is off.

use std::sync::Arc;

use axum::{
    extract::{
        multipart::MultipartError, DefaultBodyLimit, FromRequestParts, Multipart, Path as UrlPath,
        Query, Request, State,
    },
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::{
    api::{bad_request, db_error, error, storage_error},
    auth::CurrentUser,
    orgs::Org,
    rbac::{self, Permission},
    storage::{StorageError, StoragePath, UploadError},
    AppState,
};

/// Name of the folder inside each org's directory that holds its documents.
pub const DOCUMENTS_FOLDER: &str = "documents";

const MAX_NAME_BYTES: usize = 200;
const MAX_FOLDER_DEPTH: usize = 8;
/// Room for multipart boundaries and the folder field on top of the file itself.
const MULTIPART_OVERHEAD: usize = 64 * 1024;

/// The org's documents root in storage: `orgs/<slug>/documents`.
pub fn documents_root(org: &Org) -> Result<StoragePath, Response> {
    StoragePath::root()
        .join_all("orgs")
        .and_then(|p| p.join(&org.slug))
        .and_then(|p| p.join(DOCUMENTS_FOLDER))
        .map_err(|_| bad_request("Invalid organisation."))
}

/// A folder (output of `parse_folder`) under the org's documents root.
fn folder_path(org: &Org, folder: &str) -> Result<StoragePath, Response> {
    documents_root(org)?
        .join_all(folder)
        .map_err(|_| bad_request("Invalid folder."))
}

/// The visible subfolders of `dir`: names that pass `clean_name` (so no dot-folders such
/// as temporary uploads), sorted case-insensitively.
async fn subfolders(state: &AppState, dir: &StoragePath) -> Result<Vec<String>, StorageError> {
    let mut names: Vec<String> = state
        .storage
        .list_dirs(dir)
        .await?
        .into_iter()
        .filter(|n| clean_name(n).as_deref() == Ok(n.as_str()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

pub fn routes(max_bytes: u64) -> Router<Arc<AppState>> {
    let body_limit = usize::try_from(max_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(MULTIPART_OVERHEAD);
    Router::new()
        .route(
            "/api/orgs/{org}/documents",
            get(list)
                .post(upload)
                .layer(DefaultBodyLimit::max(body_limit)),
        )
        .route("/api/orgs/{org}/documents/folders", get(all_folders))
        .route("/api/documents/{id}/download", get(download))
}

#[derive(Serialize, sqlx::FromRow)]
struct Document {
    id: i64,
    folder: String,
    filename: String,
    size: i64,
    content_type: Option<String>,
    uploaded_at: String,
    /// Uploader's name (or email), if the account still exists.
    uploaded_by: Option<String>,
    public: bool,
}

/// The `Document` columns plus a static tail (sqlx only takes `&'static str` SQL).
macro_rules! select_document {
    ($tail:literal) => {
        concat!(
            "SELECT d.id, d.folder, d.filename, d.size, d.content_type, d.uploaded_at,
                    coalesce(u.name, u.email) AS uploaded_by, d.public
             FROM documents d LEFT JOIN users u ON u.id = d.uploaded_by ",
            $tail
        )
    };
}

#[derive(Deserialize)]
struct FolderQuery {
    #[serde(default)]
    folder: String,
}

/// GET /api/orgs/{org}/documents?folder=a/b — subfolders and documents in one folder.
async fn list(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Query(q): Query<FolderQuery>,
) -> Result<Json<Value>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadDocs).await?;
    let folder = parse_folder(&q.folder).map_err(bad_request)?;
    let dir = folder_path(&org, &folder)?;
    let folders = match subfolders(&state, &dir).await {
        Ok(f) => f,
        // The org's root may not exist yet (nothing uploaded); that's just empty.
        Err(StorageError::NotFound) if folder.is_empty() => Vec::new(),
        Err(StorageError::NotFound) => {
            return Err(error(StatusCode::NOT_FOUND, "Folder not found."))
        }
        Err(e) => return Err(storage_error(e)),
    };
    let documents: Vec<Document> = sqlx::query_as(select_document!(
        "WHERE d.org_id = ? AND d.folder = ? ORDER BY d.filename COLLATE NOCASE"
    ))
    .bind(org.id)
    .bind(&folder)
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;

    Ok(Json(json!({
        "folder": folder,
        "folders": folders,
        "documents": documents,
        "max_upload_bytes": state.max_upload_bytes,
    })))
}

/// GET /api/orgs/{org}/documents/folders — every folder path, for the upload destination
/// picker (so `write-docs` alone is enough, as is `read-docs`).
async fn all_folders(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<Vec<String>>, Response> {
    let (org, perms) = rbac::org_access(&state.db, &user, &slug).await?;
    if !perms.has(Permission::ReadDocs) {
        perms.require(Permission::WriteDocs)?;
    }
    let mut out = Vec::new();
    let mut pending = vec![String::new()];
    while let Some(folder) = pending.pop() {
        let depth = if folder.is_empty() {
            0
        } else {
            folder.split('/').count()
        };
        if depth >= MAX_FOLDER_DEPTH {
            continue;
        }
        let names = match subfolders(&state, &folder_path(&org, &folder)?).await {
            Ok(n) => n,
            Err(StorageError::NotFound) => Vec::new(),
            Err(e) => return Err(storage_error(e)),
        };
        for name in names {
            let path = if folder.is_empty() {
                name
            } else {
                format!("{folder}/{name}")
            };
            out.push(path.clone());
            pending.push(path);
        }
    }
    out.sort_by_key(|p| p.to_lowercase());
    Ok(Json(out))
}

/// POST /api/orgs/{org}/documents — multipart with `folder` (text, may be new) then `file`.
/// A name already taken in the folder gets a " (1)", " (2)", … suffix.
async fn upload(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Document>), Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteDocs).await?;
    let mut folder = String::new();
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("folder") => {
                let raw = field.text().await.map_err(multipart_error)?;
                folder = parse_folder(&raw).map_err(bad_request)?;
            }
            Some("file") => {
                let filename = field
                    .file_name()
                    .ok_or("The upload has no file name.")
                    .and_then(upload_name)
                    .map_err(bad_request)?;
                let content_type = field.content_type().map(str::to_owned);
                let dir = folder_path(&org, &folder)?;
                state.storage.make_dirs(&dir).await.map_err(storage_error)?;
                let uploaded = state
                    .storage
                    .upload(&dir, &filename, &mut field, state.max_upload_bytes)
                    .await
                    .map_err(|e| upload_error(e, state.max_upload_bytes))?;
                let stored = store(
                    &state,
                    &org,
                    user.id,
                    &folder,
                    &dir,
                    &filename,
                    content_type,
                    &uploaded,
                )
                .await;
                if stored.is_err() {
                    let _ = state.storage.delete(&uploaded.temp).await;
                }
                let doc = stored?;
                tracing::info!(id = doc.id, org = %org.slug, folder = %doc.folder, filename = %doc.filename, size = doc.size, user = user.id, "document uploaded");
                return Ok((StatusCode::CREATED, Json(doc)));
            }
            _ => {}
        }
    }
    Err(bad_request("No file was sent."))
}

/// Claims a free name in the folder (row first, so concurrent uploads can't collide),
/// then moves the uploaded file to it. A name taken in storage but not in the database
/// (a file put there by hand) is skipped too.
#[allow(clippy::too_many_arguments)]
async fn store(
    state: &AppState,
    org: &Org,
    user_id: i64,
    folder: &str,
    dir: &StoragePath,
    filename: &str,
    content_type: Option<String>,
    uploaded: &crate::storage::Uploaded,
) -> Result<Document, Response> {
    for n in 0..100 {
        let name = numbered(filename, n);
        let target = dir
            .join(&name)
            .map_err(|_| bad_request("Invalid file name."))?;
        let inserted = sqlx::query_scalar::<_, i64>(
            "INSERT INTO documents (org_id, folder, filename, size, content_type, sha256, uploaded_by)
             VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(org.id)
        .bind(folder)
        .bind(&name)
        .bind(uploaded.size as i64)
        .bind(&content_type)
        .bind(&uploaded.sha256)
        .bind(user_id)
        .fetch_one(&state.db)
        .await;
        let id = match inserted {
            Ok(id) => id,
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => continue,
            Err(e) => return Err(db_error(e)),
        };
        let forget_row = || async {
            let _ = sqlx::query("DELETE FROM documents WHERE id = ?")
                .bind(id)
                .execute(&state.db)
                .await;
        };
        match state.storage.commit(&uploaded.temp, &target).await {
            Ok(()) => return fetch_document(&state.db, id).await,
            Err(StorageError::AlreadyExists) => {
                forget_row().await;
                continue;
            }
            Err(e) => {
                forget_row().await;
                return Err(storage_error(e));
            }
        }
    }
    Err(error(
        StatusCode::CONFLICT,
        "Too many files with that name in this folder.",
    ))
}

async fn fetch_document(db: &SqlitePool, id: i64) -> Result<Document, Response> {
    sqlx::query_as(select_document!("WHERE d.id = ?"))
        .bind(id)
        .fetch_one(db)
        .await
        .map_err(db_error)
}

/// What a download needs: the document and the org whose folder holds it.
#[derive(sqlx::FromRow)]
struct DownloadRow {
    org_id: i64,
    slug: String,
    name: String,
    folder: String,
    filename: String,
    content_type: Option<String>,
    public: bool,
}

/// GET /api/documents/{id}/download — always served as an attachment, never rendered inline.
async fn download(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<i64>,
    req: Request,
) -> Result<Response, Response> {
    let not_found = || error(StatusCode::NOT_FOUND, "Document not found.");
    let row: Option<DownloadRow> = sqlx::query_as(
        "SELECT o.id AS org_id, o.slug, o.name, d.folder, d.filename, d.content_type, d.public
             FROM documents d JOIN organisations o ON o.id = d.org_id WHERE d.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_error)?;
    let DownloadRow {
        org_id,
        slug,
        name,
        folder,
        filename,
        content_type,
        public,
    } = row.ok_or_else(not_found)?;
    let org = Org {
        id: org_id,
        slug,
        name,
    };

    // Private documents: `read-docs` in the document's org. Anyone else gets a 404, so
    // the document's existence isn't revealed.
    if !public {
        let (mut parts, _) = req.into_parts();
        let user = CurrentUser::from_request_parts(&mut parts, &state).await?;
        let perms = if user.is_admin() {
            rbac::Perms::all()
        } else {
            rbac::member_perms(&state.db, org.id, user.id)
                .await?
                .unwrap_or_default()
        };
        if !perms.has(Permission::ReadDocs) {
            return Err(not_found());
        }
    }

    let path = folder_path(&org, &folder)?
        .join(&filename)
        .map_err(|_| not_found())?;
    let blob = match state.storage.get(&path).await {
        Ok(b) => b,
        Err(StorageError::NotFound) => {
            tracing::warn!(id, %path, "document row has no file in storage");
            return Err(not_found());
        }
        Err(e) => return Err(storage_error(e)),
    };
    let len = blob.len;

    let mut res = blob.body.into_response();
    let headers = res.headers_mut();
    let ctype = content_type
        .and_then(|t| HeaderValue::from_str(&t).ok())
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, ctype);
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    if let Ok(v) = HeaderValue::from_str(&content_disposition(&filename)) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    // Uploaded content is untrusted: never sniff it or let it run as a page on this origin.
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

// ---- names and paths ----

/// One file or folder name: no separators, control characters or leading dot.
fn clean_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Names can't be empty.");
    }
    if name.len() > MAX_NAME_BYTES {
        return Err("That name is too long.");
    }
    if name.starts_with('.') {
        return Err("Names can't start with a dot.");
    }
    if name
        .chars()
        .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return Err("Names can't contain slashes or control characters.");
    }
    Ok(name.to_owned())
}

/// The browser's file name, minus any directory part some clients include.
fn upload_name(raw: &str) -> Result<String, &'static str> {
    clean_name(raw.rsplit(['/', '\\']).next().unwrap_or(raw))
}

/// "a/b/c" (or "", the top level) from user input; each segment must be a clean name.
fn parse_folder(raw: &str) -> Result<String, &'static str> {
    let raw = raw.trim().trim_matches('/');
    if raw.is_empty() {
        return Ok(String::new());
    }
    let segments = raw
        .split('/')
        .map(clean_name)
        .collect::<Result<Vec<_>, _>>()?;
    if segments.len() > MAX_FOLDER_DEPTH {
        return Err("Folders are nested too deeply.");
    }
    Ok(segments.join("/"))
}

/// "report.pdf" → "report (2).pdf" for n = 2.
fn numbered(filename: &str, n: u32) -> String {
    if n == 0 {
        return filename.to_owned();
    }
    match filename.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({n}){}", &filename[..dot], &filename[dot..]),
        _ => format!("{filename} ({n})"),
    }
}

/// RFC 6266: an ASCII fallback plus the exact UTF-8 name.
fn content_disposition(filename: &str) -> String {
    let fallback: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in filename.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

// ---- errors ----

fn upload_error(e: UploadError<MultipartError>, max_bytes: u64) -> Response {
    match e {
        UploadError::TooLarge => too_large(max_bytes),
        UploadError::Empty => bad_request("The file is empty."),
        UploadError::Source(e) => multipart_error(e),
        UploadError::Storage(e) => storage_error(e),
    }
}

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
    fn folders_are_normalised_and_cannot_escape() {
        assert_eq!(parse_folder("").unwrap(), "");
        assert_eq!(parse_folder(" / ").unwrap(), "");
        assert_eq!(parse_folder("/Brand/ Logos /").unwrap(), "Brand/Logos");
        for bad in [
            "..", "a/../b", "a//b", ".hidden", "a\\b", "a/\u{0}", ".uploads",
        ] {
            assert!(parse_folder(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(parse_folder(&["x"; MAX_FOLDER_DEPTH + 1].join("/")).is_err());
    }

    #[test]
    fn upload_names_drop_client_paths() {
        assert_eq!(upload_name("C:\\Users\\me\\deck.pdf").unwrap(), "deck.pdf");
        assert_eq!(upload_name("some/dir/deck.pdf").unwrap(), "deck.pdf");
        assert!(upload_name("../").is_err());
        assert!(upload_name(".env").is_err());
    }

    #[test]
    fn duplicate_names_get_numbered() {
        assert_eq!(numbered("deck.pdf", 0), "deck.pdf");
        assert_eq!(numbered("deck.pdf", 2), "deck (2).pdf");
        assert_eq!(numbered("archive.tar.gz", 1), "archive.tar (1).gz");
        assert_eq!(numbered("README", 1), "README (1)");
        assert_eq!(numbered(".x", 1), ".x (1)");
    }

    #[test]
    fn content_disposition_keeps_utf8_and_quotes_safely() {
        assert_eq!(
            content_disposition("Brand \"kit\" é.pdf"),
            "attachment; filename=\"Brand _kit_ _.pdf\"; filename*=UTF-8''Brand%20%22kit%22%20%C3%A9.pdf"
        );
    }
}
