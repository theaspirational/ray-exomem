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

fn builtin_rule_specs(exom: &str) -> Vec<(&'static str, &'static str, String)> {
    vec![
        (
            "fact-row",
            "Logical fact rows stripped of system metadata joins.",
            format!(
                "(rule {exom} (fact-row ?fact ?pred ?value) (?fact '{pred} ?pred) (?fact '{value} ?value))",
                pred = attrs::fact::PREDICATE,
                value = attrs::fact::VALUE,
            ),
        ),
        (
            "fact-meta",
            "Fact metadata view with confidence, provenance, valid start, and tx entity.",
            format!(
                "(rule {exom} (fact-meta ?fact ?confidence ?prov ?vf ?tx) (?fact '{confidence} ?confidence) (?fact '{prov} ?prov) (?fact '{vf} ?vf) (?fact '{tx} ?tx))",
                confidence = attrs::fact::CONFIDENCE,
                prov = attrs::fact::PROVENANCE,
                vf = attrs::fact::VALID_FROM,
                tx = attrs::fact::CREATED_BY,
            ),
        ),
        (
            "fact-with-tx",
            "Joined fact row with provenance and transaction actor/time.",
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
            "tx-row",
            "Transaction row with actor, action, time, and branch.",
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
            "observation-row",
            "Observation row with source type, content, and tx entity.",
            format!(
                "(rule {exom} (observation-row ?obs ?source_type ?content ?tx) (?obs '{st} ?source_type) (?obs '{content} ?content) (?obs '{tx} ?tx))",
                st = attrs::observation::SOURCE_TYPE,
                content = attrs::observation::CONTENT,
                tx = attrs::observation::TX,
            ),
        ),
        (
            "belief-row",
            "Belief row with claim text, status, and creating transaction.",
            format!(
                "(rule {exom} (belief-row ?belief ?claim ?status ?tx) (?belief '{claim_attr} ?claim) (?belief '{status_attr} ?status) (?belief '{tx_attr} ?tx))",
                claim_attr = attrs::belief::CLAIM_TEXT,
                status_attr = attrs::belief::STATUS,
                tx_attr = attrs::belief::CREATED_BY,
            ),
        ),
        (
            "branch-row",
            "Branch row with id, name, archive state, and creating transaction.",
            format!(
                "(rule {exom} (branch-row ?branch ?id ?name ?archived ?created_tx) (?branch '{id_attr} ?id) (?branch '{name_attr} ?name) (?branch '{archived_attr} ?archived) (?branch '{created_attr} ?created_tx))",
                id_attr = attrs::branch::ID,
                name_attr = attrs::branch::NAME,
                archived_attr = attrs::branch::ARCHIVED,
                created_attr = attrs::branch::CREATED_BY,
            ),
        ),
        (
            "merge-row",
            "Merge transaction row with source and target branches.",
            format!(
                "(rule {exom} (merge-row ?tx ?source ?target ?actor ?when) (?tx '{source_attr} ?source) (?tx '{target_attr} ?target) (?tx '{actor_attr} ?actor) (?tx '{time_attr} ?when))",
                source_attr = attrs::tx::MERGE_SOURCE,
                target_attr = attrs::tx::MERGE_TARGET,
                actor_attr = attrs::tx::ACTOR,
                time_attr = attrs::tx::TIME,
            ),
        ),
        (
            "claim-owner-row",
            "Claim ownership facts using the coordination namespace.",
            format!(
                "(rule {exom} (claim-owner-row ?fact ?owner) (?fact '{claim_owner} ?owner))",
                claim_owner = attrs::coord::CLAIM_OWNER,
            ),
        ),
        (
            "claim-status-row",
            "Claim status facts using the coordination namespace.",
            format!(
                "(rule {exom} (claim-status-row ?fact ?status) (?fact '{claim_status} ?status))",
                claim_status = attrs::coord::CLAIM_STATUS,
            ),
        ),
        (
            "task-dependency-row",
            "Task dependency facts using the coordination namespace.",
            format!(
                "(rule {exom} (task-dependency-row ?fact ?depends_on) (?fact '{task_dep} ?depends_on))",
                task_dep = attrs::coord::TASK_DEPENDS_ON,
            ),
        ),
        (
            "agent-session-row",
            "Agent session facts using the coordination namespace.",
            format!(
                "(rule {exom} (agent-session-row ?fact ?session) (?fact '{agent_session} ?session))",
                agent_session = attrs::coord::AGENT_SESSION,
            ),
        ),
    ]
}

pub fn builtin_rules(exom: &str) -> Result<Vec<ParsedRule>> {
    builtin_rule_specs(exom)
        .into_iter()
        .map(|(_, _, rule)| {
            rules::parse_rule_line(&rule, MutationContext::default(), "builtin".to_string())
        })
        .collect()
}

pub fn builtin_views(exom: &str) -> Vec<BuiltinView> {
    builtin_rule_specs(exom)
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
    for rule in user_rules {
        user_preds.insert(rule.head_predicate.clone());
    }
    ExomOntology {
        format_version: 1,
        exom: exom.to_string(),
        system_attributes: system_attributes(),
        coordination_attributes: coordination_attributes(),
        builtin_views: builtin_views(exom),
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
