//! Database trait definitions and row types for auth and exom persistence.

use async_trait::async_trait;

use crate::auth::UserRole;

pub mod jsonl_auth;

#[cfg(feature = "postgres")]
pub mod pg_auth;

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StoredUser {
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub role: UserRole,
    pub active: bool,
    pub created_at: String,
    pub last_login: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_id: String,
    pub email: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct StoredApiKey {
    pub key_id: String,
    pub key_hash: String,
    pub email: String,
    pub label: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ApiKeyWithUser {
    pub key_id: String,
    pub key_hash: String,
    pub email: String,
    pub label: String,
    pub created_at: String,
    pub user: StoredUser,
}

#[derive(Debug, Clone)]
pub struct ShareGrant {
    pub share_id: String,
    pub owner_email: String,
    pub path: String,
    pub grantee_email: String,
    pub permission: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct AllowedEmail {
    pub email: String,
    pub alias: String,
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AuthDb: Send + Sync {
    async fn upsert_user(&self, user: &StoredUser) -> anyhow::Result<()>;

    async fn get_user(&self, email: &str) -> anyhow::Result<Option<StoredUser>>;

    async fn list_users(&self) -> anyhow::Result<Vec<StoredUser>>;

    async fn set_role(&self, email: &str, role: UserRole) -> anyhow::Result<()>;

    async fn deactivate_user(&self, email: &str) -> anyhow::Result<()>;

    async fn activate_user(&self, email: &str) -> anyhow::Result<()>;

    async fn delete_user(&self, email: &str) -> anyhow::Result<bool>;

    async fn update_last_login(&self, email: &str, at: &str) -> anyhow::Result<()>;

    async fn create_session(&self, session: &SessionRow) -> anyhow::Result<()>;

    async fn get_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRow>>;

    async fn delete_session(&self, session_id: &str) -> anyhow::Result<()>;

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionRow>>;

    async fn cleanup_expired_sessions(&self) -> anyhow::Result<usize>;

    async fn store_api_key(&self, key: &StoredApiKey) -> anyhow::Result<()>;

    async fn revoke_api_key(&self, key_id: &str) -> anyhow::Result<bool>;

    async fn rename_api_key(&self, key_id: &str, new_label: &str) -> anyhow::Result<bool>;

    async fn list_api_keys(&self) -> anyhow::Result<Vec<ApiKeyWithUser>>;

    async fn list_api_keys_for_user(&self, email: &str) -> anyhow::Result<Vec<StoredApiKey>>;

    async fn get_api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyWithUser>>;

    async fn add_share(&self, grant: &ShareGrant) -> anyhow::Result<()>;

    async fn revoke_share(&self, share_id: &str) -> anyhow::Result<bool>;

    async fn shares_for_grantee(&self, grantee_email: &str) -> anyhow::Result<Vec<ShareGrant>>;

    async fn shares_for_owner(&self, owner_email: &str) -> anyhow::Result<Vec<ShareGrant>>;

    async fn list_all_shares(&self) -> anyhow::Result<Vec<ShareGrant>>;

    async fn update_share_paths(&self, old_prefix: &str, new_prefix: &str) -> anyhow::Result<u64>;

    async fn add_domain(&self, domain: &str) -> anyhow::Result<()>;

    async fn remove_domain(&self, domain: &str) -> anyhow::Result<()>;

    async fn list_domains(&self) -> anyhow::Result<Vec<String>>;

    async fn add_allowed_email(&self, email: &str, alias: &str) -> anyhow::Result<()>;

    async fn remove_allowed_email(&self, email: &str) -> anyhow::Result<()>;

    async fn list_allowed_emails(&self) -> anyhow::Result<Vec<AllowedEmail>>;

    /// Wipe all user-derived auth state (users, sessions, api keys, shares).
    /// `allowed_domains` and `allowed_emails` are preserved so login policy
    /// survives the reset.
    async fn factory_reset(&self) -> anyhow::Result<()>;
}

#[cfg(feature = "postgres")]
pub async fn create_pool(database_url: &str) -> anyhow::Result<sqlx::PgPool> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

