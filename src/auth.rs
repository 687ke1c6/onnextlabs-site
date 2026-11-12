//! Sign-in via OpenID Connect (authorization code + PKCE + nonce) and server-side sessions.
//!
//! Any OIDC provider works: Google is the only one configured today, and Keycloak (or
//! anything else) would be another `Provider` with a different issuer URL.
//! Signing in creates a `users` row but grants nothing; access comes from the account's
//! admin flag (or ADMIN_EMAILS) and org memberships.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts, Path, Query, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    reqwest, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, TokenResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::{subscribe::client_ip, AppState};

const SESSION_COOKIE: &str = "sid";
const STATE_COOKIE: &str = "oauth_state";
const STATE_COOKIE_PATH: &str = "/api/auth";
const SESSION_DAYS: i64 = 30;
/// How long a user has to finish signing in at the provider.
const LOGIN_TTL: Duration = Duration::from_secs(600);
/// Cap on in-flight sign-ins, so unauthenticated requests can't grow memory without bound.
const MAX_PENDING: usize = 10_000;
/// Re-fetch provider metadata (incl. signing keys) this often, to pick up key rotation.
const METADATA_TTL: Duration = Duration::from_secs(3600);
pub const DEFAULT_RETURN_TO: &str = "/portal";

type OidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub struct Provider {
    /// URL segment and display key, e.g. "google".
    pub name: &'static str,
    issuer: IssuerUrl,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect: RedirectUrl,
    http: reqwest::Client,
    metadata: RwLock<Option<(CoreProviderMetadata, Instant)>>,
}

impl Provider {
    pub fn new(
        name: &'static str,
        issuer: &str,
        client_id: String,
        client_secret: String,
        public_url: &str,
    ) -> Result<Self, String> {
        let redirect = format!(
            "{}/api/auth/callback/{name}",
            public_url.trim_end_matches('/')
        );
        Ok(Self {
            name,
            issuer: IssuerUrl::new(issuer.into()).map_err(|e| format!("issuer url: {e}"))?,
            client_id: ClientId::new(client_id),
            client_secret: ClientSecret::new(client_secret),
            redirect: RedirectUrl::new(redirect).map_err(|e| format!("redirect url: {e}"))?,
            http: reqwest::ClientBuilder::new()
                // Following redirects opens the client up to SSRF.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| format!("http client: {e}"))?,
            metadata: RwLock::default(),
        })
    }

    pub fn redirect_url(&self) -> &str {
        self.redirect.as_str()
    }

    /// Provider metadata via OIDC discovery, cached. Falls back to stale metadata if a
    /// refresh fails, so a provider blip doesn't break sign-in.
    async fn client(&self) -> Result<OidcClient, String> {
        let cached = self.metadata.read().await.clone();
        let meta = match cached {
            Some((meta, at)) if at.elapsed() < METADATA_TTL => meta,
            stale => {
                match CoreProviderMetadata::discover_async(self.issuer.clone(), &self.http).await {
                    Ok(meta) => {
                        *self.metadata.write().await = Some((meta.clone(), Instant::now()));
                        meta
                    }
                    Err(e) => {
                        tracing::error!(provider = self.name, error = %e, "OIDC discovery failed");
                        stale.map(|(m, _)| m).ok_or("discovery failed")?
                    }
                }
            }
        };
        Ok(CoreClient::from_provider_metadata(
            meta,
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect.clone()))
    }
}

struct PendingLogin {
    provider: &'static str,
    verifier: PkceCodeVerifier,
    nonce: Nonce,
    return_to: String,
    started: Instant,
}

pub struct Auth {
    providers: Vec<Provider>,
    /// Lowercased emails that always get the admin role (ADMIN_EMAILS).
    admin_emails: Vec<String>,
    /// Set `Secure` on cookies (true when the public URL is https).
    secure_cookies: bool,
    /// In-flight sign-ins keyed by OAuth `state`. Lost on restart; users just retry.
    pending: Mutex<HashMap<String, PendingLogin>>,
}

impl Auth {
    pub fn new(providers: Vec<Provider>, admin_emails: Vec<String>, secure_cookies: bool) -> Self {
        Self {
            providers,
            admin_emails,
            secure_cookies,
            pending: Mutex::default(),
        }
    }

    /// Whether ADMIN_EMAILS makes this address an admin. Checked live, so removing an
    /// address revokes it; only provider-verified emails count, or anyone could claim one.
    pub fn is_env_admin(&self, email: Option<&str>, verified: bool) -> bool {
        verified
            && email.is_some_and(|e| self.admin_emails.iter().any(|a| a.eq_ignore_ascii_case(e)))
    }

    fn provider(&self, name: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.name == name)
    }

    fn cookie(
        &self,
        name: &'static str,
        value: String,
        path: &'static str,
        max_age: time::Duration,
    ) -> Cookie<'static> {
        Cookie::build((name, value))
            .path(path)
            .http_only(true)
            .secure(self.secure_cookies)
            // Lax (not Strict) so the cookie survives the top-level redirect back from the provider.
            .same_site(SameSite::Lax)
            .max_age(max_age)
            .build()
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/auth/providers", get(providers))
        .route("/api/auth/login/{provider}", get(login))
        .route("/api/auth/callback/{provider}", get(callback))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
}

/// Only same-site relative paths, so `return_to` can't become an open redirect.
fn safe_return_to(raw: Option<&str>) -> String {
    match raw {
        Some(p)
            if p.starts_with('/')
                && !p.starts_with("//")
                && !p.contains('\\')
                && p.len() <= 512
                && !p.chars().any(char::is_control) =>
        {
            p.to_owned()
        }
        _ => DEFAULT_RETURN_TO.to_owned(),
    }
}

fn login_error(code: &str) -> Redirect {
    Redirect::to(&format!("/login?error={code}"))
}

fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// GET /api/auth/providers: which sign-in options the login page should offer.
async fn providers(State(state): State<Arc<AppState>>) -> Json<Vec<&'static str>> {
    Json(state.auth.providers.iter().map(|p| p.name).collect())
}

#[derive(Deserialize)]
struct LoginQuery {
    return_to: Option<String>,
}

/// GET /api/auth/login/{provider}: redirect to the provider's consent screen.
async fn login(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(q): Query<LoginQuery>,
    jar: CookieJar,
) -> Response {
    let auth = &state.auth;
    let Some(provider) = auth.provider(&name) else {
        return login_error("unavailable").into_response();
    };
    let client = match provider.client().await {
        Ok(c) => c,
        Err(_) => return login_error("provider").into_response(),
    };

    let (pkce_challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, csrf, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .set_pkce_challenge(pkce_challenge)
        // Let people pick which Google account, rather than silently reusing the last one.
        .add_extra_param("prompt", "select_account")
        .url();

    {
        let mut pending = auth.pending.lock().unwrap();
        pending.retain(|_, p| p.started.elapsed() < LOGIN_TTL);
        if pending.len() >= MAX_PENDING {
            tracing::warn!("too many pending sign-ins");
            return login_error("busy").into_response();
        }
        pending.insert(
            csrf.secret().clone(),
            PendingLogin {
                provider: provider.name,
                verifier,
                nonce,
                return_to: safe_return_to(q.return_to.as_deref()),
                started: Instant::now(),
            },
        );
    }

    // Binds the callback to this browser (prevents login CSRF).
    let state_cookie = auth.cookie(
        STATE_COOKIE,
        csrf.secret().clone(),
        STATE_COOKIE_PATH,
        time::Duration::seconds(LOGIN_TTL.as_secs() as i64),
    );
    (jar.add(state_cookie), Redirect::to(url.as_str())).into_response()
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

struct Identity {
    issuer: String,
    subject: String,
    email: Option<String>,
    email_verified: bool,
    name: Option<String>,
    picture: Option<String>,
}

/// GET /api/auth/callback/{provider}: finish sign-in, start a session, go to `return_to`.
async fn callback(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(q): Query<CallbackQuery>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let auth = &state.auth;
    let cookie_state = jar.get(STATE_COOKIE).map(|c| c.value().to_owned());
    let jar = jar.remove(Cookie::build(STATE_COOKIE).path(STATE_COOKIE_PATH));
    let fail = |jar: CookieJar, code: &str| (jar, login_error(code)).into_response();

    let Some(st) = q.state.filter(|s| Some(s) == cookie_state.as_ref()) else {
        return fail(jar, "state");
    };
    let Some(pending) = auth.pending.lock().unwrap().remove(&st) else {
        return fail(jar, "expired");
    };
    if pending.started.elapsed() >= LOGIN_TTL || pending.provider != name {
        return fail(jar, "expired");
    }
    if let Some(err) = q.error {
        tracing::info!(provider = %name, error = %err, "sign-in cancelled or refused");
        return fail(jar, "denied");
    }
    let (Some(code), Some(provider)) = (q.code, auth.provider(&name)) else {
        return fail(jar, "state");
    };

    let identity = match exchange(provider, code, pending.verifier, &pending.nonce).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(provider = %name, error = %e, "sign-in failed");
            return fail(jar, "provider");
        }
    };

    let ip = client_ip(&headers, state.client_ip_header.as_ref(), peer).to_string();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.chars().take(512).collect::<String>());

    let token = match start_session(&state.db, &identity, &ip, user_agent).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "creating session failed");
            return fail(jar, "server");
        }
    };
    tracing::info!(%ip, email = ?identity.email, "signed in");

    let session_cookie = auth.cookie(
        SESSION_COOKIE,
        token,
        "/",
        time::Duration::days(SESSION_DAYS),
    );
    (jar.add(session_cookie), Redirect::to(&pending.return_to)).into_response()
}

/// Swaps the code for tokens and verifies the ID token (signature, issuer, audience, expiry, nonce).
async fn exchange(
    provider: &Provider,
    code: String,
    verifier: PkceCodeVerifier,
    nonce: &Nonce,
) -> Result<Identity, String> {
    let client = provider.client().await?;
    let tokens = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|e| format!("no token endpoint: {e}"))?
        .set_pkce_verifier(verifier)
        .request_async(&provider.http)
        .await
        .map_err(|e| format!("token exchange: {e}"))?;
    let id_token = tokens.id_token().ok_or("no id_token in response")?;
    let claims = id_token
        .claims(&client.id_token_verifier(), nonce)
        .map_err(|e| format!("id_token: {e}"))?;

    Ok(Identity {
        issuer: claims.issuer().as_str().to_owned(),
        subject: claims.subject().as_str().to_owned(),
        email: claims.email().map(|e| e.as_str().to_lowercase()),
        email_verified: claims.email_verified().unwrap_or(false),
        name: claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.as_str().to_owned()),
        picture: claims
            .picture()
            .and_then(|p| p.get(None))
            .map(|p| p.as_str().to_owned()),
    })
}

/// Upserts the user (refreshing profile fields) and creates a session; returns the cookie token.
async fn start_session(
    db: &SqlitePool,
    id: &Identity,
    ip: &str,
    user_agent: Option<String>,
) -> sqlx::Result<String> {
    let mut tx = db.begin().await?;
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email, email_verified, name, picture)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (issuer, subject) DO UPDATE SET
             email = excluded.email,
             email_verified = excluded.email_verified,
             name = excluded.name,
             picture = excluded.picture,
             last_login_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         RETURNING id",
    )
    .bind(&id.issuer)
    .bind(&id.subject)
    .bind(&id.email)
    .bind(id.email_verified)
    .bind(&id.name)
    .bind(&id.picture)
    .fetch_one(&mut *tx)
    .await?;

    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, ip, user_agent, expires_at)
         VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?))",
    )
    .bind(hash_token(&token))
    .bind(user_id)
    .bind(ip)
    .bind(user_agent)
    .bind(format!("+{SESSION_DAYS} days"))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(token)
}

/// POST /api/auth/logout
async fn logout(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        let res = sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(hash_token(c.value()))
            .execute(&state.db)
            .await;
        if let Err(e) = res {
            tracing::error!(error = %e, "deleting session failed");
        }
    }
    (
        jar.remove(Cookie::build(SESSION_COOKIE).path("/")),
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

/// GET /api/auth/me
async fn me(user: CurrentUser) -> Json<CurrentUser> {
    Json(user)
}

/// Deletes expired sessions; called periodically.
pub async fn sweep_sessions(db: &SqlitePool) {
    let sql = "DELETE FROM sessions WHERE expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
    if let Err(e) = sqlx::query(sql).execute(db).await {
        tracing::error!(error = %e, "sweeping sessions failed");
    }
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// The signed-in user. As an extractor, rejects with 401 when there's no valid session.
#[derive(Serialize, sqlx::FromRow)]
pub struct CurrentUser {
    pub id: i64,
    pub email: Option<String>,
    pub name: Option<String>,
    pub picture: Option<String>,
    #[serde(skip)]
    pub email_verified: bool,
    /// Admins can do everything, in every organisation.
    pub admin: bool,
    /// Why they're an admin: "flag" (set by an admin or `grant-admin`), "admin-emails".
    /// Empty if not.
    #[sqlx(skip)]
    pub admin_via: Vec<&'static str>,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.admin
    }
}

impl FromRequestParts<Arc<AppState>> for CurrentUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let unauthorized = || json_error(StatusCode::UNAUTHORIZED, "Not signed in.");
        let internal = |e: sqlx::Error| {
            tracing::error!(error = %e, "session lookup failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
        };

        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(SESSION_COOKIE).ok_or_else(unauthorized)?;

        let user: Option<CurrentUser> = sqlx::query_as(
            "SELECT u.id, u.email, u.name, u.picture, u.email_verified, u.is_admin AS admin
                 FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ? AND s.expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(hash_token(token.value()))
        .fetch_optional(&state.db)
        .await
        .map_err(internal)?;
        let mut user = user.ok_or_else(unauthorized)?;

        // Why they're an admin, for "Your access".
        if user.admin {
            user.admin_via.push("flag");
        }
        if state
            .auth
            .is_env_admin(user.email.as_deref(), user.email_verified)
        {
            user.admin_via.push("admin-emails");
            user.admin = true;
        }

        Ok(user)
    }
}

/// A signed-in admin. Rejects with 401 / 403 otherwise.
pub struct Admin(#[allow(dead_code)] pub CurrentUser);

impl FromRequestParts<Arc<AppState>> for Admin {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if user.is_admin() {
            Ok(Admin(user))
        } else {
            Err(json_error(StatusCode::FORBIDDEN, "Access not granted."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_is_same_site_only() {
        assert_eq!(
            safe_return_to(Some("/portal/documents?x=1")),
            "/portal/documents?x=1"
        );
        for bad in [
            None,
            Some(""),
            Some("portal"),
            Some("//evil.com"),
            Some("/\\evil.com"),
            Some("https://evil.com"),
            Some("/a\nb"),
        ] {
            assert_eq!(safe_return_to(bad), DEFAULT_RETURN_TO, "{bad:?}");
        }
    }
}
