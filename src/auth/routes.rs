//! Auth route handlers: login, logout, me, api-keys, shares.

use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, State},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::middleware::{clear_session_cookie, session_cookie};
use crate::auth::store::AuthStore;
use crate::auth::{User, UserRole};
use crate::http_error::ApiError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn auth_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/{key_id}", delete(revoke_api_key))
        .route("/shares", get(list_shares).post(create_share))
        .route("/shares/{share_id}", delete(revoke_share))
        .route("/shared-with-me", get(shared_with_me))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginRequest {
    id_token: String,
    #[serde(default)]
    provider: Option<String>,
}

#[derive(Serialize)]
struct LoginResponse {
    email: String,
    display_name: String,
    role: String,
}

#[derive(Serialize)]
struct MeResponse {
    email: String,
    display_name: String,
    provider: String,
    role: String,
}

#[derive(Deserialize)]
struct CreateApiKeyRequest {
    label: String,
}

#[derive(Serialize)]
struct CreateApiKeyResponse {
    key_id: String,
    raw_key: String,
    label: String,
    mcp_config_snippet: serde_json::Value,
}

#[derive(Deserialize)]
struct CreateShareRequest {
    path: String,
    grantee_email: String,
    permission: String,
}

#[derive(Serialize)]
struct CreateShareResponse {
    share_id: String,
    path: String,
    grantee_email: String,
    permission: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_auth_store(state: &AppState) -> Result<&Arc<AuthStore>, ApiError> {
    state.auth_store.as_ref().ok_or_else(|| {
        ApiError::new("auth_not_configured", "authentication is not configured")
            .with_status(501)
    })
}

fn require_auth_provider(
    state: &AppState,
) -> Result<&Arc<dyn crate::auth::provider::AuthProvider>, ApiError> {
    state.auth_provider.as_ref().ok_or_else(|| {
        ApiError::new("auth_not_configured", "authentication provider is not configured")
            .with_status(501)
    })
}

fn role_label(role: &UserRole) -> &'static str {
    match role {
        UserRole::Regular => "regular",
        UserRole::Admin => "admin",
        UserRole::TopAdmin => "top-admin",
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /auth/login
///
/// Body: { id_token, provider? }
/// Validates token via the configured provider, checks domain restrictions,
/// creates a session, caches the user. First user ever becomes top-admin.
async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;
    let provider = require_auth_provider(&state)?;

    // Validate the token.
    let identity = provider
        .validate_token(&body.id_token)
        .await
        .map_err(|e| {
            ApiError::new("invalid_token", format!("token validation failed: {e}"))
                .with_status(401)
        })?;

    // Check domain restriction.
    if !store.check_domain(&identity.email) {
        return Err(
            ApiError::new("domain_not_allowed", "your email domain is not allowed")
                .with_status(403)
                .with_suggestion("contact an administrator to add your domain"),
        );
    }

    // Resolve role. First user ever (empty session + api_key caches) becomes top-admin.
    let role = if store.session_cache.is_empty() && store.api_key_cache.is_empty() {
        UserRole::TopAdmin
    } else {
        store.resolve_role(&identity.email)
    };

    // Create session.
    let session_id = uuid::Uuid::new_v4().to_string();

    let user = User {
        email: identity.email.clone(),
        display_name: identity.display_name.clone(),
        provider: identity.provider.clone(),
        session_id: Some(session_id.clone()),
        role: role.clone(),
    };

    // Cache the session.
    store.session_cache.insert(session_id.clone(), user.clone());

    // Determine if we should set Secure flag on the cookie.
    let secure = state
        .bind_addr
        .as_deref()
        .map(|b| !b.starts_with("127.0.0.1") && !b.starts_with("localhost"))
        .unwrap_or(false);

    let cookie = session_cookie(&session_id, 30, secure);

    let response = LoginResponse {
        email: identity.email,
        display_name: identity.display_name,
        role: role_label(&role).to_string(),
    };

    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(response),
    ))
}

/// POST /auth/logout
async fn logout(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;

    // Evict session if present.
    if let Some(sid) = &user.session_id {
        store.evict_session(sid);
    }

    let cookie = clear_session_cookie();

    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    ))
}

/// GET /auth/me
async fn me(user: User) -> impl IntoResponse {
    Json(MeResponse {
        email: user.email,
        display_name: user.display_name,
        provider: user.provider,
        role: role_label(&user.role).to_string(),
    })
}

/// POST /auth/api-keys
async fn create_api_key(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;

    let (key_id, raw_key) = store.generate_api_key(&user.email, &body.label);
    let key_hash = AuthStore::hash_api_key(&raw_key);

    // Cache the key -> user mapping.
    let api_user = User {
        session_id: None,
        ..user.clone()
    };
    store.api_key_cache.insert(key_hash, api_user);

    let bind = state.bind_addr.as_deref().unwrap_or("127.0.0.1:9780");
    let mcp_snippet = serde_json::json!({
        "mcpServers": {
            "ray-exomem": {
                "url": format!("http://{bind}/ray-exomem/api"),
                "headers": {
                    "Authorization": format!("Bearer {raw_key}")
                }
            }
        }
    });

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            key_id,
            raw_key,
            label: body.label,
            mcp_config_snippet: mcp_snippet,
        }),
    ))
}

/// GET /auth/api-keys
async fn list_api_keys(_user: User) -> impl IntoResponse {
    // TODO: query system exom for user's keys
    Json(serde_json::json!({ "keys": [] }))
}

/// DELETE /auth/api-keys/:key_id
async fn revoke_api_key(
    _user: User,
    AxumPath(_key_id): AxumPath<String>,
) -> impl IntoResponse {
    // TODO: revoke key from system exom and evict from cache
    Json(serde_json::json!({ "ok": true }))
}

/// POST /auth/shares
async fn create_share(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<CreateShareRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;

    // Validate permission.
    if body.permission != "read" && body.permission != "read-write" {
        return Err(
            ApiError::new(
                "invalid_permission",
                format!("permission must be 'read' or 'read-write', got '{}'", body.permission),
            )
            .with_status(400),
        );
    }

    // Verify user owns the path (path must start with user's email).
    if body.path != user.email && !body.path.starts_with(&format!("{}/", user.email)) {
        if !user.is_admin() {
            return Err(
                ApiError::new("not_owner", "you can only share paths you own")
                    .with_status(403),
            );
        }
    }

    let share_id = uuid::Uuid::new_v4().to_string();

    let created_at = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    store.add_share_grant(crate::auth::store::ShareGrant {
        share_id: share_id.clone(),
        owner_email: user.email.clone(),
        path: body.path.clone(),
        grantee_email: body.grantee_email.clone(),
        permission: body.permission.clone(),
        created_at,
    });

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateShareResponse {
            share_id,
            path: body.path,
            grantee_email: body.grantee_email,
            permission: body.permission,
        }),
    ))
}

/// GET /auth/shares
async fn list_shares(_user: User) -> impl IntoResponse {
    // TODO: query system exom for user's shares
    Json(serde_json::json!({ "shares": [] }))
}

/// DELETE /auth/shares/:share_id
async fn revoke_share(
    _user: User,
    AxumPath(_share_id): AxumPath<String>,
) -> impl IntoResponse {
    // TODO: revoke share from system exom
    Json(serde_json::json!({ "ok": true }))
}

/// GET /auth/shared-with-me
async fn shared_with_me(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;
    let grants = store.shares_for_grantee(&user.email);
    let items: Vec<serde_json::Value> = grants
        .iter()
        .map(|g| {
            serde_json::json!({
                "share_id": g.share_id,
                "owner_email": g.owner_email,
                "path": g.path,
                "permission": g.permission,
                "created_at": g.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "shares": items })))
}
