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

/// Literal value embedded in bootstrap specs. Numeric fields are typed as
/// `FactValue::I64` so authored rules can use native `facts_i64` comparisons.
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

struct BootstrapFactSpec {
    fact_id: &'static str,
    predicate: &'static str,
    value: BootstrapLiteral,
    confidence: f64,
    provenance: &'static str,
    valid_from: &'static str,
    valid_to: Option<&'static str>,
    tx_time: &'static str,
    actor: Option<&'static str>,
    branch: &'static str,
}

struct BootstrapObservationSpec {
    obs_id: &'static str,
    source_type: &'static str,
    source_ref: &'static str,
    content: &'static str,
    confidence: f64,
    tags: &'static [&'static str],
    valid_from: &'static str,
    valid_to: Option<&'static str>,
    tx_time: &'static str,
    actor: Option<&'static str>,
    branch: &'static str,
}

struct BootstrapBeliefSpec {
    belief_id: &'static str,
    claim_text: &'static str,
    status: crate::brain::BeliefStatus,
    confidence: f64,
    supported_by: &'static [&'static str],
    rationale: &'static str,
    valid_from: &'static str,
    valid_to: Option<&'static str>,
    tx_time: &'static str,
    actor: Option<&'static str>,
    branch: &'static str,
}

struct BootstrapBranchSpec {
    branch_id: &'static str,
    name: &'static str,
    parent_branch_id: &'static str,
    archived: bool,
    claimed_by: Option<&'static str>,
    tx_time: &'static str,
    actor: Option<&'static str>,
}

struct BootstrapRuleSpec {
    text: String,
    defined_at: &'static str,
    actor: &'static str,
}

struct BootstrapSeed {
    facts: Vec<BootstrapFactSpec>,
    observations: Vec<BootstrapObservationSpec>,
    beliefs: Vec<BootstrapBeliefSpec>,
    branches: Vec<BootstrapBranchSpec>,
    rules: Vec<BootstrapRuleSpec>,
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
                claimed_by: Some(email.to_string()),
            }],
            next_tx: 1,
        }
    }

    fn actor(&self, explicit: Option<&str>) -> String {
        explicit.unwrap_or(self.email).to_string()
    }

    fn push_tx(
        &mut self,
        action: crate::brain::TxAction,
        refs: Vec<String>,
        note: String,
        tx_time: &str,
        actor: String,
        branch: &str,
    ) -> crate::brain::TxId {
        let tx_id = self.next_tx;
        self.next_tx += 1;
        let parent_tx_id = self.transactions.last().map(|tx| tx.tx_id);
        self.transactions.push(crate::brain::Tx {
            tx_id,
            tx_time: tx_time.to_string(),
            user_email: Some(self.email.to_string()),
            actor,
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
        let actor = self.actor(spec.actor);
        let tx_id = self.push_tx(
            crate::brain::TxAction::CreateBranch,
            vec![spec.branch_id.to_string()],
            format!("branch: {}", spec.name),
            spec.tx_time,
            actor,
            spec.parent_branch_id,
        );
        self.branches.push(crate::brain::Branch {
            branch_id: spec.branch_id.to_string(),
            name: spec.name.to_string(),
            parent_branch_id: Some(spec.parent_branch_id.to_string()),
            created_tx_id: tx_id,
            archived: spec.archived,
            claimed_by: spec.claimed_by.map(str::to_string),
        });
    }

    fn add_fact(&mut self, spec: BootstrapFactSpec) {
        let value = spec.value.as_fact_value();
        let actor = self.actor(spec.actor);
        let tx_id = self.push_tx(
            crate::brain::TxAction::AssertFact,
            vec![spec.fact_id.to_string()],
            format!("assert: {} = {}", spec.predicate, value),
            spec.tx_time,
            actor,
            spec.branch,
        );
        self.facts.push(crate::brain::Fact {
            fact_id: spec.fact_id.to_string(),
            predicate: spec.predicate.to_string(),
            value,
            created_at: spec.tx_time.to_string(),
            created_by_tx: tx_id,
            superseded_by_tx: None,
            revoked_by_tx: None,
            confidence: spec.confidence,
            provenance: spec.provenance.to_string(),
            valid_from: spec.valid_from.to_string(),
            valid_to: spec.valid_to.map(str::to_string),
        });
    }

    fn add_observation(&mut self, spec: BootstrapObservationSpec) {
        let actor = self.actor(spec.actor);
        let tx_id = self.push_tx(
            crate::brain::TxAction::AssertObservation,
            vec![spec.obs_id.to_string()],
            format!("observe: {}", spec.obs_id),
            spec.tx_time,
            actor,
            spec.branch,
        );
        self.observations.push(crate::brain::Observation {
            obs_id: spec.obs_id.to_string(),
            source_type: spec.source_type.to_string(),
            source_ref: spec.source_ref.to_string(),
            content: spec.content.to_string(),
            created_at: spec.tx_time.to_string(),
            confidence: spec.confidence,
            tx_id,
            tags: spec.tags.iter().map(|tag| tag.to_string()).collect(),
            valid_from: spec.valid_from.to_string(),
            valid_to: spec.valid_to.map(str::to_string),
        });
    }

    fn add_belief(&mut self, spec: BootstrapBeliefSpec) {
        let actor = self.actor(spec.actor);
        let tx_id = self.push_tx(
            crate::brain::TxAction::ReviseBelief,
            vec![spec.belief_id.to_string()],
            format!("revise: {}", spec.claim_text),
            spec.tx_time,
            actor,
            spec.branch,
        );
        self.beliefs.push(crate::brain::Belief {
            belief_id: spec.belief_id.to_string(),
            claim_text: spec.claim_text.to_string(),
            status: spec.status,
            confidence: spec.confidence,
            supported_by: spec.supported_by.iter().map(|id| id.to_string()).collect(),
            created_by_tx: tx_id,
            valid_from: spec.valid_from.to_string(),
            valid_to: spec.valid_to.map(str::to_string),
            rationale: spec.rationale.to_string(),
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

fn bf(
    fact_id: &'static str,
    predicate: &'static str,
    value: BootstrapLiteral,
    confidence: f64,
    provenance: &'static str,
    valid_from: &'static str,
    tx_time: &'static str,
) -> BootstrapFactSpec {
    BootstrapFactSpec {
        fact_id,
        predicate,
        value,
        confidence,
        provenance,
        valid_from,
        valid_to: None,
        tx_time,
        actor: None,
        branch: "main",
    }
}

fn bf_to(
    fact_id: &'static str,
    predicate: &'static str,
    value: BootstrapLiteral,
    confidence: f64,
    provenance: &'static str,
    valid_from: &'static str,
    valid_to: &'static str,
    tx_time: &'static str,
) -> BootstrapFactSpec {
    BootstrapFactSpec {
        valid_to: Some(valid_to),
        ..bf(
            fact_id, predicate, value, confidence, provenance, valid_from, tx_time,
        )
    }
}

fn branch_fact(
    branch: &'static str,
    fact_id: &'static str,
    predicate: &'static str,
    value: BootstrapLiteral,
    confidence: f64,
    provenance: &'static str,
    valid_from: &'static str,
    tx_time: &'static str,
) -> BootstrapFactSpec {
    BootstrapFactSpec {
        branch,
        ..bf(
            fact_id, predicate, value, confidence, provenance, valid_from, tx_time,
        )
    }
}

fn obs(
    obs_id: &'static str,
    source_type: &'static str,
    source_ref: &'static str,
    content: &'static str,
    confidence: f64,
    tags: &'static [&'static str],
    valid_from: &'static str,
    tx_time: &'static str,
) -> BootstrapObservationSpec {
    BootstrapObservationSpec {
        obs_id,
        source_type,
        source_ref,
        content,
        confidence,
        tags,
        valid_from,
        valid_to: None,
        tx_time,
        actor: None,
        branch: "main",
    }
}

fn belief(
    belief_id: &'static str,
    claim_text: &'static str,
    status: crate::brain::BeliefStatus,
    confidence: f64,
    supported_by: &'static [&'static str],
    rationale: &'static str,
    valid_from: &'static str,
    valid_to: Option<&'static str>,
    tx_time: &'static str,
) -> BootstrapBeliefSpec {
    BootstrapBeliefSpec {
        belief_id,
        claim_text,
        status,
        confidence,
        supported_by,
        rationale,
        valid_from,
        valid_to,
        tx_time,
        actor: None,
        branch: "main",
    }
}

fn branch(
    branch_id: &'static str,
    name: &'static str,
    archived: bool,
    claimed_by: Option<&'static str>,
    tx_time: &'static str,
) -> BootstrapBranchSpec {
    BootstrapBranchSpec {
        branch_id,
        name,
        parent_branch_id: "main",
        archived,
        claimed_by,
        tx_time,
        actor: None,
    }
}

fn rule(text: String, defined_at: &'static str) -> BootstrapRuleSpec {
    BootstrapRuleSpec {
        text,
        defined_at,
        actor: "rule-curator",
    }
}

fn dashboard_rules(exom: &str) -> Vec<BootstrapRuleSpec> {
    vec![
        rule(
            format!(
                r#"(rule {exom} (high_priority ?id) (facts_i64 ?id 'project/priority ?p) (>= ?p 8))"#
            ),
            "2026-01-16T10:20:00Z",
        ),
        rule(
            format!(
                r#"(rule {exom} (at_risk ?id) (facts_i64 ?id 'risk/score ?score) (>= ?score 7))"#
            ),
            "2026-01-29T15:40:00Z",
        ),
        rule(
            format!(
                r#"(rule {exom} (stale_open_question ?id) (facts_i64 ?id 'question/age_days ?days) (>= ?days 14))"#
            ),
            "2026-02-19T09:15:00Z",
        ),
        rule(
            format!(
                r#"(rule {exom} (decision_review_due ?id) (facts_i64 ?id 'decision/review_due_days ?days) (< ?days 14))"#
            ),
            "2026-03-08T12:10:00Z",
        ),
        rule(
            format!(
                r#"(rule {exom} (recent_incident ?id) (facts_i64 ?id 'incident/days_since ?days) (< ?days 30))"#
            ),
            "2026-03-31T17:05:00Z",
        ),
        rule(
            format!(
                r#"(rule {exom} (mature_memory ?id) (facts_i64 ?id 'memory/age_days ?days) (>= ?days 180))"#
            ),
            "2026-04-12T11:30:00Z",
        ),
    ]
}

fn dashboard_seed(exom: &str) -> BootstrapSeed {
    use crate::brain::BeliefStatus::{Active, Superseded};
    use BootstrapLiteral::{Str, I64};

    BootstrapSeed {
        branches: vec![
            branch(
                "candidate-graph-shape",
                "candidate graph shape",
                false,
                Some("graph-review"),
                "2026-04-17T13:20:00Z",
            ),
            branch(
                "incident-followup",
                "incident follow-up",
                false,
                Some("ops-review"),
                "2026-04-05T09:10:00Z",
            ),
            branch(
                "archived-import-notes",
                "archived import notes",
                true,
                Some("archive-curator"),
                "2025-11-03T16:45:00Z",
            ),
        ],
        facts: vec![
            bf("brain/home#type", "entity/type", Str("brain-dashboard"), 1.0, "source:system-index", "2025-09-02T08:30:00Z", "2025-09-02T08:30:00Z"),
            bf("brain/home#purpose", "brain/purpose", Str("cross-project memory index for decisions, evidence, rules, open questions, and operating constraints"), 0.98, "source:memory-charter", "2025-09-02T08:35:00Z", "2025-09-02T08:35:00Z"),
            bf("brain/home#memory-model", "memory/model", Str("facts are current claims; observations are evidence; beliefs are revised interpretations; rules derive working sets"), 0.97, "source:architecture-notes", "2025-09-03T10:15:00Z", "2025-09-03T10:15:00Z"),
            bf("brain/home#branch-policy", "branch/policy", Str("main is accepted memory; candidate branches hold alternatives until merged or archived"), 0.96, "source:branching-notes", "2025-10-12T14:00:00Z", "2025-10-12T14:00:00Z"),
            bf("brain/home#retention-policy", "memory/retention_policy", Str("keep decisions, incidents, constraints, commands, and evidence; expire raw scratch notes"), 0.94, "source:ops-runbook", "2025-11-01T09:00:00Z", "2025-11-01T09:00:00Z"),
            bf("brain/home#query-style", "query/preference", Str("prefer stable entity ids, explicit provenance, and valid-time intervals"), 0.96, "source:query-log", "2025-11-18T12:20:00Z", "2025-11-18T12:20:00Z"),
            bf("brain/home#default-surface", "interface/default_surface", Str("facts-branches-history-graph-rules"), 0.92, "source:ui-review", "2026-03-26T16:40:00Z", "2026-03-26T16:40:00Z"),
            bf("brain/home#first-login-contract", "constraint/value", Str("first-run state must teach exoms, facts, observations, beliefs, rules, provenance, history, and branches inside the native UI"), 0.99, "source:product-constraint", "2026-04-23T11:10:00Z", "2026-04-23T11:10:00Z"),
            bf("team/platform#type", "entity/type", Str("team"), 0.93, "source:org-map", "2025-09-05T09:00:00Z", "2025-09-05T09:00:00Z"),
            bf("team/platform#scope", "team/scope", Str("memory platform, native UI, rule engine, retrieval evaluation, and operations hygiene"), 0.91, "source:org-map", "2025-09-05T09:05:00Z", "2025-09-05T09:05:00Z"),
            bf("role/platform-owner#type", "entity/type", Str("role"), 0.91, "source:org-map", "2025-09-06T10:00:00Z", "2025-09-06T10:00:00Z"),
            bf("role/platform-owner#owns", "owns", Str("project/ray-exomem"), 0.9, "source:org-map", "2025-09-06T10:05:00Z", "2025-09-06T10:05:00Z"),
            bf("role/ops-reviewer#type", "entity/type", Str("role"), 0.88, "source:org-map", "2025-09-06T10:10:00Z", "2025-09-06T10:10:00Z"),
            bf("role/ops-reviewer#owns", "owns", Str("project/ops-runbooks"), 0.88, "source:org-map", "2025-09-06T10:15:00Z", "2025-09-06T10:15:00Z"),
            bf("project/ray-exomem#type", "entity/type", Str("project"), 0.99, "source:repo", "2025-09-10T08:00:00Z", "2025-09-10T08:00:00Z"),
            bf("project/ray-exomem#name", "entity/name", Str("ray-exomem"), 1.0, "source:repo", "2025-09-10T08:01:00Z", "2025-09-10T08:01:00Z"),
            bf("project/ray-exomem#status", "project/status", Str("active"), 0.98, "source:project-board", "2025-09-10T08:10:00Z", "2025-09-10T08:10:00Z"),
            bf("project/ray-exomem#area", "project/area", Str("memory-platform"), 0.96, "source:project-board", "2025-09-10T08:12:00Z", "2025-09-10T08:12:00Z"),
            bf("project/ray-exomem#repo", "project/repo", Str("repo:ray-exomem"), 0.99, "source:repo", "2025-09-10T08:15:00Z", "2025-09-10T08:15:00Z"),
            bf("project/ray-exomem#priority", "project/priority", I64(9), 0.94, "source:planning", "2026-01-15T11:30:00Z", "2026-01-15T11:30:00Z"),
            bf("project/ray-exomem#risk", "risk/score", I64(6), 0.78, "source:ops-review", "2026-04-06T10:20:00Z", "2026-04-06T10:20:00Z"),
            bf("project/ray-exomem#owner", "owned_by", Str("role/platform-owner"), 0.95, "source:org-map", "2025-09-12T09:00:00Z", "2025-09-12T09:00:00Z"),
            bf("project/ray-exomem#depends-rayforce", "depends_on", Str("project/rayfall-engine"), 0.92, "source:architecture-notes", "2025-10-04T13:40:00Z", "2025-10-04T13:40:00Z"),
            bf("project/ray-exomem#doc-live-test", "documents", Str("doc/live-test-loop"), 0.96, "source:CLAUDE.md", "2026-04-12T17:25:00Z", "2026-04-12T17:25:00Z"),
            bf("project/ray-exomem#decision-valid-time", "has_decision", Str("decision/valid-time"), 0.94, "source:architecture-notes", "2025-12-03T15:00:00Z", "2025-12-03T15:00:00Z"),
            bf("project/ray-exomem#incident-auth", "has_incident", Str("incident/auth-replay"), 0.91, "source:incident-log", "2026-03-22T19:20:00Z", "2026-03-22T19:20:00Z"),
            bf("project/native-ui#type", "entity/type", Str("project"), 0.98, "source:project-board", "2025-10-15T09:00:00Z", "2025-10-15T09:00:00Z"),
            bf("project/native-ui#name", "entity/name", Str("Native exomem UI"), 0.98, "source:project-board", "2025-10-15T09:01:00Z", "2025-10-15T09:01:00Z"),
            bf("project/native-ui#status", "project/status", Str("active"), 0.96, "source:project-board", "2026-02-10T11:00:00Z", "2026-02-10T11:00:00Z"),
            bf("project/native-ui#priority", "project/priority", I64(8), 0.92, "source:planning", "2026-03-28T13:30:00Z", "2026-03-28T13:30:00Z"),
            bf("project/native-ui#risk", "risk/score", I64(7), 0.81, "source:ui-review", "2026-04-17T16:00:00Z", "2026-04-17T16:00:00Z"),
            bf("project/native-ui#doc-polish", "documents", Str("doc/ui-polish-spec"), 0.95, "source:docs", "2026-04-13T12:20:00Z", "2026-04-13T12:20:00Z"),
            bf("project/native-ui#question-graph", "asks_question", Str("question/graph-density"), 0.9, "source:ui-review", "2026-04-17T16:05:00Z", "2026-04-17T16:05:00Z"),
            bf("project/rayfall-engine#type", "entity/type", Str("project"), 0.96, "source:architecture-notes", "2025-10-04T13:20:00Z", "2025-10-04T13:20:00Z"),
            bf("project/rayfall-engine#status", "project/status", Str("active"), 0.94, "source:project-board", "2026-01-04T10:00:00Z", "2026-01-04T10:00:00Z"),
            bf("project/rayfall-engine#priority", "project/priority", I64(8), 0.9, "source:planning", "2026-01-15T11:35:00Z", "2026-01-15T11:35:00Z"),
            bf("project/rayfall-engine#risk", "risk/score", I64(5), 0.76, "source:rule-audit", "2026-03-30T14:20:00Z", "2026-03-30T14:20:00Z"),
            bf("project/rayfall-engine#supports", "supports", Str("project/ray-exomem"), 0.92, "source:architecture-notes", "2025-10-04T13:45:00Z", "2025-10-04T13:45:00Z"),
            bf("project/retrieval-eval#type", "entity/type", Str("project"), 0.9, "source:research-log", "2025-11-20T10:00:00Z", "2025-11-20T10:00:00Z"),
            bf("project/retrieval-eval#status", "project/status", Str("active"), 0.88, "source:research-log", "2026-02-28T12:00:00Z", "2026-02-28T12:00:00Z"),
            bf("project/retrieval-eval#priority", "project/priority", I64(7), 0.86, "source:planning", "2026-03-04T15:00:00Z", "2026-03-04T15:00:00Z"),
            bf("project/retrieval-eval#depends", "depends_on", Str("project/ray-exomem"), 0.84, "source:research-log", "2026-03-04T15:05:00Z", "2026-03-04T15:05:00Z"),
            bf("project/ops-runbooks#type", "entity/type", Str("project"), 0.89, "source:ops-log", "2025-11-05T09:20:00Z", "2025-11-05T09:20:00Z"),
            bf("project/ops-runbooks#status", "project/status", Str("maintenance"), 0.88, "source:ops-log", "2026-03-15T09:20:00Z", "2026-03-15T09:20:00Z"),
            bf("project/ops-runbooks#priority", "project/priority", I64(6), 0.82, "source:ops-log", "2026-03-15T09:25:00Z", "2026-03-15T09:25:00Z"),
            bf("project/ops-runbooks#owner", "owned_by", Str("role/ops-reviewer"), 0.86, "source:org-map", "2025-11-05T09:25:00Z", "2025-11-05T09:25:00Z"),
            bf("decision/entity-ids#type", "entity/type", Str("decision"), 0.95, "source:architecture-notes", "2025-11-18T12:00:00Z", "2025-11-18T12:00:00Z"),
            bf("decision/entity-ids#title", "entity/name", Str("Stable entity ids use prefix#attribute fact ids"), 0.93, "source:architecture-notes", "2025-11-18T12:01:00Z", "2025-11-18T12:01:00Z"),
            bf("decision/entity-ids#status", "decision/status", Str("accepted"), 0.94, "source:architecture-notes", "2025-11-18T12:05:00Z", "2025-11-18T12:05:00Z"),
            bf("decision/entity-ids#review", "decision/review_due_days", I64(42), 0.8, "source:review-calendar", "2026-04-10T09:00:00Z", "2026-04-10T09:00:00Z"),
            bf("decision/entity-ids#supported", "supported_by", Str("obs/entity-id-collisions"), 0.86, "source:architecture-notes", "2025-11-18T12:10:00Z", "2025-11-18T12:10:00Z"),
            bf("decision/valid-time#type", "entity/type", Str("decision"), 0.92, "source:architecture-notes", "2025-12-03T15:00:00Z", "2025-12-03T15:00:00Z"),
            bf("decision/valid-time#status", "decision/status", Str("accepted"), 0.9, "source:architecture-notes", "2025-12-03T15:05:00Z", "2025-12-03T15:05:00Z"),
            bf("decision/valid-time#review", "decision/review_due_days", I64(5), 0.85, "source:review-calendar", "2026-04-18T09:00:00Z", "2026-04-18T09:00:00Z"),
            bf("decision/graph-shape#type", "entity/type", Str("decision"), 0.86, "source:ui-review", "2026-01-12T10:00:00Z", "2026-01-12T10:00:00Z"),
            bf_to("decision/graph-shape#status", "decision/status", Str("prototype"), 0.58, "source:ui-review", "2026-01-12T10:05:00Z", "2026-04-17T13:30:00Z", "2026-01-12T10:05:00Z"),
            bf("decision/graph-shape#status", "decision/status", Str("accepted"), 0.91, "source:ui-review", "2026-04-17T13:30:00Z", "2026-04-17T13:30:00Z"),
            bf("decision/graph-shape#supported", "supported_by", Str("obs/graph-predicate-only"), 0.9, "source:ui-review", "2026-04-17T13:35:00Z", "2026-04-17T13:35:00Z"),
            bf("decision/no-wizard#type", "entity/type", Str("decision"), 0.91, "source:product-constraint", "2026-04-23T11:15:00Z", "2026-04-23T11:15:00Z"),
            bf("decision/no-wizard#status", "decision/status", Str("accepted"), 0.96, "source:product-constraint", "2026-04-23T11:20:00Z", "2026-04-23T11:20:00Z"),
            bf("decision/no-wizard#applies", "applies_to", Str("project/native-ui"), 0.93, "source:product-constraint", "2026-04-23T11:21:00Z", "2026-04-23T11:21:00Z"),
            bf("incident/auth-replay#type", "entity/type", Str("incident"), 0.94, "source:incident-log", "2026-03-22T19:20:00Z", "2026-03-22T19:20:00Z"),
            bf("incident/auth-replay#status", "incident/status", Str("resolved"), 0.93, "source:incident-log", "2026-03-23T10:00:00Z", "2026-03-23T10:00:00Z"),
            bf("incident/auth-replay#severity", "incident/severity", I64(5), 0.86, "source:incident-log", "2026-03-22T19:25:00Z", "2026-03-22T19:25:00Z"),
            bf("incident/auth-replay#days", "incident/days_since", I64(31), 0.75, "source:ops-log", "2026-04-23T09:00:00Z", "2026-04-23T09:00:00Z"),
            bf("incident/auth-replay#affects", "affects", Str("project/ray-exomem"), 0.9, "source:incident-log", "2026-03-22T19:26:00Z", "2026-03-22T19:26:00Z"),
            bf("incident/symbol-table#type", "entity/type", Str("incident"), 0.88, "source:engine-log", "2026-04-04T18:00:00Z", "2026-04-04T18:00:00Z"),
            bf("incident/symbol-table#status", "incident/status", Str("monitoring"), 0.82, "source:engine-log", "2026-04-05T09:30:00Z", "2026-04-05T09:30:00Z"),
            bf("incident/symbol-table#severity", "incident/severity", I64(7), 0.79, "source:engine-log", "2026-04-04T18:05:00Z", "2026-04-04T18:05:00Z"),
            bf("incident/symbol-table#days", "incident/days_since", I64(19), 0.75, "source:ops-log", "2026-04-23T09:00:00Z", "2026-04-23T09:00:00Z"),
            bf("incident/symbol-table#affects", "affects", Str("project/rayfall-engine"), 0.82, "source:engine-log", "2026-04-04T18:08:00Z", "2026-04-04T18:08:00Z"),
            bf("doc/ui-polish-spec#type", "entity/type", Str("document"), 0.95, "source:docs", "2026-04-13T12:20:00Z", "2026-04-13T12:20:00Z"),
            bf("doc/ui-polish-spec#path", "document/path", Str("docs/superpowers/specs/2026-04-13-ui-polish-design.md"), 0.98, "source:docs", "2026-04-13T12:20:00Z", "2026-04-13T12:20:00Z"),
            bf("doc/ui-polish-spec#relates", "relates_to", Str("project/native-ui"), 0.91, "source:docs", "2026-04-13T12:25:00Z", "2026-04-13T12:25:00Z"),
            bf("doc/onboarding-template-plan#type", "entity/type", Str("document"), 0.9, "source:docs", "2026-04-18T09:00:00Z", "2026-04-18T09:00:00Z"),
            bf("doc/onboarding-template-plan#status", "document/status", Str("superseded-by-native-seed"), 0.83, "source:product-constraint", "2026-04-23T11:35:00Z", "2026-04-23T11:35:00Z"),
            bf("doc/live-test-loop#type", "entity/type", Str("runbook"), 0.96, "source:CLAUDE.md", "2026-04-12T17:25:00Z", "2026-04-12T17:25:00Z"),
            bf("doc/live-test-loop#command", "uses_command", Str("command/live-test-build"), 0.94, "source:CLAUDE.md", "2026-04-12T17:30:00Z", "2026-04-12T17:30:00Z"),
            bf("question/graph-density#type", "entity/type", Str("question"), 0.9, "source:ui-review", "2026-04-17T16:05:00Z", "2026-04-17T16:05:00Z"),
            bf("question/graph-density#status", "question/status", Str("open"), 0.86, "source:ui-review", "2026-04-17T16:06:00Z", "2026-04-17T16:06:00Z"),
            bf("question/graph-density#age", "question/age_days", I64(6), 0.8, "source:ui-review", "2026-04-23T09:00:00Z", "2026-04-23T09:00:00Z"),
            bf("question/graph-density#about", "about", Str("project/native-ui"), 0.88, "source:ui-review", "2026-04-17T16:08:00Z", "2026-04-17T16:08:00Z"),
            bf("question/branch-merge#type", "entity/type", Str("question"), 0.87, "source:branch-review", "2026-03-30T13:00:00Z", "2026-03-30T13:00:00Z"),
            bf("question/branch-merge#status", "question/status", Str("open"), 0.83, "source:branch-review", "2026-03-30T13:05:00Z", "2026-03-30T13:05:00Z"),
            bf("question/branch-merge#age", "question/age_days", I64(24), 0.8, "source:branch-review", "2026-04-23T09:00:00Z", "2026-04-23T09:00:00Z"),
            bf("question/branch-merge#about", "about", Str("decision/graph-shape"), 0.8, "source:branch-review", "2026-03-30T13:08:00Z", "2026-03-30T13:08:00Z"),
            bf("question/rule-errors#type", "entity/type", Str("question"), 0.84, "source:rule-audit", "2026-04-19T11:00:00Z", "2026-04-19T11:00:00Z"),
            bf("question/rule-errors#status", "question/status", Str("monitoring"), 0.8, "source:rule-audit", "2026-04-19T11:05:00Z", "2026-04-19T11:05:00Z"),
            bf("question/rule-errors#age", "question/age_days", I64(4), 0.75, "source:rule-audit", "2026-04-23T09:00:00Z", "2026-04-23T09:00:00Z"),
            bf_to("preference/ui-density#value", "preference/value", Str("avoid oversized welcome cards and decorative hero layouts"), 0.78, "source:ui-review", "2025-11-22T10:00:00Z", "2026-03-26T16:40:00Z", "2025-11-22T10:00:00Z"),
            bf("preference/ui-density#value", "preference/value", Str("dense, scan-first operational UI with compact controls and strong information scent"), 0.94, "source:ui-polish-spec", "2026-03-26T16:40:00Z", "2026-03-26T16:40:00Z"),
            bf("preference/provenance#value", "preference/value", Str("surface provenance next to claims instead of burying it in raw export views"), 0.9, "source:research-log", "2026-02-12T14:10:00Z", "2026-02-12T14:10:00Z"),
            bf("constraint/domain-neutral-seed#value", "constraint/value", Str("first-run data must read as a technical/work memory, not a domain-specific demo"), 0.99, "source:product-constraint", "2026-04-23T11:10:00Z", "2026-04-23T11:10:00Z"),
            bf("constraint/no-wizard#value", "constraint/value", Str("do not create a separate onboarding wizard; native surfaces carry the teaching load"), 0.99, "source:product-constraint", "2026-04-23T11:11:00Z", "2026-04-23T11:11:00Z"),
            bf("constraint/live-test#value", "constraint/value", Str("auth, server, storage, and rayfall changes require release build plus live daemon verification"), 0.98, "source:CLAUDE.md", "2026-04-12T17:25:00Z", "2026-04-12T17:25:00Z"),
            bf("command/live-test-build#type", "entity/type", Str("command"), 0.94, "source:CLAUDE.md", "2026-04-12T17:30:00Z", "2026-04-12T17:30:00Z"),
            bf("command/live-test-build#value", "command/value", Str("cargo build --release --features postgres --bin ray-exomem"), 0.94, "source:CLAUDE.md", "2026-04-12T17:30:00Z", "2026-04-12T17:30:00Z"),
            bf("command/status-check#type", "entity/type", Str("command"), 0.92, "source:CLAUDE.md", "2026-04-12T17:31:00Z", "2026-04-12T17:31:00Z"),
            bf("command/status-check#value", "command/value", Str("curl -s http://127.0.0.1:9780/ray-exomem/api/status"), 0.92, "source:CLAUDE.md", "2026-04-12T17:31:00Z", "2026-04-12T17:31:00Z"),
            branch_fact("candidate-graph-shape", "decision/graph-shape#branch-note", "branch/claim", Str("entity graph should derive subject from fact_id prefix and target from fact value"), 0.86, "source:branch-note", "2026-04-17T13:25:00Z", "2026-04-17T13:25:00Z"),
            branch_fact("candidate-graph-shape", "project/native-ui#branch-risk", "risk/score", I64(8), 0.78, "source:branch-note", "2026-04-17T13:28:00Z", "2026-04-17T13:28:00Z"),
            branch_fact("incident-followup", "incident/symbol-table#followup", "followup/status", Str("watch next release build for sym-table load regressions"), 0.8, "source:ops-followup", "2026-04-05T09:35:00Z", "2026-04-05T09:35:00Z"),
            branch_fact("archived-import-notes", "archive/import-2025#summary", "archive/summary", Str("legacy flat exom import notes retained for reference only"), 0.7, "source:archive-import", "2025-11-03T16:50:00Z", "2025-11-03T16:50:00Z"),
        ],
        observations: vec![
            obs("obs/first-run-mismatch", "product-review", "src/auth/routes.rs", "The previous first-run namespace used a narrow domain demo and generic work examples, which made the product feel clinical instead of like a durable technical memory.", 0.95, &["first-run", "product", "seed"], "2026-04-23T11:00:00Z", "2026-04-23T11:00:00Z"),
            obs("obs/graph-predicate-only", "code-review", "src/server.rs::api_relation_graph", "The graph endpoint returned predicate nodes with no edges, so even good facts could not demonstrate a relational memory.", 0.97, &["graph", "backend", "ui"], "2026-04-17T13:10:00Z", "2026-04-17T13:10:00Z"),
            obs("obs/native-tabs-ready", "ui-review", "ui/src/routes/tree/[...path]/ExomView.svelte", "The native exom view already exposes Facts, Branches, History, Graph, and Rules; the seed should make those tabs useful instead of adding another onboarding surface.", 0.94, &["ui", "native", "tabs"], "2026-04-13T12:30:00Z", "2026-04-13T12:30:00Z"),
            obs("obs/entity-id-collisions", "import-audit", "archive/2025-import", "Imported notes were easiest to merge when fact ids used stable entity prefixes and per-attribute suffixes.", 0.86, &["provenance", "ids", "import"], "2025-11-18T11:40:00Z", "2025-11-18T11:40:00Z"),
            obs("obs/typed-rules-work", "rule-audit", "tests/typed_facts_e2e.rs", "Numeric fact values populate facts_i64, which makes threshold-style Rayfall rules useful for project risk, stale questions, and review windows.", 0.93, &["rules", "typed-facts"], "2026-01-16T10:00:00Z", "2026-01-16T10:00:00Z"),
            obs("obs/auth-replay", "incident-log", "auth.jsonl replay", "Repeated user records must preserve active and last_login fields or deactivation appears to succeed while access remains live.", 0.9, &["auth", "incident", "history"], "2026-03-22T19:20:00Z", "2026-03-22T19:20:00Z"),
            obs("obs/open-questions-stale", "query-log", "saved queries", "Open questions older than two weeks were rarely revisited unless represented as queryable facts with age_days.", 0.82, &["questions", "rules", "workflow"], "2026-02-19T09:00:00Z", "2026-02-19T09:00:00Z"),
        ],
        beliefs: vec![
            belief("belief/welcome-template", "A separate welcome template will teach the product fastest", Superseded, 0.52, &["doc/onboarding-template-plan#status"], "The idea covered template choice, but it fought the native-interface constraint and delayed the user from the real workspace.", "2026-04-18T09:20:00Z", Some("2026-04-23T11:35:00Z"), "2026-04-18T09:20:00Z"),
            belief("belief/native-first-run", "First-run education belongs in the native memory surfaces", Active, 0.93, &["obs/native-tabs-ready", "constraint/no-wizard#value", "decision/no-wizard#status"], "The normal UI already contains the conceptual surfaces; strong state makes them legible without a tour.", "2026-04-23T11:40:00Z", None, "2026-04-23T11:40:00Z"),
            belief("belief/entity-graph", "Graph credibility depends on entity-to-entity edges, not predicate-only summaries", Active, 0.94, &["obs/graph-predicate-only", "decision/graph-shape#status"], "Technical users expect connected things, decisions, docs, incidents, and projects; predicate counts read like diagnostics.", "2026-04-17T13:40:00Z", None, "2026-04-17T13:40:00Z"),
            belief("belief/stable-ids", "Stable entity ids are the backbone of useful provenance and revision history", Active, 0.89, &["obs/entity-id-collisions", "decision/entity-ids#status"], "Entity-prefixed fact ids let the graph, history, and provenance agree on what a claim is about.", "2025-11-18T12:20:00Z", None, "2025-11-18T12:20:00Z"),
            belief("belief/rules-value", "Rules are valuable when they name operating conditions, not toy recommendations", Active, 0.87, &["obs/typed-rules-work", "obs/open-questions-stale"], "Threshold rules should expose stale questions, risk, review windows, and mature memories.", "2026-02-19T09:20:00Z", None, "2026-02-19T09:20:00Z"),
            belief("belief/branch-value", "Branches should read as alternatives and follow-ups rather than internal plumbing", Active, 0.78, &["brain/home#branch-policy", "decision/graph-shape#branch-note"], "Branch labels and branch-local facts make the model discoverable from the Branches and History tabs.", "2026-04-05T09:45:00Z", None, "2026-04-05T09:45:00Z"),
        ],
        rules: dashboard_rules(exom),
    }
}

fn compact_seed(facts: Vec<BootstrapFactSpec>) -> BootstrapSeed {
    BootstrapSeed {
        facts,
        observations: Vec::new(),
        beliefs: Vec::new(),
        branches: Vec::new(),
        rules: Vec::new(),
    }
}

fn work_seed() -> BootstrapSeed {
    use BootstrapLiteral::{Str, I64};
    compact_seed(vec![
        bf(
            "work/index#type",
            "entity/type",
            Str("folder-index"),
            0.93,
            "source:tree-index",
            "2025-09-10T08:00:00Z",
            "2025-09-10T08:00:00Z",
        ),
        bf(
            "work/index#focus",
            "index/focus",
            Str("active projects, operational constraints, and release-critical memory"),
            0.91,
            "source:tree-index",
            "2025-09-10T08:05:00Z",
            "2025-09-10T08:05:00Z",
        ),
        bf(
            "work/index#contains-platform",
            "contains",
            Str("project/ray-exomem"),
            0.9,
            "source:tree-index",
            "2025-09-10T08:06:00Z",
            "2025-09-10T08:06:00Z",
        ),
        bf(
            "work/index#contains-ui",
            "contains",
            Str("project/native-ui"),
            0.88,
            "source:tree-index",
            "2025-10-15T09:05:00Z",
            "2025-10-15T09:05:00Z",
        ),
        bf(
            "work/index#priority",
            "project/priority",
            I64(8),
            0.84,
            "source:planning",
            "2026-03-28T13:30:00Z",
            "2026-03-28T13:30:00Z",
        ),
    ])
}

fn memory_daemon_seed() -> BootstrapSeed {
    use BootstrapLiteral::{Str, I64};
    compact_seed(vec![
        bf(
            "project/ray-exomem#type",
            "entity/type",
            Str("project"),
            0.99,
            "source:repo",
            "2025-09-10T08:00:00Z",
            "2025-09-10T08:00:00Z",
        ),
        bf(
            "project/ray-exomem#status",
            "project/status",
            Str("active"),
            0.98,
            "source:project-board",
            "2025-09-10T08:10:00Z",
            "2025-09-10T08:10:00Z",
        ),
        bf(
            "project/ray-exomem#priority",
            "project/priority",
            I64(9),
            0.94,
            "source:planning",
            "2026-01-15T11:30:00Z",
            "2026-01-15T11:30:00Z",
        ),
        bf(
            "project/ray-exomem#constraint-live",
            "governed_by",
            Str("constraint/live-test#value"),
            0.96,
            "source:CLAUDE.md",
            "2026-04-12T17:25:00Z",
            "2026-04-12T17:25:00Z",
        ),
        bf(
            "project/ray-exomem#next",
            "next_action",
            Str("verify rich first-run seed against live daemon after release build"),
            0.9,
            "source:worklog",
            "2026-04-23T12:00:00Z",
            "2026-04-23T12:00:00Z",
        ),
    ])
}

fn native_ui_seed() -> BootstrapSeed {
    use BootstrapLiteral::{Str, I64};
    compact_seed(vec![
        bf(
            "project/native-ui#type",
            "entity/type",
            Str("project"),
            0.98,
            "source:project-board",
            "2025-10-15T09:00:00Z",
            "2025-10-15T09:00:00Z",
        ),
        bf(
            "project/native-ui#status",
            "project/status",
            Str("active"),
            0.96,
            "source:project-board",
            "2026-02-10T11:00:00Z",
            "2026-02-10T11:00:00Z",
        ),
        bf(
            "project/native-ui#priority",
            "project/priority",
            I64(8),
            0.92,
            "source:planning",
            "2026-03-28T13:30:00Z",
            "2026-03-28T13:30:00Z",
        ),
        bf(
            "project/native-ui#risk",
            "risk/score",
            I64(7),
            0.81,
            "source:ui-review",
            "2026-04-17T16:00:00Z",
            "2026-04-17T16:00:00Z",
        ),
        bf(
            "project/native-ui#constraint",
            "governed_by",
            Str("constraint/no-wizard#value"),
            0.96,
            "source:product-constraint",
            "2026-04-23T11:11:00Z",
            "2026-04-23T11:11:00Z",
        ),
        bf(
            "project/native-ui#open-question",
            "asks_question",
            Str("question/graph-density"),
            0.9,
            "source:ui-review",
            "2026-04-17T16:05:00Z",
            "2026-04-17T16:05:00Z",
        ),
    ])
}

fn rayfall_seed() -> BootstrapSeed {
    use BootstrapLiteral::{Str, I64};
    compact_seed(vec![
        bf(
            "project/rayfall-engine#type",
            "entity/type",
            Str("project"),
            0.96,
            "source:architecture-notes",
            "2025-10-04T13:20:00Z",
            "2025-10-04T13:20:00Z",
        ),
        bf(
            "project/rayfall-engine#status",
            "project/status",
            Str("active"),
            0.94,
            "source:project-board",
            "2026-01-04T10:00:00Z",
            "2026-01-04T10:00:00Z",
        ),
        bf(
            "project/rayfall-engine#priority",
            "project/priority",
            I64(8),
            0.9,
            "source:planning",
            "2026-01-15T11:35:00Z",
            "2026-01-15T11:35:00Z",
        ),
        bf(
            "project/rayfall-engine#risk",
            "risk/score",
            I64(5),
            0.76,
            "source:rule-audit",
            "2026-03-30T14:20:00Z",
            "2026-03-30T14:20:00Z",
        ),
        bf(
            "project/rayfall-engine#supports",
            "supports",
            Str("project/ray-exomem"),
            0.92,
            "source:architecture-notes",
            "2025-10-04T13:45:00Z",
            "2025-10-04T13:45:00Z",
        ),
    ])
}

fn operations_seed() -> BootstrapSeed {
    use BootstrapLiteral::{Str, I64};
    compact_seed(vec![
        bf(
            "project/ops-runbooks#type",
            "entity/type",
            Str("project"),
            0.89,
            "source:ops-log",
            "2025-11-05T09:20:00Z",
            "2025-11-05T09:20:00Z",
        ),
        bf(
            "project/ops-runbooks#status",
            "project/status",
            Str("maintenance"),
            0.88,
            "source:ops-log",
            "2026-03-15T09:20:00Z",
            "2026-03-15T09:20:00Z",
        ),
        bf(
            "project/ops-runbooks#priority",
            "project/priority",
            I64(6),
            0.82,
            "source:ops-log",
            "2026-03-15T09:25:00Z",
            "2026-03-15T09:25:00Z",
        ),
        bf(
            "project/ops-runbooks#uses",
            "uses_command",
            Str("command/status-check"),
            0.86,
            "source:CLAUDE.md",
            "2026-04-12T17:31:00Z",
            "2026-04-12T17:31:00Z",
        ),
    ])
}

fn incidents_seed() -> BootstrapSeed {
    use BootstrapLiteral::{Str, I64};
    compact_seed(vec![
        bf(
            "incident/auth-replay#type",
            "entity/type",
            Str("incident"),
            0.94,
            "source:incident-log",
            "2026-03-22T19:20:00Z",
            "2026-03-22T19:20:00Z",
        ),
        bf(
            "incident/auth-replay#status",
            "incident/status",
            Str("resolved"),
            0.93,
            "source:incident-log",
            "2026-03-23T10:00:00Z",
            "2026-03-23T10:00:00Z",
        ),
        bf(
            "incident/auth-replay#severity",
            "incident/severity",
            I64(5),
            0.86,
            "source:incident-log",
            "2026-03-22T19:25:00Z",
            "2026-03-22T19:25:00Z",
        ),
        bf(
            "incident/symbol-table#type",
            "entity/type",
            Str("incident"),
            0.88,
            "source:engine-log",
            "2026-04-04T18:00:00Z",
            "2026-04-04T18:00:00Z",
        ),
        bf(
            "incident/symbol-table#status",
            "incident/status",
            Str("monitoring"),
            0.82,
            "source:engine-log",
            "2026-04-05T09:30:00Z",
            "2026-04-05T09:30:00Z",
        ),
        bf(
            "incident/symbol-table#severity",
            "incident/severity",
            I64(7),
            0.79,
            "source:engine-log",
            "2026-04-04T18:05:00Z",
            "2026-04-04T18:05:00Z",
        ),
    ])
}

fn research_seed() -> BootstrapSeed {
    use BootstrapLiteral::{Str, I64};
    compact_seed(vec![
        bf(
            "research/index#type",
            "entity/type",
            Str("folder-index"),
            0.88,
            "source:research-log",
            "2025-11-20T10:00:00Z",
            "2025-11-20T10:00:00Z",
        ),
        bf(
            "research/index#focus",
            "index/focus",
            Str("agent memory, retrieval quality, provenance ergonomics, and evaluation hygiene"),
            0.86,
            "source:research-log",
            "2025-11-20T10:05:00Z",
            "2025-11-20T10:05:00Z",
        ),
        bf(
            "project/retrieval-eval#priority",
            "project/priority",
            I64(7),
            0.86,
            "source:planning",
            "2026-03-04T15:00:00Z",
            "2026-03-04T15:00:00Z",
        ),
    ])
}

fn knowledge_seed() -> BootstrapSeed {
    use BootstrapLiteral::Str;
    compact_seed(vec![
        bf(
            "knowledge/index#type",
            "entity/type",
            Str("folder-index"),
            0.87,
            "source:docs",
            "2025-12-03T15:00:00Z",
            "2025-12-03T15:00:00Z",
        ),
        bf(
            "knowledge/index#focus",
            "index/focus",
            Str("architecture decisions, constraints, runbooks, and reusable commands"),
            0.86,
            "source:docs",
            "2025-12-03T15:05:00Z",
            "2025-12-03T15:05:00Z",
        ),
        bf(
            "knowledge/index#contains",
            "contains",
            Str("doc/live-test-loop"),
            0.9,
            "source:docs",
            "2026-04-12T17:25:00Z",
            "2026-04-12T17:25:00Z",
        ),
    ])
}

fn archive_seed() -> BootstrapSeed {
    use BootstrapLiteral::Str;
    compact_seed(vec![
        bf("archive/index#type", "entity/type", Str("folder-index"), 0.8, "source:archive-import", "2025-11-03T16:45:00Z", "2025-11-03T16:45:00Z"),
        bf("archive/index#policy", "archive/policy", Str("keep superseded imports and retired decisions queryable, but out of active project rules"), 0.78, "source:archive-import", "2025-11-03T16:50:00Z", "2025-11-03T16:50:00Z"),
        bf("archive/import-2025#status", "archive/status", Str("closed"), 0.75, "source:archive-import", "2025-11-03T16:55:00Z", "2025-11-03T16:55:00Z"),
    ])
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
                    actor: rule_spec.actor.to_string(),
                    session: None,
                    model: None,
                    user_email: Some(email.to_string()),
                },
                rule_spec.defined_at.to_string(),
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

async fn bootstrap_user_namespace(state: &AppState, email: &str) -> Result<(), ApiError> {
    if state.auth_store.is_none() {
        return Ok(());
    }

    let Some(tree_root) = state.tree_root.as_ref() else {
        return Ok(());
    };

    let project_paths = [
        email.to_string(),
        format!("{email}/work"),
        format!("{email}/work/platform"),
        format!("{email}/work/platform/memory-daemon"),
        format!("{email}/work/platform/native-ui"),
        format!("{email}/work/platform/rayfall"),
        format!("{email}/work/operations"),
        format!("{email}/work/operations/incidents"),
        format!("{email}/research"),
        format!("{email}/research/agent-memory"),
        format!("{email}/research/retrieval-eval"),
        format!("{email}/knowledge"),
        format!("{email}/knowledge/architecture"),
        format!("{email}/archive"),
        format!("{email}/archive/2025-import"),
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

    let dashboard_exom = format!("{email}/main");
    seed_bootstrap_exom(
        state,
        &dashboard_exom,
        email,
        dashboard_seed(&dashboard_exom),
    )
    .await?;

    let seed_jobs = [
        (format!("{email}/work/main"), work_seed()),
        (
            format!("{email}/work/platform/memory-daemon/main"),
            memory_daemon_seed(),
        ),
        (
            format!("{email}/work/platform/native-ui/main"),
            native_ui_seed(),
        ),
        (
            format!("{email}/work/platform/rayfall/main"),
            rayfall_seed(),
        ),
        (format!("{email}/work/operations/main"), operations_seed()),
        (
            format!("{email}/work/operations/incidents/main"),
            incidents_seed(),
        ),
        (format!("{email}/research/main"), research_seed()),
        (format!("{email}/knowledge/main"), knowledge_seed()),
        (format!("{email}/archive/main"), archive_seed()),
        (format!("{email}/archive/2025-import/main"), archive_seed()),
    ];

    for (exom, seed) in seed_jobs {
        seed_bootstrap_exom(state, &exom, email, seed).await?;
    }

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
    bootstrap_user_namespace(&state, &identity.email).await?;

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
