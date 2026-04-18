//! Auth route handlers: login, logout, me, api-keys, shares.

use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, State},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth::middleware::{clear_session_cookie, session_cookie, MaybeUser};
use crate::auth::store::AuthStore;
use crate::auth::{User, UserRole};
use crate::context::MutationContext;
use crate::http_error::ApiError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn auth_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/info", get(auth_info))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/session", get(session))
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
struct AuthUserResponse {
    email: String,
    display_name: String,
    provider: String,
    role: String,
}

#[derive(Serialize)]
struct SessionResponse {
    authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<AuthUserResponse>,
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
        ApiError::new("auth_not_configured", "authentication is not configured").with_status(501)
    })
}

fn require_auth_provider(
    state: &AppState,
) -> Result<&Arc<dyn crate::auth::provider::AuthProvider>, ApiError> {
    state.auth_provider.as_ref().ok_or_else(|| {
        ApiError::new(
            "auth_not_configured",
            "authentication provider is not configured",
        )
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

fn auth_user_response(user: &User) -> AuthUserResponse {
    AuthUserResponse {
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        provider: user.provider.clone(),
        role: role_label(&user.role).to_string(),
    }
}

const BOOTSTRAP_SENTINEL_PREDICATE: &str = "onboarding/bootstrap_version";
const BOOTSTRAP_SENTINEL_VALUE: &str = "v1";

/// Literal value embedded in bootstrap specs. Stored in a static table so we
/// pre-type numeric profile fields as `FactValue::I64` at definition time —
/// this is what makes datalog `cmp` rules (e.g. `(< ?w 60)`) work against
/// the seeded health-profile facts without string parsing.
#[derive(Clone, Copy)]
enum BootstrapLiteral {
    I64(i64),
    Str(&'static str),
}

impl BootstrapLiteral {
    fn as_fact_value(self) -> crate::fact_value::FactValue {
        match self {
            BootstrapLiteral::I64(n) => crate::fact_value::FactValue::I64(n),
            BootstrapLiteral::Str(s) => crate::fact_value::FactValue::Str(s.to_string()),
        }
    }
}

type BootstrapFactSpec = (&'static str, &'static str, BootstrapLiteral);

fn bootstrap_ctx(email: &str, session_id: &str) -> MutationContext {
    MutationContext {
        actor: email.to_string(),
        session: Some(session_id.to_string()),
        model: None,
        user_email: Some(email.to_string()),
    }
}

fn health_bootstrap_facts() -> &'static [BootstrapFactSpec] {
    &[
        ("health/profile/age", "profile/age", BootstrapLiteral::I64(30)),
        (
            "health/profile/height_cm",
            "profile/height_cm",
            BootstrapLiteral::I64(175),
        ),
        (
            "health/profile/weight_kg",
            "profile/weight_kg",
            BootstrapLiteral::I64(75),
        ),
        (
            "health/profile/units",
            "profile/units",
            BootstrapLiteral::Str("metric"),
        ),
        (
            "health/onboarding/disclaimer",
            "onboarding/disclaimer",
            BootstrapLiteral::Str("general_wellness_example_not_medical_advice"),
        ),
        (
            "onboarding/bootstrap_version",
            BOOTSTRAP_SENTINEL_PREDICATE,
            BootstrapLiteral::Str(BOOTSTRAP_SENTINEL_VALUE),
        ),
    ]
}

fn work_main_bootstrap_facts() -> &'static [BootstrapFactSpec] {
    &[
        (
            "workspace/purpose",
            "workspace/purpose",
            BootstrapLiteral::Str("personal work area"),
        ),
        (
            "workspace/next_step",
            "workspace/next_step",
            BootstrapLiteral::Str("create projects, facts, or sessions here"),
        ),
        (
            "onboarding/bootstrap_version",
            BOOTSTRAP_SENTINEL_PREDICATE,
            BootstrapLiteral::Str(BOOTSTRAP_SENTINEL_VALUE),
        ),
    ]
}

fn work_example_bootstrap_facts() -> &'static [BootstrapFactSpec] {
    &[
        (
            "project/name",
            "project/name",
            BootstrapLiteral::Str("Example Project"),
        ),
        (
            "project/status",
            "project/status",
            BootstrapLiteral::Str("active"),
        ),
        (
            "project/next_step",
            "project/next_step",
            BootstrapLiteral::Str("inspect facts, graph, and sessions"),
        ),
        (
            "onboarding/bootstrap_version",
            BOOTSTRAP_SENTINEL_PREDICATE,
            BootstrapLiteral::Str(BOOTSTRAP_SENTINEL_VALUE),
        ),
    ]
}

fn health_bootstrap_rules(_exom: &str) -> Vec<String> {
    // Bootstrap rule set intentionally left empty after Task T2.
    //
    // History:
    //   * Before T2 the onboarding seeded six `(rule {exom}
    //     (health/recommended-water-ml "X") (health/water-band "Y"))`
    //     style rules. They tripped rayforce2's rule parser (string
    //     head constants are not accepted) and the ontology emission
    //     layer as soon as the UI tried to resolve them.
    //
    //   * T2 introduced the per-type splay tables `facts_i64`,
    //     `facts_str`, `facts_sym` so live numeric cmp (`(< ?w 60)`)
    //     works inside Datalog rules. The declarative water-band /
    //     step-band derivations are expressible as Datalog rules over
    //     `facts_i64` — but the only compatible rule shape is a
    //     VARIABLE-head rule joined against a seeded auxiliary EDB
    //     (see `tests/typed_facts_e2e.rs::water_band_rules`). Shipping
    //     such an EDB as part of bootstrap requires binding it in the
    //     shared env per-query (the way `facts_i64` is bound), plus a
    //     per-exom backing table. That plumbing is out of scope for
    //     this commit.
    //
    //   * Constant-head rules (e.g. `(health/water-band 'medium) ...`)
    //     remain BROKEN upstream: rayforce2's `dl_project` drops the
    //     constant column entirely, and the surrounding heap-reuse
    //     code can surface as memory corruption on the next IDB
    //     materialization. Tracked separately.
    //
    // Until the auxiliary-table binding lands, we ship the health exom
    // with bootstrap FACTS only (see `health_bootstrap_facts`) and no
    // derived rules. Users can add their own rules via `/api/actions/
    // eval` once they understand the constraints above.
    Vec::new()
}

fn exom_is_bootstrapped(es: &crate::server::ExomState) -> bool {
    es.brain.current_facts().iter().any(|fact| {
        fact.predicate == BOOTSTRAP_SENTINEL_PREDICATE && fact.value == BOOTSTRAP_SENTINEL_VALUE
    }) || !es.brain.all_facts().is_empty()
        || !es.rules.is_empty()
}

async fn seed_bootstrap_exom(
    state: &AppState,
    exom: &str,
    facts: &[BootstrapFactSpec],
    rules: &[String],
    ctx: &MutationContext,
) -> Result<(), ApiError> {
    crate::server::mutate_exom_async(state, exom, |es| {
        if exom_is_bootstrapped(es) {
            return Ok(());
        }

        for (fact_id, predicate, literal) in facts {
            es.brain.assert_fact(
                fact_id,
                predicate,
                literal.as_fact_value(),
                1.0,
                "bootstrap",
                None,
                None,
                ctx,
            )?;
        }

        for rule_text in rules {
            es.rules.push(crate::rules::parse_rule_line(
                rule_text,
                ctx.clone(),
                crate::brain::now_iso(),
            )?);
        }

        Ok(())
    })
    .await
    .map_err(|e| {
        ApiError::new(
            "bootstrap_failed",
            format!("failed to bootstrap {exom}: {e}"),
        )
        .with_status(500)
    })?;
    Ok(())
}

async fn bootstrap_user_namespace(
    state: &AppState,
    email: &str,
    session_id: &str,
) -> Result<(), ApiError> {
    if state.auth_store.is_none() {
        return Ok(());
    }

    let Some(tree_root) = state.tree_root.as_ref() else {
        return Ok(());
    };

    let project_paths = [
        format!("{email}/personal/health"),
        format!("{email}/work"),
        format!("{email}/work/example"),
    ];

    let mut changed = false;
    for raw in project_paths {
        let path: crate::path::TreePath = raw
            .parse()
            .map_err(|e: crate::path::PathError| ApiError::new("bad_path", e.to_string()))?;
        let main_path = path
            .join("main")
            .map_err(|e| ApiError::new("bad_path", e.to_string()))?;
        let main_disk = main_path.to_disk_path(tree_root);
        if crate::tree::classify(&main_disk) == crate::tree::NodeKind::Missing {
            changed = true;
        }
        crate::scaffold::init_project(tree_root, &path).map_err(ApiError::from)?;
    }

    let ctx = bootstrap_ctx(email, session_id);
    let health_exom = format!("{email}/personal/health/main");
    seed_bootstrap_exom(
        state,
        &health_exom,
        health_bootstrap_facts(),
        &health_bootstrap_rules(&health_exom),
        &ctx,
    )
    .await?;

    let work_main_exom = format!("{email}/work/main");
    seed_bootstrap_exom(
        state,
        &work_main_exom,
        work_main_bootstrap_facts(),
        &[],
        &ctx,
    )
    .await?;

    let work_example_exom = format!("{email}/work/example/main");
    seed_bootstrap_exom(
        state,
        &work_example_exom,
        work_example_bootstrap_facts(),
        &[],
        &ctx,
    )
    .await?;

    if changed {
        let _ = state
            .sse_tx
            .send(r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /auth/info
///
/// Public (no session required). Returns auth provider info so the login page
/// knows which providers are available and can initialize GSI.
async fn auth_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (provider, google_client_id) = match &state.auth_provider {
        Some(p) => {
            let name = p.provider_name().to_string();
            let cid = p.client_id().map(|s| s.to_string());
            (Some(name), cid)
        }
        None => (None, None),
    };
    Json(serde_json::json!({
        "provider": provider,
        "google_client_id": google_client_id,
    }))
}

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
    let identity = provider.validate_token(&body.id_token).await.map_err(|e| {
        ApiError::new("invalid_token", format!("token validation failed: {e}")).with_status(401)
    })?;

    // Check domain restriction.
    if !store.check_domain(&identity.email).await {
        return Err(
            ApiError::new("domain_not_allowed", "your email domain is not allowed")
                .with_status(403)
                .with_suggestion("contact an administrator to add your domain"),
        );
    }

    // Resolve role from persisted auth state so a fresh process cannot
    // accidentally bootstrap a second top-admin.
    let role = store.login_role(&identity.email).await.map_err(|e| {
        ApiError::new(
            "auth_state_unavailable",
            format!("failed to resolve login role: {e}"),
        )
        .with_status(500)
    })?;

    if let Some(existing) = store.get_user_record(&identity.email).await {
        if !existing.active {
            return Err(
                ApiError::new("user_deactivated", "this account has been deactivated")
                    .with_status(403),
            );
        }
    }

    // Create session.
    let session_id = uuid::Uuid::new_v4().to_string();
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();

    let user = User {
        email: identity.email.clone(),
        display_name: identity.display_name.clone(),
        provider: identity.provider.clone(),
        session_id: Some(session_id.clone()),
        role: role.clone(),
    };

    // Cache the session.
    store.session_cache.insert(session_id.clone(), user.clone());

    // Persist user record.
    store
        .record_user(&identity.email, &identity.display_name, &identity.provider)
        .await;
    store
        .record_session(&session_id, &identity.email, &expires_at)
        .await;
    bootstrap_user_namespace(&state, &identity.email, &session_id).await?;

    // First user ever becomes persisted top-admin.
    if role == UserRole::TopAdmin {
        store.set_top_admin(&identity.email).await;
    }

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
        store.delete_session(sid).await;
    }

    let cookie = clear_session_cookie();

    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    ))
}

/// GET /auth/session
///
/// Public session probe for the SPA bootstrap path. Returns `authenticated: false`
/// instead of a 401 so the app can redirect cleanly without logging expected auth
/// misses as network errors.
async fn session(maybe_user: MaybeUser) -> impl IntoResponse {
    let user = maybe_user.0.as_ref().map(auth_user_response);
    Json(SessionResponse {
        authenticated: user.is_some(),
        user,
    })
}

/// GET /auth/me
async fn me(user: User) -> impl IntoResponse {
    Json(auth_user_response(&user))
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

    // Persist the API key.
    store
        .record_api_key(&key_id, &key_hash, &user.email, &body.label)
        .await;

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
async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;
    let keys: Vec<serde_json::Value> = store
        .list_api_keys_for_user(&user.email)
        .await
        .iter()
        .map(|k| {
            serde_json::json!({
                "key_id": k.key_id,
                "label": k.label,
                "created_at": k.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "keys": keys })))
}

/// DELETE /auth/api-keys/:key_id
async fn revoke_api_key(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(key_id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;

    // Verify the key belongs to this user (unless admin).
    if !user.is_admin() {
        let keys = store.list_api_keys_for_user(&user.email).await;
        if !keys.iter().any(|k| k.key_id == key_id) {
            return Err(ApiError::new("not_found", "API key not found").with_status(404));
        }
    }

    store.revoke_api_key_by_id(&key_id).await;
    Ok(Json(serde_json::json!({ "ok": true })))
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
        return Err(ApiError::new(
            "invalid_permission",
            format!(
                "permission must be 'read' or 'read-write', got '{}'",
                body.permission
            ),
        )
        .with_status(400));
    }

    // Verify user owns the path (path must start with user's email).
    if body.path != user.email && !body.path.starts_with(&format!("{}/", user.email)) {
        if !user.is_admin() {
            return Err(
                ApiError::new("not_owner", "you can only share paths you own").with_status(403),
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

    store
        .add_share_grant(crate::auth::store::ShareGrant {
            share_id: share_id.clone(),
            owner_email: user.email.clone(),
            path: body.path.clone(),
            grantee_email: body.grantee_email.clone(),
            permission: body.permission.clone(),
            created_at,
        })
        .await;

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
async fn list_shares(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;
    let shares: Vec<serde_json::Value> = store
        .list_shares_for_owner(&user.email)
        .await
        .iter()
        .map(|g| {
            serde_json::json!({
                "share_id": g.share_id,
                "path": g.path,
                "grantee_email": g.grantee_email,
                "permission": g.permission,
                "created_at": g.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "shares": shares })))
}

/// DELETE /auth/shares/:share_id
async fn revoke_share(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(share_id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;

    // Verify the share belongs to this user (unless admin).
    if !user.is_admin() {
        let shares = store.list_shares_for_owner(&user.email).await;
        if !shares.iter().any(|s| s.share_id == share_id) {
            return Err(ApiError::new("not_found", "share not found").with_status(404));
        }
    }

    store.revoke_share_by_id(&share_id).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /auth/shared-with-me
async fn shared_with_me(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;
    let grants = store.shares_for_grantee(&user.email).await;
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
