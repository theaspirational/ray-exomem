//! Admin panel route handlers.

use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, State},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::auth::store::AuthStore;
use crate::auth::User;
use crate::http_error::ApiError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn admin_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/users", get(list_users))
        .route("/users/{email}", delete(deactivate_user))
        .route("/users/{email}/activate", post(activate_user))
        .route("/admins", post(grant_admin))
        .route("/admins/{email}", delete(revoke_admin))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{id}", delete(kill_session))
        .route("/api-keys", get(list_all_api_keys))
        .route("/api-keys/{key_id}", delete(revoke_any_api_key))
        .route("/shares", get(list_all_shares))
        .route("/allowed-domains", get(list_domains).post(add_domain))
        .route("/allowed-domains/{domain}", delete(remove_domain))
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

fn require_admin(user: &User) -> Result<(), ApiError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(
            ApiError::new("forbidden", "admin access required")
                .with_status(403),
        )
    }
}

fn require_top_admin(user: &User) -> Result<(), ApiError> {
    if user.is_top_admin() {
        Ok(())
    } else {
        Err(
            ApiError::new("forbidden", "top-admin access required")
                .with_status(403),
        )
    }
}

fn require_auth_store(state: &AppState) -> Result<&Arc<AuthStore>, ApiError> {
    state.auth_store.as_ref().ok_or_else(|| {
        ApiError::new("auth_not_configured", "authentication is not configured")
            .with_status(501)
    })
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GrantAdminRequest {
    email: String,
}

#[derive(Deserialize)]
struct AddDomainRequest {
    domain: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /auth/admin/users
async fn list_users(user: User) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    // TODO: query system exom for all users
    Ok(Json(serde_json::json!({ "users": [] })))
}

/// DELETE /auth/admin/users/:email
async fn deactivate_user(
    user: User,
    AxumPath(_email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    // TODO: mark user as deactivated in system exom
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// POST /auth/admin/users/:email/activate
async fn activate_user(
    user: User,
    AxumPath(_email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    // TODO: re-activate user in system exom
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// POST /auth/admin/admins
async fn grant_admin(
    user: User,
    Json(_body): Json<GrantAdminRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    // TODO: update user role to Admin in system exom
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// DELETE /auth/admin/admins/:email
async fn revoke_admin(
    user: User,
    AxumPath(_email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    // TODO: downgrade user from Admin to Regular in system exom
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /auth/admin/sessions
async fn list_sessions(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;

    let sessions: Vec<serde_json::Value> = store
        .session_cache
        .iter()
        .map(|entry| {
            let u = entry.value();
            serde_json::json!({
                "session_id": entry.key(),
                "email": u.email,
                "display_name": u.display_name,
                "provider": u.provider,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "sessions": sessions })))
}

/// DELETE /auth/admin/sessions/:id
async fn kill_session(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    store.evict_session(&id);
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /auth/admin/api-keys
async fn list_all_api_keys(user: User) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    // TODO: query system exom for all api keys
    Ok(Json(serde_json::json!({ "keys": [] })))
}

/// DELETE /auth/admin/api-keys/:key_id
async fn revoke_any_api_key(
    user: User,
    AxumPath(_key_id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    // TODO: revoke key from system exom and evict from cache
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /auth/admin/shares
async fn list_all_shares(user: User) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    // TODO: query system exom for all shares
    Ok(Json(serde_json::json!({ "shares": [] })))
}

/// GET /auth/admin/allowed-domains
async fn list_domains(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    let domains = store.list_allowed_domains();
    Ok(Json(serde_json::json!({ "domains": domains })))
}

/// POST /auth/admin/allowed-domains
async fn add_domain(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<AddDomainRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    let store = require_auth_store(&state)?;

    let domain = body.domain.trim().to_lowercase();
    if domain.is_empty() {
        return Err(ApiError::new("invalid_domain", "domain must not be empty").with_status(400));
    }

    {
        let mut domains = store.allowed_domains.lock().unwrap();
        if !domains.contains(&domain) {
            domains.push(domain.clone());
        }
    }

    Ok(Json(serde_json::json!({ "ok": true, "domain": domain })))
}

/// DELETE /auth/admin/allowed-domains/:domain
async fn remove_domain(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(domain): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    let store = require_auth_store(&state)?;

    {
        let mut domains = store.allowed_domains.lock().unwrap();
        domains.retain(|d| d != &domain);
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}
