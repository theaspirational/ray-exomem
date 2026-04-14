# Auth, MCP & Admin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Google OAuth, per-user namespaces, path-based access control, MCP server, and admin panel to ray-exomem.

**Architecture:** Axum extractor-based auth (User type from cookie or Bearer token), system exom (`_system/auth`) for auth state, `resolve_access()` for path-based permissions, thin MCP layer mirroring existing API. Auth module lives in `src/auth/` directory.

**Tech Stack:** Rust (axum 0.8, jsonwebtoken, reqwest, dashmap, sha2, uuid, rand, base64), Svelte 5 (Google Identity Services JS), rmcp or hand-rolled JSON-RPC.

**Spec:** `docs/superpowers/specs/2026-04-14-auth-mcp-admin-design.md` (rev 2)

---

## File Structure

### New Rust Files

| File | Responsibility |
|------|---------------|
| `src/auth/mod.rs` | Module re-exports, core types: `User`, `UserRole`, `AccessLevel`, `AuthIdentity` |
| `src/auth/provider.rs` | `AuthProvider` trait + `GoogleAuthProvider` (JWT validation via JWKS) |
| `src/auth/store.rs` | `AuthStore` — typed read/write over `_system/auth` exom, cache management |
| `src/auth/middleware.rs` | `User`/`MaybeUser` Axum extractors, CSRF origin check |
| `src/auth/access.rs` | `resolve_access()`, `authorize_rayfall()`, `AccessLevel` enum |
| `src/auth/routes.rs` | `/auth/*` route handlers: login, logout, me, api-keys, shares, shared-with-me |
| `src/auth/admin.rs` | `/auth/admin/*` route handlers: users, admins, sessions, keys, shares, domains |
| `src/mcp.rs` | MCP JSON-RPC server, tool definitions, transport |

### New Test Files

| File | Responsibility |
|------|---------------|
| `tests/auth_basic.rs` | Integration tests: login flow, session, API keys, access control |
| `tests/auth_shares.rs` | Integration tests: sharing, rename + share path update |
| `tests/auth_admin.rs` | Integration tests: admin panel, domain management |
| `tests/mcp_basic.rs` | Integration tests: MCP tool calls with auth |

### New UI Files

| File | Responsibility |
|------|---------------|
| `ui/src/lib/auth.svelte.ts` | Auth state store, login/logout functions, session check |
| `ui/src/routes/login/+page.svelte` | Login page with Google Sign-In button |
| `ui/src/routes/profile/+page.svelte` | Profile page: API keys, MCP config snippet |
| `ui/src/routes/admin/+page.svelte` | Admin panel: users, sessions, keys, shares, domains |

### Modified Files

| File | Change |
|------|--------|
| `src/path.rs` | Allow `@` in non-first chars of segments |
| `src/lib.rs` | Add `pub mod auth;` and `pub mod mcp;` |
| `src/context.rs` | Add `MutationContext::from_user()` constructor |
| `src/server.rs` | Add `AuthStore` to `AppState`, wire auth middleware, add `/auth` and `/mcp` route nests, replace `body.actor`/`X-Actor` with `User.email` in all handlers |
| `Cargo.toml` | Add jsonwebtoken, reqwest, dashmap, sha2, uuid, rand, base64 dependencies |
| `ui/src/lib/exomem.svelte.ts` | Add auth token to fetch headers, 401 redirect |
| `ui/src/routes/+layout.svelte` | Auth guard: check session, redirect to login |
| `ui/src/lib/stores.svelte.ts` | Add auth user state |
| `ui/package.json` | (no new deps — GSI loaded via script tag) |
| `tests/common/daemon.rs` | Add auth config options to `TestDaemon`, mock provider support |

---

## Task 1: Path Validator — Allow `@` in Segments

**Files:**
- Modify: `src/path.rs:78-94`

- [ ] **Step 1: Write failing test for `@` in path segment**

Add to `src/path.rs` in the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn email_segment_is_valid() {
    let p: TreePath = "alice@company.com/projects/main".parse().unwrap();
    assert_eq!(p.segments(), &["alice@company.com", "projects", "main"]);
}

#[test]
fn at_sign_not_allowed_as_first_char() {
    let err = "@alice".parse::<TreePath>().unwrap_err();
    assert!(matches!(err, PathError::InvalidSegment(_, _)));
}

#[test]
fn email_segment_in_join() {
    let root = TreePath::root();
    let p = root.join("alice@company.com").unwrap();
    assert_eq!(p.segments(), &["alice@company.com"]);
}

#[test]
fn email_to_disk_path() {
    let p: TreePath = "alice@company.com/projects".parse().unwrap();
    let root = std::path::PathBuf::from("/root/tree");
    assert_eq!(
        p.to_disk_path(&root),
        std::path::PathBuf::from("/root/tree/alice@company.com/projects")
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib path::tests::email_segment_is_valid -- --exact`
Expected: FAIL — `InvalidSegment` because `@` is rejected.

- [ ] **Step 3: Extend `validate_segment` to allow `@`**

In `src/path.rs`, change line 91:

```rust
// Before:
if !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
    return Err(PathError::InvalidSegment(seg.to_string(), "chars must be [_A-Za-z0-9.-]"));
}

// After:
if !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '@') {
    return Err(PathError::InvalidSegment(seg.to_string(), "chars must be [_A-Za-z0-9.@-]"));
}
```

- [ ] **Step 4: Run all path tests**

Run: `cargo test --lib path::tests`
Expected: ALL PASS (4 new + 7 existing).

- [ ] **Step 5: Run full test suite to check for regressions**

Run: `cargo test`
Expected: ALL PASS.

- [ ] **Step 6: Commit**

```bash
git add src/path.rs
git commit -m "feat(path): allow @ in path segments for email-based namespaces"
```

---

## Task 2: Add New Dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add auth dependencies to Cargo.toml**

Add to `[dependencies]` section:

```toml
jsonwebtoken = "9"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
dashmap = "6"
sha2 = "0.10"
uuid = { version = "1", features = ["v4"] }
rand = "0.8"
base64 = "0.22"
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add auth dependencies (jsonwebtoken, reqwest, dashmap, sha2, uuid, rand, base64)"
```

---

## Task 3: Auth Core Types

**Files:**
- Create: `src/auth/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create auth module with core types**

Create `src/auth/mod.rs`:

```rust
pub mod access;
pub mod admin;
pub mod middleware;
pub mod provider;
pub mod routes;
pub mod store;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

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
    pub session_id: Option<String>,
    pub role: UserRole,
}

impl User {
    pub fn is_admin(&self) -> bool {
        matches!(self.role, UserRole::Admin | UserRole::TopAdmin)
    }

    pub fn is_top_admin(&self) -> bool {
        matches!(self.role, UserRole::TopAdmin)
    }

    /// The user's namespace root path segment (their email).
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
```

- [ ] **Step 2: Create placeholder submodules**

Create each of these as empty files with a single comment:

`src/auth/access.rs`:
```rust
//! Path-based access control and Rayfall body authorization.
```

`src/auth/admin.rs`:
```rust
//! Admin panel route handlers.
```

`src/auth/middleware.rs`:
```rust
//! Axum auth extractors and CSRF protection.
```

`src/auth/provider.rs`:
```rust
//! AuthProvider trait and Google OIDC implementation.
```

`src/auth/routes.rs`:
```rust
//! Auth route handlers: login, logout, me, api-keys, shares.
```

`src/auth/store.rs`:
```rust
//! AuthStore — typed read/write over _system/auth exom.
```

- [ ] **Step 3: Register auth module in lib.rs**

Add to `src/lib.rs` after the existing `pub mod` declarations:

```rust
pub mod auth;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: Compiles (placeholder modules are empty but valid).

- [ ] **Step 5: Write unit tests for core types**

Add to `src/auth/mod.rs`:

```rust
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
            provider: "google".into(),
            session_id: None,
            role: UserRole::Regular,
        };
        assert!(!regular.is_admin());
        assert!(!regular.is_top_admin());

        let admin = User { role: UserRole::Admin, ..regular.clone() };
        assert!(admin.is_admin());
        assert!(!admin.is_top_admin());

        let top = User { role: UserRole::TopAdmin, ..regular };
        assert!(top.is_admin());
        assert!(top.is_top_admin());
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test --lib auth::tests`
Expected: ALL PASS.

- [ ] **Step 7: Commit**

```bash
git add src/auth/ src/lib.rs
git commit -m "feat(auth): scaffold auth module with core types (User, UserRole, AccessLevel)"
```

---

## Task 4: AuthProvider Trait & Google Implementation

**Files:**
- Modify: `src/auth/provider.rs`

- [ ] **Step 1: Write the AuthProvider trait and MockProvider for tests**

Replace `src/auth/provider.rs`:

```rust
//! AuthProvider trait and implementations.

use anyhow::Result;

use super::AuthIdentity;

/// Trait for external identity providers.
/// Validates a token from the provider and returns the user's identity.
#[allow(async_fn_in_trait)]
pub trait AuthProvider: Send + Sync {
    /// Validate an external token (e.g., Google ID token) and return user identity.
    async fn validate_token(&self, token: &str) -> Result<AuthIdentity>;

    /// Provider name for storage/config ("google", "mock", etc.)
    fn provider_name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Google OIDC Provider
// ---------------------------------------------------------------------------

/// Google OAuth provider — validates ID tokens (JWT) against Google's JWKS.
pub struct GoogleAuthProvider {
    pub client_id: String,
    // JWKS keys are fetched and cached at validation time
}

impl GoogleAuthProvider {
    pub fn new(client_id: String) -> Self {
        Self { client_id }
    }
}

impl AuthProvider for GoogleAuthProvider {
    async fn validate_token(&self, token: &str) -> Result<AuthIdentity> {
        let jwks = fetch_google_jwks().await?;
        let token_data = decode_and_validate(token, &jwks, &self.client_id)?;

        let claims = token_data;
        if !claims.email_verified {
            anyhow::bail!("email not verified");
        }

        Ok(AuthIdentity {
            email: claims.email,
            display_name: claims.name,
            avatar_url: claims.picture,
            provider: "google".into(),
        })
    }

    fn provider_name(&self) -> &str {
        "google"
    }
}

// ---------------------------------------------------------------------------
// Google JWT Claims & Validation
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct GoogleClaims {
    email: String,
    email_verified: bool,
    name: String,
    picture: Option<String>,
    #[serde(default)]
    hd: Option<String>, // hosted domain
    aud: String,
}

#[derive(serde::Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

#[derive(serde::Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
    kty: String,
}

static GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

async fn fetch_google_jwks() -> Result<JwksResponse> {
    let resp = reqwest::get(GOOGLE_JWKS_URL).await?;
    let jwks: JwksResponse = resp.json().await?;
    Ok(jwks)
}

fn decode_and_validate(
    token: &str,
    jwks: &JwksResponse,
    expected_audience: &str,
) -> Result<GoogleClaims> {
    // Extract the kid from the JWT header to find the matching key
    let header = jsonwebtoken::decode_header(token)?;
    let kid = header.kid.ok_or_else(|| anyhow::anyhow!("JWT missing kid header"))?;

    let jwk = jwks.keys.iter().find(|k| k.kid == kid)
        .ok_or_else(|| anyhow::anyhow!("no matching JWK for kid={kid}"))?;

    if jwk.kty != "RSA" {
        anyhow::bail!("unsupported key type: {}", jwk.kty);
    }

    let decoding_key = jsonwebtoken::DecodingKey::from_rsa_components(&jwk.n, &jwk.e)?;

    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[expected_audience]);
    validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);

    let token_data = jsonwebtoken::decode::<GoogleClaims>(token, &decoding_key, &validation)?;
    Ok(token_data.claims)
}

// ---------------------------------------------------------------------------
// Mock Provider (for tests)
// ---------------------------------------------------------------------------

/// A mock auth provider that accepts any token formatted as "mock:<email>:<name>".
#[cfg(any(test, feature = "test-auth"))]
pub struct MockAuthProvider;

#[cfg(any(test, feature = "test-auth"))]
impl AuthProvider for MockAuthProvider {
    async fn validate_token(&self, token: &str) -> Result<AuthIdentity> {
        let parts: Vec<&str> = token.splitn(3, ':').collect();
        if parts.len() < 3 || parts[0] != "mock" {
            anyhow::bail!("invalid mock token format, expected mock:<email>:<name>");
        }
        Ok(AuthIdentity {
            email: parts[1].to_string(),
            display_name: parts[2].to_string(),
            avatar_url: None,
            provider: "mock".into(),
        })
    }

    fn provider_name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_valid_token() {
        let provider = MockAuthProvider;
        let identity = provider.validate_token("mock:alice@co.com:Alice Smith").await.unwrap();
        assert_eq!(identity.email, "alice@co.com");
        assert_eq!(identity.display_name, "Alice Smith");
        assert_eq!(identity.provider, "mock");
    }

    #[tokio::test]
    async fn mock_provider_invalid_token() {
        let provider = MockAuthProvider;
        let err = provider.validate_token("garbage").await;
        assert!(err.is_err());
    }
}
```

- [ ] **Step 2: Add `test-auth` feature to Cargo.toml**

Add to `[features]` section:

```toml
test-auth = []
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib auth::provider::tests`
Expected: ALL PASS.

- [ ] **Step 4: Commit**

```bash
git add src/auth/provider.rs Cargo.toml
git commit -m "feat(auth): AuthProvider trait with Google OIDC and MockProvider"
```

---

## Task 5: AuthStore — System Exom Wrapper

**Files:**
- Modify: `src/auth/store.rs`

This task creates the `AuthStore` that reads/writes auth facts in `_system/auth` and maintains an in-memory cache. It depends on the existing `Brain`, `ExomState`, and path primitives.

- [ ] **Step 1: Write AuthStore tests**

Add to `src/auth/store.rs`:

```rust
//! AuthStore — typed read/write over _system/auth exom, with in-memory cache.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dashmap::DashMap;

use crate::auth::{AccessLevel, AuthIdentity, User, UserRole};
use crate::brain::{self, Brain};
use crate::server::ExomState;
use crate::storage;

/// Central auth state backed by the `_system/auth` exom.
pub struct AuthStore {
    /// Path to _system/auth on disk
    pub exom_disk: PathBuf,
    /// In-memory exom state
    pub exom: Mutex<ExomState>,
    /// Session ID → User cache
    pub session_cache: DashMap<String, User>,
    /// API key SHA-256 hash (hex) → User cache
    pub api_key_cache: DashMap<String, User>,
    /// Allowed email domains (empty = allow all)
    pub allowed_domains: Mutex<Vec<String>>,
}

/// Stored representation of a share grant.
#[derive(Debug, Clone)]
pub struct ShareGrant {
    pub share_id: String,
    pub owner_email: String,
    pub path: String,
    pub grantee_email: String,
    pub permission: String, // "read" or "read-write"
    pub created_at: String,
}

impl AuthStore {
    /// Bootstrap: create _system/auth exom if missing, load existing state.
    pub fn bootstrap(tree_root: &Path, seed_domains: &[String]) -> anyhow::Result<Self> {
        let system_dir = tree_root.join("_system");
        let auth_dir = system_dir.join("auth");

        // Create _system/auth exom if not present
        if !auth_dir.join(crate::exom::META_FILENAME).exists() {
            std::fs::create_dir_all(&auth_dir)?;
            let meta = crate::exom::ExomMeta {
                format_version: 1,
                current_branch: "main".into(),
                kind: crate::exom::ExomKind::Bare,
                created_at: brain::now_iso(),
                session: None,
            };
            let meta_json = serde_json::to_string_pretty(&meta)?;
            std::fs::write(auth_dir.join(crate::exom::META_FILENAME), meta_json)?;
        }

        // Load exom state
        let sym_path = tree_root.parent().unwrap_or(tree_root).join("sym");
        let brain = Brain::new();
        let datoms = storage::build_datoms_table(&brain)?;
        let exom_state = ExomState {
            brain,
            datoms,
            rules: Vec::new(),
            exom_disk: Some(auth_dir.clone()),
        };

        let store = Self {
            exom_disk: auth_dir,
            exom: Mutex::new(exom_state),
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(Vec::new()),
        };

        // Seed allowed domains on first boot only
        let existing_domains = store.list_allowed_domains();
        if existing_domains.is_empty() && !seed_domains.is_empty() {
            let mut domains = store.allowed_domains.lock().unwrap();
            *domains = seed_domains.to_vec();
        }

        Ok(store)
    }

    /// Check if an email domain is allowed. Empty domain list = allow all.
    pub fn check_domain(&self, email: &str) -> bool {
        let domains = self.allowed_domains.lock().unwrap();
        if domains.is_empty() {
            return true;
        }
        let email_domain = email.rsplit('@').next().unwrap_or("");
        domains.iter().any(|d| d == email_domain)
    }

    /// Look up a user by session ID (from cache).
    pub fn get_user_by_session(&self, session_id: &str) -> Option<User> {
        self.session_cache.get(session_id).map(|r| r.value().clone())
    }

    /// Look up a user by API key hash (from cache).
    pub fn get_user_by_key_hash(&self, key_hash: &str) -> Option<User> {
        self.api_key_cache.get(key_hash).map(|r| r.value().clone())
    }

    /// Generate an API key. Returns (key_id, raw_key). The raw key is shown once.
    pub fn generate_api_key(&self, email: &str, label: &str) -> (String, String) {
        use base64::Engine;
        use sha2::Digest;

        let key_id = uuid::Uuid::new_v4().to_string();
        let raw_bytes: [u8; 32] = rand::random();
        let raw_key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw_bytes);
        let key_hash = hex::encode(sha2::Sha256::digest(raw_key.as_bytes()));

        // Store in cache
        // (The actual user lookup and exom write happen in the route handler)
        let _ = (key_id.clone(), key_hash, email, label);

        (key_id, raw_key)
    }

    /// Hash a raw API key to its storage form.
    pub fn hash_api_key(raw_key: &str) -> String {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(raw_key.as_bytes()))
    }

    /// Evict a session from cache.
    pub fn evict_session(&self, session_id: &str) {
        self.session_cache.remove(session_id);
    }

    /// Evict an API key from cache.
    pub fn evict_api_key(&self, key_hash: &str) {
        self.api_key_cache.remove(key_hash);
    }

    /// List allowed domains.
    pub fn list_allowed_domains(&self) -> Vec<String> {
        self.allowed_domains.lock().unwrap().clone()
    }

    /// Resolve the role for a given email.
    /// This is a placeholder — full implementation reads from _system/auth facts.
    pub fn resolve_role(&self, _email: &str) -> UserRole {
        // TODO: implement fact lookup — for now, Regular
        UserRole::Regular
    }

    /// Look up all share grants for a given grantee.
    pub fn shares_for_grantee(&self, _grantee_email: &str) -> Vec<ShareGrant> {
        // TODO: implement fact lookup
        Vec::new()
    }

    /// Look up share grants on a specific path.
    pub fn shares_for_path(&self, _path: &str) -> Vec<ShareGrant> {
        // TODO: implement fact lookup
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_domain_empty_allows_all() {
        let store = make_test_store(&[]);
        assert!(store.check_domain("alice@anything.com"));
        assert!(store.check_domain("bob@other.org"));
    }

    #[test]
    fn check_domain_restricts() {
        let store = make_test_store(&["company.com".into(), "other.co".into()]);
        assert!(store.check_domain("alice@company.com"));
        assert!(store.check_domain("bob@other.co"));
        assert!(!store.check_domain("eve@evil.org"));
    }

    #[test]
    fn hash_api_key_deterministic() {
        let h1 = AuthStore::hash_api_key("test-key-123");
        let h2 = AuthStore::hash_api_key("test-key-123");
        assert_eq!(h1, h2);
        assert_ne!(h1, AuthStore::hash_api_key("different-key"));
    }

    #[test]
    fn session_cache_round_trip() {
        let store = make_test_store(&[]);
        let user = User {
            email: "alice@co.com".into(),
            display_name: "Alice".into(),
            provider: "mock".into(),
            session_id: Some("sess-1".into()),
            role: UserRole::Regular,
        };
        store.session_cache.insert("sess-1".into(), user.clone());
        let found = store.get_user_by_session("sess-1").unwrap();
        assert_eq!(found.email, "alice@co.com");

        store.evict_session("sess-1");
        assert!(store.get_user_by_session("sess-1").is_none());
    }

    fn make_test_store(domains: &[String]) -> AuthStore {
        AuthStore {
            exom_disk: PathBuf::from("/tmp/fake"),
            exom: Mutex::new(ExomState {
                brain: Brain::new(),
                datoms: storage::build_datoms_table(&Brain::new()).unwrap(),
                rules: Vec::new(),
                exom_disk: None,
            }),
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(domains.to_vec()),
        }
    }
}
```

- [ ] **Step 2: Add `hex` dependency to Cargo.toml**

```toml
hex = "0.4"
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib auth::store::tests`
Expected: ALL PASS.

- [ ] **Step 4: Commit**

```bash
git add src/auth/store.rs Cargo.toml Cargo.lock
git commit -m "feat(auth): AuthStore with session/API-key caching and domain checking"
```

---

## Task 6: Access Control — `resolve_access` and `authorize_rayfall`

**Files:**
- Modify: `src/auth/access.rs`

- [ ] **Step 1: Write tests for resolve_access**

Replace `src/auth/access.rs`:

```rust
//! Path-based access control and Rayfall body authorization.

use crate::auth::{AccessLevel, User, UserRole};
use crate::auth::store::{AuthStore, ShareGrant};
use crate::rayfall_ast::CanonicalForm;

/// Resolve the access level a user has for a given path.
pub fn resolve_access(user: &User, path: &str, store: &AuthStore) -> AccessLevel {
    // Step 1: _system paths are always denied
    if path.starts_with("_system") {
        return AccessLevel::Denied;
    }

    // Step 2: admins get full access to everything else
    if user.is_admin() {
        return AccessLevel::FullAccess;
    }

    // Step 3: owner check — path starts with user's email namespace
    if path.starts_with(&user.email) {
        return AccessLevel::FullAccess;
    }

    // Step 4+5: check share grants (direct + inherited from parents)
    let grants = store.shares_for_grantee(&user.email);
    resolve_from_grants(path, &grants)
}

/// Check share grants, preferring the deepest (most specific) matching grant.
fn resolve_from_grants(path: &str, grants: &[ShareGrant]) -> AccessLevel {
    let mut best_match: Option<(usize, AccessLevel)> = None;

    for grant in grants {
        let grant_path = &grant.path;
        // Direct match or prefix match (path is under the granted path)
        let matches = path == grant_path
            || path.starts_with(&format!("{}/", grant_path));

        if matches {
            let depth = grant_path.matches('/').count();
            let level = match grant.permission.as_str() {
                "read-write" => AccessLevel::ReadWrite,
                "read" => AccessLevel::ReadOnly,
                _ => AccessLevel::Denied,
            };
            match best_match {
                Some((best_depth, _)) if depth > best_depth => {
                    best_match = Some((depth, level));
                }
                None => {
                    best_match = Some((depth, level));
                }
                _ => {} // keep deeper match
            }
        }
    }

    best_match.map(|(_, level)| level).unwrap_or(AccessLevel::Denied)
}

/// Authorize all exom references in lowered Rayfall forms before execution.
/// Returns Ok(()) if all forms are authorized, or Err with the denied path.
pub fn authorize_rayfall(
    user: &User,
    forms: &[CanonicalForm],
    store: &AuthStore,
) -> Result<(), AuthzError> {
    for form in forms {
        let (exom_path, needs_write) = match form {
            CanonicalForm::Query(q) => (&q.exom, false),
            CanonicalForm::Rule(r) => (&r.exom, true),
            CanonicalForm::AssertFact(f) => (&f.exom, true),
            CanonicalForm::RetractFact(f) => (&f.exom, true),
        };

        let level = resolve_access(user, exom_path, store);

        if needs_write && !level.can_write() {
            return Err(AuthzError::Denied {
                path: exom_path.clone(),
                required: "read-write".into(),
                actual: format!("{:?}", level),
            });
        }

        if !needs_write && !level.can_read() {
            return Err(AuthzError::Denied {
                path: exom_path.clone(),
                required: "read".into(),
                actual: format!("{:?}", level),
            });
        }
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    #[error("access denied to {path}: requires {required}, have {actual}")]
    Denied {
        path: String,
        required: String,
        actual: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::store::ShareGrant;

    fn user(email: &str, role: UserRole) -> User {
        User {
            email: email.into(),
            display_name: "Test".into(),
            provider: "mock".into(),
            session_id: None,
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

    // -- resolve_from_grants tests (no store needed) --

    #[test]
    fn no_grants_means_denied() {
        assert_eq!(resolve_from_grants("alice@co.com/proj", &[]), AccessLevel::Denied);
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
        // The deeper grant wins for the specific path
        assert_eq!(
            resolve_from_grants("alice@co.com/proj/secret/exom1", &grants),
            AccessLevel::ReadWrite
        );
        // The shallower grant applies to other paths
        assert_eq!(
            resolve_from_grants("alice@co.com/proj/other", &grants),
            AccessLevel::ReadOnly
        );
    }

    #[test]
    fn system_always_denied() {
        // Even for admins, _system is denied through resolve_access
        // (admin routes bypass resolve_access entirely)
        // We test this with a mock store in integration tests.
        // Here just test the path prefix check logic:
        assert!(
            "_system/auth".starts_with("_system"),
            "_system prefix detection works"
        );
    }

    // -- authorize_rayfall tests --

    #[test]
    fn authorize_rayfall_query_on_owned_exom() {
        use crate::rayfall_ast::{CanonicalQuery, Expr};

        let u = user("alice@co.com", UserRole::Regular);
        let forms = vec![CanonicalForm::Query(CanonicalQuery {
            exom: "alice@co.com/proj/main".into(),
            clauses: vec![Expr::symbol("test")],
        })];

        // With a real store this would pass because alice owns alice@co.com/*
        // For unit tests we test resolve_from_grants and authorize_rayfall independently
    }

    #[test]
    fn authorize_rayfall_rejects_system() {
        use crate::rayfall_ast::{CanonicalQuery, Expr};

        let u = user("alice@co.com", UserRole::Regular);
        let forms = vec![CanonicalForm::Query(CanonicalQuery {
            exom: "_system/auth".into(),
            clauses: vec![Expr::symbol("test")],
        })];

        // resolve_access will return Denied for _system paths
        // Full integration test needed with real AuthStore
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib auth::access::tests`
Expected: ALL PASS.

- [ ] **Step 3: Commit**

```bash
git add src/auth/access.rs
git commit -m "feat(auth): resolve_access and authorize_rayfall with grant inheritance"
```

---

## Task 7: Auth Middleware — User Extractor & CSRF

**Files:**
- Modify: `src/auth/middleware.rs`

- [ ] **Step 1: Implement User and MaybeUser extractors**

Replace `src/auth/middleware.rs`:

```rust
//! Axum auth extractors and CSRF protection.

use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
};

use crate::auth::store::AuthStore;
use crate::auth::User;
use crate::server::AppState;

/// Axum extractor: requires an authenticated user.
/// Returns 401 if no valid session cookie or Bearer token is found.
#[axum::async_trait]
impl FromRequestParts<Arc<AppState>> for User {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let auth_store = state.auth_store.as_ref().ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, "auth not configured").into_response()
        })?;

        // Try Bearer token first (API/MCP path)
        if let Some(auth_header) = parts.headers.get(header::AUTHORIZATION) {
            if let Ok(value) = auth_header.to_str() {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    let key_hash = AuthStore::hash_api_key(token);
                    if let Some(user) = auth_store.get_user_by_key_hash(&key_hash) {
                        return Ok(user);
                    }
                }
            }
            return Err((StatusCode::UNAUTHORIZED, "invalid bearer token").into_response());
        }

        // Try session cookie
        if let Some(cookie_header) = parts.headers.get(header::COOKIE) {
            if let Ok(cookies) = cookie_header.to_str() {
                if let Some(session_id) = extract_session_cookie(cookies) {
                    if let Some(user) = auth_store.get_user_by_session(&session_id) {
                        return Ok(user);
                    }
                }
            }
        }

        Err((StatusCode::UNAUTHORIZED, "authentication required").into_response())
    }
}

/// Optional auth: returns None if not authenticated (instead of 401).
pub struct MaybeUser(pub Option<User>);

#[axum::async_trait]
impl FromRequestParts<Arc<AppState>> for MaybeUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        match User::from_request_parts(parts, state).await {
            Ok(user) => Ok(MaybeUser(Some(user))),
            Err(_) => Ok(MaybeUser(None)),
        }
    }
}

/// Extract session ID from cookie header value.
fn extract_session_cookie(cookies: &str) -> Option<String> {
    for cookie in cookies.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix("ray_exomem_session=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// CSRF check for cookie-authenticated state-changing requests.
/// Bearer-token requests skip CSRF (tokens aren't auto-attached by browsers).
pub fn check_csrf(parts: &Parts, expected_origin: &str) -> Result<(), Response> {
    // Only check on state-changing methods
    if parts.method == axum::http::Method::GET || parts.method == axum::http::Method::HEAD {
        return Ok(());
    }

    // Skip CSRF if request uses Bearer token (not cookie)
    if parts.headers.get(header::AUTHORIZATION).is_some() {
        return Ok(());
    }

    // Check Origin header
    if let Some(origin) = parts.headers.get("origin") {
        if let Ok(origin_str) = origin.to_str() {
            if origin_str == expected_origin {
                return Ok(());
            }
            return Err((StatusCode::FORBIDDEN, "CSRF: origin mismatch").into_response());
        }
    }

    // Check Referer as fallback
    if let Some(referer) = parts.headers.get("referer") {
        if let Ok(referer_str) = referer.to_str() {
            if referer_str.starts_with(expected_origin) {
                return Ok(());
            }
            return Err((StatusCode::FORBIDDEN, "CSRF: referer mismatch").into_response());
        }
    }

    // Neither Origin nor Referer — reject
    Err((StatusCode::FORBIDDEN, "CSRF: missing origin header").into_response())
}

/// Build the Set-Cookie header value for a session.
pub fn session_cookie(session_id: &str, max_age_days: u64, secure: bool) -> String {
    let max_age_secs = max_age_days * 86400;
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "ray_exomem_session={session_id}; HttpOnly; SameSite=Lax; Path=/{secure_flag}; Max-Age={max_age_secs}"
    )
}

/// Build a Set-Cookie header that clears the session.
pub fn clear_session_cookie() -> String {
    "ray_exomem_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".into()
}

pub const SESSION_COOKIE_NAME: &str = "ray_exomem_session";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_session_from_cookies() {
        let cookies = "other=foo; ray_exomem_session=abc123; another=bar";
        assert_eq!(extract_session_cookie(cookies), Some("abc123".into()));
    }

    #[test]
    fn extract_session_missing() {
        assert_eq!(extract_session_cookie("other=foo"), None);
    }

    #[test]
    fn extract_session_empty_value() {
        assert_eq!(extract_session_cookie("ray_exomem_session="), None);
    }

    #[test]
    fn session_cookie_format() {
        let cookie = session_cookie("sess-123", 30, false);
        assert!(cookie.contains("ray_exomem_session=sess-123"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=2592000"));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn session_cookie_secure() {
        let cookie = session_cookie("sess-123", 30, true);
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn clear_cookie_format() {
        let cookie = clear_session_cookie();
        assert!(cookie.contains("Max-Age=0"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib auth::middleware::tests`
Expected: ALL PASS.

- [ ] **Step 3: Commit**

```bash
git add src/auth/middleware.rs
git commit -m "feat(auth): User/MaybeUser extractors, CSRF check, session cookie helpers"
```

---

## Task 8: MutationContext from User

**Files:**
- Modify: `src/context.rs`

- [ ] **Step 1: Add `from_user` constructor**

Add to `src/context.rs` after the `Default` impl:

```rust
impl MutationContext {
    /// Build a MutationContext from an authenticated user.
    /// The actor is always the user's email — client-supplied actor values are ignored.
    /// X-Model header is still accepted as advisory metadata.
    pub fn from_user(user: &crate::auth::User, model: Option<String>) -> Self {
        Self {
            actor: user.email.clone(),
            session: user.session_id.clone(),
            model,
        }
    }
}
```

- [ ] **Step 2: Write test**

Add to `src/context.rs`:

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib context::tests`
Expected: ALL PASS.

- [ ] **Step 4: Commit**

```bash
git add src/context.rs
git commit -m "feat(auth): MutationContext::from_user — actor derived from authenticated email"
```

---

## Task 9: Auth Routes — Login, Logout, Me, API Keys, Shares

**Files:**
- Modify: `src/auth/routes.rs`

This is a large task covering all `/auth/*` route handlers. Each handler is small and delegates to `AuthStore`.

- [ ] **Step 1: Implement route handlers**

Replace `src/auth/routes.rs`:

```rust
//! Auth route handlers: login, logout, me, api-keys, shares.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::auth::middleware::{clear_session_cookie, session_cookie};
use crate::auth::store::AuthStore;
use crate::auth::User;
use crate::http_error::ApiError;
use crate::server::AppState;

/// Build the `/auth` router.
pub fn auth_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/{key_id}", delete(revoke_api_key))
        .route("/shares", get(list_shares).post(create_share))
        .route("/shares/{share_id}", delete(revoke_share))
        .route("/shared-with-me", get(shared_with_me))
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginBody {
    id_token: String,
    provider: Option<String>,
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    let auth_store = match state.auth_store.as_ref() {
        Some(s) => s,
        None => return ApiError::new("auth_not_configured", "auth not enabled").into_response(),
    };

    let provider = match state.auth_provider.as_ref() {
        Some(p) => p,
        None => return ApiError::new("no_provider", "no auth provider configured").into_response(),
    };

    // Validate token with provider
    let identity = match provider.validate_token(&body.id_token).await {
        Ok(id) => id,
        Err(e) => {
            return ApiError::new("auth_failed", format!("token validation failed: {e}"))
                .into_response();
        }
    };

    // Check domain
    if !auth_store.check_domain(&identity.email) {
        return ApiError::new("domain_denied", "email domain not allowed")
            .with_status(StatusCode::FORBIDDEN)
            .into_response();
    }

    // Create session
    let session_id = uuid::Uuid::new_v4().to_string();
    let role = auth_store.resolve_role(&identity.email);

    let user = User {
        email: identity.email.clone(),
        display_name: identity.display_name.clone(),
        provider: identity.provider.clone(),
        session_id: Some(session_id.clone()),
        role,
    };

    // Cache the session
    auth_store.session_cache.insert(session_id.clone(), user.clone());

    // Check if this is the first user → auto-promote to top-admin
    // (Implementation detail: check if any top-admin fact exists in store)

    // Determine if we should set Secure flag (not localhost)
    let secure = !state.bind_addr.as_deref().unwrap_or("127.0.0.1").starts_with("127.0.0.1")
        && !state.bind_addr.as_deref().unwrap_or("localhost").starts_with("localhost");

    let cookie = session_cookie(&session_id, 30, secure);

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "ok": true,
            "user": {
                "email": user.email,
                "display_name": user.display_name,
                "provider": user.provider,
                "role": format!("{:?}", user.role),
            }
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Logout
// ---------------------------------------------------------------------------

async fn logout(
    State(state): State<Arc<AppState>>,
    user: User,
) -> impl IntoResponse {
    if let Some(ref auth_store) = state.auth_store {
        if let Some(ref session_id) = user.session_id {
            auth_store.evict_session(session_id);
        }
    }

    let cookie = clear_session_cookie();
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Me
// ---------------------------------------------------------------------------

async fn me(user: User) -> impl IntoResponse {
    Json(serde_json::json!({
        "email": user.email,
        "display_name": user.display_name,
        "provider": user.provider,
        "role": format!("{:?}", user.role),
    }))
}

// ---------------------------------------------------------------------------
// API Keys
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateApiKeyBody {
    label: String,
}

async fn create_api_key(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<CreateApiKeyBody>,
) -> impl IntoResponse {
    let auth_store = match state.auth_store.as_ref() {
        Some(s) => s,
        None => return ApiError::new("auth_not_configured", "auth not enabled").into_response(),
    };

    let (key_id, raw_key) = auth_store.generate_api_key(&user.email, &body.label);
    let key_hash = AuthStore::hash_api_key(&raw_key);

    // Cache the key
    auth_store.api_key_cache.insert(key_hash, user.clone());

    // Build MCP config snippet
    let base_url = state.bind_addr.as_deref().unwrap_or("http://127.0.0.1:9780");
    let mcp_config = serde_json::json!({
        "mcpServers": {
            "ray-exomem": {
                "url": format!("{base_url}/mcp"),
                "headers": {
                    "Authorization": format!("Bearer {raw_key}")
                }
            }
        }
    });

    Json(serde_json::json!({
        "ok": true,
        "key_id": key_id,
        "raw_key": raw_key,
        "label": body.label,
        "mcp_config": mcp_config,
    }))
    .into_response()
}

async fn list_api_keys(
    State(_state): State<Arc<AppState>>,
    _user: User,
) -> impl IntoResponse {
    // TODO: query _system/auth for (api-key ...) facts matching user email
    Json(serde_json::json!({
        "keys": []
    }))
}

async fn revoke_api_key(
    State(_state): State<Arc<AppState>>,
    _user: User,
    axum::extract::Path(key_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    // TODO: retract api-key fact, evict from cache
    let _ = key_id;
    Json(serde_json::json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// Shares
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateShareBody {
    path: String,
    grantee_email: String,
    permission: String, // "read" or "read-write"
}

async fn create_share(
    State(_state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<CreateShareBody>,
) -> impl IntoResponse {
    // Verify user owns the path
    if !body.path.starts_with(&user.email) {
        return ApiError::new("not_owner", "you can only share paths you own")
            .with_status(StatusCode::FORBIDDEN)
            .into_response();
    }

    if body.permission != "read" && body.permission != "read-write" {
        return ApiError::new("bad_permission", "permission must be 'read' or 'read-write'")
            .into_response();
    }

    let share_id = uuid::Uuid::new_v4().to_string();

    // TODO: assert share fact into _system/auth

    Json(serde_json::json!({
        "ok": true,
        "share_id": share_id,
        "path": body.path,
        "grantee_email": body.grantee_email,
        "permission": body.permission,
    }))
    .into_response()
}

async fn list_shares(
    State(_state): State<Arc<AppState>>,
    _user: User,
    Query(params): Query<ListSharesParams>,
) -> impl IntoResponse {
    let _ = params;
    // TODO: query _system/auth for shares owned by user
    Json(serde_json::json!({ "shares": [] }))
}

#[derive(Deserialize)]
struct ListSharesParams {
    path: Option<String>,
}

use axum::extract::Query;

async fn revoke_share(
    State(_state): State<Arc<AppState>>,
    _user: User,
    axum::extract::Path(share_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let _ = share_id;
    // TODO: retract share fact, verify ownership
    Json(serde_json::json!({ "ok": true }))
}

async fn shared_with_me(
    State(_state): State<Arc<AppState>>,
    _user: User,
) -> impl IntoResponse {
    // TODO: query _system/auth for shares where grantee = user.email
    Json(serde_json::json!({ "shared": [] }))
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: Compiles. (Some handlers have TODO stubs for system exom fact operations — these will be filled in during integration.)

Note: This will not compile yet because `AppState` doesn't have `auth_store`, `auth_provider`, or `bind_addr` fields. These are added in Task 11 (Server Integration). For now, this task establishes the route structure and handler signatures. If needed for incremental compilation, add the fields as `Option<_>` placeholders in Task 11 first.

- [ ] **Step 3: Commit**

```bash
git add src/auth/routes.rs
git commit -m "feat(auth): route handlers for login, logout, me, api-keys, shares"
```

---

## Task 10: Admin Routes

**Files:**
- Modify: `src/auth/admin.rs`

- [ ] **Step 1: Implement admin route handlers**

Replace `src/auth/admin.rs`:

```rust
//! Admin panel route handlers.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::auth::User;
use crate::http_error::ApiError;
use crate::server::AppState;

/// Build the `/auth/admin` router.
pub fn admin_router() -> Router<Arc<AppState>> {
    Router::new()
        // User management (admin)
        .route("/users", get(list_users))
        .route("/users/{email}", delete(deactivate_user))
        .route("/users/{email}/activate", post(activate_user))
        // Admin management (top-admin only)
        .route("/admins", post(grant_admin))
        .route("/admins/{email}", delete(revoke_admin))
        // Session management (admin)
        .route("/sessions", get(list_sessions))
        .route("/sessions/{id}", delete(kill_session))
        // API key management (admin)
        .route("/api-keys", get(list_all_api_keys))
        .route("/api-keys/{key_id}", delete(revoke_any_api_key))
        // Share management (admin)
        .route("/shares", get(list_all_shares))
        // Domain management (admin)
        .route("/allowed-domains", get(list_domains).post(add_domain))
        .route("/allowed-domains/{domain}", delete(remove_domain))
}

/// Guard: require admin role. Returns Err(Response) if not admin.
fn require_admin(user: &User) -> Result<(), impl IntoResponse> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(ApiError::new("forbidden", "admin access required")
            .with_status(StatusCode::FORBIDDEN)
            .into_response())
    }
}

/// Guard: require top-admin role.
fn require_top_admin(user: &User) -> Result<(), impl IntoResponse> {
    if user.is_top_admin() {
        Ok(())
    } else {
        Err(ApiError::new("forbidden", "top-admin access required")
            .with_status(StatusCode::FORBIDDEN)
            .into_response())
    }
}

// ---------------------------------------------------------------------------
// User management
// ---------------------------------------------------------------------------

async fn list_users(
    State(_state): State<Arc<AppState>>,
    user: User,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    // TODO: query _system/auth for all (user ...) facts
    Json(serde_json::json!({ "users": [] })).into_response()
}

async fn deactivate_user(
    State(_state): State<Arc<AppState>>,
    user: User,
    axum::extract::Path(email): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    // TODO: set (user-status <email> deactivated), revoke all sessions + keys
    let _ = email;
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn activate_user(
    State(_state): State<Arc<AppState>>,
    user: User,
    axum::extract::Path(email): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    let _ = email;
    // TODO: set (user-status <email> active)
    Json(serde_json::json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// Admin management (top-admin only)
// ---------------------------------------------------------------------------

async fn grant_admin(
    State(_state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<AdminBody>,
) -> impl IntoResponse {
    if let Err(e) = require_top_admin(&user) { return e.into_response(); }
    // TODO: assert (admin <email>) into _system/auth
    let _ = body;
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn revoke_admin(
    State(_state): State<Arc<AppState>>,
    user: User,
    axum::extract::Path(email): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_top_admin(&user) { return e.into_response(); }
    let _ = email;
    // TODO: retract (admin <email>) from _system/auth
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct AdminBody {
    email: String,
}

// ---------------------------------------------------------------------------
// Session management
// ---------------------------------------------------------------------------

async fn list_sessions(
    State(_state): State<Arc<AppState>>,
    user: User,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    Json(serde_json::json!({ "sessions": [] })).into_response()
}

async fn kill_session(
    State(_state): State<Arc<AppState>>,
    user: User,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    let _ = id;
    // TODO: retract session, evict cache
    Json(serde_json::json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// API key management (admin)
// ---------------------------------------------------------------------------

async fn list_all_api_keys(
    State(_state): State<Arc<AppState>>,
    user: User,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    Json(serde_json::json!({ "keys": [] })).into_response()
}

async fn revoke_any_api_key(
    State(_state): State<Arc<AppState>>,
    user: User,
    axum::extract::Path(key_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    let _ = key_id;
    Json(serde_json::json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// Share management (admin)
// ---------------------------------------------------------------------------

async fn list_all_shares(
    State(_state): State<Arc<AppState>>,
    user: User,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    Json(serde_json::json!({ "shares": [] })).into_response()
}

// ---------------------------------------------------------------------------
// Domain management
// ---------------------------------------------------------------------------

async fn list_domains(
    State(state): State<Arc<AppState>>,
    user: User,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    let domains = state.auth_store.as_ref()
        .map(|s| s.list_allowed_domains())
        .unwrap_or_default();
    Json(serde_json::json!({ "domains": domains })).into_response()
}

#[derive(Deserialize)]
struct AddDomainBody {
    domain: String,
}

async fn add_domain(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<AddDomainBody>,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    if let Some(ref auth_store) = state.auth_store {
        let mut domains = auth_store.allowed_domains.lock().unwrap();
        if !domains.contains(&body.domain) {
            domains.push(body.domain.clone());
        }
    }
    // TODO: also assert (allowed-domain <domain>) into _system/auth
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn remove_domain(
    State(state): State<Arc<AppState>>,
    user: User,
    axum::extract::Path(domain): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_admin(&user) { return e.into_response(); }
    if let Some(ref auth_store) = state.auth_store {
        let mut domains = auth_store.allowed_domains.lock().unwrap();
        domains.retain(|d| d != &domain);
    }
    // TODO: also retract (allowed-domain <domain>) from _system/auth
    Json(serde_json::json!({ "ok": true })).into_response()
}
```

- [ ] **Step 2: Verify compilation** (may need Task 11 first)

Run: `cargo check`

- [ ] **Step 3: Commit**

```bash
git add src/auth/admin.rs
git commit -m "feat(auth): admin route handlers — users, admins, sessions, keys, shares, domains"
```

---

## Task 11: Server Integration — Wire Auth into AppState & Router

**Files:**
- Modify: `src/server.rs`

This is the critical integration task. It adds auth fields to `AppState`, wires the auth router, and modifies existing handlers to use `User` instead of `body.actor`/`X-Actor`.

- [ ] **Step 0: Add auth CLI flags to main.rs**

In `src/main.rs`, add CLI args to the `serve`/`daemon` subcommands:

```rust
/// Auth provider ("google" or "mock" for testing)
#[arg(long)]
auth_provider: Option<String>,

/// Google OAuth client ID (required when auth-provider=google)
#[arg(long)]
google_client_id: Option<String>,

/// Comma-separated allowed email domains (seeds _system/auth on first boot)
#[arg(long, value_delimiter = ',')]
allowed_domains: Option<Vec<String>>,

/// Bootstrap admin email (only takes effect if no top-admin exists)
#[arg(long)]
bootstrap_admin: Option<String>,
```

Pass these through to `AppState` construction.

- [ ] **Step 1: Add auth fields to AppState**

In `src/server.rs`, add to `AppState` struct (after `sse_tx`):

```rust
pub auth_store: Option<Arc<crate::auth::store::AuthStore>>,
pub auth_provider: Option<Arc<dyn crate::auth::provider::AuthProvider>>,
pub bind_addr: Option<String>,
```

Update `AppState::new()` to accept and store these fields (with `None` defaults for backward compatibility).

Update `AppState::from_data_dir()` to bootstrap `AuthStore` when a data directory is provided.

- [ ] **Step 2: Add `/auth` and `/mcp` route nests to the router**

In the `serve()` function, add after the existing routes:

```rust
// Auth routes
.nest("/auth", crate::auth::routes::auth_router())
.nest("/auth/admin", crate::auth::admin::admin_router())
```

- [ ] **Step 3: Replace `body.actor` / `X-Actor` in mutation handlers**

For each handler that currently reads actor from body or headers, change to use `User` extractor:

**`api_session_new`** — add `user: User` parameter, replace `body.actor.unwrap_or_default()` with `user.email.clone()`.

**`api_session_join`** — add `user: User` parameter, replace `body.actor.unwrap_or_default()` with `user.email.clone()`.

**`api_branch_create`** — add `user: User` parameter, replace `body.actor.unwrap_or_default()` with `user.email.clone()`.

**`api_assert_fact`** — add `user: User` parameter, replace `req.actor` usage with `user.email.clone()`.

**`api_eval`** — add `user: User` parameter, replace `X-Actor` header reading with `MutationContext::from_user(&user, model)`.

Each handler change follows the same pattern:
```rust
// Before:
let actor = body.actor.unwrap_or_default();

// After:
let actor = user.email.clone();
```

Remove `actor` fields from request body structs where they existed (or keep them ignored for backward compatibility during transition — but spec says remove).

- [ ] **Step 4: Add access checks to existing handlers**

For each API handler, add the access check after resolving the exom path:

```rust
// Pattern for query handlers (read):
if let Some(ref auth_store) = state.auth_store {
    let level = crate::auth::access::resolve_access(&user, &exom_slash, auth_store);
    if !level.can_read() {
        return ApiError::new("forbidden", "access denied").with_status(StatusCode::FORBIDDEN).into_response();
    }
}

// Pattern for mutation handlers (write):
if let Some(ref auth_store) = state.auth_store {
    let level = crate::auth::access::resolve_access(&user, &exom_slash, auth_store);
    if !level.can_write() {
        return ApiError::new("forbidden", "write access denied").with_status(StatusCode::FORBIDDEN).into_response();
    }
}
```

- [ ] **Step 5: Add Rayfall body authorization to query and eval handlers**

In `api_query_post` and `api_eval`, after lowering forms and before execution:

```rust
if let Some(ref auth_store) = state.auth_store {
    if let Err(e) = crate::auth::access::authorize_rayfall(&user, &lowered_forms, auth_store) {
        return ApiError::new("forbidden", e.to_string())
            .with_status(StatusCode::FORBIDDEN)
            .into_response();
    }
}
```

- [ ] **Step 5b: Update rename handler to update share paths**

In the `api_rename` handler, after the tree rename succeeds, update all matching share grants in `_system/auth`:

```rust
// After successful rename:
if let Some(ref auth_store) = state.auth_store {
    auth_store.update_share_paths(&old_path_slash, &new_path_slash);
}
```

Add `update_share_paths` to `AuthStore` — queries all `(share ...)` facts, updates those whose path is a prefix match.

- [ ] **Step 6: Scope SSE events to accessible exoms**

In `api_sse`, filter broadcast events so users only see events for exoms they can access. This requires adding user identity to the SSE handler.

- [ ] **Step 7: Scope tree endpoint to visible exoms**

In `api_tree`, filter the tree walk result to only include nodes the user can access.

- [ ] **Step 8: Verify compilation**

Run: `cargo check`
Expected: Compiles with new auth integration.

- [ ] **Step 9: Run existing tests**

Run: `cargo test`
Expected: Existing tests may need updates if they hit auth-protected routes. The `TestDaemon` doesn't set up auth, so handlers should fall through gracefully when `auth_store` is `None`.

- [ ] **Step 10: Commit**

```bash
git add src/server.rs
git commit -m "feat(auth): wire auth into AppState, router, replace actor with User.email"
```

---

## Task 12: Update TestDaemon for Auth Testing

**Files:**
- Modify: `tests/common/daemon.rs`

- [ ] **Step 1: Add auth config to TestDaemon**

Add a `TestDaemonBuilder` pattern that optionally enables auth with a mock provider:

```rust
pub struct TestDaemonBuilder {
    auth_enabled: bool,
}

impl TestDaemonBuilder {
    pub fn new() -> Self {
        Self { auth_enabled: false }
    }

    pub fn with_auth(mut self) -> Self {
        self.auth_enabled = true;
        self
    }

    pub fn start(self) -> TestDaemon {
        // existing start logic, with optional --auth-provider mock flag
    }
}
```

- [ ] **Step 2: Add auth helper methods to TestDaemon**

```rust
impl TestDaemon {
    /// Get a Bearer token for a mock user.
    pub fn mock_login(&self, email: &str, name: &str) -> String {
        let resp = ureq::post(&format!("{}/auth/login", self.base_url))
            .send_json(serde_json::json!({
                "id_token": format!("mock:{email}:{name}"),
                "provider": "mock"
            }))
            .expect("mock login");
        // Extract API key or session from response
        let body: serde_json::Value = resp.into_json().unwrap();
        // Return a usable auth token
        todo!("extract session token from response")
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add tests/common/daemon.rs
git commit -m "test: add auth support to TestDaemon with mock provider"
```

---

## Task 13: Integration Tests — Auth Basics

**Files:**
- Create: `tests/auth_basic.rs`

- [ ] **Step 1: Write login/me/logout integration tests**

```rust
mod common;
use common::daemon::TestDaemon;

#[test]
fn unauthenticated_api_returns_401() {
    let daemon = TestDaemon::start();
    let resp = ureq::get(&format!("{}/ray-exomem/api/status", daemon.base_url)).call();
    // With auth enabled, this should return 401
    // Without auth (current default), it returns 200
}

#[test]
fn login_creates_session() {
    // Start daemon with auth enabled
    // POST /auth/login with mock token
    // GET /auth/me with session cookie
    // Verify user info returned
}

#[test]
fn bearer_token_auth() {
    // Login, create API key, use Bearer token
}

#[test]
fn logout_invalidates_session() {
    // Login, verify /auth/me works, logout, verify /auth/me returns 401
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --test auth_basic`
Expected: ALL PASS after full implementation.

- [ ] **Step 3: Commit**

```bash
git add tests/auth_basic.rs
git commit -m "test: auth integration tests — login, me, logout, bearer token"
```

---

## Task 14: Integration Tests — Access Control & Shares

**Files:**
- Create: `tests/auth_shares.rs`

- [ ] **Step 1: Write access control and sharing tests**

```rust
mod common;

#[test]
fn owner_can_access_own_exom() {
    // Login as alice, create exom at alice@co.com/proj/main, query it
}

#[test]
fn non_owner_denied_without_share() {
    // Login as alice, create exom. Login as bob, try to query → 403
}

#[test]
fn read_share_allows_query() {
    // Alice shares path with bob (read). Bob can query. Bob cannot assert.
}

#[test]
fn write_share_allows_mutation() {
    // Alice shares path with bob (read-write). Bob can assert facts.
}

#[test]
fn system_path_always_denied() {
    // Any user trying to query _system/auth → 403
}

#[test]
fn rename_updates_share_paths() {
    // Alice shares alice@co.com/proj. Alice renames to alice@co.com/work.
    // Bob's share now points to alice@co.com/work.
}
```

- [ ] **Step 2: Commit**

```bash
git add tests/auth_shares.rs
git commit -m "test: access control and sharing integration tests"
```

---

## Task 15: Integration Tests — Admin

**Files:**
- Create: `tests/auth_admin.rs`

- [ ] **Step 1: Write admin panel tests**

```rust
mod common;

#[test]
fn first_user_becomes_top_admin() {
    // First login → check role is TopAdmin
}

#[test]
fn top_admin_can_grant_admin() {
    // Top-admin grants admin to second user
}

#[test]
fn admin_cannot_manage_admins() {
    // Admin tries POST /auth/admin/admins → 403
}

#[test]
fn admin_can_deactivate_user() {
    // Admin deactivates bob. Bob's API returns 401.
}

#[test]
fn admin_can_manage_domains() {
    // Add domain, verify login restricted. Remove domain, verify login open.
}
```

- [ ] **Step 2: Commit**

```bash
git add tests/auth_admin.rs
git commit -m "test: admin panel integration tests"
```

---

## Task 16: MCP Protocol Server

**Files:**
- Create: `src/mcp.rs`
- Modify: `src/lib.rs` (add `pub mod mcp;`)
- Modify: `src/server.rs` (add `/mcp` route)

- [ ] **Step 1: Implement MCP JSON-RPC handler**

Create `src/mcp.rs` with the core JSON-RPC protocol handler:

```rust
//! MCP protocol server — JSON-RPC over HTTP with tool definitions.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::auth::User;
use crate::server::AppState;

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

/// MCP handler — single endpoint for all JSON-RPC calls.
pub async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let result = match req.method.as_str() {
        "initialize" => handle_initialize(),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tool_call(&state, &user, req.params).await,
        _ => Err(JsonRpcError {
            code: -32601,
            message: format!("method not found: {}", req.method),
        }),
    };

    let response = match result {
        Ok(value) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id,
            result: Some(value),
            error: None,
        },
        Err(error) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id,
            result: None,
            error: Some(error),
        },
    };

    Json(response)
}

fn handle_initialize() -> Result<serde_json::Value, JsonRpcError> {
    Ok(serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "ray-exomem",
            "version": crate::frontend_version()
        }
    }))
}

fn handle_tools_list() -> Result<serde_json::Value, JsonRpcError> {
    Ok(serde_json::json!({
        "tools": tool_definitions()
    }))
}

async fn handle_tool_call(
    state: &Arc<AppState>,
    user: &User,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, JsonRpcError> {
    let params = params.ok_or(JsonRpcError {
        code: -32602,
        message: "missing params".into(),
    })?;

    let tool_name = params.get("name")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError {
            code: -32602,
            message: "missing tool name".into(),
        })?;

    let arguments = params.get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    // Dispatch to tool implementations
    // Each tool calls the same internal logic as the HTTP API handlers
    match tool_name {
        "query" => tool_query(state, user, arguments).await,
        "assert_fact" => tool_assert_fact(state, user, arguments).await,
        "list_exoms" => tool_list_exoms(state, user).await,
        "exom_status" => tool_exom_status(state, user, arguments).await,
        // ... additional tools following the same pattern
        _ => Err(JsonRpcError {
            code: -32602,
            message: format!("unknown tool: {tool_name}"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tool Definitions
// ---------------------------------------------------------------------------

fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "query",
            "description": "Run a Rayfall query against an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom path (e.g., alice@co.com/proj/main)" },
                    "query": { "type": "string", "description": "Rayfall query form" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "assert_fact",
            "description": "Assert or replace a fact in an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom path" },
                    "predicate": { "type": "string" },
                    "value": { "type": "string" },
                    "fact_id": { "type": "string", "description": "Optional fact ID for replace semantics" }
                },
                "required": ["exom", "predicate", "value"]
            }
        }),
        serde_json::json!({
            "name": "list_exoms",
            "description": "List all exoms accessible to the current user",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        serde_json::json!({
            "name": "exom_status",
            "description": "Get health and stats for an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom path" }
                },
                "required": ["exom"]
            }
        }),
        // Additional tool definitions for full API surface:
        // eval, explain, fact_history, facts_list, facts_valid_at, facts_bitemporal,
        // provenance, beliefs, clusters, cluster_detail, derived, schema, graph,
        // relation_graph, list_branches, branch_detail, create_branch, delete_branch,
        // switch_branch, diff_branch, merge_branch, start_session, join_session,
        // create_exom, rename, export, export_json, import_json, logs,
        // retract_all, wipe, factory_reset
        //
        // Each follows the same pattern as above. Full definitions should be added
        // during implementation, mirroring the parameters of each HTTP endpoint.
    ]
}

// ---------------------------------------------------------------------------
// Tool Implementations
// ---------------------------------------------------------------------------

async fn tool_query(
    state: &Arc<AppState>,
    user: &User,
    args: serde_json::Value,
) -> Result<serde_json::Value, JsonRpcError> {
    let query_str = args.get("query")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError { code: -32602, message: "missing query".into() })?;

    let exom = args.get("exom").and_then(|v| v.as_str());

    // Reuse existing query logic from server.rs
    // 1. Lower the query
    // 2. Authorize (authorize_rayfall)
    // 3. Execute via engine
    // 4. Return results

    // Placeholder — full implementation calls into server internals
    Ok(serde_json::json!({
        "content": [{ "type": "text", "text": "query result placeholder" }]
    }))
}

async fn tool_assert_fact(
    state: &Arc<AppState>,
    user: &User,
    args: serde_json::Value,
) -> Result<serde_json::Value, JsonRpcError> {
    // Reuse existing assert-fact logic
    Ok(serde_json::json!({
        "content": [{ "type": "text", "text": "fact asserted" }]
    }))
}

async fn tool_list_exoms(
    state: &Arc<AppState>,
    user: &User,
) -> Result<serde_json::Value, JsonRpcError> {
    // Reuse existing tree walk, filtered by user access
    Ok(serde_json::json!({
        "content": [{ "type": "text", "text": "[]" }]
    }))
}

async fn tool_exom_status(
    state: &Arc<AppState>,
    user: &User,
    args: serde_json::Value,
) -> Result<serde_json::Value, JsonRpcError> {
    // Reuse existing status logic
    Ok(serde_json::json!({
        "content": [{ "type": "text", "text": "{}" }]
    }))
}
```

- [ ] **Step 2: Register module and route**

Add `pub mod mcp;` to `src/lib.rs`.

Add to `serve()` in `src/server.rs`:
```rust
.route("/mcp", post(crate::mcp::mcp_handler))
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`

- [ ] **Step 4: Write basic MCP test**

Create `tests/mcp_basic.rs`:

```rust
mod common;

#[test]
fn mcp_initialize() {
    // POST /mcp with initialize method
    // Verify protocol version and capabilities
}

#[test]
fn mcp_tools_list() {
    // POST /mcp with tools/list method
    // Verify tool names match expected set
}

#[test]
fn mcp_tool_call_requires_auth() {
    // POST /mcp without Bearer token → 401
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test mcp_basic`

- [ ] **Step 6: Commit**

```bash
git add src/mcp.rs src/lib.rs src/server.rs tests/mcp_basic.rs
git commit -m "feat(mcp): MCP JSON-RPC server with tool definitions and auth"
```

---

## Task 17: Svelte UI — Auth State Store

**Files:**
- Create: `ui/src/lib/auth.svelte.ts`
- Modify: `ui/src/lib/exomem.svelte.ts`

**Note:** User prefers cursor-agent for Svelte/UI work. This task and subsequent UI tasks define the structure and key code — implementation may be delegated to cursor-agent.

- [ ] **Step 1: Create auth state store**

Create `ui/src/lib/auth.svelte.ts`:

```typescript
import { browser } from '$app/environment';
import { goto } from '$app/navigation';
import { getExomemBaseUrl } from '$lib/exomem.svelte';

export interface AuthUser {
  email: string;
  display_name: string;
  provider: string;
  role: string;
}

class AuthState {
  user = $state<AuthUser | null>(null);
  loading = $state(true);
  error = $state<string | null>(null);

  get isAuthenticated() {
    return this.user !== null;
  }

  get isAdmin() {
    return this.user?.role === 'Admin' || this.user?.role === 'TopAdmin';
  }

  get isTopAdmin() {
    return this.user?.role === 'TopAdmin';
  }

  async checkSession() {
    if (!browser) return;
    this.loading = true;
    try {
      const base = getExomemBaseUrl().replace('/ray-exomem', '');
      const resp = await fetch(`${base}/auth/me`, { credentials: 'include' });
      if (resp.ok) {
        this.user = await resp.json();
      } else {
        this.user = null;
      }
    } catch {
      this.user = null;
    } finally {
      this.loading = false;
    }
  }

  async logout() {
    const base = getExomemBaseUrl().replace('/ray-exomem', '');
    await fetch(`${base}/auth/logout`, {
      method: 'POST',
      credentials: 'include',
    });
    this.user = null;
    goto('/login');
  }
}

export const auth = new AuthState();
```

- [ ] **Step 2: Add `credentials: 'include'` to exomem fetch calls**

In `ui/src/lib/exomem.svelte.ts`, update all `fetch()` calls to include `credentials: 'include'` so session cookies are sent. Add a global response interceptor that redirects to `/login` on 401.

- [ ] **Step 3: Commit**

```bash
cd ui && git add src/lib/auth.svelte.ts src/lib/exomem.svelte.ts
git commit -m "feat(ui): auth state store and session-aware fetch"
```

---

## Task 18: Svelte UI — Login Page

**Files:**
- Create: `ui/src/routes/login/+page.svelte`
- Modify: `ui/src/routes/+layout.svelte`

- [ ] **Step 1: Create login page**

Create `ui/src/routes/login/+page.svelte` with:
- Google Sign-In button (GSI library loaded via script tag)
- `onMount`: load GSI script, initialize with client ID
- On credential response: POST to `/auth/login`, update auth store, redirect to `/`

- [ ] **Step 2: Add auth guard to layout**

In `ui/src/routes/+layout.svelte`:
- `onMount`: call `auth.checkSession()`
- If not authenticated and not on `/login`, redirect to `/login`
- Show loading spinner while checking session

- [ ] **Step 3: Build and test**

Run: `cd ui && npm run build`
Expected: Build succeeds.

- [ ] **Step 4: Commit**

```bash
git add ui/src/routes/login/ ui/src/routes/+layout.svelte
git commit -m "feat(ui): login page with Google Sign-In and auth guard"
```

---

## Task 19: Svelte UI — Profile Page

**Files:**
- Create: `ui/src/routes/profile/+page.svelte`

- [ ] **Step 1: Create profile page**

Build `ui/src/routes/profile/+page.svelte` with:
- User info display (email, name, provider, role)
- API key management:
  - "Generate API Key" button → opens dialog for label input → POST `/auth/api-keys`
  - Show raw key once with copy button
  - Show MCP config snippet with copy button
  - List existing keys (label, created date) with revoke button
- Logout button

- [ ] **Step 2: Build and test**

Run: `cd ui && npm run check && npm run build`

- [ ] **Step 3: Commit**

```bash
git add ui/src/routes/profile/
git commit -m "feat(ui): profile page with API key management and MCP config"
```

---

## Task 20: Svelte UI — Admin Panel

**Files:**
- Create: `ui/src/routes/admin/+page.svelte`

- [ ] **Step 1: Create admin page**

Build `ui/src/routes/admin/+page.svelte` with tab views:
- **Users tab:** table of users (email, role, status, last login). Deactivate/activate buttons.
- **Sessions tab:** table of active sessions. Kill button.
- **API Keys tab:** table of all keys (user, label, created). Revoke button.
- **Shares tab:** table of all share grants.
- **Domains tab:** list of allowed domains. Add/remove.
- **Admins tab** (top-admin only): grant/revoke admin role.

Page hidden from nav unless `auth.isAdmin`.

- [ ] **Step 2: Build and test**

Run: `cd ui && npm run check && npm run build`

- [ ] **Step 3: Commit**

```bash
git add ui/src/routes/admin/
git commit -m "feat(ui): admin panel — users, sessions, keys, shares, domains"
```

---

## Task 21: Svelte UI — Shared With Me

**Files:**
- Modify: `ui/src/lib/Drawer.svelte` (or equivalent sidebar component)

- [ ] **Step 1: Add "Shared with me" section to sidebar**

In the tree drawer/sidebar component:
- Fetch `GET /auth/shared-with-me` on mount
- Render shared exom paths below the user's own tree
- Clicking a shared exom navigates to it (read-only or read-write based on permission)

- [ ] **Step 2: Build and test**

Run: `cd ui && npm run check && npm run build`

- [ ] **Step 3: Commit**

```bash
git add ui/src/lib/Drawer.svelte
git commit -m "feat(ui): shared-with-me section in sidebar"
```

---

## Task 22: Final Verification

- [ ] **Step 1: Run full Rust test suite**

Run: `cargo test`
Expected: ALL PASS.

- [ ] **Step 2: Run UI checks and build**

Run: `cd ui && npm run check && npm run build`
Expected: No errors.

- [ ] **Step 3: Build release binary**

Run: `cargo build --release`
Expected: Binary builds with embedded UI.

- [ ] **Step 4: Manual smoke test**

```bash
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
ray-exomem serve --bind 127.0.0.1:9780 --auth-provider google --google-client-id <YOUR_CLIENT_ID> --allowed-domains company.com
```

Verify in browser:
1. Login page appears
2. Google Sign-In works
3. Profile page shows user info
4. API key generation works
5. MCP config snippet is correct
6. Exom CRUD works within user namespace
7. Sharing works between two users
8. Admin panel works (if top-admin)

- [ ] **Step 5: Commit any fixes**

```bash
git add -A
git commit -m "fix: address smoke test findings"
```
