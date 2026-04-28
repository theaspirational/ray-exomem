//! Datomic-style persistent brain layer for LLM agents.
//!
//! All state is immutable and append-only. Every mutation is recorded as a
//! transaction, enabling time-travel queries (`as_of`, `history`, `explain`).
//! Persistence uses rayforce2 splayed columnar tables.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::context::MutationContext;
use crate::exom::{self, now_iso8601_basic, session_id, ExomMeta, SessionMeta, SessionType};
pub use crate::fact_value::{FactValue, SymValue};
use crate::path::TreePath;
use crate::tree::{classify, NodeKind};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

pub type TxId = u64;
pub type EntityId = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub fact_id: EntityId,
    pub predicate: String,
    pub value: FactValue,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tx {
    pub tx_id: TxId,
    pub tx_time: String,
    pub user_email: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub action: TxAction,
    pub refs: Vec<EntityId>,
    pub note: String,
    pub parent_tx_id: Option<TxId>,
    pub branch_id: String,
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TxAction {
    AssertObservation,
    AssertFact,
    RetractFact,
    ReviseBelief,
    RevokeBelief,
    CreateBranch,
    Merge,
}

impl std::fmt::Display for TxAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxAction::AssertObservation => write!(f, "assert-observation"),
            TxAction::AssertFact => write!(f, "assert-fact"),
            TxAction::RetractFact => write!(f, "retract-fact"),
            TxAction::ReviseBelief => write!(f, "revise-belief"),
            TxAction::RevokeBelief => write!(f, "revoke-belief"),
            TxAction::CreateBranch => write!(f, "create-branch"),
            TxAction::Merge => write!(f, "merge"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub branch_id: String,
    pub name: String,
    pub parent_branch_id: Option<String>,
    pub created_tx_id: TxId,
    pub archived: bool,
    /// TOFU ownership: email of the user who first claimed this branch.
    /// `None` means unclaimed (any user may claim it on first write).
    pub claimed_by_user_email: Option<String>,
    /// Tool/integration the claimer used (`agent` arg or API-key label).
    pub claimed_by_agent: Option<String>,
    /// LLM the claimer was running at claim time.
    pub claimed_by_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MergeConflict {
    pub fact_id: String,
    pub predicate: String,
    pub source_value: String,
    pub target_value: String,
}

#[derive(Debug)]
pub struct MergeResult {
    pub added: Vec<String>,
    pub conflicts: Vec<MergeConflict>,
    pub tx_id: TxId,
}

#[derive(Debug, Clone, Copy)]
pub enum MergePolicy {
    /// Source overrides target on conflict.
    LastWriterWins,
    /// Target keeps its version on conflict.
    KeepTarget,
    /// Return conflicts without resolving.
    Manual,
}

// ---------------------------------------------------------------------------
// Brain — the main API surface
// ---------------------------------------------------------------------------

/// Which table was affected by a mutation.
#[derive(Clone, Copy, Debug)]
pub enum DirtyTable {
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
    /// Index: tx_id → branch_id (built at load time, updated on each alloc_tx).
    tx_branch_index: HashMap<TxId, String>,
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
            archived: false,
            claimed_by_user_email: None,
            claimed_by_agent: None,
            claimed_by_model: None,
        };
        Brain {
            observations: Vec::new(),
            facts: Vec::new(),
            beliefs: Vec::new(),
            transactions: Vec::new(),
            branches: vec![main_branch],
            next_tx: 1,
            current_branch: "main".into(),
            tx_branch_index: HashMap::new(),
            data_dir: None,
            sym_path: None,
        }
    }

    /// Reset to empty state, preserving data_dir and sym_path for persistence.
    pub fn reset(&mut self) {
        let data_dir = self.data_dir.take();
        let sym_path = self.sym_path.take();
        *self = Brain::new();
        self.data_dir = data_dir;
        self.sym_path = sym_path;
    }

    /// Open a brain from a splayed table directory. Loads all tables into memory.
    pub fn open_exom(exom_dir: &Path, sym_path: &Path) -> Result<Self> {
        use crate::storage;

        storage::recover_splay_dirs(exom_dir);

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
            brain.tx_branch_index = brain
                .transactions
                .iter()
                .map(|tx| (tx.tx_id, tx.branch_id.clone()))
                .collect();
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
                brain.branches.insert(
                    0,
                    Branch {
                        branch_id: "main".into(),
                        name: "main".into(),
                        parent_branch_id: None,
                        created_tx_id: 0,
                        archived: false,
                        claimed_by_user_email: None,
            claimed_by_agent: None,
            claimed_by_model: None,
                    },
                );
            }
        }

        Ok(brain)
    }

    /// Replace all brain state wholesale (used by lossless JSON import).
    pub fn replace_state(
        &mut self,
        facts: Vec<Fact>,
        transactions: Vec<Tx>,
        observations: Vec<Observation>,
        beliefs: Vec<Belief>,
        branches: Vec<Branch>,
    ) -> Result<()> {
        self.facts = facts;
        self.transactions = transactions;
        self.observations = observations;
        self.beliefs = beliefs;
        self.branches = branches;

        // Rebuild derived state
        self.next_tx = self.transactions.last().map(|t| t.tx_id + 1).unwrap_or(1);
        self.tx_branch_index = self
            .transactions
            .iter()
            .map(|tx| (tx.tx_id, tx.branch_id.clone()))
            .collect();

        // Ensure "main" branch exists
        if !self.branches.iter().any(|b| b.branch_id == "main") {
            self.branches.insert(
                0,
                Branch {
                    branch_id: "main".into(),
                    name: "main".into(),
                    parent_branch_id: None,
                    created_tx_id: 0,
                    archived: false,
                    claimed_by_user_email: None,
            claimed_by_agent: None,
            claimed_by_model: None,
                },
            );
        }

        // Rebuild splay cache for rayforce2
        self.save()?;
        Ok(())
    }

    /// Rebuild all splayed columnar tables + sym. No-op if no `data_dir` / `sym_path`.
    pub fn save(&self) -> Result<()> {
        self.rebuild_splay(DirtyTable::Tx)?;
        self.rebuild_splay(DirtyTable::Fact)?;
        self.rebuild_splay(DirtyTable::Observation)?;
        self.rebuild_splay(DirtyTable::Belief)?;
        self.rebuild_splay(DirtyTable::Branch)?;
        Ok(())
    }

    /// Rebuild the splay table for rayforce2 query cache. Always called after mutations.
    fn rebuild_splay(&self, table: DirtyTable) -> Result<()> {
        use crate::storage;
        let (data_dir, sym_path) = match (&self.data_dir, &self.sym_path) {
            (Some(d), Some(s)) => (d, s),
            _ => return Ok(()), // in-memory mode
        };
        let (name, ray_table) = match &table {
            DirtyTable::Tx => ("tx", storage::build_tx_table(&self.transactions)),
            DirtyTable::Fact => ("fact", storage::build_fact_table(&self.facts)),
            DirtyTable::Observation => (
                "observation",
                storage::build_observation_table(&self.observations),
            ),
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
            user_email: ctx.user_email.clone(),
            agent: ctx.agent.clone(),
            model: ctx.model.clone(),
            action,
            refs,
            note: note.into(),
            parent_tx_id: self.transactions.last().map(|t| t.tx_id),
            branch_id: self.current_branch.clone(),
            session: ctx.session.clone(),
        };
        self.transactions.push(tx);
        self.tx_branch_index
            .insert(tx_id, self.current_branch.clone());
        self.rebuild_splay(DirtyTable::Tx)?;
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
        self.rebuild_splay(DirtyTable::Observation)?;
        Ok(tx_id)
    }

    // FIXME(nested-exoms-task-4.4): callers must invoke crate::brain::precheck_write before this function.
    pub fn assert_fact(
        &mut self,
        fact_id: &str,
        predicate: &str,
        value: impl Into<FactValue>,
        confidence: f64,
        provenance: &str,
        valid_from: Option<&str>,
        valid_to: Option<&str>,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        let value: FactValue = value.into();
        let (tx_id, tx_time) = self.alloc_tx(
            TxAction::AssertFact,
            vec![fact_id.into()],
            &format!("assert: {} = {}", predicate, value),
            ctx,
        )?;
        let fact = Fact {
            fact_id: fact_id.into(),
            predicate: predicate.into(),
            value,
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
        self.rebuild_splay(DirtyTable::Fact)?;
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
        if let Some(f) = self
            .facts
            .iter_mut()
            .find(|f| f.fact_id == fact_id && f.revoked_by_tx.is_none())
        {
            f.revoked_by_tx = Some(tx_id);
            if f.valid_to.is_none() {
                f.valid_to = Some(tx_time);
            }
        }
        self.rebuild_splay(DirtyTable::Fact)?;
        Ok(tx_id)
    }

    pub fn retract_fact_exact(
        &mut self,
        fact_id: &str,
        predicate: &str,
        value: impl Into<FactValue>,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        let value: FactValue = value.into();
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
                fact_id,
                predicate,
                value
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

        self.rebuild_splay(DirtyTable::Fact)?;
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
        self.rebuild_splay(DirtyTable::Belief)?;
        Ok(tx_id)
    }

    /// Withdraw an active belief without supplying a replacement claim. Sets
    /// status to `Revoked`, closes `valid_to` to the tx time, and emits a
    /// `RevokeBelief` transaction. History is preserved — `belief_history`
    /// still returns the revoked tuple, and `belief-row` exposes it with
    /// `status = "revoked"`. `current_beliefs` (and thus `beliefs_on_branch`)
    /// drops it.
    ///
    /// Errors if there's no active belief with that id on the current branch.
    pub fn revoke_belief(
        &mut self,
        belief_id: &str,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        if !self
            .beliefs
            .iter()
            .any(|b| b.belief_id == belief_id && b.status == BeliefStatus::Active)
        {
            bail!("no active belief with id '{}'", belief_id);
        }
        let (tx_id, tx_time) = self.alloc_tx(
            TxAction::RevokeBelief,
            vec![belief_id.into()],
            &format!("revoke: {}", belief_id),
            ctx,
        )?;
        if let Some(b) = self
            .beliefs
            .iter_mut()
            .find(|b| b.belief_id == belief_id && b.status == BeliefStatus::Active)
        {
            b.status = BeliefStatus::Revoked;
            if b.valid_to.is_none() {
                b.valid_to = Some(tx_time);
            }
        }
        self.rebuild_splay(DirtyTable::Belief)?;
        Ok(tx_id)
    }

    pub fn create_branch(
        &mut self,
        branch_id: &str,
        name: &str,
        ctx: &MutationContext,
    ) -> Result<TxId> {
        if self
            .branches
            .iter()
            .any(|b| b.branch_id == branch_id && !b.archived)
        {
            bail!("branch '{}' already exists", branch_id);
        }
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
            archived: false,
            claimed_by_user_email: None,
            claimed_by_agent: None,
            claimed_by_model: None,
        };
        self.branches.push(branch);
        self.rebuild_splay(DirtyTable::Branch)?;
        Ok(tx_id)
    }

    pub fn switch_branch(&mut self, branch_id: &str) -> Result<()> {
        let Some(b) = self.branches.iter().find(|b| b.branch_id == branch_id) else {
            bail!("unknown branch '{}'", branch_id);
        };
        if b.archived {
            bail!("branch '{}' is archived", branch_id);
        }
        self.current_branch = branch_id.into();
        Ok(())
    }

    /// Mark a branch as archived (soft-delete). Cannot archive `main`.
    pub fn archive_branch(&mut self, branch_id: &str) -> Result<()> {
        if branch_id == "main" {
            bail!("cannot archive branch 'main'");
        }
        let Some(branch) = self.branches.iter_mut().find(|b| b.branch_id == branch_id) else {
            bail!("unknown branch '{}'", branch_id);
        };
        branch.archived = true;
        if self.current_branch == branch_id {
            self.current_branch = "main".into();
        }
        self.rebuild_splay(DirtyTable::Branch)?;
        Ok(())
    }

    /// Returns the ancestor chain from `branch_id` up to root (non-archived branches only).
    /// Result: `[branch_id, parent, grandparent, ...]`.
    pub fn branch_ancestors(&self, branch_id: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut cur = Some(branch_id.to_string());
        let mut guard = 0u32;
        while let Some(id) = cur.take() {
            if guard > 256 {
                break;
            }
            guard += 1;
            let Some(b) = self.branches.iter().find(|x| x.branch_id == id) else {
                break;
            };
            if b.archived {
                break;
            }
            chain.push(b.branch_id.clone());
            cur = b.parent_branch_id.clone();
        }
        if chain.is_empty() {
            chain.push(branch_id.to_string());
        }
        chain
    }

    /// Look up which branch a tx was created on (O(1) via index).
    pub fn tx_branch(&self, tx_id: TxId) -> Option<&str> {
        self.tx_branch_index.get(&tx_id).map(|s| s.as_str())
    }

    fn tx_on_branches(&self, tx_id: TxId, branch_set: &HashSet<&str>) -> bool {
        self.tx_branch(tx_id)
            .map(|b| branch_set.contains(b))
            .unwrap_or(false)
    }

    /// Depth of a tx's branch in the ancestor chain (0 = closest to the viewing branch).
    pub fn branch_depth_of_tx(&self, tx_id: TxId, ancestors: &[String]) -> usize {
        match self.tx_branch(tx_id) {
            Some(b) => ancestors.iter().position(|a| a == b).unwrap_or(usize::MAX),
            None => usize::MAX,
        }
    }

    /// Whether `f` is visible on `branch_id` (branch-scoped retractions and creation branch).
    fn fact_visible_on_branch(&self, f: &Fact, branch_id: &str) -> bool {
        let ancestors = self.branch_ancestors(branch_id);
        let branch_set: HashSet<&str> = ancestors.iter().map(|s| s.as_str()).collect();
        if !self.tx_on_branches(f.created_by_tx, &branch_set) {
            return false;
        }
        if let Some(rev) = f.revoked_by_tx {
            if self.tx_on_branches(rev, &branch_set) {
                return false;
            }
        }
        true
    }

    /// Active facts visible on a specific branch (ancestor inheritance + shadowing by `fact_id`).
    pub fn facts_on_branch(&self, branch_id: &str) -> Vec<&Fact> {
        let ancestors = self.branch_ancestors(branch_id);
        let mut visible: Vec<&Fact> = self
            .facts
            .iter()
            .filter(|f| self.fact_visible_on_branch(f, branch_id))
            .collect();
        visible.sort_by_key(|f| {
            (
                self.branch_depth_of_tx(f.created_by_tx, &ancestors),
                Reverse(f.created_by_tx),
            )
        });
        visible.dedup_by(|a, b| a.fact_id == b.fact_id);
        visible
    }

    /// `"local"` | `"inherited"` | `"override"` for UI badges on the current branch view.
    pub fn fact_branch_role(&self, f: &Fact, view_branch: &str) -> &'static str {
        let ancestors = self.branch_ancestors(view_branch);
        let d = self.branch_depth_of_tx(f.created_by_tx, &ancestors);
        if d == usize::MAX {
            return "local";
        }
        if d > 0 {
            return "inherited";
        }
        for other in &self.facts {
            if other.fact_id != f.fact_id || other.created_by_tx == f.created_by_tx {
                continue;
            }
            let od = self.branch_depth_of_tx(other.created_by_tx, &ancestors);
            if od != usize::MAX && od > d {
                return "override";
            }
        }
        "local"
    }

    pub fn branches(&self) -> &[Branch] {
        &self.branches
    }

    pub fn current_branch_id(&self) -> &str {
        &self.current_branch
    }

    /// Merge `source` into `target` using `policy`. Assertions run on `target`.
    pub fn merge_branch(
        &mut self,
        source: &str,
        target: &str,
        policy: MergePolicy,
        ctx: &MutationContext,
    ) -> Result<MergeResult> {
        if !self
            .branches
            .iter()
            .any(|b| b.branch_id == source && !b.archived)
        {
            bail!("source branch '{}' not found", source);
        }
        if !self
            .branches
            .iter()
            .any(|b| b.branch_id == target && !b.archived)
        {
            bail!("target branch '{}' not found", target);
        }

        let target_ancestors: HashSet<String> = self.branch_ancestors(target).into_iter().collect();
        let target_ancestors_ref: HashSet<&str> =
            target_ancestors.iter().map(|s| s.as_str()).collect();

        let source_facts: Vec<Fact> = self.facts_on_branch(source).into_iter().cloned().collect();
        let target_facts: Vec<Fact> = self.facts_on_branch(target).into_iter().cloned().collect();
        let target_map: HashMap<&str, &Fact> = target_facts
            .iter()
            .map(|f| (f.fact_id.as_str(), f))
            .collect();

        let mut added = Vec::new();
        let mut conflicts = Vec::new();

        let saved_branch = self.current_branch.clone();
        self.current_branch = target.to_string();

        for fact in &source_facts {
            if self.tx_on_branches(fact.created_by_tx, &target_ancestors_ref) {
                continue;
            }

            match target_map.get(fact.fact_id.as_str()) {
                None => {
                    self.assert_fact(
                        &fact.fact_id,
                        &fact.predicate,
                        fact.value.clone(),
                        fact.confidence,
                        &format!("merged-from:{}", source),
                        Some(&fact.valid_from),
                        fact.valid_to.as_deref(),
                        ctx,
                    )?;
                    added.push(fact.fact_id.clone());
                }
                Some(target_fact) if target_fact.value != fact.value => match policy {
                    MergePolicy::LastWriterWins => {
                        self.retract_fact(&fact.fact_id, ctx)?;
                        self.assert_fact(
                            &fact.fact_id,
                            &fact.predicate,
                            fact.value.clone(),
                            fact.confidence,
                            &format!("merged-from:{}", source),
                            Some(&fact.valid_from),
                            fact.valid_to.as_deref(),
                            ctx,
                        )?;
                        added.push(fact.fact_id.clone());
                    }
                    MergePolicy::KeepTarget => {}
                    MergePolicy::Manual => {
                        conflicts.push(MergeConflict {
                            fact_id: fact.fact_id.clone(),
                            predicate: fact.predicate.clone(),
                            source_value: fact.value.display(),
                            target_value: target_fact.value.display(),
                        });
                    }
                },
                _ => {}
            }
        }

        let (tx_id, _) = self.alloc_tx(
            TxAction::Merge,
            vec![source.into(), target.into()],
            &format!("merge {} → {}", source, target),
            ctx,
        )?;
        self.current_branch = saved_branch;

        Ok(MergeResult {
            added,
            conflicts,
            tx_id,
        })
    }

    /// Return all observations.
    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }

    /// All facts (including revoked/superseded) — used for lossless export.
    pub fn all_facts(&self) -> &[Fact] {
        &self.facts
    }

    /// All beliefs (including superseded/revoked) — used for lossless export.
    pub fn all_beliefs(&self) -> &[Belief] {
        &self.beliefs
    }

    /// Total number of facts (including revoked).
    pub fn fact_count(&self) -> usize {
        self.facts.len()
    }

    /// Return all currently-active facts (not revoked) on the current branch.
    pub fn current_facts(&self) -> Vec<&Fact> {
        self.facts_on_branch(&self.current_branch)
    }

    /// Return transactions visible on the current branch (including inherited ancestor-branch txs).
    pub fn current_transactions(&self) -> Vec<&Tx> {
        self.transactions_on_branch(&self.current_branch)
    }

    /// Return transactions visible on a specific branch.
    pub fn transactions_on_branch(&self, branch_id: &str) -> Vec<&Tx> {
        let ancestors = self.branch_ancestors(branch_id);
        let branch_set: HashSet<&str> = ancestors.iter().map(|s| s.as_str()).collect();
        self.transactions
            .iter()
            .filter(|tx| branch_set.contains(tx.branch_id.as_str()))
            .collect()
    }

    /// Return active beliefs on the current branch (closest version wins per `claim_text`).
    pub fn current_beliefs(&self) -> Vec<&Belief> {
        self.beliefs_on_branch(&self.current_branch)
    }

    fn fact_active_as_of_on_branch(&self, f: &Fact, tx_id: TxId, branch_id: &str) -> bool {
        let ancestors = self.branch_ancestors(branch_id);
        let branch_set: HashSet<&str> = ancestors.iter().map(|s| s.as_str()).collect();
        if f.created_by_tx > tx_id {
            return false;
        }
        if !self.tx_on_branches(f.created_by_tx, &branch_set) {
            return false;
        }
        if let Some(rev) = f.revoked_by_tx {
            if rev <= tx_id && self.tx_on_branches(rev, &branch_set) {
                return false;
            }
        }
        true
    }

    /// Return facts as they were at a specific transaction (branch = that tx's branch).
    pub fn facts_as_of(&self, tx_id: TxId) -> Vec<&Fact> {
        let view_branch = self.tx_branch(tx_id).unwrap_or("main");
        let ancestors = self.branch_ancestors(view_branch);
        let mut visible: Vec<&Fact> = self
            .facts
            .iter()
            .filter(|f| self.fact_active_as_of_on_branch(f, tx_id, view_branch))
            .collect();
        visible.sort_by_key(|f| {
            (
                self.branch_depth_of_tx(f.created_by_tx, &ancestors),
                Reverse(f.created_by_tx),
            )
        });
        visible.dedup_by(|a, b| a.fact_id == b.fact_id);
        visible
    }

    /// Active beliefs on a branch (closest branch wins per `claim_text`).
    pub fn beliefs_on_branch(&self, branch_id: &str) -> Vec<&Belief> {
        let ancestors = self.branch_ancestors(branch_id);
        let branch_set: HashSet<&str> = ancestors.iter().map(|s| s.as_str()).collect();
        let mut visible: Vec<&Belief> = self
            .beliefs
            .iter()
            .filter(|b| {
                b.status == BeliefStatus::Active
                    && self.tx_on_branches(b.created_by_tx, &branch_set)
            })
            .collect();
        visible.sort_by_key(|b| {
            (
                self.branch_depth_of_tx(b.created_by_tx, &ancestors),
                Reverse(b.created_by_tx),
            )
        });
        visible.dedup_by(|a, b| a.claim_text == b.claim_text);
        visible
    }

    /// Return beliefs as they were known at a specific transaction (transaction-time travel).
    pub fn beliefs_as_of(&self, tx_id: TxId) -> Vec<&Belief> {
        let view_branch = self.tx_branch(tx_id).unwrap_or("main");
        let ancestors = self.branch_ancestors(view_branch);
        let branch_set: HashSet<&str> = ancestors.iter().map(|s| s.as_str()).collect();
        let mut latest_tx_by_claim: HashMap<&str, TxId> = HashMap::new();
        for b in &self.beliefs {
            if b.created_by_tx > tx_id {
                continue;
            }
            if !self.tx_on_branches(b.created_by_tx, &branch_set) {
                continue;
            }
            let entry = latest_tx_by_claim.entry(b.claim_text.as_str()).or_insert(0);
            if b.created_by_tx > *entry {
                *entry = b.created_by_tx;
            }
        }
        let mut out: Vec<&Belief> = self
            .beliefs
            .iter()
            .filter(|b| {
                b.created_by_tx <= tx_id
                    && self.tx_on_branches(b.created_by_tx, &branch_set)
                    && latest_tx_by_claim.get(b.claim_text.as_str()) == Some(&b.created_by_tx)
            })
            .collect();
        out.sort_by_key(|b| {
            (
                self.branch_depth_of_tx(b.created_by_tx, &ancestors),
                Reverse(b.created_by_tx),
            )
        });
        out.dedup_by(|a, b| a.claim_text == b.claim_text);
        out
    }

    // -----------------------------------------------------------------------
    // Bitemporal queries — valid-time axis
    // -----------------------------------------------------------------------

    /// Return facts that were valid at a given real-world timestamp (current knowledge).
    pub fn facts_valid_at(&self, timestamp: &str) -> Vec<&Fact> {
        let view = self.current_branch.as_str();
        self.facts_on_branch(view)
            .into_iter()
            .filter(|f| is_valid_at(&f.valid_from, f.valid_to.as_deref(), timestamp))
            .collect()
    }

    /// Bitemporal: facts as known at tx_id that were valid at the given real-world timestamp.
    pub fn facts_bitemporal(&self, tx_id: TxId, timestamp: &str) -> Vec<&Fact> {
        let view_branch = self.tx_branch(tx_id).unwrap_or("main");
        let ancestors = self.branch_ancestors(view_branch);
        let mut visible: Vec<&Fact> = self
            .facts
            .iter()
            .filter(|f| {
                self.fact_active_as_of_on_branch(f, tx_id, view_branch)
                    && is_valid_at(&f.valid_from, f.valid_to.as_deref(), timestamp)
            })
            .collect();
        visible.sort_by_key(|f| {
            (
                self.branch_depth_of_tx(f.created_by_tx, &ancestors),
                Reverse(f.created_by_tx),
            )
        });
        visible.dedup_by(|a, b| a.fact_id == b.fact_id);
        visible
    }

    /// Return beliefs that were valid at a given real-world timestamp (current knowledge).
    pub fn beliefs_valid_at(&self, timestamp: &str) -> Vec<&Belief> {
        self.beliefs_on_branch(&self.current_branch)
            .into_iter()
            .filter(|b| is_valid_at(&b.valid_from, b.valid_to.as_deref(), timestamp))
            .collect()
    }

    /// Bitemporal: beliefs as known at tx_id that were valid at the given real-world timestamp.
    pub fn beliefs_bitemporal(&self, tx_id: TxId, timestamp: &str) -> Vec<&Belief> {
        let view_branch = self.tx_branch(tx_id).unwrap_or("main");
        let ancestors = self.branch_ancestors(view_branch);
        let branch_set: HashSet<&str> = ancestors.iter().map(|s| s.as_str()).collect();
        let mut latest_tx_by_claim: HashMap<&str, TxId> = HashMap::new();
        for b in &self.beliefs {
            if b.created_by_tx > tx_id {
                continue;
            }
            if !self.tx_on_branches(b.created_by_tx, &branch_set) {
                continue;
            }
            if !is_valid_at(&b.valid_from, b.valid_to.as_deref(), timestamp) {
                continue;
            }
            let entry = latest_tx_by_claim.entry(b.claim_text.as_str()).or_insert(0);
            if b.created_by_tx > *entry {
                *entry = b.created_by_tx;
            }
        }
        let mut out: Vec<&Belief> = self
            .beliefs
            .iter()
            .filter(|b| {
                b.created_by_tx <= tx_id
                    && self.tx_on_branches(b.created_by_tx, &branch_set)
                    && latest_tx_by_claim.get(b.claim_text.as_str()) == Some(&b.created_by_tx)
                    && is_valid_at(&b.valid_from, b.valid_to.as_deref(), timestamp)
            })
            .collect();
        out.sort_by_key(|b| {
            (
                self.branch_depth_of_tx(b.created_by_tx, &ancestors),
                Reverse(b.created_by_tx),
            )
        });
        out.dedup_by(|a, b| a.claim_text == b.claim_text);
        out
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
            .assert_fact(
                "f1",
                "sky-color",
                "blue",
                0.9,
                "observation",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        let tx2 = brain
            .assert_fact(
                "f2",
                "grass-color",
                "green",
                0.85,
                "observation",
                None,
                None,
                &MutationContext::default(),
            )
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
                None,
                None,
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
        let tx4 = brain
            .retract_fact("f2", &MutationContext::default())
            .unwrap();
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
                None,
                None,
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
                b.claim_text,
                b.status,
                b.confidence,
                b.valid_from,
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
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hour, min, sec
    )
}

// ---------------------------------------------------------------------------
// Free functions — session / exom lifecycle
// ---------------------------------------------------------------------------

/// Summary statistics for a single exom directory read off the splay tables.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ExomStats {
    pub fact_count: u64,
    pub last_tx: Option<String>,
    pub branches: Vec<String>,
}

/// Read fact count, last-transaction time, and branch list from the splay
/// tables in `exom_disk`. Returns `Ok(ExomStats::default())` (zeros/None)
/// when the exom has never had any transactions written.
pub fn read_exom_stats(exom_disk: &Path, sym_path: &Path) -> std::io::Result<ExomStats> {
    use crate::storage;

    let load_table = |name: &str| -> std::io::Result<Option<storage::RayObj>> {
        let dir = exom_disk.join(name);
        if !storage::table_exists(&dir) {
            return Ok(None);
        }
        storage::load_table(&dir, sym_path)
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    };

    let fact_count = match load_table("fact")? {
        Some(tbl) => storage::load_facts(&tbl)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            .iter()
            .filter(|f| f.revoked_by_tx.is_none())
            .count() as u64,
        None => 0,
    };
    let last_tx = match load_table("tx")? {
        Some(tbl) => {
            let txs: Vec<Tx> = storage::load_txs(&tbl)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            txs.iter().max_by_key(|t| t.tx_id).map(|t| t.tx_time.clone())
        }
        None => None,
    };
    let branches_raw: Vec<Branch> = match load_table("branch")? {
        Some(tbl) => storage::load_branches(&tbl)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        None => Vec::new(),
    };

    let mut branch_names: Vec<String> = branches_raw
        .iter()
        .filter(|b| !b.archived)
        .map(|b| b.name.clone())
        .collect();

    // Always include "main" — it is implicitly created and may not have a
    // branch record in freshly-scaffolded exoms.
    if !branch_names.contains(&"main".to_string()) {
        branch_names.insert(0, "main".to_string());
    }

    Ok(ExomStats {
        fact_count,
        last_tx,
        branches: branch_names,
    })
}

// ---------------------------------------------------------------------------
// Standalone branch helpers (operate on the branch splay table directly,
// no Brain instance required).
// ---------------------------------------------------------------------------

fn io_err(e: anyhow::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
}

/// Create a new branch record for the given exom. Idempotent: if a non-archived
/// branch with this name already exists, returns Ok without modifying it.
/// `claimed_by` is always set to `None` on creation.
pub fn create_branch(
    tree_root: &Path,
    sym_path: &Path,
    exom_path: &TreePath,
    branch_name: &str,
) -> Result<(), crate::scaffold::ScaffoldError> {
    let disk = exom_path.to_disk_path(tree_root);
    let mut branches = crate::storage::load_branches_from_disk(&disk, sym_path)
        .map_err(|e| crate::scaffold::ScaffoldError::Io(io_err(e)))?;

    if branches
        .iter()
        .any(|b| b.name == branch_name && !b.archived)
    {
        return Ok(());
    }
    let parent = if branch_name == "main" {
        None
    } else {
        Some("main".to_string())
    };
    branches.push(Branch {
        branch_id: branch_name.to_string(),
        name: branch_name.to_string(),
        parent_branch_id: parent,
        created_tx_id: 0,
        archived: false,
        claimed_by_user_email: None,
        claimed_by_agent: None,
        claimed_by_model: None,
    });
    crate::storage::save_branches_to_disk(&disk, sym_path, &branches)
        .map_err(|e| crate::scaffold::ScaffoldError::Io(io_err(e)))?;
    Ok(())
}

/// TOFU-claim a branch for `user_email`, recording `agent` and `model` for
/// audit display.
///
/// - If `claimed_by_user_email` is None → set it to `user_email` (with the
///   supplied `agent`/`model`) and persist.
/// - If `claimed_by_user_email == user_email` → no-op (idempotent); existing
///   agent/model are preserved.
/// - If `claimed_by_user_email == someone_else` →
///   `Err(WriteError::BranchOwned(owner))`.
/// - If the branch doesn't exist → `Err(WriteError::BranchMissing(name))`.
///
/// "main" is considered to always exist even when the branch table is absent.
pub fn claim_branch(
    tree_root: &Path,
    sym_path: &Path,
    exom_path: &TreePath,
    branch_name: &str,
    user_email: &str,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<(), WriteError> {
    let disk = exom_path.to_disk_path(tree_root);
    let mut branches = crate::storage::load_branches_from_disk(&disk, sym_path)
        .map_err(|e| WriteError::Io(io_err(e)))?;

    let main_implicit = branches.is_empty() || !branches.iter().any(|b| b.name == "main");
    if main_implicit && branch_name == "main" {
        branches.push(Branch {
            branch_id: "main".to_string(),
            name: "main".to_string(),
            parent_branch_id: None,
            created_tx_id: 0,
            archived: false,
            claimed_by_user_email: None,
            claimed_by_agent: None,
            claimed_by_model: None,
        });
    }

    let Some(b) = branches
        .iter_mut()
        .find(|b| b.name == branch_name && !b.archived)
    else {
        return Err(WriteError::BranchMissing(branch_name.to_string()));
    };

    match &b.claimed_by_user_email {
        Some(owner) if owner != user_email => {
            return Err(WriteError::BranchOwned(owner.clone()))
        }
        Some(_) => {}
        None => {
            b.claimed_by_user_email = Some(user_email.to_string());
            b.claimed_by_agent = agent.map(str::to_string);
            b.claimed_by_model = model.map(str::to_string);
            crate::storage::save_branches_to_disk(&disk, sym_path, &branches)
                .map_err(|e| WriteError::Io(io_err(e)))?;
        }
    }
    Ok(())
}

/// Rejection codes for mutation prechecks.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("no such exom {0}")]
    NoSuchExom(String),
    #[error("session closed")]
    SessionClosed,
    #[error("branch {0} not in exom")]
    BranchMissing(String),
    #[error("branch owned by {0}")]
    BranchOwned(String),
    #[error("actor required")]
    ActorRequired,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Gate every mutation path. Call this before touching any splay table.
///
/// Enforces: exom exists, session is not closed, `user_email` is non-empty,
/// branch exists in the exom, and the user either owns the branch or is the
/// first writer (TOFU claim, recording `agent` and `model` as audit metadata).
pub fn precheck_write(
    tree_root: &Path,
    sym_path: &Path,
    exom_path: &TreePath,
    branch: &str,
    user_email: &str,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<(), WriteError> {
    if user_email.is_empty() {
        return Err(WriteError::ActorRequired);
    }
    let disk = exom_path.to_disk_path(tree_root);
    if classify(&disk) != NodeKind::Exom {
        return Err(WriteError::NoSuchExom(exom_path.to_cli_string()));
    }
    let meta = match exom::read_meta(&disk) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(WriteError::NoSuchExom(exom_path.to_cli_string()));
        }
        Err(e) => return Err(WriteError::Io(e)),
    };
    if let Some(s) = &meta.session {
        if s.closed_at.is_some() {
            return Err(WriteError::SessionClosed);
        }
    }

    let branches = crate::storage::load_branches_from_disk(&disk, sym_path)
        .map_err(|e| WriteError::Io(io_err(e)))?;

    // "main" is always implicitly present even when the branch table is absent.
    let branch_exists =
        branch == "main" || branches.iter().any(|b| b.name == branch && !b.archived);

    if !branch_exists {
        return Err(WriteError::BranchMissing(branch.to_string()));
    }

    claim_branch(tree_root, sym_path, exom_path, branch, user_email, agent, model)
}

/// Mirror writes to `session/label`, `session/closed_at`, or
/// `session/archived_at` into `exom.json` on disk.
///
/// Called by the HTTP layer (Task 4.4) after a successful assert-fact for
/// those reserved predicates. Do NOT wire into the existing `assert_fact`
/// entry point until Task 4.4 — callers must invoke this explicitly.
pub fn mirror_session_meta_to_disk(
    tree_root: &Path,
    exom_path: &TreePath,
    predicate: &str,
    value: &str,
) -> std::io::Result<()> {
    let disk = exom_path.to_disk_path(tree_root);
    let mut meta = exom::read_meta(&disk)?;
    if let Some(sess) = meta.session.as_mut() {
        match predicate {
            "session/label" => sess.label = value.to_string(),
            "session/closed_at" => sess.closed_at = Some(value.to_string()),
            "session/archived_at" => sess.archived_at = Some(value.to_string()),
            _ => return Ok(()),
        }
        exom::write_meta(&disk, &meta)?;
    }
    Ok(())
}

/// Create a session exom under `<project_path>/sessions/<id>` and write its
/// `exom.json`. No splay-table writes — metadata only.
///
/// `user_email` is the orchestrator's identity (recorded as `initiated_by` and
/// captured in the `main` branch's `claimed_by_user_email`). `agent` and
/// `model` are recorded on the `main` branch's audit fields.
///
/// `agents` are sub-agent labels (used as branch names); they receive branch
/// records but are claimed lazily via `session_join`.
pub fn session_new(
    tree_root: &Path,
    sym_path: &Path,
    project_path: &TreePath,
    session_type: SessionType,
    label: &str,
    user_email: &str,
    agent: Option<&str>,
    model: Option<&str>,
    agents: &[String],
) -> Result<TreePath, crate::scaffold::ScaffoldError> {
    if label.is_empty() || label.contains('/') || label.contains("::") || label.contains(' ') {
        return Err(crate::scaffold::ScaffoldError::Path(
            crate::path::PathError::InvalidSegment(
                label.to_string(),
                "label must be non-empty and free of '/', '::', or whitespace",
            ),
        ));
    }
    if user_email.is_empty() {
        return Err(crate::scaffold::ScaffoldError::Path(
            crate::path::PathError::InvalidSegment(
                user_email.to_string(),
                "user_email required",
            ),
        ));
    }
    // Project must exist and have sessions/ folder.
    let project_disk = project_path.to_disk_path(tree_root);
    if classify(&project_disk.join("main")) != NodeKind::Exom {
        return Err(crate::scaffold::ScaffoldError::NestInsideExom(format!(
            "project not initialised at {}",
            project_path
        )));
    }
    let ts = now_iso8601_basic();
    let dir = session_id(&ts, session_type.clone(), label);
    let session_path = project_path
        .join("sessions")
        .and_then(|p| p.join(&dir))
        .map_err(crate::scaffold::ScaffoldError::Path)?;
    let disk = session_path.to_disk_path(tree_root);
    std::fs::create_dir_all(&disk)?;

    // Build agents list: orchestrator must always be first; dedupe.
    let mut agents_final: Vec<String> = vec![user_email.to_string()];
    for a in agents {
        if !agents_final.contains(a) {
            agents_final.push(a.clone());
        }
    }

    let meta = ExomMeta::new_session(SessionMeta {
        session_type,
        label: label.to_string(),
        initiated_by: user_email.to_string(),
        agents: agents_final.clone(),
        closed_at: None,
        archived_at: None,
    });
    exom::write_meta(&disk, &meta)?;

    // Pre-create branch records for every participant.
    // Orchestrator gets "main"; each other agent gets a branch named after themselves.
    for participant in &agents_final {
        let branch_name = if participant == user_email {
            "main"
        } else {
            participant.as_str()
        };
        create_branch(tree_root, sym_path, &session_path, branch_name)?;
    }
    // TOFU-claim "main" immediately for the orchestrator, recording agent/model.
    claim_branch(
        tree_root,
        sym_path,
        &session_path,
        "main",
        user_email,
        agent,
        model,
    )
    .map_err(|e| {
        crate::scaffold::ScaffoldError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
    })?;

    Ok(session_path)
}

/// Join an existing session as the sub-agent named `branch_name`, on behalf
/// of `user_email`.
///
/// Looks up the branch named `branch_name` in the session exom (set up by
/// `session_new`). Performs TOFU ownership claim: if unclaimed → claims it
/// (recording `agent` and `model`); if already owned by `user_email` →
/// idempotent; if owned by someone else → `WriteError::BranchOwned`.
/// Returns `branch_name`.
///
/// The orchestrator's branch is "main", so orchestrators should not call this;
/// they already own "main" after `session_new`. Sub-agents call this to claim
/// their named branch.
///
/// Access-control note: the HTTP handler must verify that `branch_name` is a
/// member of the session's `agents` list before calling this function.
pub fn session_join(
    tree_root: &Path,
    sym_path: &Path,
    session_path: &TreePath,
    branch_name: &str,
    user_email: &str,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<String, WriteError> {
    // Verify the session exom exists.
    let disk = session_path.to_disk_path(tree_root);
    if classify(&disk) != NodeKind::Exom {
        return Err(WriteError::NoSuchExom(session_path.to_cli_string()));
    }
    let meta = match exom::read_meta(&disk) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(WriteError::NoSuchExom(session_path.to_cli_string()));
        }
        Err(e) => return Err(WriteError::Io(e)),
    };
    if let Some(s) = &meta.session {
        if s.closed_at.is_some() {
            return Err(WriteError::SessionClosed);
        }
    }

    // The branch must exist (created by session_new).
    let branches = crate::storage::load_branches_from_disk(&disk, sym_path)
        .map_err(|e| WriteError::Io(io_err(e)))?;
    let exists = branch_name == "main"
        || branches
            .iter()
            .any(|b| b.name == branch_name && !b.archived);
    if !exists {
        return Err(WriteError::BranchMissing(branch_name.to_string()));
    }

    claim_branch(
        tree_root,
        sym_path,
        session_path,
        branch_name,
        user_email,
        agent,
        model,
    )?;
    Ok(branch_name.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod session_tests {
    use super::*;
    use tempfile::tempdir;

    fn test_sym(d: &tempfile::TempDir) -> std::path::PathBuf {
        d.path().join("sym")
    }

    #[test]
    fn creates_session_with_correct_name() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &"work::ath".parse().unwrap()).unwrap();
        let project: TreePath = "work::ath".parse().unwrap();
        let session = session_new(
            d.path(),
            &sym,
            &project,
            SessionType::Multi,
            "landing",
            "orchestrator",
            None,
            None,
            &["agent_a".into(), "agent_b".into()],
        )
        .unwrap();
        let segs = session.segments();
        assert_eq!(segs[0], "work");
        assert_eq!(segs[1], "ath");
        assert_eq!(segs[2], "sessions");
        assert!(segs[3].ends_with("_multi_agent_landing"));
        let meta = exom::read_meta(&session.to_disk_path(d.path())).unwrap();
        assert_eq!(
            meta.session.unwrap().agents,
            vec!["orchestrator", "agent_a", "agent_b"]
        );
    }

    #[test]
    fn mirroring_updates_exom_json() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &"work".parse().unwrap()).unwrap();
        let session = session_new(
            d.path(),
            &sym,
            &"work".parse().unwrap(),
            SessionType::Single,
            "old",
            "me",
            None,
            None,
            &[],
        )
        .unwrap();
        mirror_session_meta_to_disk(d.path(), &session, "session/label", "new").unwrap();
        mirror_session_meta_to_disk(
            d.path(),
            &session,
            "session/closed_at",
            "2026-04-12T00:00:00Z",
        )
        .unwrap();
        mirror_session_meta_to_disk(
            d.path(),
            &session,
            "session/archived_at",
            "2026-04-12T01:00:00Z",
        )
        .unwrap();
        let meta = exom::read_meta(&session.to_disk_path(d.path())).unwrap();
        let s = meta.session.unwrap();
        assert_eq!(s.label, "new");
        assert_eq!(s.closed_at.as_deref(), Some("2026-04-12T00:00:00Z"));
        assert_eq!(s.archived_at.as_deref(), Some("2026-04-12T01:00:00Z"));
    }

    #[test]
    fn session_new_rejects_bad_label() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &"work".parse().unwrap()).unwrap();
        let p: TreePath = "work".parse().unwrap();
        assert!(
            session_new(d.path(), &sym, &p, SessionType::Single, "", "me", None, None, &[])
                .is_err()
        );
        assert!(session_new(
            d.path(),
            &sym,
            &p,
            SessionType::Single,
            "bad/name",
            "me",
            None,
            None,
            &[]
        )
        .is_err());
        assert!(session_new(
            d.path(),
            &sym,
            &p,
            SessionType::Single,
            "bad::name",
            "me",
            None,
            None,
            &[]
        )
        .is_err());
        assert!(session_new(
            d.path(),
            &sym,
            &p,
            SessionType::Single,
            "bad name",
            "me",
            None,
            None,
            &[]
        )
        .is_err());
        // empty user_email
        assert!(
            session_new(d.path(), &sym, &p, SessionType::Single, "ok", "", None, None, &[])
                .is_err()
        );
    }
}

#[cfg(test)]
mod tofu_tests {
    use super::*;
    use tempfile::tempdir;

    fn tp(s: &str) -> TreePath {
        s.parse().unwrap()
    }

    fn test_sym(d: &tempfile::TempDir) -> std::path::PathBuf {
        d.path().join("sym")
    }

    #[test]
    fn rejects_unknown_exom() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        let err =
            precheck_write(d.path(), &sym, &tp("work::nope"), "main", "me", None, None)
                .unwrap_err();
        assert!(matches!(err, WriteError::NoSuchExom(_)));
    }

    #[test]
    fn rejects_missing_actor() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &tp("work")).unwrap();
        let err =
            precheck_write(d.path(), &sym, &tp("work::main"), "main", "", None, None)
                .unwrap_err();
        assert!(matches!(err, WriteError::ActorRequired));
    }

    #[test]
    fn rejects_closed_session() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &tp("work")).unwrap();
        let session = session_new(
            d.path(),
            &sym,
            &tp("work"),
            SessionType::Single,
            "x",
            "me",
            None,
            None,
            &[],
        )
        .unwrap();
        // Mark closed by editing exom.json directly.
        let disk = session.to_disk_path(d.path());
        let mut meta = exom::read_meta(&disk).unwrap();
        meta.session.as_mut().unwrap().closed_at = Some("2026-04-11T00:00:00Z".into());
        exom::write_meta(&disk, &meta).unwrap();
        let err = precheck_write(d.path(), &sym, &session, "main", "me", None, None).unwrap_err();
        assert!(matches!(err, WriteError::SessionClosed));
    }

    #[test]
    fn tofu_claims_branch_on_first_write() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &tp("work")).unwrap();
        // Create a branch "agent_a" via create_branch.
        create_branch(d.path(), &sym, &tp("work::main"), "agent_a").unwrap();
        // First write claims it for alice.
        assert!(precheck_write(
            d.path(),
            &sym,
            &tp("work::main"),
            "agent_a",
            "alice",
            None,
            None
        )
        .is_ok());
        // Same actor writes again — succeeds (idempotent).
        assert!(precheck_write(
            d.path(),
            &sym,
            &tp("work::main"),
            "agent_a",
            "alice",
            None,
            None
        )
        .is_ok());
        // Different actor tries same branch — rejected.
        let err = precheck_write(
            d.path(),
            &sym,
            &tp("work::main"),
            "agent_a",
            "bob",
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, WriteError::BranchOwned(_)));
    }

    #[test]
    fn precheck_rejects_nonexistent_branch() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &tp("work")).unwrap();
        let err = precheck_write(
            d.path(),
            &sym,
            &tp("work::main"),
            "nonexistent",
            "alice",
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, WriteError::BranchMissing(_)));
    }

    #[test]
    fn precheck_allows_main_branch_without_explicit_create() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &tp("work")).unwrap();
        // "main" branch should exist implicitly for any exom.
        assert!(
            precheck_write(d.path(), &sym, &tp("work::main"), "main", "alice", None, None).is_ok()
        );
    }

    #[test]
    fn session_join_claims_agent_branch() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &tp("proj")).unwrap();
        let session = session_new(
            d.path(),
            &sym,
            &tp("proj"),
            SessionType::Multi,
            "collab",
            "orch",
            None,
            None,
            &["agent_a".into()],
        )
        .unwrap();
        // agent_a joins and claims their branch on behalf of alice.
        let branch =
            session_join(d.path(), &sym, &session, "agent_a", "alice", None, None).unwrap();
        assert_eq!(branch, "agent_a");
        // Joining again is idempotent (same user_email).
        assert!(session_join(d.path(), &sym, &session, "agent_a", "alice", None, None).is_ok());
        // A different user cannot claim the same (already-owned) branch.
        let err =
            precheck_write(d.path(), &sym, &session, "agent_a", "bob", None, None).unwrap_err();
        assert!(matches!(err, WriteError::BranchOwned(_)));
        // Branch not in the session's pre-created list is rejected.
        let err =
            session_join(d.path(), &sym, &session, "impostor", "alice", None, None).unwrap_err();
        assert!(matches!(err, WriteError::BranchMissing(_)));
    }

    #[test]
    fn session_new_precreates_agent_branches() {
        let _guard = crate::global_test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();
        let d = tempdir().unwrap();
        let sym = test_sym(&d);
        crate::scaffold::init_project(d.path(), &tp("proj")).unwrap();
        let session = session_new(
            d.path(),
            &sym,
            &tp("proj"),
            SessionType::Multi,
            "multi",
            "orch",
            Some("cursor"),
            Some("claude-opus-4-7"),
            &["sub_a".into(), "sub_b".into()],
        )
        .unwrap();
        let disk = session.to_disk_path(d.path());
        let branches = crate::storage::load_branches_from_disk(&disk, &sym).unwrap();
        let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"main"), "main branch missing");
        assert!(names.contains(&"sub_a"), "sub_a branch missing");
        assert!(names.contains(&"sub_b"), "sub_b branch missing");
        // Orchestrator already owns main with full attribution.
        let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
        assert_eq!(main_branch.claimed_by_user_email.as_deref(), Some("orch"));
        assert_eq!(main_branch.claimed_by_agent.as_deref(), Some("cursor"));
        assert_eq!(
            main_branch.claimed_by_model.as_deref(),
            Some("claude-opus-4-7")
        );
    }
}

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
            .assert_fact(
                "f1",
                "color",
                "red",
                1.0,
                "test",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        brain
            .assert_fact(
                "f2",
                "color",
                "blue",
                1.0,
                "test",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        brain
            .retract_fact("f1", &MutationContext::default())
            .unwrap();

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
            .assert_fact(
                "f1",
                "temp",
                "hot",
                1.0,
                "sensor",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        let tx2 = brain
            .assert_fact(
                "f2",
                "temp",
                "cold",
                1.0,
                "sensor",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        let tx3 = brain
            .retract_fact("f1", &MutationContext::default())
            .unwrap();

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
            .revise_belief(
                "b1",
                "sky is blue",
                0.9,
                vec![],
                "sunny day",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        brain
            .revise_belief(
                "b2",
                "sky is blue",
                0.3,
                vec![],
                "cloudy now",
                None,
                None,
                &MutationContext::default(),
            )
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
            .revise_belief(
                "b1",
                "it is warm",
                0.8,
                vec![],
                "morning",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        let tx2 = brain
            .revise_belief(
                "b2",
                "it is warm",
                0.2,
                vec![],
                "evening",
                None,
                None,
                &MutationContext::default(),
            )
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
            .assert_fact(
                "f1",
                "mood",
                "happy",
                1.0,
                "self-report",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        brain
            .retract_fact("f1", &MutationContext::default())
            .unwrap();

        let txs = brain.explain("f1");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].action, TxAction::AssertFact);
        assert_eq!(txs[1].action, TxAction::RetractFact);
    }

    #[test]
    fn retract_nonexistent_fact_errors() {
        let mut brain = Brain::new();
        let err = brain
            .retract_fact("nope", &MutationContext::default())
            .unwrap_err();
        assert!(err.to_string().contains("no active fact"));
    }

    #[test]
    fn double_retract_errors() {
        let mut brain = Brain::new();
        brain
            .assert_fact(
                "f1",
                "x",
                "y",
                1.0,
                "test",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        brain
            .retract_fact("f1", &MutationContext::default())
            .unwrap();
        let err = brain
            .retract_fact("f1", &MutationContext::default())
            .unwrap_err();
        assert!(err.to_string().contains("no active fact"));
    }

    #[test]
    fn branch_lifecycle() {
        let mut brain = Brain::new();
        brain
            .create_branch("exp", "experiment", &MutationContext::default())
            .unwrap();
        brain.switch_branch("exp").unwrap();
        assert_eq!(brain.current_branch, "exp");

        let err = brain.switch_branch("nonexistent").unwrap_err();
        assert!(err.to_string().contains("unknown branch"));
    }

    #[test]
    fn facts_on_branch_shadows_ancestor() {
        let mut brain = Brain::new();
        brain
            .assert_fact(
                "f1",
                "sky-color",
                "blue",
                1.0,
                "t",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        brain
            .create_branch("exp", "e", &MutationContext::default())
            .unwrap();
        brain.switch_branch("exp").unwrap();
        brain
            .assert_fact(
                "f1",
                "sky-color",
                "red",
                1.0,
                "t",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();

        let on_main = brain.facts_on_branch("main");
        assert_eq!(on_main.len(), 1);
        assert_eq!(on_main[0].value, "blue");

        let on_exp = brain.facts_on_branch("exp");
        assert_eq!(on_exp.len(), 1);
        assert_eq!(on_exp[0].value, "red");
    }

    #[test]
    fn revoke_belief_drops_from_current_keeps_history() {
        let mut brain = Brain::new();
        let ctx = MutationContext::default();
        brain
            .revise_belief("b1", "claim", 0.9, vec![], "rationale", None, None, &ctx)
            .unwrap();
        assert_eq!(brain.current_beliefs().len(), 1);

        let tx = brain.revoke_belief("b1", &ctx).unwrap();
        assert!(tx > 0);
        // Dropped from current view
        assert_eq!(brain.current_beliefs().len(), 0);
        // History preserved with revoked status and closed valid_to
        let stored = &brain.beliefs[0];
        assert_eq!(stored.status, BeliefStatus::Revoked);
        assert!(stored.valid_to.is_some());

        // Re-revoking the same id errors (no longer active)
        assert!(brain.revoke_belief("b1", &ctx).is_err());
        // Revoking a nonexistent id errors
        assert!(brain.revoke_belief("ghost", &ctx).is_err());
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
                None,
                None,
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
                .assert_fact(
                    "f1",
                    "color",
                    "red",
                    1.0,
                    "test",
                    None,
                    None,
                    &MutationContext::default(),
                )
                .unwrap();
            brain
                .assert_fact(
                    "f2",
                    "color",
                    "blue",
                    1.0,
                    "test",
                    None,
                    None,
                    &MutationContext::default(),
                )
                .unwrap();
            brain
                .retract_fact("f1", &MutationContext::default())
                .unwrap();
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
        let tx1 = brain
            .assert_fact(
                "f1",
                "a",
                "b",
                1.0,
                "t",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        let tx2 = brain
            .assert_fact(
                "f2",
                "c",
                "d",
                1.0,
                "t",
                None,
                None,
                &MutationContext::default(),
            )
            .unwrap();
        let tx3 = brain
            .retract_fact("f1", &MutationContext::default())
            .unwrap();
        assert!(tx1 < tx2);
        assert!(tx2 < tx3);
    }

    #[test]
    fn bitemporal_facts_with_explicit_validity() {
        let mut brain = Brain::new();
        // Fact happened on Jan 1st, ended March 1st
        brain
            .assert_fact(
                "f1",
                "location",
                "paris",
                1.0,
                "agent",
                Some("2024-01-01T00:00:00Z"),
                Some("2024-03-01T00:00:00Z"),
                &MutationContext::default(),
            )
            .unwrap();
        // Fact happened on March 1st, still valid
        brain
            .assert_fact(
                "f2",
                "location",
                "london",
                1.0,
                "agent",
                Some("2024-03-01T00:00:00Z"),
                None,
                &MutationContext::default(),
            )
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
            .assert_fact(
                "f1",
                "status",
                "ok",
                1.0,
                "sensor",
                Some("2024-01-01T00:00:00Z"),
                None,
                &MutationContext::default(),
            )
            .unwrap();
        let _tx2 = brain
            .retract_fact("f1", &MutationContext::default())
            .unwrap();
        let _tx3 = brain
            .assert_fact(
                "f2",
                "status",
                "degraded",
                1.0,
                "sensor",
                Some("2024-06-01T00:00:00Z"),
                None,
                &MutationContext::default(),
            )
            .unwrap();

        // At tx1, we only knew about f1. Query valid-at March.
        let at_tx1_march = brain.facts_bitemporal(tx1, "2024-03-01T00:00:00Z");
        assert_eq!(at_tx1_march.len(), 1);
        assert_eq!(at_tx1_march[0].fact_id, "f1");
    }

    // ---------------------------------------------------------------------
    // Typed FactValue round-trips — the splay cache must preserve the
    // variant (I64 / Str / Sym) across a save / load cycle so queries over
    // numeric facts keep matching bare int literals in rule bodies.
    // ---------------------------------------------------------------------

    #[test]
    fn typed_fact_value_survives_splay_roundtrip() {
        let _guard = test_lock().lock().unwrap();
        let _engine = crate::RayforceEngine::new().unwrap();

        let dir = std::env::temp_dir().join(format!(
            "brain-fact-value-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        let sym_path = dir.join("sym");
        let exom_dir = dir.join("exom");
        let _ = std::fs::create_dir_all(&exom_dir);

        {
            let mut brain = Brain::open_exom(&exom_dir, &sym_path).unwrap();
            brain
                .assert_fact(
                    "profile/weight_kg",
                    "profile/weight_kg",
                    FactValue::I64(55),
                    1.0,
                    "test",
                    None,
                    None,
                    &MutationContext::default(),
                )
                .unwrap();
            brain
                .assert_fact(
                    "profile/units",
                    "profile/units",
                    FactValue::Str("metric".into()),
                    1.0,
                    "test",
                    None,
                    None,
                    &MutationContext::default(),
                )
                .unwrap();
            brain
                .assert_fact(
                    "status/current",
                    "status",
                    FactValue::sym("active"),
                    1.0,
                    "test",
                    None,
                    None,
                    &MutationContext::default(),
                )
                .unwrap();
        }

        {
            let brain = Brain::open_exom(&exom_dir, &sym_path).unwrap();
            let current = brain.current_facts();
            let weight = current
                .iter()
                .find(|f| f.fact_id == "profile/weight_kg")
                .expect("weight fact should survive roundtrip");
            assert_eq!(weight.value, FactValue::I64(55));

            let units = current
                .iter()
                .find(|f| f.fact_id == "profile/units")
                .expect("units fact should survive roundtrip");
            assert_eq!(units.value, FactValue::Str("metric".into()));

            let status = current
                .iter()
                .find(|f| f.fact_id == "status/current")
                .expect("sym fact should survive roundtrip");
            assert_eq!(status.value, FactValue::sym("active"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typed_fact_value_serde_roundtrip() {
        // `Fact` is serialized to Postgres / the HTTP API via serde — it must
        // encode the typed variant via `#[serde(untagged)]` and reload cleanly.
        let fact = Fact {
            fact_id: "f1".into(),
            predicate: "weight".into(),
            value: FactValue::I64(75),
            created_at: "2024-01-01T00:00:00Z".into(),
            created_by_tx: 1,
            superseded_by_tx: None,
            revoked_by_tx: None,
            confidence: 1.0,
            provenance: "test".into(),
            valid_from: "2024-01-01T00:00:00Z".into(),
            valid_to: None,
        };
        let json = serde_json::to_string(&fact).unwrap();
        assert!(
            json.contains(r#""value":75"#),
            "I64 values should serialize as JSON number; got: {json}"
        );
        let round: Fact = serde_json::from_str(&json).unwrap();
        assert_eq!(round.value, FactValue::I64(75));

        let fact_str = Fact {
            value: FactValue::Str("Basil".into()),
            ..fact.clone()
        };
        let json = serde_json::to_string(&fact_str).unwrap();
        assert!(json.contains(r#""value":"Basil""#));
        let round: Fact = serde_json::from_str(&json).unwrap();
        assert_eq!(round.value, FactValue::Str("Basil".into()));

        let fact_sym = Fact {
            value: FactValue::sym("active"),
            ..fact
        };
        let json = serde_json::to_string(&fact_sym).unwrap();
        assert!(json.contains(r#""value":{"$sym":"active"}"#));
        let round: Fact = serde_json::from_str(&json).unwrap();
        assert_eq!(round.value, FactValue::sym("active"));
    }

    // ---------------------------------------------------------------------
    // End-to-end Datalog cmp pipeline (single-type datoms).
    //
    // The main datoms V column stays STR-tagged (see
    // `storage::encode_fact_value_datom`) to keep the column homogeneous and
    // avoid rayforce2 faults on queries that scan mixed int + str values in
    // the shared slot. Typed cmp over I64 facts is therefore wired through a
    // dedicated datoms table built just for this scenario — the same shape
    // that Phase B of the Datalog Aggregates plan will productionise via a
    // per-kind shadow relation.
    //
    // This test proves the FactValue → bare-i64 datom encoding path works
    // end-to-end: rule body `(< ?w 60)` matches `FactValue::I64(55)`.
    // ---------------------------------------------------------------------

    #[test]
    fn datalog_cmp_matches_i64_fact_value() {
        use crate::ffi;

        let _guard = test_lock().lock().unwrap();
        let engine = crate::RayforceEngine::new().unwrap();

        let exom_name = "testexom";

        // Build a 3-column datoms table by hand with a single row:
        //   E = "health/profile/weight_kg" (STR-tagged)
        //   A = 'profile/weight_kg          (SYM-tagged)
        //   V = 55                          (bare i64 — what FactValue::I64 produces)
        //
        // The direct construction matches what a future Phase B relation
        // would produce for i64-only facts.
        let table = unsafe {
            let mut tbl = ffi::ray_table_new(3);
            let mut e_col = ffi::ray_vec_new(ffi::RAY_I64, 1);
            let mut a_col = ffi::ray_vec_new(ffi::RAY_SYM, 1);
            let mut v_col = ffi::ray_vec_new(ffi::RAY_I64, 1);

            let fv = FactValue::I64(55);
            let e = crate::storage::encode_string_datom("health/profile/weight_kg");
            let a = crate::storage::sym_intern("profile/weight_kg");
            // Bypass the homogeneous-STR encoder to directly emit a bare i64
            // — this matches what the Phase B typed-cmp relation will carry.
            let v = 55_i64;
            let _ = fv;

            e_col = ffi::ray_vec_append(e_col, &e as *const i64 as *const _);
            a_col = ffi::ray_vec_append(a_col, &a as *const i64 as *const _);
            v_col = ffi::ray_vec_append(v_col, &v as *const i64 as *const _);

            let e_name = crate::storage::sym_intern("e");
            let a_name = crate::storage::sym_intern("a");
            let v_name = crate::storage::sym_intern("v");
            tbl = ffi::ray_table_add_col(tbl, e_name, e_col);
            tbl = ffi::ray_table_add_col(tbl, a_name, a_col);
            tbl = ffi::ray_table_add_col(tbl, v_name, v_col);

            crate::storage::RayObj::from_raw(tbl).unwrap()
        };

        engine
            .bind_named_db(crate::storage::sym_intern(exom_name), &table)
            .unwrap();

        // Rule: small-weight binds ?id when a weight fact for that id is
        // below 60. The bare int literal 60 in the rule body is an i64; it
        // must compare correctly against the stored `55` bare-int datom.
        let rule_body = format!(
            r#"(rule {exom_name} (small-weight ?id) (?id 'profile/weight_kg ?w) (< ?w 60))"#
        );
        let parsed = crate::rules::parse_rule_line(
            &rule_body,
            MutationContext::default(),
            crate::brain::now_iso(),
        )
        .unwrap();

        let query_source =
            format!("(query {exom_name} (find ?id) (where (small-weight ?id)))");
        let expanded = crate::rayfall_parser::rewrite_query_with_rules(
            &query_source,
            &[parsed.inline_body.clone()],
        )
        .unwrap();

        let raw = engine.eval_raw(&expanded).unwrap();
        let decoded =
            crate::storage::decode_query_table(&raw, &query_source).unwrap();
        let rows = decoded["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            !rows.is_empty(),
            "expected small-weight to match FactValue::I64(55) via cmp; got rows={:?} expanded={}",
            rows,
            expanded
        );
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
