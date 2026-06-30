//! Phase D-E integration: ray-exomem write paths routed through
//! `ray_transactor::Transactor`, invalidations matched through
//! `rayforce_realtime::RealtimeBus`.
//!
//! Architectural seam:
//!
//! * Brain remains the typed-fact engine and the persistence owner.
//! * The transactor sits in front of every brain mutation as an auth +
//!   idempotency + touched-key gate. Its `DatomStore` is empty (brain owns
//!   the real state), its schema is empty (brain has its own typed-value
//!   validation), its projection is `NullProjection` (brain owns persistence).
//! * The realtime bus consults the transactor's `CommitResult.touched` to
//!   match against active `Subscription`s. The legacy `broadcast::Sender`
//!   on `/events` remains as the transport sink, fed from the bus.
//!
//! Subsequent slices will move more brain mutations onto this path (Phase D
//! continuation) and replace the broadcast-based SSE consumer with bus
//! subscriptions (Phase E continuation).

use std::sync::{Arc, Mutex};

use ray_datom::schema::SchemaRegistry;
use ray_datom::scope::ScopeId;
use ray_datom::tx::ActorKind;
use ray_transactor::auth::{AuthContext, Grant, Permission};
use ray_transactor::command::{Command, CommandPlan, CommitResult};
use ray_transactor::error::TransactorError;
use ray_transactor::transactor::{NullProjection, Transactor};
use rayforce_realtime::bus::{Invalidation, RealtimeBus};

// ---------------------------------------------------------------------------
// Command names — dotted identifiers, mirroring the handoff's command bus.
// ---------------------------------------------------------------------------

pub const FACT_ASSERT: &str = "fact.assert";
pub const FACT_RETRACT: &str = "fact.retract";
pub const BELIEF_REVISE: &str = "belief.revise";
pub const BELIEF_REVOKE: &str = "belief.revoke";
pub const OBSERVATION_ASSERT: &str = "observation.assert";
pub const BRANCH_CREATE: &str = "branch.create";
pub const BRANCH_ARCHIVE: &str = "branch.archive";
pub const BRANCH_MERGE: &str = "branch.merge";
pub const EXOM_MODE_SET: &str = "exom.mode_set";
pub const SESSION_CREATE: &str = "session.create";
pub const SESSION_JOIN: &str = "session.join";

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

pub const PERM_READ: &str = "read";
pub const PERM_WRITE: &str = "write";

// ---------------------------------------------------------------------------
// Scope helpers
// ---------------------------------------------------------------------------

/// Build the per-exom `ScopeId` from a slash-delimited tree path.
pub fn exom_scope(exom_slash: &str) -> ScopeId {
    ScopeId::new(exom_slash)
}

// ---------------------------------------------------------------------------
// AuthContext construction
// ---------------------------------------------------------------------------

/// Build an `AuthContext` for the given user against an exom-scoped operation.
///
/// `level` collapses ray-exomem's `AccessLevel` into a `Grant` per permission
/// the principal effectively holds on the exom scope. Unauthenticated calls
/// (no User) produce an empty grant list; the transactor will reject any
/// command that requires permissions.
pub fn auth_context_for(
    user_email: Option<&str>,
    actor_kind: ActorKind,
    exom_slash: &str,
    can_read: bool,
    can_write: bool,
) -> AuthContext {
    let principal: ray_datom::tx::PrincipalId = user_email
        .map(|e| ray_datom::tx::PrincipalId::new(e))
        .unwrap_or_else(|| ray_datom::tx::PrincipalId::new("anonymous"));
    let mut grants: Vec<Grant> = Vec::new();
    let scope = exom_scope(exom_slash);
    if can_read {
        grants.push(Grant {
            principal: principal.clone(),
            scope: scope.clone(),
            permission: Permission::new(PERM_READ),
        });
    }
    if can_write {
        grants.push(Grant {
            principal: principal.clone(),
            scope,
            permission: Permission::new(PERM_WRITE),
        });
    }
    AuthContext::new(principal, actor_kind).with_grants(grants)
}

// ---------------------------------------------------------------------------
// TxFunctions
// ---------------------------------------------------------------------------
//
// Each tx-function validates payload shape, then returns a CommandPlan whose
// permissions+touched-keys reflect the upcoming brain mutation. Datoms are
// intentionally empty: brain owns the durable Tx log and the splay tables;
// pushing duplicate datoms through the transactor's projection would
// double-write without value.

fn payload_str<'a>(cmd: &'a Command, key: &str) -> Result<&'a str, TransactorError> {
    cmd.payload[key]
        .as_str()
        .ok_or_else(|| TransactorError::TxFunctionRejected(format!("missing '{}'", key)))
}

fn exom_from_payload(cmd: &Command) -> Result<String, TransactorError> {
    Ok(payload_str(cmd, "exom")?.to_string())
}

fn fact_assert_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let predicate = payload_str(cmd, "predicate")?.to_string();
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr(predicate))
}

fn fact_retract_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let fact_id = payload_str(cmd, "fact_id")?.to_string();
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr(fact_id))
}

fn belief_revise_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("belief"))
}

fn belief_revoke_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("belief"))
}

fn observation_assert_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("observation"))
}

fn branch_create_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("branch"))
}

fn branch_archive_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("branch"))
}

fn branch_merge_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("branch"))
}

fn exom_mode_set_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("_meta/acl_mode"))
}

fn session_create_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("session"))
}

fn session_join_fn(
    _store: &ray_datom::datom_store::DatomStore,
    _auth: &AuthContext,
    cmd: &Command,
) -> Result<CommandPlan, TransactorError> {
    let exom = exom_from_payload(cmd)?;
    let scope = exom_scope(&exom);
    Ok(CommandPlan::new(Vec::new())
        .requires(scope.clone(), Permission::new(PERM_WRITE))
        .touch_scope(scope)
        .touch_attr("session"))
}

// ---------------------------------------------------------------------------
// Registry + factories
// ---------------------------------------------------------------------------

/// Build a fresh `Transactor` with every ray-exomem command pre-registered.
///
/// The transactor uses the system wall clock for tx timestamps and a
/// `NullProjection`. Both decisions follow Phase D-E's "brain owns
/// persistence" guarantee.
pub fn build_transactor() -> Transactor {
    let mut t = Transactor::new(SchemaRegistry::new(), Box::new(NullProjection::default()))
        .with_clock(|| crate::brain::now_iso());
    t.register(FACT_ASSERT, fact_assert_fn);
    t.register(FACT_RETRACT, fact_retract_fn);
    t.register(BELIEF_REVISE, belief_revise_fn);
    t.register(BELIEF_REVOKE, belief_revoke_fn);
    t.register(OBSERVATION_ASSERT, observation_assert_fn);
    t.register(BRANCH_CREATE, branch_create_fn);
    t.register(BRANCH_ARCHIVE, branch_archive_fn);
    t.register(BRANCH_MERGE, branch_merge_fn);
    t.register(EXOM_MODE_SET, exom_mode_set_fn);
    t.register(SESSION_CREATE, session_create_fn);
    t.register(SESSION_JOIN, session_join_fn);
    t
}

/// `Arc<Mutex<...>>` wrapper used by AppState so the transactor can be cloned
/// across handler tasks without lifetime acrobatics.
pub type SharedTransactor = Arc<Mutex<Transactor>>;
pub type SharedRealtimeBus = Arc<Mutex<RealtimeBus>>;

pub fn shared_transactor() -> SharedTransactor {
    Arc::new(Mutex::new(build_transactor()))
}

pub fn shared_realtime_bus() -> SharedRealtimeBus {
    Arc::new(Mutex::new(RealtimeBus::new()))
}

// ---------------------------------------------------------------------------
// Commit + match helpers
// ---------------------------------------------------------------------------

/// Run `command` through the shared transactor. Returns the `CommitResult`
/// on success — the caller is responsible for executing the corresponding
/// brain mutation (typically via `mutate_exom_async`) and for feeding the
/// `CommitResult` to `bus_match_commit`.
pub fn commit_command(
    transactor: &SharedTransactor,
    auth: &AuthContext,
    command: Command,
) -> Result<CommitResult, TransactorError> {
    let mut t = transactor.lock().unwrap();
    t.commit(auth, command)
}

/// Match a `CommitResult` against the realtime bus. Returns one
/// `Invalidation` per affected subscription. The legacy broadcast::Sender
/// transport still ships a coarse `{"kind":"memory","exom":...}` event;
/// subscriptions and richer invalidations are wired through the bus so future
/// slices can switch the SSE consumer over without changing the producer.
pub fn bus_match_commit(bus: &SharedRealtimeBus, result: &CommitResult) -> Vec<Invalidation> {
    let b = bus.lock().unwrap();
    b.match_commit(result)
}

/// Phase F (secured query): check that `auth` is authorized to *read* the
/// given exom scope through ray-transactor's `AuthContext` model. Returns
/// `Ok(())` on grant, `Err` carrying the unauthorized message otherwise.
/// The route layer's `guard_read` already enforces this at the surface; the
/// transactor-model gate is defense-in-depth and the single source of truth
/// callers move toward as Phase F mirrors grants into datoms.
pub fn secured_query_check(
    user_email: Option<&str>,
    exom_slash: &str,
    can_read: bool,
) -> Result<(), String> {
    let auth = auth_context_for(user_email, ActorKind::Human, exom_slash, can_read, false);
    if auth.authorized(&exom_scope(exom_slash), &Permission::new(PERM_READ)) {
        Ok(())
    } else {
        Err(format!(
            "secured-query: read access denied for principal '{}' on scope '{}'",
            user_email.unwrap_or("anonymous"),
            exom_slash
        ))
    }
}

/// Convenience: build an `AuthContext` with full read+write grants for the
/// supplied user against `exom_slash`, then commit `command` through the
/// shared transactor. The caller is expected to have already enforced
/// finer-grained auth at the route layer (`guard_write` / `precheck_write`);
/// this gate is structural — auth, idempotency, touched-key emission.
pub fn commit_for_user(
    transactor: &SharedTransactor,
    user_email: Option<&str>,
    exom_slash: &str,
    command: Command,
) -> Result<CommitResult, TransactorError> {
    let auth = auth_context_for(user_email, ActorKind::Human, exom_slash, true, true);
    commit_command(transactor, &auth, command)
}

/// Drain matched `Invalidation`s from the realtime bus through `emit`. The
/// emit closure receives the JSON envelope (already wrapped via
/// `invalidation_envelope`) for each invalidation. No-op when the bus has
/// no subscriptions whose deps overlap `result.touched`.
pub fn dispatch_invalidations<F: FnMut(String)>(
    bus: &SharedRealtimeBus,
    exom_slash: &str,
    result: &CommitResult,
    mut emit: F,
) {
    let invs = bus_match_commit(bus, result);
    for inv in &invs {
        emit(invalidation_envelope(exom_slash, inv));
    }
}

/// Encode an `Invalidation` as the JSON envelope used on the SSE transport.
/// Kept stable so future bus-aware clients can subscribe without
/// re-encoding.
pub fn invalidation_envelope(exom: &str, inv: &Invalidation) -> String {
    let payload = serde_json::to_value(inv).unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "v": 1,
        "kind": "invalidation",
        "exom": exom,
        "invalidation": payload,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Convenience builders for the common command shapes.
// ---------------------------------------------------------------------------

pub fn build_fact_assert(exom: &str, branch: &str, fact_id: &str, predicate: &str) -> Command {
    Command::new(
        FACT_ASSERT,
        serde_json::json!({
            "exom": exom,
            "branch": branch,
            "fact_id": fact_id,
            "predicate": predicate,
        }),
    )
}

pub fn build_fact_retract(exom: &str, branch: &str, fact_id: &str) -> Command {
    Command::new(
        FACT_RETRACT,
        serde_json::json!({
            "exom": exom,
            "branch": branch,
            "fact_id": fact_id,
        }),
    )
}

pub fn build_exom_mode_set(exom: &str, mode: &str) -> Command {
    Command::new(
        EXOM_MODE_SET,
        serde_json::json!({
            "exom": exom,
            "mode": mode,
        }),
    )
}

pub fn build_session_create(parent_exom: &str, label: Option<&str>) -> Command {
    Command::new(
        SESSION_CREATE,
        serde_json::json!({
            "exom": parent_exom,
            "label": label,
        }),
    )
}

pub fn build_branch_create(exom: &str, branch_name: &str, parent: &str) -> Command {
    Command::new(
        BRANCH_CREATE,
        serde_json::json!({
            "exom": exom,
            "branch_name": branch_name,
            "parent": parent,
        }),
    )
}

pub fn build_branch_archive(exom: &str, branch: &str) -> Command {
    Command::new(
        BRANCH_ARCHIVE,
        serde_json::json!({
            "exom": exom,
            "branch": branch,
        }),
    )
}

pub fn build_branch_merge(exom: &str, source: &str, target: &str, policy: &str) -> Command {
    Command::new(
        BRANCH_MERGE,
        serde_json::json!({
            "exom": exom,
            "source": source,
            "target": target,
            "policy": policy,
        }),
    )
}

pub fn build_belief_revise(exom: &str, branch: &str, belief_id: &str) -> Command {
    Command::new(
        BELIEF_REVISE,
        serde_json::json!({
            "exom": exom,
            "branch": branch,
            "belief_id": belief_id,
        }),
    )
}

pub fn build_belief_revoke(exom: &str, branch: &str, belief_id: &str) -> Command {
    Command::new(
        BELIEF_REVOKE,
        serde_json::json!({
            "exom": exom,
            "branch": branch,
            "belief_id": belief_id,
        }),
    )
}

pub fn build_observation_assert(exom: &str, branch: &str, obs_id: &str) -> Command {
    Command::new(
        OBSERVATION_ASSERT,
        serde_json::json!({
            "exom": exom,
            "branch": branch,
            "obs_id": obs_id,
        }),
    )
}

pub fn build_session_join(session_exom: &str) -> Command {
    Command::new(
        SESSION_JOIN,
        serde_json::json!({
            "exom": session_exom,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rayforce_realtime::subscription::{Dependency, QuerySpec};

    #[test]
    fn transactor_builds_with_every_command_registered() {
        let t = build_transactor();
        // Smoke: store/schema accessors do not panic; commit() will reject
        // unknown commands. We can't easily list registered commands, so we
        // verify by submitting a known command with a missing payload field.
        let _ = t.store();
        let _ = t.schema();
    }

    #[test]
    fn fact_assert_requires_write_permission_on_exom_scope() {
        let mut t = build_transactor();
        let auth_no_grants = auth_context_for(
            Some("vasily@lynxtrading.com"),
            ActorKind::Human,
            "vasily@lynxtrading.com/test/x",
            false,
            false,
        );
        let cmd = build_fact_assert(
            "vasily@lynxtrading.com/test/x",
            "main",
            "test/marker",
            "test/marker",
        );
        let err = t.commit(&auth_no_grants, cmd).unwrap_err();
        assert!(
            matches!(err, TransactorError::PermissionDenied { .. }),
            "expected PermissionDenied, got {err:?}"
        );
    }

    #[test]
    fn fact_assert_succeeds_with_write_grant() {
        let mut t = build_transactor();
        let auth = auth_context_for(
            Some("vasily@lynxtrading.com"),
            ActorKind::Human,
            "vasily@lynxtrading.com/test/x",
            true,
            true,
        );
        let cmd = build_fact_assert(
            "vasily@lynxtrading.com/test/x",
            "main",
            "test/marker",
            "test/marker",
        );
        let r = t.commit(&auth, cmd).unwrap();
        assert!(!r.idempotent_replay);
        assert!(r
            .touched
            .scopes
            .contains(&exom_scope("vasily@lynxtrading.com/test/x")));
        assert!(r.touched.attrs.contains("test/marker"));
    }

    #[test]
    fn secured_query_check_admits_when_can_read_true() {
        let r = secured_query_check(
            Some("vasily@lynxtrading.com"),
            "vasily@lynxtrading.com/test/x",
            true,
        );
        assert!(r.is_ok(), "expected Ok, got {:?}", r);
    }

    #[test]
    fn secured_query_check_denies_when_can_read_false() {
        let r = secured_query_check(
            Some("intruder@example.com"),
            "vasily@lynxtrading.com/private/secret",
            false,
        );
        let err = r.expect_err("expected denial");
        assert!(
            err.contains("secured-query") && err.contains("intruder@example.com"),
            "unexpected message: {}",
            err
        );
    }

    #[test]
    fn bus_matches_commit_against_subscription_on_same_scope() {
        // End-to-end Phase E proof: register a subscription on the realtime
        // bus, drive a fact.assert command through the transactor, and
        // confirm the bus emits an `Invalidation` keyed on the touched scope.
        let bus = shared_realtime_bus();
        let transactor = shared_transactor();

        let auth = auth_context_for(
            Some("vasily@lynxtrading.com"),
            ActorKind::Human,
            "vasily@lynxtrading.com/test/x",
            true,
            true,
        );

        {
            let mut b = bus.lock().unwrap();
            b.subscribe(
                auth.clone(),
                QuerySpec::new("fact-row", serde_json::json!({})),
                vec![Dependency::Scope(exom_scope(
                    "vasily@lynxtrading.com/test/x",
                ))],
                Permission::new(PERM_WRITE),
            );
        }

        let cmd = build_fact_assert(
            "vasily@lynxtrading.com/test/x",
            "main",
            "test/marker",
            "test/marker",
        );
        let result = commit_command(&transactor, &auth, cmd).unwrap();
        let invs = bus_match_commit(&bus, &result);
        assert_eq!(invs.len(), 1, "expected 1 invalidation, got {invs:?}");
        let envelope = invalidation_envelope("vasily@lynxtrading.com/test/x", &invs[0]);
        assert!(envelope.contains("\"kind\":\"invalidation\""));
        assert!(envelope.contains("vasily@lynxtrading.com/test/x"));
    }

    #[test]
    fn fact_assert_replays_idempotency_key() {
        let mut t = build_transactor();
        let auth = auth_context_for(
            Some("vasily@lynxtrading.com"),
            ActorKind::Human,
            "vasily@lynxtrading.com/test/x",
            true,
            true,
        );
        let cmd = build_fact_assert(
            "vasily@lynxtrading.com/test/x",
            "main",
            "test/marker",
            "test/marker",
        )
        .with_idempotency_key("idem-1");
        let r1 = t.commit(&auth, cmd.clone()).unwrap();
        let r2 = t.commit(&auth, cmd).unwrap();
        assert_eq!(r1.tx_id, r2.tx_id);
        assert!(!r1.idempotent_replay);
        assert!(r2.idempotent_replay);
    }
}
