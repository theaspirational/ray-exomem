//! Postgres-backed exom persistence (`ExomDb`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::brain::{Belief, BeliefStatus, Branch, EntityId, Fact, Observation, Tx, TxAction, TxId};
use crate::db::ExomDb;
use crate::fact_value::FactValue;

/// Encode a typed fact value as the text payload persisted in `facts.value`.
/// Uses JSON — `20`, `"Basil"`, `{"$sym":"active"}` — so the variant is preserved
/// losslessly in the database (schema remains `text`).
fn fact_value_to_pg_text(v: &FactValue) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| v.display())
}

/// Decode a fact value from the text payload in `facts.value`.
///
/// Backward-compat: rows written before the FactValue refactor store bare
/// text like `75` or `metric` that does not parse as JSON. Those are read as
/// `FactValue::Str(raw)` so existing databases keep loading without a migration.
fn fact_value_from_pg_text(raw: &str) -> FactValue {
    if let Ok(parsed) = serde_json::from_str::<FactValue>(raw) {
        return parsed;
    }
    FactValue::Str(raw.to_string())
}

pub struct PgExomDb {
    pool: PgPool,
}

impl PgExomDb {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn action_from_str(s: &str) -> anyhow::Result<TxAction> {
    match s {
        "assert-observation" => Ok(TxAction::AssertObservation),
        "assert-fact" => Ok(TxAction::AssertFact),
        "retract-fact" => Ok(TxAction::RetractFact),
        "revise-belief" => Ok(TxAction::ReviseBelief),
        "create-branch" => Ok(TxAction::CreateBranch),
        "merge" => Ok(TxAction::Merge),
        _ => anyhow::bail!("unknown tx action: {s}"),
    }
}

fn status_from_str(s: &str) -> anyhow::Result<BeliefStatus> {
    match s {
        "active" => Ok(BeliefStatus::Active),
        "superseded" => Ok(BeliefStatus::Superseded),
        "revoked" => Ok(BeliefStatus::Revoked),
        _ => anyhow::bail!("unknown belief status: {s}"),
    }
}

fn parse_timestamptz(s: &str) -> anyhow::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| s.parse::<DateTime<Utc>>().map_err(|e| e.into()))
}

fn parse_timestamptz_opt(s: Option<&str>) -> anyhow::Result<Option<DateTime<Utc>>> {
    match s {
        None => Ok(None),
        Some("") => Ok(None),
        Some(x) => Ok(Some(parse_timestamptz(x)?)),
    }
}

fn i64_to_txid(v: i64) -> anyhow::Result<TxId> {
    if v < 0 {
        anyhow::bail!("invalid negative tx_id in database: {v}");
    }
    Ok(v as u64)
}

fn txid_to_i64(id: TxId) -> i64 {
    id as i64
}

fn tx_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<Tx> {
    let user_email: Option<String> = row.get("user_email");
    Ok(Tx {
        tx_id: i64_to_txid(row.get::<i64, _>("tx_id"))?,
        tx_time: row.get::<DateTime<Utc>, _>("tx_time").to_rfc3339(),
        user_email,
        actor: row.get::<Option<String>, _>("actor").unwrap_or_default(),
        action: action_from_str(&row.get::<String, _>("action"))?,
        refs: row.get::<Vec<String>, _>("refs"),
        note: row.get::<String, _>("note"),
        parent_tx_id: row
            .get::<Option<i64>, _>("parent_tx_id")
            .map(i64_to_txid)
            .transpose()?,
        branch_id: row.get::<String, _>("branch_id"),
        session: row.get::<Option<String>, _>("session"),
    })
}

fn fact_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<Fact> {
    let raw_value: String = row.get("value");
    Ok(Fact {
        fact_id: row.get("fact_id"),
        predicate: row.get("predicate"),
        value: fact_value_from_pg_text(&raw_value),
        created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
        created_by_tx: i64_to_txid(row.get::<i64, _>("created_by_tx"))?,
        superseded_by_tx: row
            .get::<Option<i64>, _>("superseded_by_tx")
            .map(i64_to_txid)
            .transpose()?,
        revoked_by_tx: row
            .get::<Option<i64>, _>("revoked_by_tx")
            .map(i64_to_txid)
            .transpose()?,
        confidence: row.get("confidence"),
        provenance: row.get("provenance"),
        valid_from: row.get::<DateTime<Utc>, _>("valid_from").to_rfc3339(),
        valid_to: row
            .get::<Option<DateTime<Utc>>, _>("valid_to")
            .map(|t| t.to_rfc3339()),
    })
}

fn observation_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<Observation> {
    Ok(Observation {
        obs_id: row.get("obs_id"),
        source_type: row.get("source_type"),
        source_ref: row.get("source_ref"),
        content: row.get("content"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
        confidence: row.get("confidence"),
        tx_id: i64_to_txid(row.get::<i64, _>("tx_id"))?,
        tags: row.get::<Vec<String>, _>("tags"),
        valid_from: row.get::<DateTime<Utc>, _>("valid_from").to_rfc3339(),
        valid_to: row
            .get::<Option<DateTime<Utc>>, _>("valid_to")
            .map(|t| t.to_rfc3339()),
    })
}

fn belief_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<Belief> {
    Ok(Belief {
        belief_id: row.get("belief_id"),
        claim_text: row.get("claim_text"),
        status: status_from_str(&row.get::<String, _>("status"))?,
        confidence: row.get("confidence"),
        supported_by: row.get::<Vec<String>, _>("supported_by"),
        created_by_tx: i64_to_txid(row.get::<i64, _>("created_by_tx"))?,
        valid_from: row.get::<DateTime<Utc>, _>("valid_from").to_rfc3339(),
        valid_to: row
            .get::<Option<DateTime<Utc>>, _>("valid_to")
            .map(|t| t.to_rfc3339()),
        rationale: row.get("rationale"),
    })
}

fn branch_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<Branch> {
    Ok(Branch {
        branch_id: row.get("branch_id"),
        name: row.get("name"),
        parent_branch_id: row.get("parent_branch_id"),
        created_tx_id: i64_to_txid(row.get::<i64, _>("created_tx_id"))?,
        archived: row.get("archived"),
        claimed_by: row.get("claimed_by"),
    })
}

async fn insert_transaction<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    ex: E,
    exom_path: &str,
    tx: &Tx,
) -> anyhow::Result<()> {
    let tx_time = parse_timestamptz(&tx.tx_time)?;
    let parent = tx.parent_tx_id.map(txid_to_i64);
    sqlx::query(
        r#"
        INSERT INTO transactions (exom_path, tx_id, tx_time, user_email, actor, action, refs, note, parent_tx_id, branch_id, session)
        VALUES ($1, $2, $3::timestamptz, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(exom_path)
    .bind(txid_to_i64(tx.tx_id))
    .bind(tx_time)
    .bind(tx.user_email.clone())
    .bind(if tx.actor.is_empty() {
        None::<String>
    } else {
        Some(tx.actor.clone())
    })
    .bind(tx.action.to_string())
    .bind(&tx.refs as &[EntityId])
    .bind(&tx.note)
    .bind(parent)
    .bind(&tx.branch_id)
    .bind(&tx.session)
    .execute(ex)
    .await?;
    Ok(())
}

async fn insert_fact<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    ex: E,
    exom_path: &str,
    f: &Fact,
) -> anyhow::Result<()> {
    let created_at = parse_timestamptz(&f.created_at)?;
    let valid_from = parse_timestamptz(&f.valid_from)?;
    let valid_to = parse_timestamptz_opt(f.valid_to.as_deref())?;
    let value_text = fact_value_to_pg_text(&f.value);
    sqlx::query(
        r#"
        INSERT INTO facts (
            exom_path, fact_id, predicate, value, created_at, created_by_tx,
            superseded_by_tx, revoked_by_tx, confidence, provenance, valid_from, valid_to
        )
        VALUES ($1, $2, $3, $4, $5::timestamptz, $6, $7, $8, $9, $10, $11::timestamptz, $12::timestamptz)
        "#,
    )
    .bind(exom_path)
    .bind(&f.fact_id)
    .bind(&f.predicate)
    .bind(value_text)
    .bind(created_at)
    .bind(txid_to_i64(f.created_by_tx))
    .bind(f.superseded_by_tx.map(txid_to_i64))
    .bind(f.revoked_by_tx.map(txid_to_i64))
    .bind(f.confidence)
    .bind(&f.provenance)
    .bind(valid_from)
    .bind(valid_to)
    .execute(ex)
    .await?;
    Ok(())
}

async fn insert_observation<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    ex: E,
    exom_path: &str,
    o: &Observation,
) -> anyhow::Result<()> {
    let created_at = parse_timestamptz(&o.created_at)?;
    let valid_from = parse_timestamptz(&o.valid_from)?;
    let valid_to = parse_timestamptz_opt(o.valid_to.as_deref())?;
    sqlx::query(
        r#"
        INSERT INTO observations (
            exom_path, obs_id, source_type, source_ref, content, created_at,
            confidence, tx_id, tags, valid_from, valid_to
        )
        VALUES ($1, $2, $3, $4, $5, $6::timestamptz, $7, $8, $9, $10::timestamptz, $11::timestamptz)
        "#,
    )
    .bind(exom_path)
    .bind(&o.obs_id)
    .bind(&o.source_type)
    .bind(&o.source_ref)
    .bind(&o.content)
    .bind(created_at)
    .bind(o.confidence)
    .bind(txid_to_i64(o.tx_id))
    .bind(&o.tags as &[String])
    .bind(valid_from)
    .bind(valid_to)
    .execute(ex)
    .await?;
    Ok(())
}

async fn insert_belief<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    ex: E,
    exom_path: &str,
    b: &Belief,
) -> anyhow::Result<()> {
    let valid_from = parse_timestamptz(&b.valid_from)?;
    let valid_to = parse_timestamptz_opt(b.valid_to.as_deref())?;
    sqlx::query(
        r#"
        INSERT INTO beliefs (
            exom_path, belief_id, claim_text, status, confidence, supported_by,
            created_by_tx, valid_from, valid_to, rationale
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz, $9::timestamptz, $10)
        "#,
    )
    .bind(exom_path)
    .bind(&b.belief_id)
    .bind(&b.claim_text)
    .bind(b.status.to_string())
    .bind(b.confidence)
    .bind(&b.supported_by as &[EntityId])
    .bind(txid_to_i64(b.created_by_tx))
    .bind(valid_from)
    .bind(valid_to)
    .bind(&b.rationale)
    .execute(ex)
    .await?;
    Ok(())
}

async fn insert_branch<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    ex: E,
    exom_path: &str,
    b: &Branch,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO branches (
            exom_path, branch_id, name, parent_branch_id, created_tx_id, archived, claimed_by
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(exom_path)
    .bind(&b.branch_id)
    .bind(&b.name)
    .bind(&b.parent_branch_id)
    .bind(txid_to_i64(b.created_tx_id))
    .bind(b.archived)
    .bind(&b.claimed_by)
    .execute(ex)
    .await?;
    Ok(())
}

#[async_trait]
impl ExomDb for PgExomDb {
    async fn load_transactions(&self, exom_path: &str) -> anyhow::Result<Vec<Tx>> {
        let rows = sqlx::query(
            r#"
            SELECT tx_id, tx_time, actor, action, refs, note, parent_tx_id, branch_id, session, user_email
            FROM transactions
            WHERE exom_path = $1
            ORDER BY tx_id
            "#,
        )
        .bind(exom_path)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(tx_from_row).collect()
    }

    async fn save_transactions(&self, exom_path: &str, txs: &[Tx]) -> anyhow::Result<()> {
        let mut ex = self.pool.begin().await?;
        sqlx::query("DELETE FROM transactions WHERE exom_path = $1")
            .bind(exom_path)
            .execute(&mut *ex)
            .await?;
        for tx in txs {
            insert_transaction(&mut *ex, exom_path, tx).await?;
        }
        ex.commit().await?;
        Ok(())
    }

    async fn append_transaction(&self, exom_path: &str, tx: &Tx) -> anyhow::Result<()> {
        insert_transaction(&self.pool, exom_path, tx).await
    }

    async fn load_facts(&self, exom_path: &str) -> anyhow::Result<Vec<Fact>> {
        let rows = sqlx::query(
            r#"
            SELECT fact_id, predicate, value, created_at, created_by_tx, superseded_by_tx, revoked_by_tx,
                   confidence, provenance, valid_from, valid_to
            FROM facts
            WHERE exom_path = $1
            "#,
        )
        .bind(exom_path)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(fact_from_row).collect()
    }

    async fn save_facts(&self, exom_path: &str, facts: &[Fact]) -> anyhow::Result<()> {
        let mut ex = self.pool.begin().await?;
        sqlx::query("DELETE FROM facts WHERE exom_path = $1")
            .bind(exom_path)
            .execute(&mut *ex)
            .await?;
        for f in facts {
            insert_fact(&mut *ex, exom_path, f).await?;
        }
        ex.commit().await?;
        Ok(())
    }

    async fn load_observations(&self, exom_path: &str) -> anyhow::Result<Vec<Observation>> {
        let rows = sqlx::query(
            r#"
            SELECT obs_id, source_type, source_ref, content, created_at, confidence, tx_id, tags, valid_from, valid_to
            FROM observations
            WHERE exom_path = $1
            "#,
        )
        .bind(exom_path)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(observation_from_row).collect()
    }

    async fn save_observations(
        &self,
        exom_path: &str,
        observations: &[Observation],
    ) -> anyhow::Result<()> {
        let mut ex = self.pool.begin().await?;
        sqlx::query("DELETE FROM observations WHERE exom_path = $1")
            .bind(exom_path)
            .execute(&mut *ex)
            .await?;
        for o in observations {
            insert_observation(&mut *ex, exom_path, o).await?;
        }
        ex.commit().await?;
        Ok(())
    }

    async fn load_beliefs(&self, exom_path: &str) -> anyhow::Result<Vec<Belief>> {
        let rows = sqlx::query(
            r#"
            SELECT belief_id, claim_text, status, confidence, supported_by, created_by_tx, valid_from, valid_to, rationale
            FROM beliefs
            WHERE exom_path = $1
            "#,
        )
        .bind(exom_path)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(belief_from_row).collect()
    }

    async fn save_beliefs(&self, exom_path: &str, beliefs: &[Belief]) -> anyhow::Result<()> {
        let mut ex = self.pool.begin().await?;
        sqlx::query("DELETE FROM beliefs WHERE exom_path = $1")
            .bind(exom_path)
            .execute(&mut *ex)
            .await?;
        for b in beliefs {
            insert_belief(&mut *ex, exom_path, b).await?;
        }
        ex.commit().await?;
        Ok(())
    }

    async fn load_branches(&self, exom_path: &str) -> anyhow::Result<Vec<Branch>> {
        let rows = sqlx::query(
            r#"
            SELECT branch_id, name, parent_branch_id, created_tx_id, archived, claimed_by
            FROM branches
            WHERE exom_path = $1
            "#,
        )
        .bind(exom_path)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(branch_from_row).collect()
    }

    async fn save_branches(&self, exom_path: &str, branches: &[Branch]) -> anyhow::Result<()> {
        let mut ex = self.pool.begin().await?;
        sqlx::query("DELETE FROM branches WHERE exom_path = $1")
            .bind(exom_path)
            .execute(&mut *ex)
            .await?;
        for b in branches {
            insert_branch(&mut *ex, exom_path, b).await?;
        }
        ex.commit().await?;
        Ok(())
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
        let mut db = self.pool.begin().await?;
        insert_transaction(&mut *db, exom_path, tx).await?;
        if let Some(facts) = facts {
            sqlx::query("DELETE FROM facts WHERE exom_path = $1")
                .bind(exom_path)
                .execute(&mut *db)
                .await?;
            for f in facts {
                insert_fact(&mut *db, exom_path, f).await?;
            }
        }
        if let Some(observations) = observations {
            sqlx::query("DELETE FROM observations WHERE exom_path = $1")
                .bind(exom_path)
                .execute(&mut *db)
                .await?;
            for o in observations {
                insert_observation(&mut *db, exom_path, o).await?;
            }
        }
        if let Some(beliefs) = beliefs {
            sqlx::query("DELETE FROM beliefs WHERE exom_path = $1")
                .bind(exom_path)
                .execute(&mut *db)
                .await?;
            for b in beliefs {
                insert_belief(&mut *db, exom_path, b).await?;
            }
        }
        if let Some(branches) = branches {
            sqlx::query("DELETE FROM branches WHERE exom_path = $1")
                .bind(exom_path)
                .execute(&mut *db)
                .await?;
            for b in branches {
                insert_branch(&mut *db, exom_path, b).await?;
            }
        }
        db.commit().await?;
        Ok(())
    }

    async fn delete_exoms_with_prefix(&self, prefix: &str) -> anyhow::Result<u64> {
        let mut tx = self.pool.begin().await?;
        let params = [prefix, &format!("{prefix}/%")];
        sqlx::query("DELETE FROM branches WHERE exom_path = $1 OR exom_path LIKE $2")
            .bind(params[0])
            .bind(params[1])
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM beliefs WHERE exom_path = $1 OR exom_path LIKE $2")
            .bind(params[0])
            .bind(params[1])
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM observations WHERE exom_path = $1 OR exom_path LIKE $2")
            .bind(params[0])
            .bind(params[1])
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM facts WHERE exom_path = $1 OR exom_path LIKE $2")
            .bind(params[0])
            .bind(params[1])
            .execute(&mut *tx)
            .await?;
        let res = sqlx::query("DELETE FROM transactions WHERE exom_path = $1 OR exom_path LIKE $2")
            .bind(params[0])
            .bind(params[1])
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(res.rows_affected())
    }
}
