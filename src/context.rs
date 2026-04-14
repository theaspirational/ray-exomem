/// Attribution context for every mutation (fact, rule, retraction).
/// Flows from CLI args / HTTP headers → Brain Tx metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationContext {
    pub actor: String,
    pub session: Option<String>,
    pub model: Option<String>,
}

impl MutationContext {
    /// Build from an authenticated user. Actor is always user's email.
    /// Client-supplied actor values are ignored on authenticated requests.
    pub fn from_user(user: &crate::auth::User, model: Option<String>) -> Self {
        Self {
            actor: user.email.clone(),
            session: user.session_id.clone(),
            model,
        }
    }
}

impl Default for MutationContext {
    fn default() -> Self {
        Self {
            actor: "unknown".into(),
            session: None,
            model: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{User, UserRole};

    #[test]
    fn from_user_uses_email_as_actor() {
        let user = User {
            email: "alice@co.com".into(),
            display_name: "Alice".into(),
            provider: "google".into(),
            session_id: Some("sess-1".into()),
            role: UserRole::Regular,
        };
        let ctx = MutationContext::from_user(&user, Some("claude-4".into()));
        assert_eq!(ctx.actor, "alice@co.com");
        assert_eq!(ctx.session, Some("sess-1".into()));
        assert_eq!(ctx.model, Some("claude-4".into()));
    }
}
