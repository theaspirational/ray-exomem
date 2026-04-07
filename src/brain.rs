//! Datomic-style persistent brain layer for LLM agents.
//!
//! All state is immutable and append-only. Every mutation is recorded as a
//! transaction, enabling time-travel queries (`as_of`, `history`, `explain`).
//! Persistence uses rayforce2 splayed columnar tables.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::context::MutationContext;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

pub type TxId = u64;
pub type EntityId = String;

#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub obs_id: EntityId,
    pub source_type: String,
    pub source_ref: String,
    pub content: String,
    pub created_at: String,
    pub confidence: f64,
    pub tx_id: TxId,
    pub tags: Vec<String>,
    /// When this observation became valid in the real world (ISO 8601).
    pub valid_from: String,
    /// When this observation ceased being valid (ISO 8601). None = still valid.
    pub valid_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub fact_id: EntityId,
    pub predicate: String,
    pub value: String,
    pub created_at: String,
    pub created_by_tx: TxId,
    pub superseded_by_tx: Option<TxId>,
    pub revoked_by_tx: Option<TxId>,
    pub confidence: f64,
    pub provenance: String,
    /// When this fact became true in the real world (ISO 8601).
    pub valid_from: String,
    /// When this fact ceased being true (ISO 8601). None = still true / open-ended.
    pub valid_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Belief {
    pub belief_id: EntityId,
    pub claim_text: String,
    pub status: BeliefStatus,
    pub confidence: f64,
    pub supported_by: Vec<EntityId>,
    /// Transaction that created this belief (transaction-time axis).
    pub created_by_tx: TxId,
    /// When this belief became true in the real world (ISO 8601, valid-time axis).
    pub valid_from: String,
    /// When this belief ceased being true (ISO 8601). None = still true.
    pub valid_to: Option<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BeliefStatus {
    Active,
    Superseded,
    Revoked,
}

impl std::fmt::Display for BeliefStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BeliefStatus::Active => write!(f, "active"),
            BeliefStatus::Superseded => write!(f, "superseded"),
            BeliefStatus::Revoked => write!(f, "revoked"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tx {
    pub tx_id: TxId,
    pub tx_time: String,
    pub actor: String,
    pub action: TxAction,
    pub refs: Vec<EntityId>,
    pub note: String,
    pub parent_tx_id: Option<TxId>,
    pub branch_id: String,
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TxAction {
    AssertObservation,
    AssertFact,
    RetractFact,
    ReviseBelief,
    CreateBranch,
}

impl std::fmt::Display for TxAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxAction::AssertObservation => write!(f, "assert-observation"),
            TxAction::AssertFact => write!(f, "assert-fact"),
            TxAction::RetractFact => write!(f, "retract-fact"),
            TxAction::ReviseBelief => write!(f, "revise-belief"),
            TxAction::CreateBranch => write!(f, "create-branch"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub branch_id: String,
    pub name: String,
    pub parent_branch_id: Option<String>,
    pub created_tx_id: TxId,
}

// ---------------------------------------------------------------------------
// Brain — the main API surface
// ---------------------------------------------------------------------------

/// Which table was affected by a mutation.
enum DirtyTable {
    Tx,
    Fact,
    Observation,
    Belief,
    Branch,
}

#[derive(Clone)]
pub struct Brain {
    observations: Vec<Observation>,
    facts: Vec<Fact>,
    beliefs: Vec<Belief>,
    transactions: Vec<Tx>,
    branches: Vec<Branch>,
    next_tx: TxId,
    current_branch: String,
    /// If set, the exom directory for splayed table persistence.
    data_dir: Option<PathBuf>,
    /// Path to the shared symbol table file.
    sym_path: Option<PathBuf>,
}

impl Default for Brain {
    fn default() -> Self {
        Self::new()
    }
}

impl Brain {
    /// Create a new in-memory brain (no persistence).
    pub fn new() -> Self {
        let main_branch = Branch {
            branch_id: "main".into(),
            name: "main".into(),
            parent_branch_id: None,
            created_tx_id: 0,
        };
        Brain {
            observations: Vec::new(),
            facts: Vec::new(),
            beliefs: Vec::new(),
            transactions: Vec::new(),
            branches: vec![main_branch],
            next_tx: 1,
            current_branch: "main".into(),
            data_dir: None,
            sym_path: None,
        }
    }

    /// Open a brain from a splayed table directory. Loads all tables into memory.
    pub fn open_exom(exom_dir: &Path, sym_path: &Path) -> Result<Self> {
        use crate::storage;

        let mut brain = Brain::new();
        brain.data_dir = Some(exom_dir.to_path_buf());
        brain.sym_path = Some(sym_path.to_path_buf());

        let load = |table_name: &str| -> Option<storage::RayObj> {
            let dir = exom_dir.join(table_name);
            if storage::table_exists(&dir) {
                storage::load_table(&dir, sym_path).ok()
            } else {
                None
            }
        };

        if let Some(tbl) = load("tx") {
            brain.transactions = storage::load_txs(&tbl)?;
            if let Some(last) = brain.transactions.last() {
                brain.next_tx = last.tx_id + 1;
            }
        }
        if let Some(tbl) = load("fact") {
            brain.facts = storage::load_facts(&tbl)?;
        }
        if let Some(tbl) = load("observation") {
            brain.observations = storage::load_observations(&tbl)?;
        }
        if let Some(tbl) = load("belief") {
            brain.beliefs = storage::load_beliefs(&tbl)?;
        }
        if let Some(tbl) = load("branch") {
            brain.branches = storage::load_branches(&tbl)?;
            // Ensure "main" branch exists
            if !brain.branches.iter().any(|b| b.branch_id == "main") {
                brain.branches.insert(0, Branch {
                    branch_id: "main".into(),
                    name: "main".into(),
                    parent_branch_id: None,
                    created_tx_id: 0,
                });
            }
        }

        Ok(brain)
    }

    /// Persist all tables to disk. No-op if no data_dir is set.
    pub fn save(&self) -> Result<()> {
        self.persist_table(DirtyTable::Tx)?;
        self.persist_table(DirtyTable::Fact)?;
        self.persist_table(DirtyTable::Observation)?;
        self.persist_table(DirtyTable::Belief)?;
        self.persist_table(DirtyTable::Branch)?;
        Ok(())
    }

    fn persist_table(&self, table: DirtyTable) -> Result<()> {
        use crate::storage;

        let (data_dir, sym_path) = match (&self.data_dir, &self.sym_path) {
            (Some(d), Some(s)) => (d, s),
            _ => return Ok(()), // in-memory mode
        };

        let (name, ray_table) = match table {
            DirtyTable::Tx => ("tx", storage::build_tx_table(&self.transactions)),
            DirtyTable::Fact => ("fact", storage::build_fact_table(&self.facts)),
            DirtyTable::Observation => ("observation", storage::build_observation_table(&self.observations)),
            DirtyTable::Belief => ("belief", storage::build_belief_table(&self.beliefs)),
            DirtyTable::Branch => ("branch", storage::build_branch_table(&self.branches)),
        };

        let dir = data_dir.join(name);
        storage::save_table(&ray_table, &dir, sym_path)?;
        storage::sym_save(sym_path)?;
        Ok(())
    }

    /// Allocate a new transaction, returning (tx_id, tx_time) so callers reuse the timestamp.
    fn alloc_tx(
        &mut self,
        action: TxAction,
        refs: Vec<EntityId>,
        note: &str,
        ctx: &MutationContext,
    ) -> Result<(TxId, String)> {
        let tx_id = self.next_tx;
        self.next_tx += 1;
        let tx_time = now_iso();
        let tx = Tx {
            tx_id,
            tx_time: tx_time.clone(),
            actor: ctx.actor.clone(),
            action,
            refs,
            note: note.into(),
            parent_tx_id: self.transactions.last().map(|t| t.tx_id),
            branch_id: self.current_branch.clone(),
            session: ctx.session.clone(),
        };
        self.transactions.push(tx);
        self.persist_table(DirtyTable::Tx)?;
        Ok((tx_id, tx_time))
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    pub fn assert_observation(
        &mut self,
        obs_id: &str,
        source_type: &str,
        source_ref: &str,
        content: &str,
        confidence: f64,
        tags: Vec<String>,
        valid_from: Option<&str>,
        valid_to: Option<&str>,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        let (tx_id, tx_time) = self.alloc_tx(
            TxAction::AssertObservation,
            vec![obs_id.into()],
            &format!("observe: {}", obs_id),
            ctx,
        )?;
        let obs = Observation {
            obs_id: obs_id.into(),
            source_type: source_type.into(),
            source_ref: source_ref.into(),
            content: content.into(),
            created_at: tx_time.clone(),
            confidence,
            tx_id,
            tags,
            valid_from: valid_from.unwrap_or(&tx_time).to_string(),
            valid_to: valid_to.map(|s| s.to_string()),
        };
        self.observations.push(obs);
        self.persist_table(DirtyTable::Observation)?;
        Ok(tx_id)
    }

    pub fn assert_fact(
        &mut self,
        fact_id: &str,
        predicate: &str,
        value: &str,
        confidence: f64,
        provenance: &str,
        valid_from: Option<&str>,
        valid_to: Option<&str>,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        let (tx_id, tx_time) = self.alloc_tx(
            TxAction::AssertFact,
            vec![fact_id.into()],
            &format!("assert: {} = {}", predicate, value),
            ctx,
        )?;
        let fact = Fact {
            fact_id: fact_id.into(),
            predicate: predicate.into(),
            value: value.into(),
            created_at: tx_time.clone(),
            created_by_tx: tx_id,
            superseded_by_tx: None,
            revoked_by_tx: None,
            confidence,
            provenance: provenance.into(),
            valid_from: valid_from.unwrap_or(&tx_time).to_string(),
            valid_to: valid_to.map(|s| s.to_string()),
        };
        self.facts.push(fact);
        self.persist_table(DirtyTable::Fact)?;
        Ok(tx_id)
    }

    pub fn retract_fact(&mut self, fact_id: &str, ctx: &MutationContext) -> Result<TxId> {
        if !self
            .facts
            .iter()
            .any(|f| f.fact_id == fact_id && f.revoked_by_tx.is_none())
        {
            bail!("no active fact with id '{}'", fact_id);
        }
        let (tx_id, tx_time) = self.alloc_tx(
            TxAction::RetractFact,
            vec![fact_id.into()],
            &format!("retract: {}", fact_id),
            ctx,
        )?;
        if let Some(f) = self.facts.iter_mut().find(|f| f.fact_id == fact_id && f.revoked_by_tx.is_none()) {
            f.revoked_by_tx = Some(tx_id);
            if f.valid_to.is_none() {
                f.valid_to = Some(tx_time);
            }
        }
        self.persist_table(DirtyTable::Fact)?;
        Ok(tx_id)
    }

    pub fn retract_fact_exact(
        &mut self,
        fact_id: &str,
        predicate: &str,
        value: &str,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        let matching_ids: Vec<String> = self
            .facts
            .iter()
            .filter(|f| {
                f.revoked_by_tx.is_none()
                    && f.fact_id == fact_id
                    && f.predicate == predicate
                    && f.value == value
            })
            .map(|f| f.fact_id.clone())
            .collect();

        if matching_ids.is_empty() {
            bail!(
                "no active fact matching ({}, {}, {})",
                fact_id, predicate, value
            );
        }

        let (tx_id, tx_time) = self.alloc_tx(
            TxAction::RetractFact,
            matching_ids.clone(),
            &format!("retract: {} {} {}", fact_id, predicate, value),
            ctx,
        )?;

        for fact in self.facts.iter_mut() {
            if fact.revoked_by_tx.is_none()
                && fact.fact_id == fact_id
                && fact.predicate == predicate
                && fact.value == value
            {
                fact.revoked_by_tx = Some(tx_id);
                if fact.valid_to.is_none() {
                    fact.valid_to = Some(tx_time.clone());
                }
            }
        }

        self.persist_table(DirtyTable::Fact)?;
        Ok(tx_id)
    }

    pub fn revise_belief(
        &mut self,
        belief_id: &str,
        claim_text: &str,
        confidence: f64,
        supported_by: Vec<String>,
        rationale: &str,
        valid_from: Option<&str>,
        valid_to: Option<&str>,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        let (tx_id, tx_time) = self.alloc_tx(
            TxAction::ReviseBelief,
            vec![belief_id.into()],
            &format!("revise: {}", claim_text),
            ctx,
        )?;
        // Supersede any active belief with the same claim_text
        for b in self.beliefs.iter_mut() {
            if b.claim_text == claim_text
                && b.status == BeliefStatus::Active
                && b.belief_id != belief_id
            {
                b.status = BeliefStatus::Superseded;
                if b.valid_to.is_none() {
                    b.valid_to = Some(tx_time.clone());
                }
            }
        }
        let belief = Belief {
            belief_id: belief_id.into(),
            claim_text: claim_text.into(),
            status: BeliefStatus::Active,
            confidence,
            supported_by,
            created_by_tx: tx_id,
            valid_from: valid_from.unwrap_or(&tx_time).to_string(),
            valid_to: valid_to.map(|s| s.to_string()),
            rationale: rationale.into(),
        };
        self.beliefs.push(belief);
        self.persist_table(DirtyTable::Belief)?;
        Ok(tx_id)
    }

    pub fn create_branch(&mut self, branch_id: &str, name: &str, ctx: &MutationContext) -> Result<TxId> {
        let (tx_id, _tx_time) = self.alloc_tx(
            TxAction::CreateBranch,
            vec![branch_id.into()],
            &format!("branch: {}", name),
            ctx,
        )?;
        let branch = Branch {
            branch_id: branch_id.into(),
            name: name.into(),
            parent_branch_id: Some(self.current_branch.clone()),
            created_tx_id: tx_id,
        };
        self.branches.push(branch);
        self.persist_table(DirtyTable::Branch)?;
        Ok(tx_id)
    }

    pub fn switch_branch(&mut self, branch_id: &str) -> Result<()> {
        if !self.branches.iter().any(|b| b.branch_id == branch_id) {
            bail!("unknown branch '{}'", branch_id);
        }
        self.current_branch = branch_id.into();
        Ok(())
    }

    /// Return all observations.
    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }

    /// Total number of facts (including revoked).
    pub fn fact_count(&self) -> usize {
        self.facts.len()
    }

    /// Return all currently-active facts (not revoked).
    pub fn current_facts(&self) -> Vec<&Fact> {
        self.facts
            .iter()
            .filter(|f| f.revoked_by_tx.is_none())
            .collect()
    }

    /// Return all currently-active beliefs.
    pub fn current_beliefs(&self) -> Vec<&Belief> {
        self.beliefs
            .iter()
            .filter(|b| b.status == BeliefStatus::Active)
            .collect()
    }

    /// Return facts as they were at a specific transaction.
    pub fn facts_as_of(&self, tx_id: TxId) -> Vec<&Fact> {
        self.facts
            .iter()
            .filter(|f| f.created_by_tx <= tx_id && f.revoked_by_tx.is_none_or(|rev| rev > tx_id))
            .collect()
    }

    /// Return beliefs as they were known at a specific transaction (transaction-time travel).
    pub fn beliefs_as_of(&self, tx_id: TxId) -> Vec<&Belief> {
        // Pre-compute the latest tx per claim_text within the tx window
        let mut latest_tx_by_claim: std::collections::HashMap<&str, TxId> = std::collections::HashMap::new();
        for b in &self.beliefs {
            if b.created_by_tx <= tx_id {
                let entry = latest_tx_by_claim.entry(&b.claim_text).or_insert(0);
                if b.created_by_tx > *entry {
                    *entry = b.created_by_tx;
                }
            }
        }
        self.beliefs
            .iter()
            .filter(|b| {
                b.created_by_tx <= tx_id
                    && latest_tx_by_claim.get(b.claim_text.as_str()) == Some(&b.created_by_tx)
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Bitemporal queries — valid-time axis
    // -----------------------------------------------------------------------

    /// Return facts that were valid at a given real-world timestamp (current knowledge).
    pub fn facts_valid_at(&self, timestamp: &str) -> Vec<&Fact> {
        self.facts
            .iter()
            .filter(|f| f.revoked_by_tx.is_none() && is_valid_at(&f.valid_from, f.valid_to.as_deref(), timestamp))
            .collect()
    }

    /// Bitemporal: facts as known at tx_id that were valid at the given real-world timestamp.
    pub fn facts_bitemporal(&self, tx_id: TxId, timestamp: &str) -> Vec<&Fact> {
        self.facts
            .iter()
            .filter(|f| {
                f.created_by_tx <= tx_id
                    && f.revoked_by_tx.is_none_or(|rev| rev > tx_id)
                    && is_valid_at(&f.valid_from, f.valid_to.as_deref(), timestamp)
            })
            .collect()
    }

    /// Return beliefs that were valid at a given real-world timestamp (current knowledge).
    pub fn beliefs_valid_at(&self, timestamp: &str) -> Vec<&Belief> {
        self.beliefs
            .iter()
            .filter(|b| b.status == BeliefStatus::Active && is_valid_at(&b.valid_from, b.valid_to.as_deref(), timestamp))
            .collect()
    }

    /// Bitemporal: beliefs as known at tx_id that were valid at the given real-world timestamp.
    pub fn beliefs_bitemporal(&self, tx_id: TxId, timestamp: &str) -> Vec<&Belief> {
        self.beliefs
            .iter()
            .filter(|b| {
                b.created_by_tx <= tx_id
                    && is_valid_at(&b.valid_from, b.valid_to.as_deref(), timestamp)
            })
            .collect()
    }

    /// Return all historical versions of a fact (including revoked).
    pub fn fact_history(&self, fact_id: &str) -> Vec<&Fact> {
        self.facts.iter().filter(|f| f.fact_id == fact_id).collect()
    }

    /// Return all historical versions of a belief.
    pub fn belief_history(&self, claim_text: &str) -> Vec<&Belief> {
        self.beliefs
            .iter()
            .filter(|b| b.claim_text == claim_text)
            .collect()
    }

    /// Explain an entity by showing all transactions that reference it.
    pub fn explain(&self, entity_id: &str) -> Vec<&Tx> {
        self.transactions
            .iter()
            .filter(|tx| tx.refs.iter().any(|r| r == entity_id))
            .collect()
    }

    /// Return the full transaction log.
    pub fn transactions(&self) -> &[Tx] {
        &self.transactions
    }

    /// Current transaction id (latest committed).
    pub fn latest_tx(&self) -> Option<TxId> {
        self.transactions.last().map(|t| t.tx_id)
    }

    // -----------------------------------------------------------------------
    // Demo — prints a narrative showing time-travel memory
    // -----------------------------------------------------------------------

    pub fn run_demo() -> String {
        let mut out = String::new();
        let mut brain = Brain::new();

        out.push_str("=== Brain Demo: Datomic-style Time-Travel Memory ===\n\n");

        // Step 1: Assert facts
        out.push_str("-- Step 1: Assert two facts --\n");
        let tx1 = brain
            .assert_fact("f1", "sky-color", "blue", 0.9, "observation", None, None, &MutationContext::default())
            .unwrap();
        let tx2 = brain
            .assert_fact("f2", "grass-color", "green", 0.85, "observation", None, None, &MutationContext::default())
            .unwrap();
        out.push_str(&format!("  tx{}: assert f1 (sky-color = blue)\n", tx1));
        out.push_str(&format!("  tx{}: assert f2 (grass-color = green)\n", tx2));
        out.push_str(&format!(
            "  current facts: {}\n\n",
            fmt_facts(&brain.current_facts())
        ));

        // Step 2: Assert a belief
        out.push_str("-- Step 2: Assert a belief --\n");
        let tx3 = brain
            .revise_belief(
                "b1",
                "the sky is blue",
                0.9,
                vec!["f1".into()],
                "direct observation supports this",
                None, None,
                &MutationContext::default(),
            )
            .unwrap();
        out.push_str(&format!(
            "  tx{}: believe \"the sky is blue\" (confidence=0.9)\n",
            tx3
        ));
        out.push_str(&format!(
            "  current beliefs: {}\n\n",
            fmt_beliefs(&brain.current_beliefs())
        ));

        // Step 3: Retract a fact (does NOT erase history)
        out.push_str("-- Step 3: Retract f2 (grass-color) --\n");
        let tx4 = brain.retract_fact("f2", &MutationContext::default()).unwrap();
        out.push_str(&format!("  tx{}: retract f2\n", tx4));
        out.push_str(&format!(
            "  current facts: {}\n",
            fmt_facts(&brain.current_facts())
        ));
        out.push_str(&format!(
            "  history of f2: {}\n\n",
            fmt_fact_history(&brain.fact_history("f2"))
        ));

        // Step 4: as_of query — see the world before retraction
        out.push_str("-- Step 4: Time-travel — facts as_of each transaction --\n");
        for tx in [tx1, tx2, tx3, tx4] {
            let facts = brain.facts_as_of(tx);
            out.push_str(&format!("  as_of tx{}: {}\n", tx, fmt_facts(&facts)));
        }
        out.push('\n');

        // Step 5: Revise the belief (supersedes prior version)
        out.push_str("-- Step 5: Revise belief — the sky is actually grey today --\n");
        let tx5 = brain
            .revise_belief(
                "b2",
                "the sky is blue",
                0.3,
                vec!["f1".into()],
                "overcast today, revising confidence down",
                None, None,
                &MutationContext::default(),
            )
            .unwrap();
        out.push_str(&format!(
            "  tx{}: revise \"the sky is blue\" (confidence=0.3)\n",
            tx5
        ));
        out.push_str(&format!(
            "  current beliefs: {}\n",
            fmt_beliefs(&brain.current_beliefs())
        ));
        let bh = brain.belief_history("the sky is blue");
        out.push_str(&format!(
            "  belief history: {}\n\n",
            fmt_belief_history(&bh)
        ));

        // Step 6: Explain
        out.push_str("-- Step 6: Explain f1 (all transactions referencing it) --\n");
        let txs = brain.explain("f1");
        for tx in &txs {
            out.push_str(&format!(
                "  tx{}: {} — \"{}\"\n",
                tx.tx_id, tx.action, tx.note
            ));
        }
        out.push('\n');

        // Step 7: Transaction log
        out.push_str("-- Full transaction log --\n");
        for tx in brain.transactions() {
            out.push_str(&format!(
                "  tx{}: [{}] {} refs={:?} \"{}\"\n",
                tx.tx_id, tx.branch_id, tx.action, tx.refs, tx.note
            ));
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers (for demo output)
// ---------------------------------------------------------------------------

fn fmt_facts(facts: &[&Fact]) -> String {
    if facts.is_empty() {
        return "(none)".into();
    }
    let items: Vec<String> = facts
        .iter()
        .map(|f| format!("{}={}", f.predicate, f.value))
        .collect();
    items.join(", ")
}

fn fmt_fact_history(facts: &[&Fact]) -> String {
    if facts.is_empty() {
        return "(none)".into();
    }
    let items: Vec<String> = facts
        .iter()
        .map(|f| {
            let status = if f.revoked_by_tx.is_some() {
                "revoked"
            } else {
                "active"
            };
            format!(
                "{}={} [tx{}, {}]",
                f.predicate, f.value, f.created_by_tx, status
            )
        })
        .collect();
    items.join("; ")
}

fn fmt_beliefs(beliefs: &[&Belief]) -> String {
    if beliefs.is_empty() {
        return "(none)".into();
    }
    let items: Vec<String> = beliefs
        .iter()
        .map(|b| {
            format!(
                "\"{}\" [{}] confidence={:.1}",
                b.claim_text, b.status, b.confidence
            )
        })
        .collect();
    items.join(", ")
}

fn fmt_belief_history(beliefs: &[&Belief]) -> String {
    if beliefs.is_empty() {
        return "(none)".into();
    }
    let items: Vec<String> = beliefs
        .iter()
        .map(|b| {
            format!(
                "\"{}\" [{}] confidence={:.1} valid={}..{}",
                b.claim_text, b.status, b.confidence, b.valid_from,
                b.valid_to.as_deref().unwrap_or("now")
            )
        })
        .collect();
    items.join("; ")
}

/// Check if a half-open interval [valid_from, valid_to) contains the given timestamp.
/// Uses lexicographic comparison — assumes fixed-width ISO 8601 format (YYYY-MM-DDTHH:MM:SSZ).
fn is_valid_at(valid_from: &str, valid_to: Option<&str>, timestamp: &str) -> bool {
    valid_from <= timestamp && valid_to.is_none_or(|end| end > timestamp)
}

/// ISO 8601 UTC timestamp for the current instant.
pub fn now_iso() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs();
    let days_since_epoch = (total_secs / 86400) as i64;
    let time_secs = total_secs % 86400;

    // Convert days since 1970-01-01 to (year, month, day)
    // Using the algorithm from Howard Hinnant's civil_from_days
    let z = days_since_epoch + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let hour = time_secs / 3600;
    let min = (time_secs % 3600) / 60;
    let sec = time_secs % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hour, min, sec)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::MutationContext;
    fn test_lock() -> &'static std::sync::Mutex<()> {
        crate::global_test_lock()
    }

    #[test]
    fn append_only_history_is_preserved() {
        let mut brain = Brain::new();
        brain
            .assert_fact("f1", "color", "red", 1.0, "test", None, None, &MutationContext::default())
            .unwrap();
        brain
            .assert_fact("f2", "color", "blue", 1.0, "test", None, None, &MutationContext::default())
            .unwrap();
        brain.retract_fact("f1", &MutationContext::default()).unwrap();

        // f1 still exists in the history even though it was retracted
        let history = brain.fact_history("f1");
        assert_eq!(history.len(), 1);
        assert!(history[0].revoked_by_tx.is_some());

        // The full facts vec has both facts
        assert_eq!(brain.facts.len(), 2);
    }

    #[test]
    fn current_vs_as_of_differ_after_retraction() {
        let mut brain = Brain::new();
        let tx1 = brain
            .assert_fact("f1", "temp", "hot", 1.0, "sensor", None, None, &MutationContext::default())
            .unwrap();
        let tx2 = brain
            .assert_fact("f2", "temp", "cold", 1.0, "sensor", None, None, &MutationContext::default())
            .unwrap();
        let tx3 = brain.retract_fact("f1", &MutationContext::default()).unwrap();

        // Current: only f2
        let current = brain.current_facts();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].fact_id, "f2");

        // as_of tx2: both f1 and f2 were active
        let at_tx2 = brain.facts_as_of(tx2);
        assert_eq!(at_tx2.len(), 2);

        // as_of tx1: only f1
        let at_tx1 = brain.facts_as_of(tx1);
        assert_eq!(at_tx1.len(), 1);
        assert_eq!(at_tx1[0].fact_id, "f1");

        // as_of tx3: only f2 (f1 was retracted at tx3)
        let at_tx3 = brain.facts_as_of(tx3);
        assert_eq!(at_tx3.len(), 1);
        assert_eq!(at_tx3[0].fact_id, "f2");
    }

    #[test]
    fn history_returns_prior_versions() {
        let mut brain = Brain::new();
        brain
            .revise_belief("b1", "sky is blue", 0.9, vec![], "sunny day", None, None, &MutationContext::default())
            .unwrap();
        brain
            .revise_belief("b2", "sky is blue", 0.3, vec![], "cloudy now", None, None, &MutationContext::default())
            .unwrap();

        let history = brain.belief_history("sky is blue");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].status, BeliefStatus::Superseded);
        assert_eq!(history[1].status, BeliefStatus::Active);
        assert!((history[0].confidence - 0.9).abs() < f64::EPSILON);
        assert!((history[1].confidence - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn beliefs_as_of_shows_time_travel() {
        let mut brain = Brain::new();
        let tx1 = brain
            .revise_belief("b1", "it is warm", 0.8, vec![], "morning", None, None, &MutationContext::default())
            .unwrap();
        let tx2 = brain
            .revise_belief("b2", "it is warm", 0.2, vec![], "evening", None, None, &MutationContext::default())
            .unwrap();

        // At tx1, b1 was active
        let at_tx1 = brain.beliefs_as_of(tx1);
        assert_eq!(at_tx1.len(), 1);
        assert_eq!(at_tx1[0].belief_id, "b1");

        // At tx2, b2 is active (b1 was superseded)
        let at_tx2 = brain.beliefs_as_of(tx2);
        assert_eq!(at_tx2.len(), 1);
        assert_eq!(at_tx2[0].belief_id, "b2");
    }

    #[test]
    fn explain_returns_all_related_transactions() {
        let mut brain = Brain::new();
        brain
            .assert_fact("f1", "mood", "happy", 1.0, "self-report", None, None, &MutationContext::default())
            .unwrap();
        brain.retract_fact("f1", &MutationContext::default()).unwrap();

        let txs = brain.explain("f1");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].action, TxAction::AssertFact);
        assert_eq!(txs[1].action, TxAction::RetractFact);
    }

    #[test]
    fn retract_nonexistent_fact_errors() {
        let mut brain = Brain::new();
        let err = brain.retract_fact("nope", &MutationContext::default()).unwrap_err();
        assert!(err.to_string().contains("no active fact"));
    }

    #[test]
    fn double_retract_errors() {
        let mut brain = Brain::new();
        brain.assert_fact("f1", "x", "y", 1.0, "test", None, None, &MutationContext::default()).unwrap();
        brain.retract_fact("f1", &MutationContext::default()).unwrap();
        let err = brain.retract_fact("f1", &MutationContext::default()).unwrap_err();
        assert!(err.to_string().contains("no active fact"));
    }

    #[test]
    fn branch_lifecycle() {
        let mut brain = Brain::new();
        brain.create_branch("exp", "experiment", &MutationContext::default()).unwrap();
        brain.switch_branch("exp").unwrap();
        assert_eq!(brain.current_branch, "exp");

        let err = brain.switch_branch("nonexistent").unwrap_err();
        assert!(err.to_string().contains("unknown branch"));
    }

    #[test]
    fn observation_is_recorded() {
        let mut brain = Brain::new();
        let tx = brain
            .assert_observation(
                "obs1",
                "sensor",
                "thermometer-1",
                "temperature=22C",
                0.95,
                vec!["env".into()],
                None, None,
                &MutationContext::default(),
            )
            .unwrap();
        assert!(tx > 0);
        assert_eq!(brain.observations.len(), 1);
        assert_eq!(brain.observations[0].obs_id, "obs1");
    }

    #[test]
    fn persistence_round_trip() {
        let _guard = test_lock().lock().unwrap();
        // Initialize rayforce2 runtime (needed for symbol table)
        let _engine = crate::RayforceEngine::new().unwrap();

        let dir = std::env::temp_dir().join(format!("brain-splay-{}", std::process::id()));
        let sym_path = dir.join("sym");
        let exom_dir = dir.join("exom");
        let _ = std::fs::create_dir_all(&exom_dir);

        // Write
        {
            let mut brain = Brain::open_exom(&exom_dir, &sym_path).unwrap();
            brain
                .assert_fact("f1", "color", "red", 1.0, "test", None, None, &MutationContext::default())
                .unwrap();
            brain
                .assert_fact("f2", "color", "blue", 1.0, "test", None, None, &MutationContext::default())
                .unwrap();
            brain.retract_fact("f1", &MutationContext::default()).unwrap();
        }

        // Reload and verify
        {
            let brain = Brain::open_exom(&exom_dir, &sym_path).unwrap();
            let current = brain.current_facts();
            assert_eq!(current.len(), 1);
            assert_eq!(current[0].fact_id, "f2");

            // History preserved
            let history = brain.fact_history("f1");
            assert_eq!(history.len(), 1);
            assert!(history[0].revoked_by_tx.is_some());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn demo_output_contains_timeline_markers() {
        let output = Brain::run_demo();

        // The demo should show all steps
        assert!(output.contains("Step 1"), "missing step 1");
        assert!(output.contains("Step 2"), "missing step 2");
        assert!(output.contains("Step 3"), "missing step 3");
        assert!(output.contains("Step 4"), "missing step 4");
        assert!(output.contains("Step 5"), "missing step 5");
        assert!(output.contains("Step 6"), "missing step 6");

        // Key assertions become current
        assert!(output.contains("sky-color=blue"), "missing sky-color fact");

        // Retraction doesn't erase history
        assert!(output.contains("revoked"), "missing revoked marker");
        assert!(
            output.contains("grass-color=green"),
            "history should still show grass-color"
        );

        // as_of queries show different states
        assert!(output.contains("as_of tx"), "missing as_of queries");

        // Belief revision
        assert!(output.contains("superseded"), "missing superseded belief");
        assert!(
            output.contains("confidence=0.3"),
            "missing revised confidence"
        );
        assert!(
            output.contains("confidence=0.9"),
            "missing original confidence"
        );

        // Transaction log
        assert!(
            output.contains("transaction log"),
            "missing transaction log"
        );
        assert!(output.contains("assert-fact"), "missing assert-fact in log");
        assert!(
            output.contains("retract-fact"),
            "missing retract-fact in log"
        );
    }

    #[test]
    fn transaction_ids_are_monotonic() {
        let mut brain = Brain::new();
        let tx1 = brain.assert_fact("f1", "a", "b", 1.0, "t", None, None, &MutationContext::default()).unwrap();
        let tx2 = brain.assert_fact("f2", "c", "d", 1.0, "t", None, None, &MutationContext::default()).unwrap();
        let tx3 = brain.retract_fact("f1", &MutationContext::default()).unwrap();
        assert!(tx1 < tx2);
        assert!(tx2 < tx3);
    }

    #[test]
    fn bitemporal_facts_with_explicit_validity() {
        let mut brain = Brain::new();
        // Fact happened on Jan 1st, ended March 1st
        brain
            .assert_fact("f1", "location", "paris", 1.0, "agent",
                Some("2024-01-01T00:00:00Z"), Some("2024-03-01T00:00:00Z"), &MutationContext::default())
            .unwrap();
        // Fact happened on March 1st, still valid
        brain
            .assert_fact("f2", "location", "london", 1.0, "agent",
                Some("2024-03-01T00:00:00Z"), None, &MutationContext::default())
            .unwrap();

        // Query: what was valid on Feb 15?
        let feb = brain.facts_valid_at("2024-02-15T00:00:00Z");
        assert_eq!(feb.len(), 1);
        assert_eq!(feb[0].fact_id, "f1"); // paris was valid, london not yet

        // Query: what was valid on April 1? (paris expired, london active)
        let apr = brain.facts_valid_at("2024-04-01T00:00:00Z");
        assert_eq!(apr.len(), 1);
        assert_eq!(apr[0].fact_id, "f2");

        // Query: what was valid on July 1?
        let jul = brain.facts_valid_at("2024-07-01T00:00:00Z");
        assert_eq!(jul.len(), 1);
        assert_eq!(jul[0].fact_id, "f2"); // only london still valid
    }

    #[test]
    fn bitemporal_cross_dimensional_query() {
        let mut brain = Brain::new();
        let tx1 = brain
            .assert_fact("f1", "status", "ok", 1.0, "sensor",
                Some("2024-01-01T00:00:00Z"), None, &MutationContext::default())
            .unwrap();
        let _tx2 = brain.retract_fact("f1", &MutationContext::default()).unwrap();
        let _tx3 = brain
            .assert_fact("f2", "status", "degraded", 1.0, "sensor",
                Some("2024-06-01T00:00:00Z"), None, &MutationContext::default())
            .unwrap();

        // At tx1, we only knew about f1. Query valid-at March.
        let at_tx1_march = brain.facts_bitemporal(tx1, "2024-03-01T00:00:00Z");
        assert_eq!(at_tx1_march.len(), 1);
        assert_eq!(at_tx1_march[0].fact_id, "f1");
    }
}
