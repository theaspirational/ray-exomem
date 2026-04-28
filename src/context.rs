/// Three-axis attribution carried by every mutation (fact, rule, retraction).
///
/// - `user_email`: DB-bound identity from auth. `None` only for system-internal
///   mutations (e.g. builtin rule registration).
/// - `agent`: the tool/integration making the call (e.g. `"cursor"`,
///   `"claude-code-cli"`). For Bearer auth, defaults to the API key's label;
///   an explicit `agent` arg / `x-agent` header always wins. Cookie-auth (UI)
///   writes are `None`.
/// - `model`: the LLM the agent is using (e.g. `"claude-opus-4-7"`). Explicit
///   only — no fallback.
/// - `session`: cookie session id when applicable; carried through for tx
///   audit, not for attribution display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MutationContext {
    pub user_email: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub session: Option<String>,
}

impl MutationContext {
    /// Build from an authenticated `User`. `agent` falls back to the user's
    /// `api_key_label` (Bearer auth) when not explicitly supplied; cookie-auth
    /// users have no label so `agent` stays `None` unless explicitly set.
    pub fn from_user(
        user: &crate::auth::User,
        agent: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            user_email: Some(user.email.clone()),
            agent: agent.or_else(|| user.api_key_label.clone()),
            model,
            session: user.session_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{User, UserRole};

    fn user(email: &str, session: Option<&str>, label: Option<&str>) -> User {
        User {
            email: email.into(),
            display_name: email.into(),
            provider: "google".into(),
            session_id: session.map(str::to_string),
            api_key_label: label.map(str::to_string),
            role: UserRole::Regular,
        }
    }

    #[test]
    fn from_user_cookie_no_agent_fallback() {
        let u = user("alice@co.com", Some("sess-1"), None);
        let ctx = MutationContext::from_user(&u, None, None);
        assert_eq!(ctx.user_email.as_deref(), Some("alice@co.com"));
        assert_eq!(ctx.agent, None);
        assert_eq!(ctx.model, None);
        assert_eq!(ctx.session.as_deref(), Some("sess-1"));
    }

    #[test]
    fn from_user_bearer_falls_back_to_label() {
        let u = user("alice@co.com", None, Some("my-mcp"));
        let ctx = MutationContext::from_user(&u, None, Some("claude-opus-4-7".into()));
        assert_eq!(ctx.agent.as_deref(), Some("my-mcp"));
        assert_eq!(ctx.model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn from_user_explicit_agent_overrides_label() {
        let u = user("alice@co.com", None, Some("my-mcp"));
        let ctx = MutationContext::from_user(&u, Some("cursor".into()), None);
        assert_eq!(ctx.agent.as_deref(), Some("cursor"));
    }

    #[test]
    fn default_is_all_none() {
        let ctx = MutationContext::default();
        assert_eq!(ctx.user_email, None);
        assert_eq!(ctx.agent, None);
        assert_eq!(ctx.model, None);
        assert_eq!(ctx.session, None);
    }
}
