//! Per-organisation document store. An org's files live in storage under
//! `orgs/<slug>/documents/` (folders are real folders there); each file's metadata is a
//! row in `documents`.
//!
//! The two can drift apart (files changed on the storage host directly, failures between
//! the two). Listing a folder reconciles it: a stored file without a row gets one, and a
//! row whose file is gone is flagged `missing` so someone with `write-docs` can delete it.
//!
//! API: list a folder and download private documents (`read-docs`), upload and delete
//! missing documents (`write-docs`).
//! Downloads are public unless the document's `public` flag is off.

use crate::db::{self, Db};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use axum::{
    extract::{
        multipart::MultipartError, DefaultBodyLimit, FromRequestParts, Multipart, Path as UrlPath,
        Query, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    api::{bad_request, db_error, error, storage_error, Uncompressed},
    auth::CurrentUser,
    orgs::Org,
    rbac::{self, Permission},
    storage::{ByteRange, FileInfo, StorageError, StoragePath, UploadError},
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
        .route("/api/orgs/{org}/documents/all", get(all_documents))
        .route("/api/documents/{id}/download", get(download))
        .route("/api/documents/{id}", delete(delete_missing))
}

#[derive(Serialize, Deserialize)]
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
    /// The row's file isn't in storage (set when listing a folder).
    #[serde(skip_deserializing)]
    missing: bool,
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
    let listing = match state.storage.list(&dir).await {
        Ok(l) => l,
        // The org's root may not exist yet (nothing uploaded); that's just empty.
        Err(StorageError::NotFound) if folder.is_empty() => Default::default(),
        Err(StorageError::NotFound) => {
            return Err(error(StatusCode::NOT_FOUND, "Folder not found."))
        }
        Err(e) => return Err(storage_error(e)),
    };
    let folders = visible_dirs(listing.dirs);
    let documents = reconcile(&state.db, &org, &folder, &listing.files).await?;

    Ok(Json(json!({
        "folder": folder,
        "folders": folders,
        "documents": documents,
        "max_upload_bytes": state.max_upload_bytes,
    })))
}

/// The visible subfolders of `dir`: names that pass `clean_name` (so no dot-folders such
/// as temporary uploads), sorted case-insensitively.
async fn subfolders(state: &AppState, dir: &StoragePath) -> Result<Vec<String>, StorageError> {
    Ok(visible_dirs(state.storage.list(dir).await?.dirs))
}

/// Folders people can see: valid names only (no dot-folders), sorted case-insensitively.
fn visible_dirs(dirs: Vec<String>) -> Vec<String> {
    let mut names: Vec<String> = dirs.into_iter().filter(|n| is_clean(n)).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// A name that's valid as it stands (temporary uploads and other dot-files aren't).
fn is_clean(name: &str) -> bool {
    clean_name(name).as_deref() == Ok(name)
}

/// The folder's documents, after bringing the database in line with `stored`. Storage is
/// the source of truth; rows only add metadata. So: files without a row get one, rows
/// whose file changed size take the new size (and lose their now-wrong checksum), and
/// rows without a file are flagged `missing`.
async fn reconcile(
    db: &Db,
    org: &Org,
    folder: &str,
    stored: &[FileInfo],
) -> Result<Vec<Document>, Response> {
    let select = || {
        db::query_as::<Document>(select_document!(
            "WHERE d.org_id = ? AND d.folder = ? ORDER BY d.filename COLLATE NOCASE"
        ))
        .bind(org.id)
        .bind(folder)
        .fetch_all(db)
    };
    let mut documents = select().await.map_err(db_error)?;

    let found = unrecorded(stored, &documents);
    let resized = resized(stored, &documents);
    for (id, size) in &resized {
        db::query("UPDATE documents SET size = ?, sha256 = NULL WHERE id = ?")
            .bind(i64::try_from(*size).unwrap_or(i64::MAX))
            .bind(id)
            .execute(db)
            .await
            .map_err(db_error)?;
        tracing::info!(id, org = %org.slug, folder, size, "document changed in storage, size updated");
    }
    for file in &found {
        // Private: nobody chose to publish it. Ignored if another listing got there first.
        db::query(
            "INSERT OR IGNORE INTO documents (org_id, folder, filename, size, public)
                 VALUES (?, ?, ?, ?, 0)",
        )
        .bind(org.id)
        .bind(folder)
        .bind(&file.name)
        .bind(i64::try_from(file.size).unwrap_or(i64::MAX))
        .execute(db)
        .await
        .map_err(db_error)?;
        tracing::info!(org = %org.slug, folder, filename = %file.name, size = file.size, "document found in storage, added");
    }
    if !found.is_empty() || !resized.is_empty() {
        documents = select().await.map_err(db_error)?;
    }

    let names: HashSet<&str> = stored.iter().map(|f| f.name.as_str()).collect();
    for doc in &mut documents {
        doc.missing = !names.contains(doc.filename.as_str());
    }
    Ok(documents)
}

/// Rows whose stored file now has a different size: `(id, new size)`.
fn resized(stored: &[FileInfo], documents: &[Document]) -> Vec<(i64, u64)> {
    let sizes: HashMap<&str, u64> = stored.iter().map(|f| (f.name.as_str(), f.size)).collect();
    documents
        .iter()
        .filter_map(|d| {
            let size = *sizes.get(d.filename.as_str())?;
            (u64::try_from(d.size).ok() != Some(size)).then_some((d.id, size))
        })
        .collect()
}

/// Stored files that should be documents but have no row.
fn unrecorded<'a>(stored: &'a [FileInfo], documents: &[Document]) -> Vec<&'a FileInfo> {
    let known: HashSet<&str> = documents.iter().map(|d| d.filename.as_str()).collect();
    stored
        .iter()
        .filter(|f| is_clean(&f.name) && !known.contains(f.name.as_str()))
        .collect()
}

/// A document of the org: (folder, filename), or None.
pub async fn find_in_org(
    db: &crate::db::Db,
    org_id: i64,
    id: i64,
) -> Result<Option<(String, String)>, Response> {
    #[derive(Deserialize)]
    struct Found {
        folder: String,
        filename: String,
    }
    let found: Option<Found> =
        db::query_as("SELECT folder, filename FROM documents WHERE id = ? AND org_id = ?")
            .bind(id)
            .bind(org_id)
            .fetch_optional(db)
            .await
            .map_err(crate::api::db_error)?;
    Ok(found.map(|f| (f.folder, f.filename)))
}

/// A document's whole file, read from storage (at most `max` bytes). For server-side use, e.g.
/// a logo in a statement PDF; the caller checks permissions.
pub async fn read_bytes(
    state: &AppState,
    org: &Org,
    folder: &str,
    filename: &str,
    max: usize,
) -> Result<axum::body::Bytes, Response> {
    let path = folder_path(org, folder)?
        .join(filename)
        .map_err(|_| bad_request("Invalid file name."))?;
    let blob = state
        .storage
        .get(&path, None)
        .await
        .map_err(crate::api::storage_error)?;
    if blob.total > max as u64 {
        return Err(bad_request(&format!(
            "{filename} is too large (at most {} MB).",
            max / (1024 * 1024)
        )));
    }
    axum::body::to_bytes(blob.body, max)
        .await
        .map_err(|_| bad_request(&format!("{filename} couldn't be read.")))
}

/// Whether a document's file is in storage. Storage errors count as "don't know" (true), so a
/// storage blip doesn't flag every document as missing.
pub async fn file_exists(state: &AppState, org: &Org, folder: &str, filename: &str) -> bool {
    let Ok(path) = folder_path(org, folder).and_then(|d| {
        d.join(filename)
            .map_err(|_| bad_request("Invalid file name."))
    }) else {
        return false;
    };
    state.storage.exists(&path).await.unwrap_or(true)
}

/// GET /api/orgs/{org}/documents/all — every document in the org, from the database only (no
/// storage listing), for pickers. `read-docs`.
async fn all_documents(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<Vec<Document>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadDocs).await?;
    let docs = db::query_as(select_document!(
        "WHERE d.org_id = ? ORDER BY d.folder COLLATE NOCASE, d.filename COLLATE NOCASE"
    ))
    .bind(org.id)
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;
    Ok(Json(docs))
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
                    true,
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

/// Saves a file already in memory as a document in `folder` (created if new), numbering the
/// name if it's taken, exactly like an upload. The caller checks permissions. `public: false`
/// means only members with `read-docs` can download it. Returns the document's id, folder and
/// final name.
#[allow(clippy::too_many_arguments)]
pub async fn save_bytes(
    state: &AppState,
    org: &Org,
    user_id: i64,
    folder: &str,
    filename: &str,
    content_type: Option<String>,
    bytes: axum::body::Bytes,
    public: bool,
) -> Result<(i64, String, String), Response> {
    let folder = parse_folder(folder).map_err(bad_request)?;
    let filename = upload_name(filename).map_err(bad_request)?;
    let dir = folder_path(org, &folder)?;
    state.storage.make_dirs(&dir).await.map_err(storage_error)?;
    let source = futures_util::stream::iter([Ok::<_, std::convert::Infallible>(bytes)]);
    let uploaded = state
        .storage
        .upload(&dir, &filename, source, state.max_upload_bytes)
        .await
        .map_err(|e| match e {
            UploadError::TooLarge => too_large(state.max_upload_bytes),
            UploadError::Empty => bad_request("The file is empty."),
            UploadError::Source(never) => match never {},
            UploadError::Storage(e) => storage_error(e),
        })?;
    let stored = store(
        state,
        org,
        user_id,
        &folder,
        &dir,
        &filename,
        content_type,
        &uploaded,
        public,
    )
    .await;
    if stored.is_err() {
        let _ = state.storage.delete(&uploaded.temp).await;
    }
    let doc = stored?;
    tracing::info!(id = doc.id, org = %org.slug, folder = %doc.folder, filename = %doc.filename, size = doc.size, public, user = user_id, "document saved");
    Ok((doc.id, doc.folder, doc.filename))
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
    public: bool,
) -> Result<Document, Response> {
    for n in 0..100 {
        let name = numbered(filename, n);
        let target = dir
            .join(&name)
            .map_err(|_| bad_request("Invalid file name."))?;
        let inserted = db::query_scalar::<i64>(
            "INSERT INTO documents (org_id, folder, filename, size, content_type, sha256, uploaded_by, public)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(org.id)
        .bind(folder)
        .bind(&name)
        .bind(uploaded.size as i64)
        .bind(&content_type)
        .bind(&uploaded.sha256)
        .bind(user_id)
        .bind(public)
        .fetch_one(&state.db)
        .await;
        let id = match inserted {
            Ok(id) => id,
            Err(e) if e.is_unique_violation() => continue,
            Err(e) => return Err(db_error(e)),
        };
        let forget_row = || async {
            let _ = db::query("DELETE FROM documents WHERE id = ?")
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

async fn fetch_document(db: &Db, id: i64) -> Result<Document, Response> {
    db::query_as(select_document!("WHERE d.id = ?"))
        .bind(id)
        .fetch_one(db)
        .await
        .map_err(db_error)
}

/// What a download needs: the document and the org whose folder holds it.
#[derive(Deserialize)]
struct DownloadRow {
    org_id: i64,
    slug: String,
    name: String,
    folder: String,
    filename: String,
    content_type: Option<String>,
    sha256: Option<String>,
    public: bool,
}

/// GET /api/documents/{id}/download — always served as an attachment, never rendered inline.
async fn download(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<i64>,
    req: Request,
) -> Result<Response, Response> {
    let not_found = || error(StatusCode::NOT_FOUND, "Document not found.");
    let row: Option<DownloadRow> = db::query_as(
        "SELECT o.id AS org_id, o.slug, o.name, d.folder, d.filename, d.content_type, d.sha256,
                d.public
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
        sha256,
        public,
    } = row.ok_or_else(not_found)?;
    // Stored files never change (nothing is overwritten), so the content hash is a strong
    // validator for `If-Range`. Files found in storage have no hash, so no validator.
    let etag = sha256.map(|h| format!("\"{h}\""));
    let range = requested_range(req.headers(), etag.as_deref());
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
    let blob = match state.storage.get(&path, range).await {
        Ok(b) => b,
        Err(StorageError::NotFound) => {
            tracing::warn!(id, %path, "document row has no file in storage");
            return Err(not_found());
        }
        Err(StorageError::RangeNotSatisfiable { total }) => {
            let mut res = error(
                StatusCode::RANGE_NOT_SATISFIABLE,
                "That range is outside the file.",
            );
            download_headers(res.headers_mut(), &filename, etag.as_deref());
            if let Ok(v) = HeaderValue::from_str(&format!("bytes */{total}")) {
                res.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            return Err(res);
        }
        Err(e) => return Err(storage_error(e)),
    };
    let len = blob.len();
    let (total, part) = (blob.total, blob.range);

    let mut res = blob.body.into_response();
    let headers = res.headers_mut();
    let ctype = content_type
        .and_then(|t| HeaderValue::from_str(&t).ok())
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, ctype);
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    download_headers(headers, &filename, etag.as_deref());
    if let Some((first, last)) = part {
        *res.status_mut() = StatusCode::PARTIAL_CONTENT;
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {first}-{last}/{total}")) {
            res.headers_mut().insert(header::CONTENT_RANGE, v);
        }
    }
    // Byte ranges count the stored bytes, so never gzip these on the way out.
    res.extensions_mut().insert(Uncompressed);
    Ok(res)
}

/// The `Range` to honour, if any: a single byte range, and only if `If-Range` (when sent)
/// names this exact file. Otherwise the whole file is sent.
fn requested_range(headers: &HeaderMap, etag: Option<&str>) -> Option<ByteRange> {
    let range = ByteRange::parse(headers.get(header::RANGE)?.to_str().ok()?)?;
    match headers.get(header::IF_RANGE) {
        None => Some(range),
        // Strong comparison. A date never matches: no Last-Modified is sent.
        Some(v) => (Some(v.as_bytes()) == etag.map(str::as_bytes)).then_some(range),
    }
}

/// Headers for every download response, including errors about the range.
fn download_headers(headers: &mut HeaderMap, filename: &str, etag: Option<&str>) {
    if let Ok(v) = HeaderValue::from_str(&content_disposition(filename)) {
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
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Some(Ok(v)) = etag.map(HeaderValue::from_str) {
        headers.insert(header::ETAG, v);
    }
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
pub(crate) fn content_disposition(filename: &str) -> String {
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

#[derive(Deserialize)]
struct DeleteRow {
    slug: String,
    folder: String,
    filename: String,
}

/// DELETE /api/documents/{id} — removes a document whose file is missing from storage
/// (`write-docs`). Documents whose file is still there can't be deleted (yet): 409.
async fn delete_missing(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<i64>,
) -> Result<StatusCode, Response> {
    let not_found = || error(StatusCode::NOT_FOUND, "Document not found.");
    let row: DeleteRow = db::query_as(
        "SELECT o.slug, d.folder, d.filename
             FROM documents d JOIN organisations o ON o.id = d.org_id WHERE d.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(db_error)?
    .ok_or_else(not_found)?;
    let org = rbac::authorize(&state.db, &user, &row.slug, Permission::WriteDocs).await?;

    // Checked now, not trusted from the listing: the file may have come back since.
    let path = folder_path(&org, &row.folder)?
        .join(&row.filename)
        .map_err(|_| not_found())?;
    if state.storage.exists(&path).await.map_err(storage_error)? {
        return Err(error(
            StatusCode::CONFLICT,
            "This document's file is in storage, so it isn't missing. Reload the folder.",
        ));
    }
    db::query("DELETE FROM documents WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::info!(id, org = %org.slug, folder = %row.folder, filename = %row.filename, user = user.id, "missing document deleted");
    Ok(StatusCode::NO_CONTENT)
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
    fn ranges_need_a_matching_if_range() {
        let etag = "\"abc\"";
        let with = |pairs: &[(header::HeaderName, &str)]| {
            let mut h = HeaderMap::new();
            for (k, v) in pairs {
                h.insert(k, HeaderValue::from_str(v).unwrap());
            }
            requested_range(&h, Some(etag))
        };
        assert_eq!(with(&[]), None);
        assert_eq!(
            with(&[(header::RANGE, "bytes=5-")]),
            Some(ByteRange::From(5))
        );
        assert_eq!(
            with(&[(header::RANGE, "bytes=5-"), (header::IF_RANGE, "\"abc\"")]),
            Some(ByteRange::From(5))
        );
        for stale in ["\"other\"", "W/\"abc\"", "Thu, 12 Nov 2026 14:49:41 GMT"] {
            assert_eq!(
                with(&[(header::RANGE, "bytes=5-"), (header::IF_RANGE, stale)]),
                None
            );
        }
        assert_eq!(with(&[(header::RANGE, "bytes=0-1,4-5")]), None);
        // No checksum, no validator: If-Range never matches.
        let mut h = HeaderMap::new();
        h.insert(header::RANGE, HeaderValue::from_static("bytes=5-"));
        assert_eq!(requested_range(&h, None), Some(ByteRange::From(5)));
        h.insert(header::IF_RANGE, HeaderValue::from_static("\"abc\""));
        assert_eq!(requested_range(&h, None), None);
    }

    fn doc(filename: &str) -> Document {
        Document {
            id: 1,
            folder: String::new(),
            filename: filename.into(),
            size: 1,
            content_type: None,
            uploaded_at: String::new(),
            uploaded_by: None,
            public: true,
            missing: false,
        }
    }

    #[test]
    fn only_clean_unknown_files_are_recorded() {
        let file = |name: &str| FileInfo {
            name: name.into(),
            size: 1,
        };
        let stored = [
            file("known.pdf"),
            file("new.pdf"),
            file(".new.pdf.partial-0123456789abcdef"),
            file(".DS_Store"),
            file(" padded.pdf"),
        ];
        let found = unrecorded(&stored, &[doc("known.pdf"), doc("gone.pdf")]);
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["new.pdf"]);
    }

    #[test]
    fn storage_sizes_win() {
        let stored = [
            FileInfo {
                name: "same.pdf".into(),
                size: 1,
            },
            FileInfo {
                name: "edited.pdf".into(),
                size: 99,
            },
        ];
        let mut edited = doc("edited.pdf");
        edited.id = 7;
        assert_eq!(
            resized(&stored, &[doc("same.pdf"), edited, doc("gone.pdf")]),
            [(7, 99)]
        );
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
