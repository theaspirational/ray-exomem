//! Path-based access control and Rayfall body authorization.

use crate::auth::store::{AuthStore, ShareGrant};
use crate::auth::{AccessLevel, User};
use crate::exom::AclMode;
use crate::rayfall_ast::CanonicalForm;

#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    #[error("access denied to {path}: requires {required}, have {actual}")]
    Denied {
        path: String,
        required: String,
        actual: String,
    },
}

fn access_level_label(level: AccessLevel) -> &'static str {
    match level {
        AccessLevel::Denied => "denied",
        AccessLevel::ReadOnly => "read-only",
        AccessLevel::ReadWrite => "read-write",
        AccessLevel::FullAccess => "full-access",
    }
}

/// Result of looking up the access-relevant meta of a `public/*` exom.
/// The variant determines the Model A / co-edit access decision; see
/// `resolve_access`.
#[derive(Debug, Clone, PartialEq)]
pub enum PublicOwner {
    /// No exom is loaded at this path. For `public/*` this is "creatable
    /// by any authenticated user" — the writer becomes the owner.
    /// For `public` itself (the root folder) and unloaded subtrees this
    /// also applies; tree walks already filter unreachable paths.
    Unknown,
    /// The exom exists but `created_by` is empty — pre-Model-A legacy
    /// where startup migration could not infer an owner from the `main`
    /// branch's TOFU claim. ReadOnly for everyone; only a future
    /// top-admin recovery route can adopt it.
    Ownerless,
    /// Exom exists with a stamped creator and a known write-policy mode.
    /// `acl_mode == CoEdit` elevates non-creators to `ReadWrite` in
    /// `public/*`; `SoloEdit` keeps them at `ReadOnly`.
    Owner {
        email: String,
        acl_mode: AclMode,
    },
}

/// Resolve the effective access level for `user` at `path`.
///
/// The role enum (`Regular`/`Admin`/`TopAdmin`) gates *operator* routes
/// under `/auth/admin`; it confers no implicit access to user data.
///
/// `public_owner` is the lookup result for the path (Model A): only
/// meaningful when the path is in the `public/*` namespace. Pass
/// `PublicOwner::Unknown` outside `public/*` (it's ignored there).
///
/// Evaluation order:
/// 1. Path is in the `public/` namespace ->
///    - Owner matches user -> FullAccess
///    - Owner is someone else AND exom is co-edit -> ReadWrite
///    - Owner is someone else AND exom is solo-edit -> ReadOnly (read + fork)
///    - Ownerless legacy -> ReadOnly (top-admin recovery only)
///    - Unknown (path doesn't exist) -> FullAccess (creating a new
///      exom is the writer's right; they'll be stamped as creator)
/// 2. Path starts with user's email -> FullAccess (owner)
/// 3. Share grants for (path, user.email) -> best match
/// 4. Denied
pub async fn resolve_access(
    user: &User,
    path: &str,
    store: &AuthStore,
    public_owner: PublicOwner,
) -> AccessLevel {
    if path == "public" || path.starts_with("public/") {
        return match public_owner {
            PublicOwner::Owner { ref email, .. } if email == &user.email => {
                AccessLevel::FullAccess
            }
            PublicOwner::Owner {
                acl_mode: AclMode::CoEdit,
                ..
            } => AccessLevel::ReadWrite,
            PublicOwner::Owner { .. } => AccessLevel::ReadOnly,
            PublicOwner::Ownerless => AccessLevel::ReadOnly,
            PublicOwner::Unknown => AccessLevel::FullAccess,
        };
    }

    if path == user.email || path.starts_with(&format!("{}/", user.email)) {
        return AccessLevel::FullAccess;
    }

    let grants = store.shares_for_grantee(&user.email).await;
    resolve_from_grants(path, &grants)
}

/// Pure grant-resolution logic (testable without a store).
///
/// Finds the deepest grant whose path matches `path` (exact or prefix with `/`
/// separator). Deeper grants override shallower ones.
pub fn resolve_from_grants(path: &str, grants: &[ShareGrant]) -> AccessLevel {
    let mut best: Option<(&ShareGrant, usize)> = None;

    for grant in grants {
        let matches = if path == grant.path {
            true
        } else {
            path.starts_with(&grant.path) && path.as_bytes().get(grant.path.len()) == Some(&b'/')
        };

        if !matches {
            continue;
        }

        let depth = grant.path.matches('/').count();
        if best.is_none() || depth > best.unwrap().1 {
            best = Some((grant, depth));
        }
    }

    match best {
        Some((grant, _)) => match grant.permission.as_str() {
            "read-write" => AccessLevel::ReadWrite,
            "read" => AccessLevel::ReadOnly,
            _ => AccessLevel::Denied,
        },
        None => AccessLevel::Denied,
    }
}

/// Pre-execution authorization for lowered Rayfall forms.
///
/// Extracts the exom path and operation kind from each canonical form,
/// calls `resolve_access`, and rejects the entire batch if any path is
/// denied or insufficiently privileged.
///
/// `owner_for` resolves the `created_by` of the target exom — the caller
/// (typically server::api_*) looks this up from in-memory `ExomState` so
/// this auth module stays free of any state-cache dependency. Return
/// `None` for folders or unknown paths; resolve_access treats `None` as
/// "no public owner" (which forces `public/*` paths to ReadOnly).
pub async fn authorize_rayfall<F>(
    user: &User,
    forms: &[CanonicalForm],
    store: &AuthStore,
    owner_for: F,
) -> Result<(), AuthzError>
where
    F: Fn(&str) -> PublicOwner,
{
    for form in forms {
        let (path, is_write) = match form {
            CanonicalForm::Query(q) => (q.exom.as_str(), false),
            CanonicalForm::Rule(r) => (r.exom.as_str(), true),
            CanonicalForm::AssertFact(m) => (m.exom.as_str(), true),
            CanonicalForm::RetractFact(m) => (m.exom.as_str(), true),
        };

        // Fail-closed: empty path is a deny
        if path.is_empty() {
            return Err(AuthzError::Denied {
                path: "(empty)".into(),
                required: if is_write { "read-write" } else { "read-only" }.into(),
                actual: "denied".into(),
            });
        }

        let owner = owner_for(path);
        let level = resolve_access(user, path, store, owner).await;

        if is_write && !level.can_write() {
            return Err(AuthzError::Denied {
                path: path.into(),
                required: "read-write".into(),
                actual: access_level_label(level).into(),
            });
        }

        if !is_write && !level.can_read() {
            return Err(AuthzError::Denied {
                path: path.into(),
                required: "read-only".into(),
                actual: access_level_label(level).into(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::store::ShareGrant;
    use crate::auth::UserRole;

    fn user(email: &str, role: UserRole) -> User {
        User {
            email: email.into(),
            display_name: "Test".into(),
            provider: "mock".into(),
            session_id: None,
            api_key_label: None,
            role,
        }
    }

    fn grant(path: &str, grantee: &str, permission: &str) -> ShareGrant {
        ShareGrant {
            share_id: "s1".into(),
            owner_email: "owner@co.com".into(),
            path: path.into(),
            grantee_email: grantee.into(),
            permission: permission.into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn no_grants_means_denied() {
        assert_eq!(
            resolve_from_grants("alice@co.com/proj", &[]),
            AccessLevel::Denied
        );
    }

    #[test]
    fn direct_read_grant() {
        let grants = vec![grant("alice@co.com/proj", "bob@co.com", "read")];
        assert_eq!(
            resolve_from_grants("alice@co.com/proj", &grants),
            AccessLevel::ReadOnly
        );
    }

    #[test]
    fn inherited_grant() {
        let grants = vec![grant("alice@co.com/proj", "bob@co.com", "read-write")];
        assert_eq!(
            resolve_from_grants("alice@co.com/proj/main", &grants),
            AccessLevel::ReadWrite
        );
    }

    #[test]
    fn deeper_grant_overrides() {
        let grants = vec![
            grant("alice@co.com/proj", "bob@co.com", "read"),
            grant("alice@co.com/proj/secret", "bob@co.com", "read-write"),
        ];
        assert_eq!(
            resolve_from_grants("alice@co.com/proj/secret/exom1", &grants),
            AccessLevel::ReadWrite
        );
        assert_eq!(
            resolve_from_grants("alice@co.com/proj/other", &grants),
            AccessLevel::ReadOnly
        );
    }

    #[test]
    fn no_partial_path_match() {
        // "alice@co.com/projects" should NOT match grant on "alice@co.com/proj"
        let grants = vec![grant("alice@co.com/proj", "bob@co.com", "read")];
        assert_eq!(
            resolve_from_grants("alice@co.com/projects", &grants),
            AccessLevel::Denied
        );
    }

    #[tokio::test]
    async fn admin_role_does_not_grant_data_access() {
        let admin = user("admin@co.com", UserRole::Admin);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&admin, "alice@co.com/proj", &store, PublicOwner::Unknown).await,
            AccessLevel::Denied
        );
    }

    #[tokio::test]
    async fn top_admin_role_does_not_grant_data_access() {
        let top = user("top@co.com", UserRole::TopAdmin);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&top, "alice@co.com/proj", &store, PublicOwner::Unknown).await,
            AccessLevel::Denied
        );
    }

    #[tokio::test]
    async fn admin_still_owns_their_own_namespace() {
        let admin = user("admin@co.com", UserRole::Admin);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&admin, "admin@co.com/proj", &store, PublicOwner::Unknown).await,
            AccessLevel::FullAccess
        );
    }

    #[tokio::test]
    async fn owner_gets_full_access() {
        let alice = user("alice@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&alice, "alice@co.com", &store, PublicOwner::Unknown).await,
            AccessLevel::FullAccess
        );
        assert_eq!(
            resolve_access(&alice, "alice@co.com/proj", &store, PublicOwner::Unknown).await,
            AccessLevel::FullAccess
        );
    }

    #[tokio::test]
    async fn regular_user_denied_without_grants() {
        let bob = user("bob@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&bob, "alice@co.com/proj", &store, PublicOwner::Unknown).await,
            AccessLevel::Denied
        );
    }

    #[tokio::test]
    async fn public_namespace_creator_gets_full_access() {
        // Model A: only the creator (matched against public_owner) writes a
        // public exom. Everyone else reads + forks (or writes, if co-edit).
        let alice = user("alice@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(
                &alice,
                "public/work/proj/main",
                &store,
                PublicOwner::Owner {
                    email: "alice@co.com".into(),
                    acl_mode: AclMode::SoloEdit,
                },
            )
            .await,
            AccessLevel::FullAccess
        );
    }

    #[tokio::test]
    async fn public_namespace_non_creator_is_read_only() {
        let bob = user("bob@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(
                &bob,
                "public/work/proj/main",
                &store,
                PublicOwner::Owner {
                    email: "alice@co.com".into(),
                    acl_mode: AclMode::SoloEdit,
                },
            )
            .await,
            AccessLevel::ReadOnly
        );
    }

    #[tokio::test]
    async fn public_namespace_co_edit_non_creator_is_read_write() {
        let bob = user("bob@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(
                &bob,
                "public/work/wiki/main",
                &store,
                PublicOwner::Owner {
                    email: "alice@co.com".into(),
                    acl_mode: AclMode::CoEdit,
                },
            )
            .await,
            AccessLevel::ReadWrite
        );
    }

    #[tokio::test]
    async fn public_namespace_co_edit_creator_still_full_access() {
        let alice = user("alice@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(
                &alice,
                "public/work/wiki/main",
                &store,
                PublicOwner::Owner {
                    email: "alice@co.com".into(),
                    acl_mode: AclMode::CoEdit,
                },
            )
            .await,
            AccessLevel::FullAccess
        );
    }

    #[tokio::test]
    async fn public_namespace_ownerless_legacy_is_read_only() {
        // Migration-failed legacy: exom exists but `created_by` is empty.
        // Ownership is locked until top-admin recovery; nobody auto-claims.
        let bob = user("bob@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&bob, "public/legacy/exom", &store, PublicOwner::Ownerless).await,
            AccessLevel::ReadOnly
        );
    }

    #[tokio::test]
    async fn public_namespace_unknown_path_allows_create() {
        // Path not loaded (folder root or fresh sub-path). Any
        // authenticated user can create here; they become the owner.
        let bob = user("bob@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&bob, "public", &store, PublicOwner::Unknown).await,
            AccessLevel::FullAccess
        );
        assert_eq!(
            resolve_access(
                &bob,
                "public/work/team/project/concepts/main",
                &store,
                PublicOwner::Unknown,
            )
            .await,
            AccessLevel::FullAccess
        );
    }

    #[tokio::test]
    async fn public_prefix_does_not_match_unrelated_segment() {
        // "publication/..." must NOT bind to the public-namespace clause
        let bob = user("bob@co.com", UserRole::Regular);
        let store = make_test_store();
        assert_eq!(
            resolve_access(&bob, "publication/foo", &store, PublicOwner::Unknown).await,
            AccessLevel::Denied
        );
    }

    #[test]
    fn unknown_permission_maps_to_denied() {
        let grants = vec![grant("alice@co.com/proj", "bob@co.com", "execute")];
        assert_eq!(
            resolve_from_grants("alice@co.com/proj", &grants),
            AccessLevel::Denied
        );
    }

    #[test]
    fn exact_path_grant_match() {
        let grants = vec![grant("alice@co.com/proj", "bob@co.com", "read")];
        assert_eq!(
            resolve_from_grants("alice@co.com/proj", &grants),
            AccessLevel::ReadOnly
        );
    }

    fn make_test_store() -> AuthStore {
        use dashmap::DashMap;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::Mutex;
        AuthStore {
            auth_db: None,
            exom_disk: PathBuf::from("/tmp/fake"),
            jsonl_path: PathBuf::from("/tmp/fake/auth.jsonl"),
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(Vec::new()),
            allowed_emails: Mutex::new(HashMap::new()),
            share_grants: Mutex::new(Vec::new()),
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
        }
    }
}
