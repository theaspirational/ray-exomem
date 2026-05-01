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
use crate::auth::{User, UserRole};
use crate::http_error::ApiError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn admin_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/users", get(list_users))
        .route("/users/{email}", delete(delete_user_account))
        .route("/users/{email}/deactivate", post(deactivate_user))
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
        .route("/allowed-emails", get(list_emails).post(add_email))
        .route("/allowed-emails/{email}", delete(remove_email))
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

fn require_admin(user: &User) -> Result<(), ApiError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(ApiError::new("forbidden", "admin access required").with_status(403))
    }
}

fn require_top_admin(user: &User) -> Result<(), ApiError> {
    if user.is_top_admin() {
        Ok(())
    } else {
        Err(ApiError::new("forbidden", "top-admin access required").with_status(403))
    }
}

fn require_auth_store(state: &AppState) -> Result<&Arc<AuthStore>, ApiError> {
    state.auth_store.as_ref().ok_or_else(|| {
        ApiError::new("auth_not_configured", "authentication is not configured").with_status(501)
    })
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

async fn purge_user_namespace(state: &AppState, email: &str) -> Result<usize, ApiError> {
    if let Some(ref tree_root) = state.tree_root {
        let disk = tree_root.join(email);
        if disk.exists() {
            std::fs::remove_dir_all(&disk).map_err(|e| {
                ApiError::new("namespace_delete_failed", e.to_string()).with_status(500)
            })?;
        }
    }

    let removed = {
        let mut exoms = state.exoms.lock().unwrap();
        let before = exoms.len();
        exoms.retain(|path, _| !path_matches_prefix(path, email));
        let removed = before.saturating_sub(exoms.len());
        crate::server::reconcile_engine(state, &exoms);
        removed
    };

    let _ = state
        .sse_tx
        .send((None, r#"{"v":1,"kind":"tree","op":"changed"}"#.to_string()));

    Ok(removed)
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

#[derive(Deserialize)]
struct AddEmailRequest {
    email: String,
    #[serde(default)]
    alias: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /auth/admin/users
async fn list_users(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    let mut users: Vec<serde_json::Value> = Vec::new();
    for u in store.list_users().await {
        let role = store.resolve_role(&u.email).await;
        users.push(serde_json::json!({
            "email": u.email,
            "display_name": u.display_name,
            "provider": u.provider,
            "created_at": u.created_at,
            "active": u.active,
            "status": if u.active { "active" } else { "deactivated" },
            "last_login": u.last_login,
            "role": match role {
                UserRole::TopAdmin => "top-admin",
                UserRole::Admin => "admin",
                UserRole::Regular => "regular",
            },
        }));
    }
    Ok(Json(serde_json::json!({ "users": users })))
}

/// POST /auth/admin/users/:email/deactivate
async fn deactivate_user(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    if email == user.email {
        return Err(
            ApiError::new("forbidden", "top-admin cannot deactivate themselves").with_status(403),
        );
    }
    let store = require_auth_store(&state)?;
    store.deactivate_user(&email).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// POST /auth/admin/users/:email/activate
async fn activate_user(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    let store = require_auth_store(&state)?;
    store.activate_user(&email).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// DELETE /auth/admin/users/:email
async fn delete_user_account(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    if email == user.email {
        return Err(
            ApiError::new("forbidden", "top-admin cannot delete themselves").with_status(403),
        );
    }
    let store = require_auth_store(&state)?;
    if !store.delete_user(&email).await {
        return Err(ApiError::new("not_found", "user not found").with_status(404));
    }
    let removed_exoms = purge_user_namespace(&state, &email).await?;
    Ok(Json(
        serde_json::json!({ "ok": true, "removed_exoms": removed_exoms }),
    ))
}

/// POST /auth/admin/admins
async fn grant_admin(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<GrantAdminRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    let store = require_auth_store(&state)?;
    store.grant_admin(&body.email).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// DELETE /auth/admin/admins/:email
async fn revoke_admin(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    let store = require_auth_store(&state)?;
    store.revoke_admin(&email).await;
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
        .list_sessions()
        .await
        .into_iter()
        .map(|session| {
            serde_json::json!({
                "session_id": session.session_id,
                "email": session.email,
                "created_at": session.created_at,
                "expires_at": session.expires_at,
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
    store.delete_session(&id).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /auth/admin/api-keys
async fn list_all_api_keys(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    let keys: Vec<serde_json::Value> = store
        .list_api_keys()
        .await
        .iter()
        .map(|k| {
            serde_json::json!({
                "key_id": k.key_id,
                "email": k.email,
                "label": k.label,
                "created_at": k.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "keys": keys })))
}

/// DELETE /auth/admin/api-keys/:key_id
async fn revoke_any_api_key(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(key_id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    store.revoke_api_key_by_id(&key_id).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /auth/admin/shares
async fn list_all_shares(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    let shares: Vec<serde_json::Value> = store
        .list_all_shares()
        .await
        .iter()
        .map(|g| {
            serde_json::json!({
                "share_id": g.share_id,
                "owner_email": g.owner_email,
                "path": g.path,
                "grantee_email": g.grantee_email,
                "permission": g.permission,
                "created_at": g.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "shares": shares })))
}

/// GET /auth/admin/allowed-domains
async fn list_domains(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    let domains = store.list_allowed_domains().await;
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

    store.add_domain(&domain).await;

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

    store.remove_domain(&domain).await;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /auth/admin/allowed-emails
async fn list_emails(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    let store = require_auth_store(&state)?;
    let emails: Vec<serde_json::Value> = store
        .list_allowed_emails()
        .await
        .into_iter()
        .map(|e| serde_json::json!({ "email": e.email, "alias": e.alias }))
        .collect();
    Ok(Json(serde_json::json!({ "emails": emails })))
}

/// POST /auth/admin/allowed-emails
async fn add_email(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<AddEmailRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    let store = require_auth_store(&state)?;

    let email = body.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(ApiError::new("invalid_email", "email must not be empty").with_status(400));
    }
    if !email.contains('@') {
        return Err(ApiError::new("invalid_email", "email must contain '@'").with_status(400));
    }
    let alias = body.alias.trim().to_string();

    store.add_allowed_email(&email, &alias).await;

    Ok(Json(
        serde_json::json!({ "ok": true, "email": email, "alias": alias }),
    ))
}

/// DELETE /auth/admin/allowed-emails/:email
async fn remove_email(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(email): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_top_admin(&user)?;
    let store = require_auth_store(&state)?;

    store.remove_allowed_email(&email).await;

    Ok(Json(serde_json::json!({ "ok": true })))
}
