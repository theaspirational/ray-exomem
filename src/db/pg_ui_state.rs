//! Postgres-backed UI state persistence (`UiStateDb`).

use async_trait::async_trait;
use sqlx::{PgPool, Row};

use crate::db::UiStateDb;

pub struct PgUiStateDb {
    pool: PgPool,
}

impl PgUiStateDb {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UiStateDb for PgUiStateDb {
    async fn get_graph_layout(
        &self,
        user_email: &str,
        scope: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let row = sqlx::query(
            "SELECT layout FROM ui_graph_layouts WHERE user_email = $1 AND scope = $2",
        )
        .bind(user_email)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.get::<serde_json::Value, _>("layout")))
    }

    async fn upsert_graph_layout(
        &self,
        user_email: &str,
        scope: &str,
        layout: &serde_json::Value,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO ui_graph_layouts (user_email, scope, layout, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (user_email, scope)
             DO UPDATE SET layout = EXCLUDED.layout, updated_at = now()",
        )
        .bind(user_email)
        .bind(scope)
        .bind(layout)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
