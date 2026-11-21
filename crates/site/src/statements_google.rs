//! Sending saved statements to the user's own Google Sheet.
//!
//! Signing in to the portal only asks Google for identity. Sheets access is a separate,
//! optional consent ("Connect Google Sheets") asking for the `spreadsheets` and
//! `drive.metadata.readonly` scopes. It uses the same OAuth client with its own redirect
//! URI, `<PUBLIC_URL>/api/statements/google/callback`. Online access only: the access token
//! is held in memory for each portal user until it expires (about an hour), never stored and
//! never sent to the browser. A restart forgets it, and the user connects again.
//!
//! The Sheets client (`statements-core`) is blocking, so calls run on `spawn_blocking`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use statements_core::{
    google::{self, Google, Spreadsheet, SpreadsheetDetails, Tab},
    oauth::{self, missing_scopes, pkce_challenge, random_token, AuthRequest, OAuthClient, SCOPES},
    statement::{Format, Statement, Transaction},
    upload::{self, Target},
};

use crate::{
    api::{bad_request, error},
    auth::{safe_return_to, CurrentUser},
    rbac::{self, Permission},
    statements::upload_detail,
    AppState,
};

/// How long someone has to finish Google's consent screen.
const CONNECT_TTL: Duration = Duration::from_secs(600);
/// Cap on in-flight consents, so requests can't grow memory without bound.
const MAX_PENDING: usize = 1_000;
/// Treat a token as expired this long before Google does, so a call doesn't fail midway.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);
/// Google's limit on tab names.
const MAX_TAB_NAME_CHARS: usize = 100;
const MAX_SPREADSHEET_NAME_CHARS: usize = 200;

struct Connection {
    access_token: String,
    email: Option<String>,
    expires_at: Instant,
}

struct PendingConnect {
    user_id: i64,
    verifier: String,
    return_to: String,
    started: Instant,
}

/// The Sheets connection settings and the connections themselves.
pub struct SheetsAuth {
    client: OAuthClient,
    redirect_uri: String,
    agent: ureq::Agent,
    /// By portal user id.
    connections: Mutex<HashMap<i64, Connection>>,
    /// By OAuth `state`.
    pending: Mutex<HashMap<String, PendingConnect>>,
}

impl SheetsAuth {
    pub fn new(client_id: String, client_secret: String, public_url: &str) -> Self {
        Self {
            client: OAuthClient {
                client_id,
                client_secret,
            },
            redirect_uri: format!(
                "{}/api/statements/google/callback",
                public_url.trim_end_matches('/')
            ),
            agent: google::agent(),
            connections: Mutex::default(),
            pending: Mutex::default(),
        }
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// The user's access token, if they're connected and it hasn't expired.
    fn token(&self, user_id: i64) -> Option<String> {
        let mut conns = self.connections.lock().unwrap();
        let now = Instant::now();
        conns.retain(|_, c| c.expires_at > now + EXPIRY_MARGIN);
        conns.get(&user_id).map(|c| c.access_token.clone())
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/statements/google", get(status))
        .route("/api/statements/google/connect", get(connect))
        .route("/api/statements/google/callback", get(callback))
        .route("/api/statements/google/disconnect", post(disconnect))
        .route(
            "/api/statements/google/spreadsheets",
            get(list_spreadsheets).post(create_spreadsheet),
        )
        .route("/api/statements/google/spreadsheets/{id}", get(spreadsheet))
        .route(
            "/api/statements/google/spreadsheets/{id}/tabs",
            post(create_tab),
        )
        .route("/api/orgs/{org}/statements/{id}/sheet", post(send_to_sheet))
}

fn sheets(state: &AppState) -> Result<&SheetsAuth, Response> {
    state.sheets.as_ref().ok_or_else(|| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Google Sheets isn't set up on this site.",
        )
    })
}

/// Only people who can see statements somewhere may connect.
async fn require_statements(state: &AppState, user: &CurrentUser) -> Result<(), Response> {
    if rbac::holds_anywhere(&state.db, user, Permission::ReadStatements).await? {
        Ok(())
    } else {
        Err(rbac::forbidden())
    }
}

#[derive(Serialize)]
struct Status {
    /// Whether this site has a Google client to connect with.
    available: bool,
    connected: bool,
    email: Option<String>,
}

/// GET /api/statements/google
async fn status(user: CurrentUser, State(state): State<Arc<AppState>>) -> Json<Status> {
    let Some(sheets) = state.sheets.as_ref() else {
        return Json(Status {
            available: false,
            connected: false,
            email: None,
        });
    };
    let connected = sheets.token(user.id).is_some();
    let email = connected
        .then(|| {
            let conns = sheets.connections.lock().unwrap();
            conns.get(&user.id).and_then(|c| c.email.clone())
        })
        .flatten();
    Json(Status {
        available: true,
        connected,
        email,
    })
}

#[derive(Deserialize)]
struct ConnectQuery {
    return_to: Option<String>,
}

/// GET /api/statements/google/connect?return_to=/portal/... — off to Google's consent screen.
async fn connect(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    Query(q): Query<ConnectQuery>,
) -> Result<Response, Response> {
    let sheets = sheets(&state)?;
    require_statements(&state, &user).await?;
    let internal = |e: anyhow::Error| {
        tracing::error!(error = %e, "starting Google Sheets consent");
        error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
    };
    let st = random_token(16).map_err(internal)?;
    let verifier = random_token(32).map_err(internal)?;
    {
        let mut pending = sheets.pending.lock().unwrap();
        pending.retain(|_, p| p.started.elapsed() < CONNECT_TTL);
        if pending.len() >= MAX_PENDING {
            return Err(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many people are connecting right now. Try again in a minute.",
            ));
        }
        pending.insert(
            st.clone(),
            PendingConnect {
                user_id: user.id,
                verifier: verifier.clone(),
                return_to: safe_return_to(q.return_to.as_deref()),
                started: Instant::now(),
            },
        );
    }
    // `email` shows which Google account is connected (it can differ from the portal one).
    let scope = ["openid", "email"]
        .into_iter()
        .chain(SCOPES)
        .collect::<Vec<_>>()
        .join(" ");
    let url = oauth::authorization_url(
        &sheets.client,
        &AuthRequest {
            redirect_uri: &sheets.redirect_uri,
            scope: &scope,
            state: &st,
            code_challenge: &pkce_challenge(&verifier),
            offline: false,
        },
    )
    .map_err(internal)?;
    Ok(Redirect::to(url.as_str()).into_response())
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// GET /api/statements/google/callback — Google's redirect back. Always returns to the page
/// that started it, with `?google=connected` or `?google_error=<code>`.
async fn callback(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, Response> {
    let sheets = sheets(&state)?;
    // The pending entry belongs to the user who started it, so a link someone else started
    // can't connect their Google account to this session.
    let pending = q.state.as_deref().and_then(|s| {
        let mut all = sheets.pending.lock().unwrap();
        match all.get(s) {
            Some(p) if p.user_id == user.id => all.remove(s),
            _ => None,
        }
    });
    let Some(pending) = pending.filter(|p| p.started.elapsed() < CONNECT_TTL) else {
        return Ok(back("/portal", Err("expired")));
    };
    let back_to = pending.return_to.clone();
    if q.error.is_some() {
        return Ok(back(&back_to, Err("denied")));
    }
    let Some(code) = q.code else {
        return Ok(back(&back_to, Err("denied")));
    };

    let agent = sheets.agent.clone();
    let client = sheets.client.clone();
    let redirect = sheets.redirect_uri.clone();
    let exchanged = tokio::task::spawn_blocking(move || {
        oauth::exchange_code(&agent, &client, &code, &pending.verifier, &redirect)
    })
    .await;
    let token = match exchanged {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            tracing::error!(error = %format!("{e:#}"), "Google Sheets token exchange failed");
            return Ok(back(&back_to, Err("google")));
        }
        Err(e) => {
            tracing::error!(error = %e, "Google Sheets token exchange panicked");
            return Ok(back(&back_to, Err("google")));
        }
    };
    if !missing_scopes(&token.scope).is_empty() {
        // Unticked on the consent screen: nothing would work, so don't keep it.
        let agent = sheets.agent.clone();
        let access = token.access_token.clone();
        tokio::task::spawn_blocking(move || oauth::revoke(&agent, &access));
        return Ok(back(&back_to, Err("scopes")));
    }
    let email = token.email();
    sheets.connections.lock().unwrap().insert(
        user.id,
        Connection {
            access_token: token.access_token,
            email: email.clone(),
            expires_at: Instant::now() + Duration::from_secs(token.expires_in),
        },
    );
    tracing::info!(user = user.id, google = ?email, "Google Sheets connected");
    Ok(back(&back_to, Ok(())))
}

/// `return_to` with the outcome added to its query.
fn back(return_to: &str, outcome: Result<(), &str>) -> Response {
    let sep = if return_to.contains('?') { '&' } else { '?' };
    let param = match outcome {
        Ok(()) => "google=connected".to_string(),
        Err(code) => format!("google_error={code}"),
    };
    Redirect::to(&format!("{return_to}{sep}{param}")).into_response()
}

/// POST /api/statements/google/disconnect — forget the token and revoke it at Google.
async fn disconnect(user: CurrentUser, State(state): State<Arc<AppState>>) -> StatusCode {
    let Some(sheets) = state.sheets.as_ref() else {
        return StatusCode::NO_CONTENT;
    };
    let removed = sheets.connections.lock().unwrap().remove(&user.id);
    if let Some(conn) = removed {
        let agent = sheets.agent.clone();
        tokio::task::spawn_blocking(move || oauth::revoke(&agent, &conn.access_token));
        tracing::info!(user = user.id, "Google Sheets disconnected");
    }
    StatusCode::NO_CONTENT
}

/// Runs `f` with a Sheets client for the user, off the async runtime. 401 when they aren't
/// connected (or the token expired); Google's own errors come back as 502 with its message.
async fn with_google<T: Send + 'static>(
    state: &AppState,
    user: &CurrentUser,
    f: impl FnOnce(&Google) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, Response> {
    let sheets = sheets(state)?;
    let token = sheets.token(user.id).ok_or_else(|| {
        error(
            StatusCode::UNAUTHORIZED,
            "Connect Google Sheets first (or again: connections last about an hour).",
        )
    })?;
    let agent = sheets.agent.clone();
    let result = tokio::task::spawn_blocking(move || f(&Google::new(agent, token)))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Google Sheets call panicked");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
        })?;
    result.map_err(|e| {
        let msg = format!("{e:#}");
        tracing::warn!(user = user.id, error = %msg, "Google Sheets call failed");
        error(StatusCode::BAD_GATEWAY, &format!("Google: {msg}"))
    })
}

#[derive(Serialize)]
struct SpreadsheetJson {
    id: String,
    name: String,
    modified_time: String,
}

impl From<Spreadsheet> for SpreadsheetJson {
    fn from(s: Spreadsheet) -> Self {
        Self {
            id: s.id,
            name: s.name,
            modified_time: s.modified_time,
        }
    }
}

#[derive(Serialize)]
struct TabJson {
    sheet_id: u64,
    title: String,
}

impl From<Tab> for TabJson {
    fn from(t: Tab) -> Self {
        Self {
            sheet_id: t.sheet_id,
            title: t.title,
        }
    }
}

#[derive(Serialize)]
struct SpreadsheetDetailsJson {
    id: String,
    title: String,
    tabs: Vec<TabJson>,
}

fn details_json(id: String, d: SpreadsheetDetails) -> SpreadsheetDetailsJson {
    SpreadsheetDetailsJson {
        id,
        title: d.title,
        tabs: d.tabs.into_iter().map(TabJson::from).collect(),
    }
}

/// GET /api/statements/google/spreadsheets — the user's spreadsheets, most recent first.
async fn list_spreadsheets(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SpreadsheetJson>>, Response> {
    let list = with_google(&state, &user, |g| g.list_spreadsheets()).await?;
    Ok(Json(list.into_iter().map(SpreadsheetJson::from).collect()))
}

/// GET /api/statements/google/spreadsheets/{id} — its title and tabs.
async fn spreadsheet(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<SpreadsheetDetailsJson>, Response> {
    check_id(&id)?;
    let key = id.clone();
    let details = with_google(&state, &user, move |g| g.spreadsheet(&key)).await?;
    Ok(Json(details_json(id, details)))
}

#[derive(Deserialize)]
struct NewSpreadsheet {
    name: String,
    tab: String,
}

/// POST /api/statements/google/spreadsheets — a new spreadsheet (in the root of the user's
/// Drive) with one tab.
async fn create_spreadsheet(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<NewSpreadsheet>,
) -> Result<Json<SpreadsheetDetailsJson>, Response> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_SPREADSHEET_NAME_CHARS {
        return Err(bad_request(
            "Give the spreadsheet a name of up to 200 characters.",
        ));
    }
    let tab = check_tab_name(&body.tab)?;
    let title = name.clone();
    let (id, tab) = with_google(&state, &user, move |g| g.create_spreadsheet(&name, &tab)).await?;
    Ok(Json(SpreadsheetDetailsJson {
        id,
        title,
        tabs: vec![TabJson::from(tab)],
    }))
}

#[derive(Deserialize)]
struct NewTab {
    title: String,
}

/// POST /api/statements/google/spreadsheets/{id}/tabs
async fn create_tab(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<NewTab>,
) -> Result<Json<TabJson>, Response> {
    check_id(&id)?;
    let title = check_tab_name(&body.title)?;
    let tab = with_google(&state, &user, move |g| {
        let existing = g.spreadsheet(&id)?;
        if let Some(t) = existing
            .tabs
            .iter()
            .find(|t| t.title.to_lowercase() == title.to_lowercase())
        {
            anyhow::bail!("there is already a tab named {:?}", t.title);
        }
        g.add_tab(&id, &title)
    })
    .await?;
    Ok(Json(TabJson::from(tab)))
}

#[derive(Deserialize)]
struct SendRequest {
    spreadsheet_id: String,
    tab: String,
}

#[derive(Serialize)]
struct SendResult {
    spreadsheet_title: String,
    tab: String,
    url: String,
    parsed: usize,
    /// Rows whose id was already in the tab.
    existing: usize,
    appended: usize,
    /// Existing rows whose type was overwritten.
    updated: usize,
    wrote_header: bool,
    unreadable_ids: usize,
}

/// POST /api/orgs/{org}/statements/{id}/sheet — sends every row of the saved statement to
/// the tab: new ids appended, existing ones get their type overwritten (never blanked).
async fn send_to_sheet(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<SendRequest>,
) -> Result<Json<SendResult>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    check_id(&body.spreadsheet_id)?;
    let detail = upload_detail(&state.db, org.id, id).await?;
    let format = Format::from_id(&detail.upload.format)
        .ok_or_else(|| bad_request("This statement's format is no longer supported."))?;
    let statement = Statement {
        format,
        account_number: Some(detail.upload.account_number.clone()).filter(|a| !a.is_empty()),
        transactions: detail
            .transactions
            .iter()
            .map(|t| Transaction {
                // Stored from an i32, so it always fits.
                id: i32::try_from(t.txn_id).unwrap_or_default(),
                date: t.date.clone(),
                description: t.description.clone(),
                amount: t.amount.clone(),
                currency: t.currency.clone(),
                category: t.category.clone(),
            })
            .collect(),
    };

    let (target, summary) = with_google(&state, &user, move |g| {
        let details = g.spreadsheet(&body.spreadsheet_id)?;
        let tab = details
            .tabs
            .into_iter()
            .find(|t| t.title == body.tab)
            .ok_or_else(|| anyhow::anyhow!("no tab named {:?} in {:?}", body.tab, details.title))?;
        let target = Target {
            spreadsheet_id: body.spreadsheet_id,
            spreadsheet_title: details.title,
            tab,
        };
        let summary = upload::upload(g, &target, &statement)?;
        Ok((target, summary))
    })
    .await?;
    tracing::info!(
        upload = id,
        org = %org.slug,
        user = user.id,
        appended = summary.appended,
        updated = summary.updated,
        "statement sent to Google Sheets"
    );
    Ok(Json(SendResult {
        url: format!(
            "https://docs.google.com/spreadsheets/d/{}/edit#gid={}",
            target.spreadsheet_id, target.tab.sheet_id
        ),
        spreadsheet_title: target.spreadsheet_title,
        tab: target.tab.title,
        parsed: summary.parsed,
        existing: summary.existing,
        appended: summary.appended,
        updated: summary.updated,
        wrote_header: summary.wrote_header,
        unreadable_ids: summary.unreadable_ids,
    }))
}

/// Spreadsheet ids are URL-safe; refusing anything else keeps them out of URL paths unescaped.
fn check_id(id: &str) -> Result<(), Response> {
    let ok = !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(bad_request("That isn't a spreadsheet id."))
    }
}

fn check_tab_name(title: &str) -> Result<String, Response> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_TAB_NAME_CHARS {
        return Err(bad_request(&format!(
            "Give the tab a name of up to {MAX_TAB_NAME_CHARS} characters."
        )));
    }
    Ok(title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreadsheet_ids_are_checked() {
        assert!(check_id("1AbC-d_9").is_ok());
        assert!(check_id("").is_err());
        assert!(check_id("a/b").is_err());
        assert!(check_id("a?b").is_err());
    }

    #[test]
    fn tab_names_are_checked() {
        assert_eq!(check_tab_name("  Transactions ").unwrap(), "Transactions");
        assert!(check_tab_name(" ").is_err());
        assert!(check_tab_name(&"x".repeat(101)).is_err());
    }

    #[test]
    fn outcomes_are_added_to_the_return_path() {
        let location = |r: Response| r.headers()["location"].to_str().unwrap().to_owned();
        assert_eq!(
            location(back("/portal/acme/statements/3", Ok(()))),
            "/portal/acme/statements/3?google=connected"
        );
        assert_eq!(
            location(back("/portal/x?a=1", Err("denied"))),
            "/portal/x?a=1&google_error=denied"
        );
    }

    #[test]
    fn expired_tokens_are_not_handed_out() {
        let sheets = SheetsAuth::new("id".into(), "secret".into(), "http://localhost:5173/");
        assert_eq!(
            sheets.redirect_uri(),
            "http://localhost:5173/api/statements/google/callback"
        );
        sheets.connections.lock().unwrap().insert(
            1,
            Connection {
                access_token: "fresh".into(),
                email: None,
                expires_at: Instant::now() + Duration::from_secs(3600),
            },
        );
        sheets.connections.lock().unwrap().insert(
            2,
            Connection {
                access_token: "stale".into(),
                email: None,
                expires_at: Instant::now() + Duration::from_secs(30),
            },
        );
        assert_eq!(sheets.token(1).as_deref(), Some("fresh"));
        assert_eq!(sheets.token(2), None);
        assert_eq!(sheets.token(3), None);
    }
}
