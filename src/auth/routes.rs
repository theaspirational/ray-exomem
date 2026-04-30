//! Auth route handlers: login, logout, me, api-keys, shares.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Path as AxumPath, State},
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
        .route("/dev-login", get(dev_login))
        .route("/logout", post(logout))
        .route("/session", get(session))
        .route("/me", get(me))
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route(
            "/api-keys/{key_id}",
            delete(revoke_api_key).patch(rename_api_key),
        )
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

#[derive(Deserialize)]
struct RenameApiKeyRequest {
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

// ---------------------------------------------------------------------------
// First-login bootstrap: seed the public tree from committed Notion fixtures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct BootstrapFactSpec {
    fact_id: String,
    predicate: String,
    value: crate::fact_value::FactValue,
    confidence: f64,
    provenance: String,
    valid_from: String,
    #[serde(default)]
    valid_to: Option<String>,
    tx_time: String,
    #[serde(default = "default_branch")]
    branch: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BootstrapObservationSpec {
    obs_id: String,
    source_type: String,
    source_ref: String,
    content: String,
    confidence: f64,
    #[serde(default)]
    tags: Vec<String>,
    valid_from: String,
    #[serde(default)]
    valid_to: Option<String>,
    tx_time: String,
    #[serde(default = "default_branch")]
    branch: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BootstrapBeliefSpec {
    belief_id: String,
    claim_text: String,
    status: crate::brain::BeliefStatus,
    confidence: f64,
    #[serde(default)]
    supported_by: Vec<String>,
    rationale: String,
    valid_from: String,
    #[serde(default)]
    valid_to: Option<String>,
    tx_time: String,
    #[serde(default = "default_branch")]
    branch: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BootstrapBranchSpec {
    branch_id: String,
    name: String,
    #[serde(default = "default_branch")]
    parent_branch_id: String,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    claimed_by: Option<String>,
    tx_time: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BootstrapRuleSpec {
    text: String,
    defined_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct BootstrapSeed {
    /// Tree path the seed targets, e.g. `public/work/team/project/concepts`.
    /// The seed is materialized at `<path>/main`. Required.
    path: String,
    #[serde(default)]
    branches: Vec<BootstrapBranchSpec>,
    #[serde(default)]
    facts: Vec<BootstrapFactSpec>,
    #[serde(default)]
    observations: Vec<BootstrapObservationSpec>,
    #[serde(default)]
    beliefs: Vec<BootstrapBeliefSpec>,
    #[serde(default)]
    rules: Vec<BootstrapRuleSpec>,
}

fn default_branch() -> String {
    "main".to_string()
}

struct SeedBuilder<'a> {
    email: &'a str,
    transactions: Vec<crate::brain::Tx>,
    facts: Vec<crate::brain::Fact>,
    observations: Vec<crate::brain::Observation>,
    beliefs: Vec<crate::brain::Belief>,
    branches: Vec<crate::brain::Branch>,
    next_tx: crate::brain::TxId,
}

impl<'a> SeedBuilder<'a> {
    fn new(email: &'a str) -> Self {
        Self {
            email,
            transactions: Vec::new(),
            facts: Vec::new(),
            observations: Vec::new(),
            beliefs: Vec::new(),
            branches: vec![crate::brain::Branch {
                branch_id: "main".to_string(),
                name: "main".to_string(),
                parent_branch_id: None,
                created_tx_id: 0,
                archived: false,
                claimed_by_user_email: Some(email.to_string()),
                claimed_by_agent: None,
                claimed_by_model: None,
            }],
            next_tx: 1,
        }
    }

    fn push_tx(
        &mut self,
        action: crate::brain::TxAction,
        refs: Vec<String>,
        note: String,
        tx_time: &str,
        branch: &str,
    ) -> crate::brain::TxId {
        let tx_id = self.next_tx;
        self.next_tx += 1;
        let parent_tx_id = self.transactions.last().map(|tx| tx.tx_id);
        self.transactions.push(crate::brain::Tx {
            tx_id,
            tx_time: tx_time.to_string(),
            user_email: Some(self.email.to_string()),
            agent: None,
            model: None,
            action,
            refs,
            note,
            parent_tx_id,
            branch_id: branch.to_string(),
            session: None,
        });
        tx_id
    }

    fn add_branch(&mut self, spec: BootstrapBranchSpec) {
        let tx_id = self.push_tx(
            crate::brain::TxAction::CreateBranch,
            vec![spec.branch_id.clone()],
            format!("branch: {}", spec.name),
            &spec.tx_time,
            &spec.parent_branch_id,
        );
        self.branches.push(crate::brain::Branch {
            branch_id: spec.branch_id,
            name: spec.name,
            parent_branch_id: Some(spec.parent_branch_id),
            created_tx_id: tx_id,
            archived: spec.archived,
            claimed_by_user_email: spec.claimed_by,
            claimed_by_agent: None,
            claimed_by_model: None,
        });
    }

    fn add_fact(&mut self, spec: BootstrapFactSpec) {
        let tx_id = self.push_tx(
            crate::brain::TxAction::AssertFact,
            vec![spec.fact_id.clone()],
            format!("assert: {} = {}", spec.predicate, spec.value),
            &spec.tx_time,
            &spec.branch,
        );
        self.facts.push(crate::brain::Fact {
            fact_id: spec.fact_id,
            predicate: spec.predicate,
            value: spec.value,
            created_at: spec.tx_time,
            created_by_tx: tx_id,
            superseded_by_tx: None,
            revoked_by_tx: None,
            confidence: spec.confidence,
            provenance: spec.provenance,
            valid_from: spec.valid_from,
            valid_to: spec.valid_to,
        });
    }

    fn add_observation(&mut self, spec: BootstrapObservationSpec) {
        let tx_id = self.push_tx(
            crate::brain::TxAction::AssertObservation,
            vec![spec.obs_id.clone()],
            format!("observe: {}", spec.obs_id),
            &spec.tx_time,
            &spec.branch,
        );
        self.observations.push(crate::brain::Observation {
            obs_id: spec.obs_id,
            source_type: spec.source_type,
            source_ref: spec.source_ref,
            content: spec.content,
            created_at: spec.tx_time,
            confidence: spec.confidence,
            tx_id,
            tags: spec.tags,
            valid_from: spec.valid_from,
            valid_to: spec.valid_to,
        });
    }

    fn add_belief(&mut self, spec: BootstrapBeliefSpec) {
        let tx_id = self.push_tx(
            crate::brain::TxAction::ReviseBelief,
            vec![spec.belief_id.clone()],
            format!("revise: {}", spec.claim_text),
            &spec.tx_time,
            &spec.branch,
        );
        self.beliefs.push(crate::brain::Belief {
            belief_id: spec.belief_id,
            claim_text: spec.claim_text,
            status: spec.status,
            confidence: spec.confidence,
            supported_by: spec.supported_by,
            created_by_tx: tx_id,
            valid_from: spec.valid_from,
            valid_to: spec.valid_to,
            rationale: spec.rationale,
        });
    }

    fn mark_fact_revisions(&mut self) {
        use std::collections::HashMap;

        let tx_time_by_id: HashMap<crate::brain::TxId, String> = self
            .transactions
            .iter()
            .map(|tx| (tx.tx_id, tx.tx_time.clone()))
            .collect();
        let mut by_fact: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, fact) in self.facts.iter().enumerate() {
            by_fact.entry(fact.fact_id.clone()).or_default().push(idx);
        }
        for indexes in by_fact.values_mut() {
            indexes.sort_by_key(|idx| self.facts[*idx].created_by_tx);
            for pair in indexes.windows(2) {
                let older = pair[0];
                let newer = pair[1];
                let newer_tx = self.facts[newer].created_by_tx;
                self.facts[older].superseded_by_tx = Some(newer_tx);
                if self.facts[older].valid_to.is_none() {
                    self.facts[older].valid_to = tx_time_by_id.get(&newer_tx).cloned();
                }
            }
        }
    }

    fn finish(
        mut self,
    ) -> (
        Vec<crate::brain::Fact>,
        Vec<crate::brain::Tx>,
        Vec<crate::brain::Observation>,
        Vec<crate::brain::Belief>,
        Vec<crate::brain::Branch>,
    ) {
        self.mark_fact_revisions();
        (
            self.facts,
            self.transactions,
            self.observations,
            self.beliefs,
            self.branches,
        )
    }
}

fn exom_is_bootstrapped(es: &crate::server::ExomState) -> bool {
    !es.brain.all_facts().is_empty()
        || !es.brain.observations().is_empty()
        || !es.brain.all_beliefs().is_empty()
        || !es.rules.is_empty()
}

async fn seed_bootstrap_exom(
    state: &AppState,
    exom: &str,
    email: &str,
    seed: BootstrapSeed,
) -> Result<(), ApiError> {
    crate::server::mutate_exom_async(state, exom, move |es| {
        if exom_is_bootstrapped(es) {
            return Ok(());
        }

        let mut builder = SeedBuilder::new(email);
        for branch in seed.branches {
            builder.add_branch(branch);
        }
        for fact in seed.facts {
            builder.add_fact(fact);
        }
        for observation in seed.observations {
            builder.add_observation(observation);
        }
        for belief in seed.beliefs {
            builder.add_belief(belief);
        }
        let (facts, transactions, observations, beliefs, branches) = builder.finish();
        es.brain
            .replace_state(facts, transactions, observations, beliefs, branches)?;

        es.rules.clear();
        for rule_spec in seed.rules {
            es.rules.push(crate::rules::parse_rule_line(
                &rule_spec.text,
                MutationContext {
                    user_email: Some(email.to_string()),
                    agent: None,
                    model: None,
                    session: None,
                },
                rule_spec.defined_at,
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

// Drop-in seed fixtures discovered by `build.rs` from `bootstrap/*.json`.
// Each entry is `(filename, file_contents)`. The list is empty when no
// seed files are present, which is a valid deployment.
include!(concat!(env!("OUT_DIR"), "/bootstrap_seeds.rs"));

/// Tree paths (slash form) of every exom seeded by the embedded bootstrap
/// fixtures. Returns one entry per fixture, suffixed with `/main`. The
/// welcome summary endpoint uses this to know which exoms to inspect for
/// featured entities and seed activity.
pub(crate) fn bootstrap_seed_exom_paths() -> Vec<String> {
    BOOTSTRAP_SEED_FILES
        .iter()
        .filter_map(|(_, contents)| {
            serde_json::from_str::<BootstrapSeed>(contents)
                .ok()
                .map(|s| format!("{}/main", s.path))
        })
        .collect()
}

fn parse_seed(label: &str, json: &str) -> Result<BootstrapSeed, ApiError> {
    serde_json::from_str(json).map_err(|e| {
        ApiError::new(
            "bootstrap_fixture_parse",
            format!("failed to parse {label} seed fixture: {e}"),
        )
        .with_status(500)
    })
}

/// Idempotently scaffolds and seeds tree paths declared in
/// `bootstrap/*.json`. Each fixture file embeds its own `path` (e.g.
/// `public/work/team/project/concepts`) and is materialized at
/// `<path>/main`. Subsequent logins are no-ops because
/// `seed_bootstrap_exom` checks `exom_is_bootstrapped` per exom.
async fn bootstrap_public_tree(state: &AppState, actor_email: &str) -> Result<(), ApiError> {
    if state.auth_store.is_none() {
        return Ok(());
    }
    let Some(tree_root) = state.tree_root.as_ref() else {
        return Ok(());
    };

    let mut changed = false;
    for (filename, contents) in BOOTSTRAP_SEED_FILES {
        let seed = parse_seed(filename, contents)?;
        let project_path: crate::path::TreePath = seed
            .path
            .parse()
            .map_err(|e: crate::path::PathError| {
                ApiError::new(
                    "bad_path",
                    format!("bootstrap fixture {filename} declares invalid path: {e}"),
                )
            })?;

        let main_path = project_path
            .join("main")
            .map_err(|e| ApiError::new("bad_path", e.to_string()))?;
        let main_disk = main_path.to_disk_path(tree_root);
        if crate::tree::classify(&main_disk) == crate::tree::NodeKind::Missing {
            changed = true;
        }
        crate::scaffold::init_project(tree_root, &project_path).map_err(ApiError::from)?;

        let exom_path = format!("{}/main", seed.path);
        seed_bootstrap_exom(state, &exom_path, actor_email, seed).await?;
    }

    if changed {
        let _ = state.sse_tx.send((
            None,
            r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string(),
        ));
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

    if let Some(ref name) = body.provider {
        if name != provider.provider_name() {
            return Err(
                ApiError::new(
                    "provider_mismatch",
                    format!(
                        "login body requested provider {name:?} but server is configured for {}",
                        provider.provider_name()
                    ),
                )
                .with_status(400),
            );
        }
    }

    // Validate the token.
    let identity = provider.validate_token(&body.id_token).await.map_err(|e| {
        ApiError::new("invalid_token", format!("token validation failed: {e}")).with_status(401)
    })?;

    // Check sign-up allowlist (domain wildcard or individual email).
    if !store.is_login_allowed(&identity.email).await {
        return Err(
            ApiError::new("login_not_allowed", "your email is not allowed to sign in")
                .with_status(403)
                .with_suggestion(
                    "contact an administrator to add your domain or email to the allowlist",
                ),
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
        api_key_label: None,
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
    bootstrap_public_tree(&state, &identity.email).await?;

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

/// GET /auth/dev-login
///
/// Loopback-only OAuth bypass for local UI development. Mints a session for
/// the email configured via `--dev-login-email` (or `RAY_EXOMEM_DEV_LOGIN_EMAIL`)
/// at daemon startup. Returns 404 when the flag is unset, 403 when the request
/// peer is not a loopback address. Mirrors `login()`'s session creation,
/// bootstrap seeding, and top-admin promotion so a dev session is
/// indistinguishable from a real Google login at the cookie layer.
async fn dev_login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, ApiError> {
    let email = state.dev_login_email.as_ref().ok_or_else(|| {
        ApiError::new("not_found", "dev-login is not enabled").with_status(404)
    })?;
    if !addr.ip().is_loopback() {
        return Err(
            ApiError::new("forbidden", "dev-login is loopback-only").with_status(403),
        );
    }
    let store = require_auth_store(&state)?;

    let role = store.login_role(email).await.map_err(|e| {
        ApiError::new(
            "auth_state_unavailable",
            format!("failed to resolve login role: {e}"),
        )
        .with_status(500)
    })?;

    if let Some(existing) = store.get_user_record(email).await {
        if !existing.active {
            return Err(
                ApiError::new("user_deactivated", "this account has been deactivated")
                    .with_status(403),
            );
        }
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    let display_name = email.split('@').next().unwrap_or(email).to_string();
    let provider = "dev-login".to_string();

    let user = User {
        email: email.clone(),
        display_name: display_name.clone(),
        provider: provider.clone(),
        session_id: Some(session_id.clone()),
        api_key_label: None,
        role: role.clone(),
    };

    store.session_cache.insert(session_id.clone(), user);
    store.record_user(email, &display_name, &provider).await;
    store.record_session(&session_id, email, &expires_at).await;
    bootstrap_public_tree(&state, email).await?;

    if role == UserRole::TopAdmin {
        store.set_top_admin(email).await;
    }

    let secure = state
        .bind_addr
        .as_deref()
        .map(|b| !b.starts_with("127.0.0.1") && !b.starts_with("localhost"))
        .unwrap_or(false);
    let cookie = session_cookie(&session_id, 30, secure);

    let redirect_to = if crate::server::BASE_PATH.is_empty() {
        "/".to_string()
    } else {
        format!("{}/", crate::server::BASE_PATH)
    };

    Ok((
        axum::http::StatusCode::SEE_OTHER,
        [
            (axum::http::header::SET_COOKIE, cookie),
            (axum::http::header::LOCATION, redirect_to),
        ],
        "",
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
        api_key_label: Some(body.label.clone()),
        ..user.clone()
    };
    store.api_key_cache.insert(key_hash, api_user);

    let bind = state.bind_addr.as_deref().unwrap_or("127.0.0.1:9780");
    let mcp_snippet = serde_json::json!({
        "mcpServers": {
            "ray-exomem": {
                "url": format!("http://{bind}{}/mcp", crate::server::BASE_PATH),
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

/// PATCH /auth/api-keys/:key_id
async fn rename_api_key(
    State(state): State<Arc<AppState>>,
    user: User,
    AxumPath(key_id): AxumPath<String>,
    Json(body): Json<RenameApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;

    let label = body.label.trim().to_string();
    if label.is_empty() {
        return Err(ApiError::new("invalid_label", "label must not be empty").with_status(400));
    }

    if !user.is_admin() {
        let keys = store.list_api_keys_for_user(&user.email).await;
        if !keys.iter().any(|k| k.key_id == key_id) {
            return Err(ApiError::new("not_found", "API key not found").with_status(404));
        }
    }

    if !store.rename_api_key_by_id(&key_id, &label).await {
        return Err(ApiError::new("not_found", "API key not found").with_status(404));
    }
    Ok(Json(serde_json::json!({ "ok": true, "label": label })))
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
    // The Admin/TopAdmin role does not grant the ability to mint share
    // grants on namespaces they do not own — a share is owner consent,
    // and operator status doesn't substitute for it.
    if body.path != user.email && !body.path.starts_with(&format!("{}/", user.email)) {
        return Err(
            ApiError::new("not_owner", "you can only share paths you own").with_status(403),
        );
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

#[cfg(test)]
mod bootstrap_fixture_tests {
    use super::*;

    #[test]
    fn discovered_fixtures_parse() {
        for (name, contents) in BOOTSTRAP_SEED_FILES {
            let _: BootstrapSeed = serde_json::from_str(contents)
                .unwrap_or_else(|e| panic!("{name} must deserialize into BootstrapSeed: {e}"));
        }
    }

    #[test]
    fn discovered_fixture_rules_reference_their_declared_path() {
        // Rule heads reference the exom path; if a fixture's `path` doesn't
        // match its rule heads, bootstrap registers rules against the wrong
        // exom and firings get silently misrouted.
        for (name, contents) in BOOTSTRAP_SEED_FILES {
            let seed: BootstrapSeed = serde_json::from_str(contents).unwrap();
            for rule in &seed.rules {
                assert!(
                    rule.text.contains(&seed.path),
                    "{name}: rule does not reference declared path {}: {}",
                    seed.path,
                    rule.text
                );
            }
        }
    }
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
