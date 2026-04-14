//! Exom persistence backed by JSONL sidecar files under a tree root directory.

use std::path::PathBuf;

use crate::brain::{Belief, Branch, Fact, Observation, Tx};
use crate::db::ExomDb;
use crate::storage;

pub struct JsonlExomDb {
    tree_root: PathBuf,
}

impl JsonlExomDb {
    pub fn new(tree_root: PathBuf) -> Self {
        Self { tree_root }
    }

    fn exom_dir(&self, exom_path: &str) -> PathBuf {
        let mut p = self.tree_root.clone();
        for segment in exom_path.split('/').filter(|s| !s.is_empty()) {
            p.push(segment);
        }
        p
    }
}

#[async_trait::async_trait]
impl ExomDb for JsonlExomDb {
    async fn load_transactions(&self, exom_path: &str) -> anyhow::Result<Vec<Tx>> {
        let path = self.exom_dir(exom_path).join("tx.jsonl");
        storage::load_jsonl(&path)
    }

    async fn save_transactions(&self, exom_path: &str, txs: &[Tx]) -> anyhow::Result<()> {
        let path = self.exom_dir(exom_path).join("tx.jsonl");
        storage::save_jsonl(txs, &path)
    }

    async fn append_transaction(&self, exom_path: &str, tx: &Tx) -> anyhow::Result<()> {
        let path = self.exom_dir(exom_path).join("tx.jsonl");
        let mut txs: Vec<Tx> = storage::load_jsonl(&path)?;
        txs.push(tx.clone());
        storage::save_jsonl(&txs, &path)
    }

    async fn load_facts(&self, exom_path: &str) -> anyhow::Result<Vec<Fact>> {
        let path = self.exom_dir(exom_path).join("fact.jsonl");
        storage::load_jsonl(&path)
    }

    async fn save_facts(&self, exom_path: &str, facts: &[Fact]) -> anyhow::Result<()> {
        let path = self.exom_dir(exom_path).join("fact.jsonl");
        storage::save_jsonl(facts, &path)
    }

    async fn load_observations(&self, exom_path: &str) -> anyhow::Result<Vec<Observation>> {
        let path = self.exom_dir(exom_path).join("observation.jsonl");
        storage::load_jsonl(&path)
    }

    async fn save_observations(
        &self,
        exom_path: &str,
        observations: &[Observation],
    ) -> anyhow::Result<()> {
        let path = self.exom_dir(exom_path).join("observation.jsonl");
        storage::save_jsonl(observations, &path)
    }

    async fn load_beliefs(&self, exom_path: &str) -> anyhow::Result<Vec<Belief>> {
        let path = self.exom_dir(exom_path).join("belief.jsonl");
        storage::load_jsonl(&path)
    }

    async fn save_beliefs(&self, exom_path: &str, beliefs: &[Belief]) -> anyhow::Result<()> {
        let path = self.exom_dir(exom_path).join("belief.jsonl");
        storage::save_jsonl(beliefs, &path)
    }

    async fn load_branches(&self, exom_path: &str) -> anyhow::Result<Vec<Branch>> {
        let path = self.exom_dir(exom_path).join("branch.jsonl");
        storage::load_jsonl(&path)
    }

    async fn save_branches(&self, exom_path: &str, branches: &[Branch]) -> anyhow::Result<()> {
        let path = self.exom_dir(exom_path).join("branch.jsonl");
        storage::save_jsonl(branches, &path)
    }

    async fn write_mutation(
        &self,
        exom_path: &str,
        tx: &Tx,
        facts: Option<&[Fact]>,
        observations: Option<&[Observation]>,
        beliefs: Option<&[Belief]>,
        branches: Option<&[Branch]>,
    ) -> anyhow::Result<()> {
        self.append_transaction(exom_path, tx).await?;
        if let Some(facts) = facts {
            self.save_facts(exom_path, facts).await?;
        }
        if let Some(observations) = observations {
            self.save_observations(exom_path, observations).await?;
        }
        if let Some(beliefs) = beliefs {
            self.save_beliefs(exom_path, beliefs).await?;
        }
        if let Some(branches) = branches {
            self.save_branches(exom_path, branches).await?;
        }
        Ok(())
    }
}
