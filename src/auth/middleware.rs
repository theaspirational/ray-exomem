//! Axum auth extractors and CSRF protection.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::auth::store::AuthStore;
use crate::auth::User;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const SESSION_COOKIE_NAME: &str = "ray_exomem_session";

// ---------------------------------------------------------------------------
// User extractor
// ---------------------------------------------------------------------------

impl FromRequestParts<Arc<AppState>> for User {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let auth_store = state.auth_store.as_ref().ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, "auth not configured").into_response()
        })?;

        // 1. Try Authorization: Bearer <key>
        if let Some(user) = try_bearer(parts, auth_store).await {
            return Ok(user);
        }

        // 2. Try session cookie
        if let Some(user) = try_session_cookie(parts, auth_store).await {
            return Ok(user);
        }

        // 3. Neither
        Err((StatusCode::UNAUTHORIZED, "authentication required").into_response())
    }
}

// ---------------------------------------------------------------------------
// MaybeUser extractor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<User>);

impl FromRequestParts<Arc<AppState>> for MaybeUser {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let Some(auth_store) = state.auth_store.as_ref() else {
            return Ok(MaybeUser(None));
        };

        if let Some(user) = try_bearer(parts, auth_store).await {
            return Ok(MaybeUser(Some(user)));
        }
        if let Some(user) = try_session_cookie(parts, auth_store).await {
            return Ok(MaybeUser(Some(user)));
        }

        Ok(MaybeUser(None))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers for extraction
// ---------------------------------------------------------------------------

async fn try_bearer(parts: &Parts, store: &AuthStore) -> Option<User> {
    let auth_header = parts.headers.get("authorization")?.to_str().ok()?;
    let token = auth_header.strip_prefix("Bearer ")?;
    if token.is_empty() {
        return None;
    }
    let key_hash = AuthStore::hash_api_key(token);
    store.get_user_by_key_hash(&key_hash).await
}

async fn try_session_cookie(parts: &Parts, store: &AuthStore) -> Option<User> {
    let cookie_header = parts.headers.get("cookie")?.to_str().ok()?;
    let session_id = extract_session_cookie(cookie_header)?;
    store.get_user_by_session(&session_id).await
}

// ---------------------------------------------------------------------------
// Cookie helpers
// ---------------------------------------------------------------------------

/// Parse a `Cookie` header string and return the value of
/// `ray_exomem_session`, if present and non-empty.
pub fn extract_session_cookie(cookies: &str) -> Option<String> {
    for pair in cookies.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(SESSION_COOKIE_NAME) {
            let value = value.strip_prefix('=')?;
            if value.is_empty() {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

/// Build a `Set-Cookie` header value for the session cookie.
pub fn session_cookie(session_id: &str, max_age_days: u32, secure: bool) -> String {
    let max_age_secs = max_age_days as u64 * 86400;
    let mut cookie = format!(
        "{SESSION_COOKIE_NAME}={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Build a `Set-Cookie` header that clears the session cookie.
pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
}

// ---------------------------------------------------------------------------
// CSRF check
// ---------------------------------------------------------------------------

/// Validate CSRF protection for state-changing requests.
///
/// - GET/HEAD are always allowed.
/// - Requests with an `Authorization` header (Bearer token) skip the check.
/// - Otherwise, the `Origin` (or `Referer` fallback) must match
///   `expected_origin`.
pub fn check_csrf(parts: &Parts, expected_origin: &str) -> Result<(), Response> {
    // Safe methods: no check needed.
    if parts.method == Method::GET || parts.method == Method::HEAD {
        return Ok(());
    }

    // Bearer-authenticated requests are not cookie-based, so CSRF is N/A.
    if parts.headers.contains_key("authorization") {
        return Ok(());
    }

    // Check Origin header.
    if let Some(origin) = parts.headers.get("origin").and_then(|v| v.to_str().ok()) {
        if origin == expected_origin {
            return Ok(());
        }
        return Err((StatusCode::FORBIDDEN, "CSRF origin mismatch").into_response());
    }

    // Fallback: check Referer.
    if let Some(referer) = parts.headers.get("referer").and_then(|v| v.to_str().ok()) {
        if referer.starts_with(expected_origin) {
            return Ok(());
        }
        return Err((StatusCode::FORBIDDEN, "CSRF referer mismatch").into_response());
    }

    // Neither Origin nor Referer present.
    Err((
        StatusCode::FORBIDDEN,
        "CSRF check failed: no origin or referer",
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_session_from_cookies() {
        let cookies = "other=foo; ray_exomem_session=abc123; another=bar";
        assert_eq!(extract_session_cookie(cookies), Some("abc123".into()));
    }

    #[test]
    fn extract_session_missing() {
        assert_eq!(extract_session_cookie("other=foo"), None);
    }

    #[test]
    fn extract_session_empty_value() {
        assert_eq!(extract_session_cookie("ray_exomem_session="), None);
    }

    #[test]
    fn session_cookie_format() {
        let cookie = session_cookie("sess-123", 30, false);
        assert!(cookie.contains("ray_exomem_session=sess-123"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=2592000"));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn session_cookie_secure() {
        let cookie = session_cookie("sess-123", 30, true);
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn clear_cookie_format() {
        let cookie = clear_session_cookie();
        assert!(cookie.contains("Max-Age=0"));
    }
}
