//! Postgres-backed auth persistence (`AuthDb`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::auth::UserRole;
use crate::db::{ApiKeyWithUser, AuthDb, SessionRow, ShareGrant, StoredApiKey, StoredUser};

pub struct PgAuthDb {
    pool: PgPool,
}

impl PgAuthDb {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn role_to_str(role: &UserRole) -> &'static str {
    match role {
        UserRole::Regular => "regular",
        UserRole::Admin => "admin",
        UserRole::TopAdmin => "top-admin",
    }
}

fn str_to_role(s: &str) -> UserRole {
    match s {
        "admin" => UserRole::Admin,
        "top-admin" => UserRole::TopAdmin,
        _ => UserRole::Regular,
    }
}

fn ts_opt_string(dt: Option<DateTime<Utc>>) -> Option<String> {
    dt.map(|t| t.to_rfc3339())
}

fn user_from_users_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<StoredUser> {
    Ok(StoredUser {
        email: row.get("email"),
        display_name: row.get("display_name"),
        provider: row.get("provider"),
        role: str_to_role(&row.get::<String, _>("role")),
        active: row.get("active"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
        last_login: ts_opt_string(row.get::<Option<DateTime<Utc>>, _>("last_login")),
    })
}

fn session_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<SessionRow> {
    Ok(SessionRow {
        session_id: row.get("session_id"),
        email: row.get("email"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at").to_rfc3339(),
    })
}

fn stored_api_key_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<StoredApiKey> {
    Ok(StoredApiKey {
        key_id: row.get("key_id"),
        key_hash: row.get("key_hash"),
        email: row.get("email"),
        label: row.get("label"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
    })
}

fn share_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<ShareGrant> {
    Ok(ShareGrant {
        share_id: row.get("share_id"),
        owner_email: row.get("owner_email"),
        path: row.get("path"),
        grantee_email: row.get("grantee_email"),
        permission: row.get("permission"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
    })
}

fn api_key_with_user_from_join_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<ApiKeyWithUser> {
    let user = StoredUser {
        email: row.get::<String, _>("u_email"),
        display_name: row.get("display_name"),
        provider: row.get("provider"),
        role: str_to_role(&row.get::<String, _>("role")),
        active: row.get("active"),
        created_at: row.get::<DateTime<Utc>, _>("u_created_at").to_rfc3339(),
        last_login: ts_opt_string(row.get::<Option<DateTime<Utc>>, _>("last_login")),
    };
    Ok(ApiKeyWithUser {
        key_id: row.get("key_id"),
        key_hash: row.get("key_hash"),
        email: row.get("email"),
        label: row.get("label"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
        user,
    })
}

#[async_trait]
impl AuthDb for PgAuthDb {
    async fn upsert_user(&self, user: &StoredUser) -> anyhow::Result<()> {
        let role = role_to_str(&user.role);
        sqlx::query(
            r#"
            INSERT INTO users (email, display_name, provider, role, active, created_at)
            VALUES ($1, $2, $3, $4, true, now())
            ON CONFLICT (email) DO UPDATE SET
                display_name = $2,
                last_login = now()
            "#,
        )
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&user.provider)
        .bind(role)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_user(&self, email: &str) -> anyhow::Result<Option<StoredUser>> {
        let row = sqlx::query("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(user_from_users_row(&r)?)),
            None => Ok(None),
        }
    }

    async fn list_users(&self) -> anyhow::Result<Vec<StoredUser>> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(user_from_users_row).collect()
    }

    async fn set_role(&self, email: &str, role: UserRole) -> anyhow::Result<()> {
        let r = role_to_str(&role);
        sqlx::query("UPDATE users SET role = $1 WHERE email = $2")
            .bind(r)
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn deactivate_user(&self, email: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET active = false WHERE email = $1")
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn activate_user(&self, email: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET active = true WHERE email = $1")
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_last_login(&self, email: &str, at: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET last_login = $2::timestamptz WHERE email = $1")
            .bind(email)
            .bind(at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn create_session(&self, session: &SessionRow) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions (session_id, email, created_at, expires_at)
            VALUES ($1, $2, now(), $3::timestamptz)
            "#,
        )
        .bind(&session.session_id)
        .bind(&session.email)
        .bind(&session.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRow>> {
        let row =
            sqlx::query("SELECT * FROM sessions WHERE session_id = $1 AND expires_at > now()")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?;
        match row {
            Some(r) => Ok(Some(session_from_row(&r)?)),
            None => Ok(None),
        }
    }

    async fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionRow>> {
        let rows = sqlx::query("SELECT * FROM sessions WHERE expires_at > now()")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(session_from_row).collect()
    }

    async fn cleanup_expired_sessions(&self) -> anyhow::Result<usize> {
        let res = sqlx::query("DELETE FROM sessions WHERE expires_at < now()")
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() as usize)
    }

    async fn store_api_key(&self, key: &StoredApiKey) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO api_keys (key_id, key_hash, email, label, created_at)
            VALUES ($1, $2, $3, $4, now())
            "#,
        )
        .bind(&key.key_id)
        .bind(&key.key_hash)
        .bind(&key.email)
        .bind(&key.label)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn revoke_api_key(&self, key_id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM api_keys WHERE key_id = $1")
            .bind(key_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_api_keys(&self) -> anyhow::Result<Vec<ApiKeyWithUser>> {
        let rows = sqlx::query(
            r#"
            SELECT k.*, u.email AS u_email, u.display_name, u.provider, u.role, u.active,
                   u.created_at AS u_created_at, u.last_login
            FROM api_keys k
            JOIN users u ON k.email = u.email
            ORDER BY k.created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(api_key_with_user_from_join_row).collect()
    }

    async fn list_api_keys_for_user(&self, email: &str) -> anyhow::Result<Vec<StoredApiKey>> {
        let rows = sqlx::query("SELECT * FROM api_keys WHERE email = $1")
            .bind(email)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(stored_api_key_from_row).collect()
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyWithUser>> {
        let row = sqlx::query(
            r#"
            SELECT k.key_id, k.key_hash, k.email, k.label, k.created_at,
                   u.email AS u_email, u.display_name, u.provider, u.role, u.active,
                   u.created_at AS u_created_at, u.last_login
            FROM api_keys k
            JOIN users u ON k.email = u.email
            WHERE k.key_hash = $1
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => Ok(Some(api_key_with_user_from_join_row(&r)?)),
            None => Ok(None),
        }
    }

    async fn add_share(&self, grant: &ShareGrant) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO shares (share_id, owner_email, path, grantee_email, permission, created_at)
            VALUES ($1, $2, $3, $4, $5, now())
            "#,
        )
        .bind(&grant.share_id)
        .bind(&grant.owner_email)
        .bind(&grant.path)
        .bind(&grant.grantee_email)
        .bind(&grant.permission)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn revoke_share(&self, share_id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM shares WHERE share_id = $1")
            .bind(share_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn shares_for_grantee(&self, grantee_email: &str) -> anyhow::Result<Vec<ShareGrant>> {
        let rows = sqlx::query("SELECT * FROM shares WHERE grantee_email = $1")
            .bind(grantee_email)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(share_from_row).collect()
    }

    async fn shares_for_owner(&self, owner_email: &str) -> anyhow::Result<Vec<ShareGrant>> {
        let rows = sqlx::query("SELECT * FROM shares WHERE owner_email = $1")
            .bind(owner_email)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(share_from_row).collect()
    }

    async fn list_all_shares(&self) -> anyhow::Result<Vec<ShareGrant>> {
        let rows = sqlx::query("SELECT * FROM shares")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(share_from_row).collect()
    }

    async fn update_share_paths(&self, old_prefix: &str, new_prefix: &str) -> anyhow::Result<u64> {
        let res = sqlx::query(
            r#"
            UPDATE shares
            SET path = $2 || substring(path, length($1) + 1)
            WHERE path = $1 OR path LIKE $1 || '/%'
            "#,
        )
        .bind(old_prefix)
        .bind(new_prefix)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn add_domain(&self, domain: &str) -> anyhow::Result<()> {
        let domain = domain.trim().to_lowercase();
        if domain.is_empty() {
            return Ok(());
        }
        sqlx::query(
            r#"
            INSERT INTO allowed_domains (domain) VALUES ($1)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(&domain)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_domain(&self, domain: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM allowed_domains WHERE domain = $1")
            .bind(domain)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_domains(&self) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query("SELECT domain FROM allowed_domains ORDER BY domain")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| r.try_get::<String, _>("domain").map_err(Into::into))
            .collect()
    }
}
