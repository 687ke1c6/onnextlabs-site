use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, HeaderName, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;

#[derive(Deserialize)]
pub struct SubscribeRequest {
    email: String,
    /// Honeypot: hidden in the form, so only bots fill it in.
    #[serde(default)]
    website: String,
    /// Base64 ALTCHA proof-of-work payload from the widget.
    #[serde(default)]
    altcha: String,
}

/// GET /api/altcha/challenge
pub async fn challenge(State(state): State<Arc<AppState>>) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(state.altcha.create_challenge()),
    )
        .into_response()
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// POST /api/subscribe
pub async fn subscribe(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SubscribeRequest>,
) -> Response {
    let ip = client_ip(&headers, state.client_ip_header.as_ref(), peer);

    if let Err(wait) = state.limiter.check(ip) {
        let secs = wait.as_secs().max(1);
        tracing::warn!(%ip, "subscribe rate limited");
        let mut res = error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Please try again later.",
        );
        res.headers_mut().insert(header::RETRY_AFTER, secs.into());
        return res;
    }

    // Pretend success so bots don't learn to skip the field.
    if !req.website.is_empty() {
        tracing::info!(%ip, "subscribe honeypot triggered");
        return Json(json!({ "ok": true })).into_response();
    }

    if let Err(reason) = state.altcha.verify(&req.altcha) {
        tracing::info!(%ip, reason, "subscribe altcha rejected");
        return error(
            StatusCode::BAD_REQUEST,
            "Verification failed. Please try again.",
        );
    }

    let Some(email) = normalize_email(&req.email) else {
        return error(
            StatusCode::BAD_REQUEST,
            "Please enter a valid email address.",
        );
    };
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.chars().take(512).collect::<String>());

    // Repeat signups succeed silently (no "already subscribed" leak) and re-activate an
    // unsubscribed address; the original ip/user_agent/created_at are kept.
    let result = sqlx::query(
        "INSERT INTO subscribers (email, ip, user_agent) VALUES (?, ?, ?)
         ON CONFLICT (email) DO UPDATE SET unsubscribed_at = NULL",
    )
    .bind(&email)
    .bind(ip.to_string())
    .bind(user_agent)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            tracing::info!(%ip, "subscribed");
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "subscribe insert failed");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong. Please try again.",
            )
        }
    }
}

/// Client IP: from `ip_header` when running behind a trusted proxy, else the socket peer.
/// For X-Forwarded-For the rightmost entry is used, since that's the one our proxy appended
/// (anything to its left is client-supplied and spoofable).
pub(crate) fn client_ip(
    headers: &HeaderMap,
    ip_header: Option<&HeaderName>,
    peer: SocketAddr,
) -> std::net::IpAddr {
    ip_header
        .and_then(|name| headers.get(name))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| peer.ip())
}

/// Trims and lowercases; rejects anything that isn't plausibly `local@domain.tld`.
/// Deliberately loose: the real check is whether mail gets delivered.
fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    if email.len() > 254 || email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (local, domain) = email.rsplit_once('@')?;
    let domain_ok = domain.contains('.')
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && !domain.contains("..");
    (!local.is_empty() && local.len() <= 64 && !local.contains('@') && domain_ok).then_some(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails() {
        assert_eq!(
            normalize_email("  Foo.Bar@Example.COM "),
            Some("foo.bar@example.com".into())
        );
        assert_eq!(
            normalize_email("a+tag@sub.example.co.uk"),
            Some("a+tag@sub.example.co.uk".into())
        );
        for bad in [
            "",
            "foo",
            "@example.com",
            "foo@",
            "foo@localhost",
            "a@b@c.com",
            "a b@c.com",
            "a@.com",
            "a@c..com",
            "a@c.com.",
        ] {
            assert_eq!(normalize_email(bad), None, "{bad}");
        }
    }

    #[test]
    fn ip_from_header() {
        let peer: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        let xff = HeaderName::from_static("x-forwarded-for");
        let mut h = HeaderMap::new();
        h.insert(&xff, "6.6.6.6, 203.0.113.7".parse().unwrap());
        assert_eq!(client_ip(&h, Some(&xff), peer).to_string(), "203.0.113.7");
        assert_eq!(client_ip(&h, None, peer).to_string(), "10.0.0.1");
        h.insert(&xff, "garbage".parse().unwrap());
        assert_eq!(client_ip(&h, Some(&xff), peer).to_string(), "10.0.0.1");
    }
}
