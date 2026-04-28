pub mod access;
pub mod admin;
pub mod middleware;
pub mod provider;
pub mod routes;
pub mod store;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserRole {
    Regular,
    Admin,
    TopAdmin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub email: String,
    pub display_name: String,
    pub provider: String,
    /// Cookie-auth session id. Mutually exclusive with `api_key_label`.
    pub session_id: Option<String>,
    /// Bearer-auth API key label. Mutually exclusive with `session_id`.
    /// Surfaced as the default `agent` for writes when no explicit `agent`
    /// arg/header is provided.
    pub api_key_label: Option<String>,
    pub role: UserRole,
}

impl User {
    pub fn is_admin(&self) -> bool {
        matches!(self.role, UserRole::Admin | UserRole::TopAdmin)
    }

    pub fn is_top_admin(&self) -> bool {
        matches!(self.role, UserRole::TopAdmin)
    }

    pub fn namespace_root(&self) -> &str {
        &self.email
    }
}

#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AccessLevel {
    Denied,
    ReadOnly,
    ReadWrite,
    FullAccess,
}

impl AccessLevel {
    pub fn can_read(&self) -> bool {
        *self >= AccessLevel::ReadOnly
    }

    pub fn can_write(&self) -> bool {
        *self >= AccessLevel::ReadWrite
    }

    pub fn is_owner(&self) -> bool {
        *self == AccessLevel::FullAccess
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_level_ordering() {
        assert!(AccessLevel::Denied < AccessLevel::ReadOnly);
        assert!(AccessLevel::ReadOnly < AccessLevel::ReadWrite);
        assert!(AccessLevel::ReadWrite < AccessLevel::FullAccess);
    }

    #[test]
    fn access_level_permissions() {
        assert!(!AccessLevel::Denied.can_read());
        assert!(!AccessLevel::Denied.can_write());
        assert!(AccessLevel::ReadOnly.can_read());
        assert!(!AccessLevel::ReadOnly.can_write());
        assert!(AccessLevel::ReadWrite.can_read());
        assert!(AccessLevel::ReadWrite.can_write());
        assert!(!AccessLevel::ReadWrite.is_owner());
        assert!(AccessLevel::FullAccess.can_read());
        assert!(AccessLevel::FullAccess.can_write());
        assert!(AccessLevel::FullAccess.is_owner());
    }

    #[test]
    fn user_role_checks() {
        let regular = User {
            email: "alice@co.com".into(),
            display_name: "Alice".into(),
            provider: "mock".into(),
            session_id: None,
            api_key_label: None,
            role: UserRole::Regular,
        };
        assert!(!regular.is_admin());
        assert!(!regular.is_top_admin());

        let admin = User {
            role: UserRole::Admin,
            ..regular.clone()
        };
        assert!(admin.is_admin());
        assert!(!admin.is_top_admin());

        let top = User {
            role: UserRole::TopAdmin,
            ..regular
        };
        assert!(top.is_admin());
        assert!(top.is_top_admin());
    }
}
