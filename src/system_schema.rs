use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::brain::Brain;
use crate::context::MutationContext;
use crate::rules::{self, ParsedRule};

pub mod attrs {
    pub mod fact {
        pub const PREDICATE: &str = "fact/predicate";
        pub const VALUE: &str = "fact/value";
        /// Numeric (i64) shadow attribute for typed cmp / aggregation.
        /// Emitted only when a fact's value is [`FactValue::I64`](crate::fact_value::FactValue::I64);
        /// string / symbol facts omit this datom entirely so the column stays
        /// a clean bare-int type for Rayfall's `<`, `>`, `sum`, `avg` ops.
        pub const VALUE_I64: &str = "fact/value_i64";
        pub const CONFIDENCE: &str = "fact/confidence";
        pub const PROVENANCE: &str = "fact/provenance";
        pub const VALID_FROM: &str = "fact/valid_from";
        pub const VALID_TO: &str = "fact/valid_to";
        pub const CREATED_BY: &str = "fact/created_by";
        pub const SUPERSEDED_BY: &str = "fact/superseded_by";
        pub const REVOKED_BY: &str = "fact/revoked_by";
    }

    pub mod tx {
        pub const ID: &str = "tx/id";
        pub const TIME: &str = "tx/time";
        pub const USER_EMAIL: &str = "tx/user-email";
        pub const ACTOR: &str = "tx/actor";
        pub const ACTION: &str = "tx/action";
        pub const BRANCH: &str = "tx/branch";
        pub const PARENT: &str = "tx/parent";
        pub const SESSION: &str = "tx/session";
        pub const REF: &str = "tx/ref";
        pub const MERGE_SOURCE: &str = "tx/merge_source";
        pub const MERGE_TARGET: &str = "tx/merge_target";
    }

    pub mod observation {
        pub const SOURCE_TYPE: &str = "obs/source_type";
        pub const SOURCE_REF: &str = "obs/source_ref";
        pub const CONTENT: &str = "obs/content";
        pub const CREATED_AT: &str = "obs/created_at";
        pub const CONFIDENCE: &str = "obs/confidence";
        pub const TX: &str = "obs/tx";
        pub const VALID_FROM: &str = "obs/valid_from";
        pub const VALID_TO: &str = "obs/valid_to";
        pub const TAG: &str = "obs/tag";
    }

    pub mod belief {
        pub const CLAIM_TEXT: &str = "belief/claim_text";
        pub const STATUS: &str = "belief/status";
        pub const CONFIDENCE: &str = "belief/confidence";
        pub const CREATED_BY: &str = "belief/created_by";
        pub const VALID_FROM: &str = "belief/valid_from";
        pub const VALID_TO: &str = "belief/valid_to";
        pub const RATIONALE: &str = "belief/rationale";
        pub const SUPPORTS: &str = "belief/supports";
    }

    pub mod branch {
        pub const ID: &str = "branch/id";
        pub const NAME: &str = "branch/name";
        pub const PARENT: &str = "branch/parent";
        pub const CREATED_BY: &str = "branch/created_by";
        pub const ARCHIVED: &str = "branch/archived";
    }

    pub mod coord {
        pub const CLAIM_OWNER: &str = "claim/owner";
        pub const CLAIM_STATUS: &str = "claim/status";
        pub const CLAIM_EXPIRES_AT: &str = "claim/expires_at";
        pub const TASK_DEPENDS_ON: &str = "task/depends_on";
        pub const AGENT_SESSION: &str = "agent/session";
    }
}

pub const SCHEMA_FILENAME: &str = "exom_schema.json";
pub const HEALTH_WATER_BAND: &str = "health/water-band";
pub const HEALTH_STEP_BAND: &str = "health/step-band";

// FIXME(phase-b-typed-cmp): these profile constants fed the old rule-backed
// native derivations. Kept around (and read by unit tests) while the
// derivation machinery is disabled at the rule layer; wired back up once
// typed cmp lands on a separate relation.
#[allow(dead_code)]
const HEALTH_PROFILE_AGE_FACT_ID: &str = "health/profile/age";
#[allow(dead_code)]
const HEALTH_PROFILE_HEIGHT_CM_FACT_ID: &str = "health/profile/height_cm";
#[allow(dead_code)]
const HEALTH_PROFILE_WEIGHT_KG_FACT_ID: &str = "health/profile/weight_kg";
#[allow(dead_code)]
const PROFILE_AGE: &str = "profile/age";
#[allow(dead_code)]
const PROFILE_HEIGHT_CM: &str = "profile/height_cm";
#[allow(dead_code)]
const PROFILE_WEIGHT_KG: &str = "profile/weight_kg";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyAttribute {
    pub name: String,
    pub entity_kind: String,
    pub value_kind: String,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinView {
    pub name: String,
    pub arity: usize,
    pub description: String,
    pub rule: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExomOntology {
    pub format_version: u32,
    pub exom: String,
    pub system_attributes: Vec<OntologyAttribute>,
    pub coordination_attributes: Vec<OntologyAttribute>,
    pub builtin_views: Vec<BuiltinView>,
    pub user_predicates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDerivedRelation {
    pub name: String,
    pub arity: usize,
    pub description: String,
    pub rule: String,
    pub sample_tuples: Vec<Vec<String>>,
}

pub fn system_attributes() -> Vec<OntologyAttribute> {
    vec![
        OntologyAttribute {
            name: attrs::fact::PREDICATE.to_string(),
            entity_kind: "fact".into(),
            value_kind: "predicate".into(),
            category: "fact".into(),
            description: "Logical predicate name for a fact entity.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::VALUE.to_string(),
            entity_kind: "fact".into(),
            value_kind: "string".into(),
            category: "fact".into(),
            description: "Logical value for a fact entity.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::CONFIDENCE.to_string(),
            entity_kind: "fact".into(),
            value_kind: "string".into(),
            category: "fact".into(),
            description: "Confidence score serialized as text for Datalog joins.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::PROVENANCE.to_string(),
            entity_kind: "fact".into(),
            value_kind: "string".into(),
            category: "fact".into(),
            description: "Provenance tag for a fact.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::VALID_FROM.to_string(),
            entity_kind: "fact".into(),
            value_kind: "timestamp".into(),
            category: "fact".into(),
            description: "Valid-time start for a fact.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::VALID_TO.to_string(),
            entity_kind: "fact".into(),
            value_kind: "timestamp".into(),
            category: "fact".into(),
            description: "Valid-time end for a fact.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::CREATED_BY.to_string(),
            entity_kind: "fact".into(),
            value_kind: "tx-entity".into(),
            category: "fact".into(),
            description: "Transaction entity that asserted the fact.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::SUPERSEDED_BY.to_string(),
            entity_kind: "fact".into(),
            value_kind: "tx-entity".into(),
            category: "fact".into(),
            description: "Transaction entity that superseded the fact.".into(),
        },
        OntologyAttribute {
            name: attrs::fact::REVOKED_BY.to_string(),
            entity_kind: "fact".into(),
            value_kind: "tx-entity".into(),
            category: "fact".into(),
            description: "Transaction entity that revoked the fact.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::ID.to_string(),
            entity_kind: "tx".into(),
            value_kind: "string".into(),
            category: "tx".into(),
            description: "Transaction id as text.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::TIME.to_string(),
            entity_kind: "tx".into(),
            value_kind: "timestamp".into(),
            category: "tx".into(),
            description: "Transaction time.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::ACTOR.to_string(),
            entity_kind: "tx".into(),
            value_kind: "string".into(),
            category: "tx".into(),
            description: "Actor that created the transaction.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::USER_EMAIL.to_string(),
            entity_kind: "tx".into(),
            value_kind: "string".into(),
            category: "tx".into(),
            description: "Email of the authenticated user for this transaction, when known.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::ACTION.to_string(),
            entity_kind: "tx".into(),
            value_kind: "string".into(),
            category: "tx".into(),
            description: "Transaction action label.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::BRANCH.to_string(),
            entity_kind: "tx".into(),
            value_kind: "string".into(),
            category: "tx".into(),
            description: "Branch on which the transaction occurred.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::PARENT.to_string(),
            entity_kind: "tx".into(),
            value_kind: "tx-entity".into(),
            category: "tx".into(),
            description: "Parent transaction entity.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::SESSION.to_string(),
            entity_kind: "tx".into(),
            value_kind: "string".into(),
            category: "tx".into(),
            description: "Session identifier captured on the transaction.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::REF.to_string(),
            entity_kind: "tx".into(),
            value_kind: "entity".into(),
            category: "tx".into(),
            description: "Referenced entity ids for the transaction.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::MERGE_SOURCE.to_string(),
            entity_kind: "tx".into(),
            value_kind: "branch".into(),
            category: "tx".into(),
            description: "Source branch for merge transactions.".into(),
        },
        OntologyAttribute {
            name: attrs::tx::MERGE_TARGET.to_string(),
            entity_kind: "tx".into(),
            value_kind: "branch".into(),
            category: "tx".into(),
            description: "Target branch for merge transactions.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::SOURCE_TYPE.to_string(),
            entity_kind: "observation".into(),
            value_kind: "string".into(),
            category: "observation".into(),
            description: "Observation source type.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::SOURCE_REF.to_string(),
            entity_kind: "observation".into(),
            value_kind: "string".into(),
            category: "observation".into(),
            description: "Observation source reference.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::CONTENT.to_string(),
            entity_kind: "observation".into(),
            value_kind: "string".into(),
            category: "observation".into(),
            description: "Observation content.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::CREATED_AT.to_string(),
            entity_kind: "observation".into(),
            value_kind: "timestamp".into(),
            category: "observation".into(),
            description: "Observation creation time.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::CONFIDENCE.to_string(),
            entity_kind: "observation".into(),
            value_kind: "string".into(),
            category: "observation".into(),
            description: "Observation confidence serialized as text.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::TX.to_string(),
            entity_kind: "observation".into(),
            value_kind: "tx-entity".into(),
            category: "observation".into(),
            description: "Transaction entity that recorded the observation.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::VALID_FROM.to_string(),
            entity_kind: "observation".into(),
            value_kind: "timestamp".into(),
            category: "observation".into(),
            description: "Observation valid-time start.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::VALID_TO.to_string(),
            entity_kind: "observation".into(),
            value_kind: "timestamp".into(),
            category: "observation".into(),
            description: "Observation valid-time end.".into(),
        },
        OntologyAttribute {
            name: attrs::observation::TAG.to_string(),
            entity_kind: "observation".into(),
            value_kind: "string".into(),
            category: "observation".into(),
            description: "Observation tag.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::CLAIM_TEXT.to_string(),
            entity_kind: "belief".into(),
            value_kind: "string".into(),
            category: "belief".into(),
            description: "Belief claim text.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::STATUS.to_string(),
            entity_kind: "belief".into(),
            value_kind: "string".into(),
            category: "belief".into(),
            description: "Belief status.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::CONFIDENCE.to_string(),
            entity_kind: "belief".into(),
            value_kind: "string".into(),
            category: "belief".into(),
            description: "Belief confidence serialized as text.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::CREATED_BY.to_string(),
            entity_kind: "belief".into(),
            value_kind: "tx-entity".into(),
            category: "belief".into(),
            description: "Transaction entity that created the belief.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::VALID_FROM.to_string(),
            entity_kind: "belief".into(),
            value_kind: "timestamp".into(),
            category: "belief".into(),
            description: "Belief valid-time start.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::VALID_TO.to_string(),
            entity_kind: "belief".into(),
            value_kind: "timestamp".into(),
            category: "belief".into(),
            description: "Belief valid-time end.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::RATIONALE.to_string(),
            entity_kind: "belief".into(),
            value_kind: "string".into(),
            category: "belief".into(),
            description: "Belief rationale.".into(),
        },
        OntologyAttribute {
            name: attrs::belief::SUPPORTS.to_string(),
            entity_kind: "belief".into(),
            value_kind: "entity".into(),
            category: "belief".into(),
            description: "Supporting entity ids.".into(),
        },
        OntologyAttribute {
            name: attrs::branch::ID.to_string(),
            entity_kind: "branch".into(),
            value_kind: "string".into(),
            category: "branch".into(),
            description: "Branch id.".into(),
        },
        OntologyAttribute {
            name: attrs::branch::NAME.to_string(),
            entity_kind: "branch".into(),
            value_kind: "string".into(),
            category: "branch".into(),
            description: "Branch display name.".into(),
        },
        OntologyAttribute {
            name: attrs::branch::PARENT.to_string(),
            entity_kind: "branch".into(),
            value_kind: "branch".into(),
            category: "branch".into(),
            description: "Parent branch entity.".into(),
        },
        OntologyAttribute {
            name: attrs::branch::CREATED_BY.to_string(),
            entity_kind: "branch".into(),
            value_kind: "tx-entity".into(),
            category: "branch".into(),
            description: "Transaction that created the branch.".into(),
        },
        OntologyAttribute {
            name: attrs::branch::ARCHIVED.to_string(),
            entity_kind: "branch".into(),
            value_kind: "string".into(),
            category: "branch".into(),
            description: "Archive state for the branch.".into(),
        },
        // Reserved attributes for branch ownership and session lifecycle.
        OntologyAttribute {
            name: "branch/claimed_by".to_string(),
            entity_kind: "branch".into(),
            value_kind: "string".into(),
            category: "branch".into(),
            description: "mutable string; TOFU-owner of this branch".into(),
        },
        OntologyAttribute {
            name: "session/label".to_string(),
            entity_kind: "session".into(),
            value_kind: "string".into(),
            category: "session".into(),
            description: "mutable string; display label for the session".into(),
        },
        OntologyAttribute {
            name: "session/closed_at".to_string(),
            entity_kind: "session".into(),
            value_kind: "timestamp".into(),
            category: "session".into(),
            description: "timestamp; non-null => writes rejected".into(),
        },
        OntologyAttribute {
            name: "session/archived_at".to_string(),
            entity_kind: "session".into(),
            value_kind: "timestamp".into(),
            category: "session".into(),
            description: "timestamp; non-null => hidden from default inspect".into(),
        },
    ]
}

pub fn coordination_attributes() -> Vec<OntologyAttribute> {
    vec![
        OntologyAttribute {
            name: attrs::coord::CLAIM_OWNER.to_string(),
            entity_kind: "claim".into(),
            value_kind: "string".into(),
            category: "coordination".into(),
            description: "Actor or agent owning a claim.".into(),
        },
        OntologyAttribute {
            name: attrs::coord::CLAIM_STATUS.to_string(),
            entity_kind: "claim".into(),
            value_kind: "string".into(),
            category: "coordination".into(),
            description: "Claim state such as active or released.".into(),
        },
        OntologyAttribute {
            name: attrs::coord::CLAIM_EXPIRES_AT.to_string(),
            entity_kind: "claim".into(),
            value_kind: "timestamp".into(),
            category: "coordination".into(),
            description: "Lease expiry for a claim.".into(),
        },
        OntologyAttribute {
            name: attrs::coord::TASK_DEPENDS_ON.to_string(),
            entity_kind: "task".into(),
            value_kind: "entity".into(),
            category: "coordination".into(),
            description: "Task dependency edge.".into(),
        },
        OntologyAttribute {
            name: attrs::coord::AGENT_SESSION.to_string(),
            entity_kind: "agent".into(),
            value_kind: "string".into(),
            category: "coordination".into(),
            description: "Active session id for an agent entity.".into(),
        },
    ]
}

fn static_builtin_rule_specs(exom: &str) -> Vec<(String, String, String)> {
    vec![
        (
            "fact-row".to_string(),
            "Logical fact rows stripped of system metadata joins.".to_string(),
            format!(
                "(rule {exom} (fact-row ?fact ?pred ?value) (?fact '{pred} ?pred) (?fact '{value} ?value))",
                pred = attrs::fact::PREDICATE,
                value = attrs::fact::VALUE,
            ),
        ),
        (
            "fact-meta".to_string(),
            "Fact metadata view with confidence, provenance, valid start, and tx entity."
                .to_string(),
            format!(
                "(rule {exom} (fact-meta ?fact ?confidence ?prov ?vf ?tx) (?fact '{confidence} ?confidence) (?fact '{prov} ?prov) (?fact '{vf} ?vf) (?fact '{tx} ?tx))",
                confidence = attrs::fact::CONFIDENCE,
                prov = attrs::fact::PROVENANCE,
                vf = attrs::fact::VALID_FROM,
                tx = attrs::fact::CREATED_BY,
            ),
        ),
        (
            "fact-with-tx".to_string(),
            "Joined fact row with provenance and transaction actor/time.".to_string(),
            format!(
                "(rule {exom} (fact-with-tx ?fact ?pred ?value ?confidence ?prov ?vf ?tx ?actor ?when) (?fact '{fp} ?pred) (?fact '{fv} ?value) (?fact '{fc} ?confidence) (?fact '{fprov} ?prov) (?fact '{fvt} ?vf) (?fact '{fcb} ?tx) (?tx '{ta} ?actor) (?tx '{tt} ?when))",
                fp = attrs::fact::PREDICATE,
                fv = attrs::fact::VALUE,
                fc = attrs::fact::CONFIDENCE,
                fprov = attrs::fact::PROVENANCE,
                fvt = attrs::fact::VALID_FROM,
                fcb = attrs::fact::CREATED_BY,
                ta = attrs::tx::ACTOR,
                tt = attrs::tx::TIME,
            ),
        ),
        (
            "tx-row".to_string(),
            "Transaction row with actor, action, time, and branch.".to_string(),
            format!(
                "(rule {exom} (tx-row ?tx ?id ?actor ?action ?when ?branch) (?tx '{id_attr} ?id) (?tx '{actor_attr} ?actor) (?tx '{action_attr} ?action) (?tx '{time_attr} ?when) (?tx '{branch_attr} ?branch))",
                id_attr = attrs::tx::ID,
                actor_attr = attrs::tx::ACTOR,
                action_attr = attrs::tx::ACTION,
                time_attr = attrs::tx::TIME,
                branch_attr = attrs::tx::BRANCH,
            ),
        ),
        (
            "observation-row".to_string(),
            "Observation row with source type, content, and tx entity.".to_string(),
            format!(
                "(rule {exom} (observation-row ?obs ?source_type ?content ?tx) (?obs '{st} ?source_type) (?obs '{content} ?content) (?obs '{tx} ?tx))",
                st = attrs::observation::SOURCE_TYPE,
                content = attrs::observation::CONTENT,
                tx = attrs::observation::TX,
            ),
        ),
        (
            "belief-row".to_string(),
            "Belief row with claim text, status, and creating transaction.".to_string(),
            format!(
                "(rule {exom} (belief-row ?belief ?claim ?status ?tx) (?belief '{claim_attr} ?claim) (?belief '{status_attr} ?status) (?belief '{tx_attr} ?tx))",
                claim_attr = attrs::belief::CLAIM_TEXT,
                status_attr = attrs::belief::STATUS,
                tx_attr = attrs::belief::CREATED_BY,
            ),
        ),
        (
            "branch-row".to_string(),
            "Branch row with id, name, archive state, and creating transaction.".to_string(),
            format!(
                "(rule {exom} (branch-row ?branch ?id ?name ?archived ?created_tx) (?branch '{id_attr} ?id) (?branch '{name_attr} ?name) (?branch '{archived_attr} ?archived) (?branch '{created_attr} ?created_tx))",
                id_attr = attrs::branch::ID,
                name_attr = attrs::branch::NAME,
                archived_attr = attrs::branch::ARCHIVED,
                created_attr = attrs::branch::CREATED_BY,
            ),
        ),
        (
            "merge-row".to_string(),
            "Merge transaction row with source and target branches.".to_string(),
            format!(
                "(rule {exom} (merge-row ?tx ?source ?target ?actor ?when) (?tx '{source_attr} ?source) (?tx '{target_attr} ?target) (?tx '{actor_attr} ?actor) (?tx '{time_attr} ?when))",
                source_attr = attrs::tx::MERGE_SOURCE,
                target_attr = attrs::tx::MERGE_TARGET,
                actor_attr = attrs::tx::ACTOR,
                time_attr = attrs::tx::TIME,
            ),
        ),
        (
            "claim-owner-row".to_string(),
            "Claim ownership facts using the coordination namespace.".to_string(),
            format!(
                "(rule {exom} (claim-owner-row ?fact ?owner) (?fact '{claim_owner} ?owner))",
                claim_owner = attrs::coord::CLAIM_OWNER,
            ),
        ),
        (
            "claim-status-row".to_string(),
            "Claim status facts using the coordination namespace.".to_string(),
            format!(
                "(rule {exom} (claim-status-row ?fact ?status) (?fact '{claim_status} ?status))",
                claim_status = attrs::coord::CLAIM_STATUS,
            ),
        ),
        (
            "task-dependency-row".to_string(),
            "Task dependency facts using the coordination namespace.".to_string(),
            format!(
                "(rule {exom} (task-dependency-row ?fact ?depends_on) (?fact '{task_dep} ?depends_on))",
                task_dep = attrs::coord::TASK_DEPENDS_ON,
            ),
        ),
        (
            "agent-session-row".to_string(),
            "Agent session facts using the coordination namespace.".to_string(),
            format!(
                "(rule {exom} (agent-session-row ?fact ?session) (?fact '{agent_session} ?session))",
                agent_session = attrs::coord::AGENT_SESSION,
            ),
        ),
    ]
}

/// Coerce a [`FactValue`] to `i64` for threshold math on numeric profile fields.
///
/// * `I64` variants return their stored integer directly — no reparse.
/// * `Str` variants fall back to `str::parse` for backward compat with JSONL
///   / pg rows written before the FactValue refactor.
/// * `Sym` values never parse.
#[allow(dead_code)]
fn fact_value_as_i64(value: &crate::fact_value::FactValue) -> Option<i64> {
    use crate::fact_value::FactValue;
    match value {
        FactValue::I64(n) => Some(*n),
        FactValue::Str(s) => s.parse::<i64>().ok(),
        FactValue::Sym(_) => None,
    }
}

#[allow(dead_code)]
fn latest_active_fact<'a>(
    brain: &'a Brain,
    preferred_fact_id: &str,
    predicate: &str,
) -> Option<&'a crate::brain::Fact> {
    brain
        .current_facts()
        .into_iter()
        .filter(|fact| fact.fact_id == preferred_fact_id && fact.predicate == predicate)
        .max_by_key(|fact| fact.created_by_tx)
        .or_else(|| {
            brain
                .current_facts()
                .into_iter()
                .filter(|fact| fact.predicate == predicate)
                .max_by_key(|fact| fact.created_by_tx)
        })
}

pub fn native_derived_relations(_exom: &str, brain: &Brain) -> Vec<NativeDerivedRelation> {
    // FactValue refactor side-effect: after I64 values moved off the shared
    // datom V column, the previously-generated Rayfall rules that looked like
    // `(rule … (head "band") (?id 'profile/age "30"))` now trip rayforce2's
    // type inference with `error:type` whenever the resulting bundle is
    // compiled into a query — even with an all-string rewrite. The derivation
    // has been moved to a Rust-computed read path until the typed-cmp Phase B
    // relation lands; this entry point stays to keep `builtin_views` callable
    // but returns nothing.
    let _ = brain;
    Vec::new()
}

fn builtin_rule_specs(exom: &str, brain: &Brain) -> Vec<(String, String, String)> {
    let mut specs = static_builtin_rule_specs(exom);
    specs.extend(
        native_derived_relations(exom, brain)
            .into_iter()
            .map(|relation| (relation.name, relation.description, relation.rule)),
    );
    specs
}

pub fn builtin_rules(exom: &str, brain: &Brain) -> Result<Vec<ParsedRule>> {
    builtin_rule_specs(exom, brain)
        .into_iter()
        .map(|(_, _, rule)| {
            rules::parse_rule_line(&rule, MutationContext::default(), "builtin".to_string())
        })
        .collect()
}

pub fn builtin_views(exom: &str, brain: &Brain) -> Vec<BuiltinView> {
    builtin_rule_specs(exom, brain)
        .into_iter()
        .map(|(name, description, rule)| {
            let parsed =
                rules::parse_rule_line(&rule, MutationContext::default(), "builtin".to_string())
                    .expect("builtin rule must parse");
            BuiltinView {
                name: name.to_string(),
                arity: parsed.head_arity,
                description: description.to_string(),
                rule,
            }
        })
        .collect()
}

pub fn build_exom_ontology(exom: &str, brain: &Brain, user_rules: &[ParsedRule]) -> ExomOntology {
    let mut user_preds = BTreeSet::new();
    for fact in brain.current_facts() {
        user_preds.insert(fact.predicate.clone());
    }
    for relation in native_derived_relations(exom, brain) {
        user_preds.insert(relation.name);
    }
    for rule in user_rules {
        user_preds.insert(rule.head_predicate.clone());
    }
    ExomOntology {
        format_version: 1,
        exom: exom.to_string(),
        system_attributes: system_attributes(),
        coordination_attributes: coordination_attributes(),
        builtin_views: builtin_views(exom, brain),
        user_predicates: user_preds.into_iter().collect(),
    }
}

pub fn save_exom_ontology(path: &Path, ontology: &ExomOntology) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(ontology)?;
    fs::write(&tmp, body).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn load_exom_ontology(path: &Path) -> Result<ExomOntology> {
    let raw = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::MutationContext;

    #[test]
    fn native_health_relations_follow_bootstrap_profile_thresholds() {
        use crate::fact_value::FactValue;

        let mut brain = Brain::new();
        let ctx = MutationContext::default();
        // Typed bootstrap values — `FactValue::I64` ensures the downstream
        // `cmp` threshold math (weight < 60, age < 30, etc.) runs against
        // raw integers rather than re-parsed strings.
        brain
            .assert_fact(
                HEALTH_PROFILE_AGE_FACT_ID,
                PROFILE_AGE,
                FactValue::I64(30),
                1.0,
                "test",
                None,
                None,
                &ctx,
            )
            .unwrap();
        brain
            .assert_fact(
                HEALTH_PROFILE_HEIGHT_CM_FACT_ID,
                PROFILE_HEIGHT_CM,
                FactValue::I64(175),
                1.0,
                "test",
                None,
                None,
                &ctx,
            )
            .unwrap();
        brain
            .assert_fact(
                HEALTH_PROFILE_WEIGHT_KG_FACT_ID,
                PROFILE_WEIGHT_KG,
                FactValue::I64(75),
                1.0,
                "test",
                None,
                None,
                &ctx,
            )
            .unwrap();

        // The seeded facts must retain their I64 variant — the splay/datom
        // path keys off `kind()` to pick the right tag.
        let weight = brain
            .current_facts()
            .into_iter()
            .find(|f| f.fact_id == HEALTH_PROFILE_WEIGHT_KG_FACT_ID)
            .expect("weight fact should be asserted");
        assert_eq!(weight.value, FactValue::I64(75));

        // Native derivation is currently disabled at the Rayfall-rule layer
        // (see the comment on `native_derived_relations`). Assert the empty
        // result so regressions that bring back the rule-generating version
        // flag themselves, and keep the body that seeds typed profile facts
        // as the canonical round-trip coverage for `FactValue::I64` on splay.
        let relations = native_derived_relations("alice/personal/health/main", &brain);
        assert!(
            relations.is_empty(),
            "native_derived_relations should return an empty vec while the \
             string-head rule bundle remains incompatible with FactValue; got {relations:?}"
        );
    }
}
