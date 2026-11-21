//! Google OAuth 2.0 pieces shared by the CLI (installed-app flow, loopback redirect) and the web server (web-app
//! flow): the consent URL, PKCE, exchanging the code, refreshing and revoking tokens.

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Read and write spreadsheets, and list spreadsheet names (Drive metadata only, never file contents).
pub const SCOPES: [&str; 2] = [
    "https://www.googleapis.com/auth/spreadsheets",
    "https://www.googleapis.com/auth/drive.metadata.readonly",
];

/// An OAuth client from the Google Cloud console: "Desktop app" for the CLI, "Web application" for the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
}

/// What goes into Google's consent URL.
pub struct AuthRequest<'a> {
    pub redirect_uri: &'a str,
    /// Space-separated, e.g. [`SCOPES`] joined, plus `openid email` to learn who signed in.
    pub scope: &'a str,
    /// Echoed back on the redirect; must be checked against what was sent.
    pub state: &'a str,
    /// [`pkce_challenge`] of a verifier kept until the code is exchanged.
    pub code_challenge: &'a str,
    /// Ask for a refresh token (`access_type=offline`, with a consent prompt so one is always issued).
    pub offline: bool,
}

/// Google's consent page for `request`. Always lets the user choose the account.
pub fn authorization_url(client: &OAuthClient, request: &AuthRequest) -> Result<Url> {
    let mut params = vec![
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", request.redirect_uri),
        ("response_type", "code"),
        ("scope", request.scope),
        ("code_challenge", request.code_challenge),
        ("code_challenge_method", "S256"),
        ("state", request.state),
        ("include_granted_scopes", "true"),
    ];
    if request.offline {
        params.push(("access_type", "offline"));
        params.push(("prompt", "consent select_account"));
    } else {
        params.push(("prompt", "select_account"));
    }
    Ok(Url::parse_with_params(AUTH_URL, params)?)
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
    /// Only with offline access; a refresh keeps the existing one.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Space-separated scopes actually granted (the user can untick some on the consent screen).
    #[serde(default)]
    pub scope: String,
    /// Only when `openid` was asked for.
    #[serde(default)]
    pub id_token: Option<String>,
}

impl TokenResponse {
    /// The signed-in user's email, from the ID token (needs the `openid email` scopes).
    ///
    /// The signature isn't checked: the token came straight from Google's token endpoint over TLS, which OpenID
    /// Connect allows in place of validation. It is only used to show who is signed in, never to authorise.
    pub fn email(&self) -> Option<String> {
        #[derive(Deserialize)]
        struct Claims {
            email: Option<String>,
        }
        let payload = self.id_token.as_deref()?.split('.').nth(1)?;
        let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
        serde_json::from_slice::<Claims>(&bytes).ok()?.email
    }
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: String,
}

fn default_expires_in() -> u64 {
    3600
}

/// Exchanges the code from the redirect for tokens.
pub fn exchange_code(
    agent: &ureq::Agent,
    client: &OAuthClient,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse> {
    token_request(
        agent,
        &[
            ("code", code),
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ],
    )
}

pub fn refresh(
    agent: &ureq::Agent,
    client: &OAuthClient,
    refresh_token: &str,
) -> Result<TokenResponse> {
    token_request(
        agent,
        &[
            ("refresh_token", refresh_token),
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("grant_type", "refresh_token"),
        ],
    )
}

fn token_request(agent: &ureq::Agent, form: &[(&str, &str)]) -> Result<TokenResponse> {
    let mut response = agent
        .post(TOKEN_URL)
        .send_form(form.iter().copied())
        .context("requesting the Google access token")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.body_mut().read_to_string().unwrap_or_default();
        return Err(match serde_json::from_str::<TokenError>(&body) {
            Ok(e) => anyhow!(
                "Google refused the sign-in ({status}): {} {}",
                e.error,
                e.error_description
            ),
            Err(_) => anyhow!("Google refused the sign-in ({status}): {body}"),
        });
    }
    response
        .body_mut()
        .read_json()
        .context("reading the Google access token")
}

/// Revokes a token at Google (a refresh token also revokes its access tokens). `Ok(false)` when Google declined,
/// e.g. because the token had already expired.
pub fn revoke(agent: &ureq::Agent, token: &str) -> Result<bool> {
    let response = agent
        .post(REVOKE_URL)
        .send_form([("token", token)])
        .context("revoking the Google sign-in")?;
    Ok(response.status().is_success())
}

/// [`SCOPES`] the user did not grant.
pub fn missing_scopes(granted: &str) -> Vec<&'static str> {
    let granted: Vec<&str> = granted.split_whitespace().collect();
    SCOPES
        .into_iter()
        .filter(|s| !granted.contains(s))
        .collect()
}

/// `bytes` random bytes as base64url without padding (32 bytes -> a 43-character PKCE verifier).
pub fn random_token(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("generating random bytes: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// PKCE S256: base64url(sha256(verifier)).
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc_7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_tokens_are_url_safe_and_distinct() {
        let a = random_token(32).unwrap();
        assert_eq!(a.len(), 43);
        assert!(
            a.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(a, random_token(32).unwrap());
    }

    #[test]
    fn detects_unticked_scopes() {
        assert!(missing_scopes(&SCOPES.join(" ")).is_empty());
        assert_eq!(
            missing_scopes("openid https://www.googleapis.com/auth/spreadsheets"),
            ["https://www.googleapis.com/auth/drive.metadata.readonly"]
        );
    }

    #[test]
    fn consent_url_online_and_offline() {
        let client = OAuthClient {
            client_id: "id.apps.googleusercontent.com".into(),
            client_secret: "s".into(),
        };
        let request = |offline| AuthRequest {
            redirect_uri: "https://x.example/auth/callback",
            scope: "a b",
            state: "st",
            code_challenge: "ch",
            offline,
        };
        let param = |url: &Url, k: &str| {
            url.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };
        let online = authorization_url(&client, &request(false)).unwrap();
        assert_eq!(param(&online, "access_type"), None);
        assert_eq!(param(&online, "prompt").as_deref(), Some("select_account"));
        assert_eq!(
            param(&online, "redirect_uri").as_deref(),
            Some("https://x.example/auth/callback")
        );
        assert_eq!(param(&online, "scope").as_deref(), Some("a b"));
        let offline = authorization_url(&client, &request(true)).unwrap();
        assert_eq!(param(&offline, "access_type").as_deref(), Some("offline"));
    }

    #[test]
    fn email_from_id_token() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"someone@example.com","sub":"1"}"#);
        let token = TokenResponse {
            access_token: "at".into(),
            expires_in: 3600,
            refresh_token: None,
            scope: String::new(),
            id_token: Some(format!("header.{payload}.sig")),
        };
        assert_eq!(token.email().as_deref(), Some("someone@example.com"));
        let none = TokenResponse {
            id_token: Some("garbage".into()),
            ..token
        };
        assert_eq!(none.email(), None);
    }
}
