# Postgres Storage Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace JSONL file-based persistence with PostgreSQL via trait-based storage adapters, while preserving JSONL as a fallback when no database URL is provided.

**Architecture:** Two async traits (`AuthDb`, `ExomDb`) with Postgres and JSONL implementations. `AppState` holds `Arc<dyn AuthDb>` and `Arc<dyn ExomDb>`, selected at startup based on `--database-url`. Splay tables and rayforce2 FFI are unchanged — Postgres replaces JSONL as the durable source of truth only.

**Tech Stack:** sqlx (async Postgres driver), async-trait, Axum, Tokio

**Design spec:** `archive/2026-04-14_postgres-migration/design.md`

---

## File Map

### New Files
| File | Responsibility |
|---|---|
| `src/db/mod.rs` | Trait definitions (`AuthDb`, `ExomDb`), row types, PgPool init |
| `src/db/pg_auth.rs` | `PgAuthDb` — Postgres implementation of `AuthDb` |
| `src/db/pg_exom.rs` | `PgExomDb` — Postgres implementation of `ExomDb` |
| `src/db/jsonl_auth.rs` | `JsonlAuthDb` — JSONL implementation of `AuthDb` (extracted from `auth/store.rs`) |
| `src/db/jsonl_exom.rs` | `JsonlExomDb` — JSONL implementation of `ExomDb` (extracted from `storage.rs`) |
| `migrations/001_initial.sql` | Full schema: 10 tables (5 auth + 5 exom) |

### Modified Files
| File | Changes |
|---|---|
| `Cargo.toml` | Add `sqlx`, `async-trait` dependencies |
| `src/lib.rs` or `src/main.rs` | Add `pub mod db;` |
| `src/auth/store.rs` | Refactor to thin orchestrator over `Arc<dyn AuthDb>` + caches |
| `src/auth/access.rs` | Remove `_system` special case (lines 35-45) |
| `src/brain.rs` | Add `user_email: Option<String>` to `Tx` struct (line 92-102) |
| `src/system_schema.rs` | Add `tx/user-email` constant (line 25-36) |
| `src/storage.rs` | `persist_*` and `load_*` delegate to `ExomDb` |
| `src/server.rs` | Wire `Arc<dyn AuthDb>` + `Arc<dyn ExomDb>` into `AppState`; extract `user_email` on mutations |
| `src/main.rs` | Add `--database-url` CLI flag; construct Pg or JSONL adapters at startup |
| `ui/src/lib/actorPrompt.svelte.ts` | Conditional: skip when `auth.isAuthenticated` |
| `ui/src/lib/ActorIdentityDialog.svelte` | Conditional: don't render when `auth.isAuthenticated` |

---

## Task 1: Add dependencies and migration SQL

**Files:**
- Modify: `Cargo.toml`
- Create: `migrations/001_initial.sql`

- [ ] **Step 1: Add sqlx and async-trait to Cargo.toml**

In `Cargo.toml`, add to `[features]`:
```toml
[features]
default = ["postgres"]
daemon = []
test-auth = []
postgres = ["sqlx"]
```

Add to `[dependencies]`:
```toml
async-trait = "0.1"
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "migrate", "chrono"], optional = true }
```

- [ ] **Step 2: Create migration file**

Create `migrations/001_initial.sql` with the full schema from the design spec (all 10 tables + indexes). Copy directly from design.md "Database Schema" section — both auth tables and core exom tables.

- [ ] **Step 3: Verify build**

Run: `cargo check`
Expected: compiles with no errors (sqlx is only imported, not yet used)

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml migrations/
git commit -m "chore: add sqlx, async-trait deps and initial migration SQL"
```

---

## Task 2: Define storage adapter traits and row types

**Files:**
- Create: `src/db/mod.rs`
- Modify: `src/lib.rs` (add `pub mod db;`)

- [ ] **Step 1: Create `src/db/mod.rs` with trait definitions**

```rust
//! Storage adapter traits and shared row types.

#[cfg(feature = "postgres")]
pub mod pg_auth;
#[cfg(feature = "postgres")]
pub mod pg_exom;
pub mod jsonl_auth;
pub mod jsonl_exom;

use crate::auth::{UserRole};
use crate::brain::{Tx, Fact, Observation, Belief, Branch};

/// Stored user row (shared across backends).
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

/// Session row.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_id: String,
    pub email: String,
    pub created_at: String,
    pub expires_at: String,
}

/// API key row with joined user info.
#[derive(Debug, Clone)]
pub struct ApiKeyWithUser {
    pub key_id: String,
    pub key_hash: String,
    pub email: String,
    pub label: String,
    pub created_at: String,
    pub user: StoredUser,
}

/// Stored API key (without user join).
#[derive(Debug, Clone)]
pub struct StoredApiKey {
    pub key_id: String,
    pub key_hash: String,
    pub email: String,
    pub label: String,
    pub created_at: String,
}

/// Share grant row.
#[derive(Debug, Clone)]
pub struct ShareGrant {
    pub share_id: String,
    pub owner_email: String,
    pub path: String,
    pub grantee_email: String,
    pub permission: String,
    pub created_at: String,
}

#[async_trait::async_trait]
pub trait AuthDb: Send + Sync {
    // Users
    async fn upsert_user(&self, email: &str, display_name: &str, provider: &str) -> anyhow::Result<()>;
    async fn get_user(&self, email: &str) -> anyhow::Result<Option<StoredUser>>;
    async fn list_users(&self) -> anyhow::Result<Vec<StoredUser>>;
    async fn set_role(&self, email: &str, role: UserRole) -> anyhow::Result<()>;
    async fn deactivate_user(&self, email: &str) -> anyhow::Result<()>;
    async fn activate_user(&self, email: &str) -> anyhow::Result<()>;
    async fn update_last_login(&self, email: &str) -> anyhow::Result<()>;

    // Sessions
    async fn create_session(&self, session_id: &str, email: &str, expires_at: &str) -> anyhow::Result<()>;
    async fn get_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRow>>;
    async fn delete_session(&self, session_id: &str) -> anyhow::Result<()>;
    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionRow>>;
    async fn cleanup_expired_sessions(&self) -> anyhow::Result<u64>;

    // API Keys
    async fn store_api_key(&self, key_id: &str, key_hash: &str, email: &str, label: &str) -> anyhow::Result<()>;
    async fn revoke_api_key(&self, key_id: &str) -> anyhow::Result<bool>;
    async fn list_api_keys(&self) -> anyhow::Result<Vec<StoredApiKey>>;
    async fn list_api_keys_for_user(&self, email: &str) -> anyhow::Result<Vec<StoredApiKey>>;
    async fn get_api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyWithUser>>;

    // Shares
    async fn add_share(&self, grant: &ShareGrant) -> anyhow::Result<()>;
    async fn revoke_share(&self, share_id: &str) -> anyhow::Result<bool>;
    async fn shares_for_grantee(&self, email: &str) -> anyhow::Result<Vec<ShareGrant>>;
    async fn shares_for_owner(&self, email: &str) -> anyhow::Result<Vec<ShareGrant>>;
    async fn list_all_shares(&self) -> anyhow::Result<Vec<ShareGrant>>;
    async fn update_share_paths(&self, old_prefix: &str, new_prefix: &str) -> anyhow::Result<u64>;

    // Domains
    async fn add_domain(&self, domain: &str) -> anyhow::Result<()>;
    async fn remove_domain(&self, domain: &str) -> anyhow::Result<()>;
    async fn list_domains(&self) -> anyhow::Result<Vec<String>>;
}

#[async_trait::async_trait]
pub trait ExomDb: Send + Sync {
    async fn load_transactions(&self, exom_path: &str) -> anyhow::Result<Vec<Tx>>;
    async fn save_transactions(&self, exom_path: &str, txs: &[Tx]) -> anyhow::Result<()>;
    async fn append_transaction(&self, exom_path: &str, tx: &Tx) -> anyhow::Result<()>;

    async fn load_facts(&self, exom_path: &str) -> anyhow::Result<Vec<Fact>>;
    async fn save_facts(&self, exom_path: &str, facts: &[Fact]) -> anyhow::Result<()>;

    async fn load_observations(&self, exom_path: &str) -> anyhow::Result<Vec<Observation>>;
    async fn save_observations(&self, exom_path: &str, obs: &[Observation]) -> anyhow::Result<()>;

    async fn load_beliefs(&self, exom_path: &str) -> anyhow::Result<Vec<Belief>>;
    async fn save_beliefs(&self, exom_path: &str, beliefs: &[Belief]) -> anyhow::Result<()>;

    async fn load_branches(&self, exom_path: &str) -> anyhow::Result<Vec<Branch>>;
    async fn save_branches(&self, exom_path: &str, branches: &[Branch]) -> anyhow::Result<()>;

    /// Atomically persist a mutation: tx row + affected table state.
    /// Postgres impl wraps in BEGIN/COMMIT. JSONL impl calls individual methods.
    async fn write_mutation(
        &self,
        exom_path: &str,
        tx: &Tx,
        facts: Option<&[Fact]>,
        observations: Option<&[Observation]>,
        beliefs: Option<&[Belief]>,
        branches: Option<&[Branch]>,
    ) -> anyhow::Result<()>;
}
```

Note: The `Fact`, `Observation`, `Belief`, `Branch` types are re-exported from `brain.rs`. If they currently live elsewhere, adjust imports. The `StoredUser`, `StoredApiKey`, `ShareGrant` types here replace the ones in `auth/store.rs` — the old ones should be removed and everything repointed to `db::StoredUser` etc.

- [ ] **Step 2: Register the module**

Add `pub mod db;` to `src/lib.rs` (or wherever modules are declared — check the existing module structure).

- [ ] **Step 3: Create stub files for submodules**

Create empty files so the build passes:
- `src/db/jsonl_auth.rs` — `// TODO: extract from auth/store.rs`
- `src/db/jsonl_exom.rs` — `// TODO: extract from storage.rs`
- `src/db/pg_auth.rs` — `// TODO: implement`
- `src/db/pg_exom.rs` — `// TODO: implement`

- [ ] **Step 4: Verify build**

Run: `cargo check`
Expected: compiles (traits defined, stubs exist, nothing calls them yet)

- [ ] **Step 5: Commit**

```bash
git add src/db/
git commit -m "feat(db): define AuthDb and ExomDb storage adapter traits"
```

---

## Task 3: Extract JSONL auth into `JsonlAuthDb`

**Files:**
- Create: `src/db/jsonl_auth.rs`
- Modify: `src/auth/store.rs`

This task extracts the JSONL read/write logic from `AuthStore` into `JsonlAuthDb` that implements `AuthDb`. `AuthStore` methods that currently do `self.append_entry()` + `self.apply_entry()` will instead call `self.auth_db.method()`.

- [ ] **Step 1: Implement `JsonlAuthDb`**

`src/db/jsonl_auth.rs` wraps the existing JSONL append + in-memory state pattern. It holds the same `Mutex<HashMap<...>>` fields currently on `AuthStore`, plus the JSONL file path. All the `apply_entry` / `append_entry` logic moves here.

Key implementation notes:
- Constructor takes a `PathBuf` (the `auth.jsonl` path in data dir — NOT `_system/auth/`)
- On construction, replay JSONL to populate in-memory state
- Each write method: append to JSONL + update in-memory state
- Session methods: `create_session` / `get_session` / `delete_session` use an internal `DashMap<String, SessionRow>` (ephemeral, not persisted — matches current behavior)
- `cleanup_expired_sessions` is a no-op (sessions are ephemeral in JSONL mode)

- [ ] **Step 2: Verify trait implementation compiles**

Run: `cargo check`
Expected: `JsonlAuthDb` implements all `AuthDb` methods

- [ ] **Step 3: Write test for JsonlAuthDb round-trip**

In `src/db/jsonl_auth.rs` (or a test module), write a test that:
1. Creates a `JsonlAuthDb` with a temp dir path
2. Calls `upsert_user`, `store_api_key`, `add_share`, `add_domain`
3. Drops the instance, creates a new one from same path (replay)
4. Asserts all data is present via `list_users`, `list_api_keys`, etc.

This validates JSONL replay correctness — the critical property.

- [ ] **Step 4: Run tests**

Run: `cargo test jsonl_auth`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/db/jsonl_auth.rs
git commit -m "feat(db): implement JsonlAuthDb — JSONL backend for AuthDb trait"
```

---

## Task 4: Extract JSONL exom into `JsonlExomDb`

**Files:**
- Create: `src/db/jsonl_exom.rs`

- [ ] **Step 1: Implement `JsonlExomDb`**

Wraps existing `storage::save_jsonl` / `storage::load_jsonl` functions. Constructor takes a `PathBuf` (tree root). Methods map exom_path to filesystem paths:
- `load_facts("work/main")` → `load_jsonl::<Fact>(tree_root.join("work/main/fact.jsonl"))`
- `save_facts("work/main", facts)` → `save_jsonl(facts, tree_root.join("work/main/fact.jsonl"))`
- Same pattern for transactions, observations, beliefs, branches

`write_mutation` default implementation: calls `append_transaction` then conditionally calls `save_facts`, `save_observations`, etc. for each `Some(...)` argument. Non-atomic (same as current behavior).

Note: `append_transaction` for JSONL should do a full `save_transactions` (current behavior is atomic overwrite, not true append). Check current `brain.rs` to confirm.

- [ ] **Step 2: Write test for JsonlExomDb round-trip**

Test that save + load roundtrips for each table type (facts, tx, observations, beliefs, branches) using a temp directory.

- [ ] **Step 3: Run tests**

Run: `cargo test jsonl_exom`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/db/jsonl_exom.rs
git commit -m "feat(db): implement JsonlExomDb — JSONL backend for ExomDb trait"
```

---

## Task 5: Implement `PgAuthDb`

**Files:**
- Create: `src/db/pg_auth.rs`

- [ ] **Step 1: Implement `PgAuthDb`**

Struct holds `sqlx::PgPool`. Each trait method is a single SQL query via `sqlx::query` / `sqlx::query_as`. Use runtime-checked queries (not compile-time macros) to avoid needing Postgres at build time.

Key implementation notes:
- `upsert_user`: `INSERT INTO users ... ON CONFLICT (email) DO UPDATE SET display_name = $2, last_login = now()`
- `set_role`: `UPDATE users SET role = $2 WHERE email = $1`
- `create_session`: `INSERT INTO sessions (session_id, email, expires_at) VALUES ($1, $2, $3)` — caller sets expires_at to `now() + 7 days` (session TTL)
- `get_session`: `SELECT s.*, u.* FROM sessions s JOIN users u ON s.email = u.email WHERE s.session_id = $1 AND s.expires_at > now()`
- `cleanup_expired_sessions`: `DELETE FROM sessions WHERE expires_at < now()` returning count
- `get_api_key_by_hash`: `SELECT k.*, u.* FROM api_keys k JOIN users u ON k.email = u.email WHERE k.key_hash = $1`
- `update_share_paths`: `UPDATE shares SET path = $2 || substring(path from length($1) + 1) WHERE path = $1 OR path LIKE $1 || '/%'`

Use `sqlx::FromRow` derive on intermediate row structs for query_as, then convert to `db::StoredUser` etc.

- [ ] **Step 2: Compile check**

Run: `cargo check --features postgres`
Expected: compiles (queries are runtime-checked, no DB connection needed)

- [ ] **Step 3: Commit**

```bash
git add src/db/pg_auth.rs
git commit -m "feat(db): implement PgAuthDb — Postgres backend for AuthDb trait"
```

Integration tests come in Task 9 (need running Postgres).

---

## Task 6: Implement `PgExomDb`

**Files:**
- Create: `src/db/pg_exom.rs`

- [ ] **Step 1: Implement `PgExomDb`**

Struct holds `sqlx::PgPool`. Each method maps to SQL operations on the exom tables, filtered by `exom_path`.

Key implementation notes:
- `load_facts`: `SELECT * FROM facts WHERE exom_path = $1`
- `save_facts`: `DELETE FROM facts WHERE exom_path = $1` then batch `INSERT` — wrap in transaction
- `append_transaction`: `INSERT INTO transactions (...) VALUES (...)`
- `write_mutation`: Use `pool.begin()` to start a Postgres transaction. Insert tx row, then for each `Some(table_data)`, delete existing rows for exom_path and insert new ones. Call `tx.commit()`. This is the atomic write path.

For `save_*` methods (full table replace), use a Postgres transaction:
```rust
let mut pg_tx = self.pool.begin().await?;
sqlx::query("DELETE FROM facts WHERE exom_path = $1")
    .bind(exom_path).execute(&mut *pg_tx).await?;
for fact in facts {
    sqlx::query("INSERT INTO facts (...) VALUES (...)")
        .bind(...).execute(&mut *pg_tx).await?;
}
pg_tx.commit().await?;
```

Map `Tx`, `Fact`, `Observation`, `Belief`, `Branch` structs to/from SQL rows. Handle type conversions (String timestamps → chrono if needed, Vec → TEXT[]).

- [ ] **Step 2: Compile check**

Run: `cargo check --features postgres`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add src/db/pg_exom.rs
git commit -m "feat(db): implement PgExomDb — Postgres backend for ExomDb trait"
```

---

## Task 7: Refactor `AuthStore` to use `AuthDb` trait

**Files:**
- Modify: `src/auth/store.rs`
- Modify: `src/auth/routes.rs`
- Modify: `src/auth/admin.rs`

This is the biggest refactor. `AuthStore` becomes a thin orchestrator holding caches + delegating to `Arc<dyn AuthDb>`.

- [ ] **Step 1: Refactor `AuthStore` struct**

Replace internal `Mutex<HashMap<...>>` fields with `Arc<dyn AuthDb>`:

```rust
pub struct AuthStore {
    pub auth_db: Arc<dyn AuthDb>,
    // Hot caches (remain in-memory for fast path)
    pub session_cache: DashMap<String, User>,
    pub api_key_cache: DashMap<String, User>,
}
```

Remove: `exom_disk`, `jsonl_path`, `users`, `api_keys`, `api_key_by_hash`, `top_admin`, `admins`, `allowed_domains`, `share_grants` fields.

- [ ] **Step 2: Refactor `AuthStore` methods**

Each method now delegates to `self.auth_db`. Examples:

- `record_user(email, name, provider)` → `self.auth_db.upsert_user(email, name, provider).await`
- `list_users()` → `self.auth_db.list_users().await`
- `check_domain(email)` → `let domains = self.auth_db.list_domains().await?; domains.iter().any(|d| email.ends_with(d))`
- `resolve_role(email)` → `self.auth_db.get_user(email).await?.map(|u| u.role).unwrap_or(UserRole::Regular)`

Session lookup becomes cache-then-DB:
```rust
pub async fn get_user_by_session(&self, session_id: &str) -> Option<User> {
    // Fast path: cache hit
    if let Some(user) = self.session_cache.get(session_id) {
        return Some(user.clone());
    }
    // Slow path: DB lookup (Postgres mode — survives restart)
    if let Ok(Some(row)) = self.auth_db.get_session(session_id).await {
        let user = self.build_user_from_session(&row).await;
        if let Some(ref u) = user {
            self.session_cache.insert(session_id.to_string(), u.clone());
        }
        return user;
    }
    None
}
```

API key lookup remains cache-first (cache populated on startup from DB).

- [ ] **Step 3: Update `bootstrap` to accept `Arc<dyn AuthDb>`**

Replace `AuthStore::bootstrap(tree_root, domains)` with:
```rust
pub async fn new(auth_db: Arc<dyn AuthDb>) -> anyhow::Result<Self> {
    let store = Self {
        auth_db,
        session_cache: DashMap::new(),
        api_key_cache: DashMap::new(),
    };
    store.rebuild_api_key_cache().await?;
    Ok(store)
}
```

The caller constructs the appropriate `AuthDb` implementation and passes it in.

- [ ] **Step 4: Make AuthStore methods async**

All public methods that call `auth_db` become `async`. This cascades to `routes.rs` and `admin.rs` handlers — but they're already async Axum handlers, so the change is adding `.await` calls.

Update every call site in `src/auth/routes.rs` and `src/auth/admin.rs` to await the new async methods.

- [ ] **Step 5: Verify build**

Run: `cargo check`
Expected: compiles (with possible warnings about unused old code)

- [ ] **Step 6: Run existing tests**

Run: `cargo test`
Expected: all existing tests pass (or identify tests that need updating)

- [ ] **Step 7: Commit**

```bash
git add src/auth/
git commit -m "refactor(auth): AuthStore delegates to AuthDb trait adapter"
```

---

## Task 8: Wire database adapters into AppState and CLI

**Files:**
- Modify: `src/server.rs` (AppState struct, lines 66-76)
- Modify: `src/main.rs` (CLI flags, startup)
- Modify: `src/db/mod.rs` (add pool init function)

- [ ] **Step 1: Add PgPool init to `db/mod.rs`**

```rust
#[cfg(feature = "postgres")]
pub async fn create_pool(database_url: &str) -> anyhow::Result<sqlx::PgPool> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
```

- [ ] **Step 2: Update AppState**

In `src/server.rs`, add fields:
```rust
pub struct AppState {
    // ... existing fields ...
    pub auth_db: Arc<dyn AuthDb>,
    pub exom_db: Arc<dyn ExomDb>,
}
```

Remove `auth_store` field — `AuthStore` is now constructed from `auth_db` and stored separately or inlined.

- [ ] **Step 3: Add `--database-url` CLI flag**

In `src/main.rs`, add to the `Serve` (and `Daemon`) subcommand args:
```rust
#[arg(long, env = "DATABASE_URL")]
database_url: Option<String>,
```

- [ ] **Step 4: Construct adapters at startup**

In the startup path (where `AuthStore::bootstrap` is currently called, around line 1488):

```rust
let (auth_db, exom_db): (Arc<dyn AuthDb>, Arc<dyn ExomDb>) = if let Some(ref db_url) = args.database_url {
    #[cfg(feature = "postgres")]
    {
        let pool = db::create_pool(db_url).await?;
        let pool = Arc::new(pool);
        (
            Arc::new(db::pg_auth::PgAuthDb::new(pool.clone())),
            Arc::new(db::pg_exom::PgExomDb::new(pool)),
        )
    }
    #[cfg(not(feature = "postgres"))]
    {
        anyhow::bail!("--database-url requires the 'postgres' feature");
    }
} else {
    let data_dir = /* existing data dir resolution */;
    (
        Arc::new(db::jsonl_auth::JsonlAuthDb::new(data_dir.join("auth.jsonl"))?),
        Arc::new(db::jsonl_exom::JsonlExomDb::new(data_dir.join("tree"))),
    )
};

let auth_store = if args.auth_provider.is_some() {
    Some(Arc::new(AuthStore::new(auth_db.clone()).await?))
} else {
    None
};
```

- [ ] **Step 5: Spawn session cleanup (Postgres mode)**

After constructing auth_store, if database_url is set:
```rust
if args.database_url.is_some() {
    let cleanup_db = auth_db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(900));
        loop {
            interval.tick().await;
            let _ = cleanup_db.cleanup_expired_sessions().await;
        }
    });
}
```

- [ ] **Step 6: Verify build**

Run: `cargo check`
Expected: compiles

- [ ] **Step 7: Commit**

```bash
git add src/server.rs src/main.rs src/db/mod.rs
git commit -m "feat: wire Postgres/JSONL adapters into AppState and CLI"
```

---

## Task 9: Add `user_email` to Tx and system schema

**Files:**
- Modify: `src/brain.rs` (Tx struct, line 91-102)
- Modify: `src/system_schema.rs` (tx attributes, line 25-36)
- Modify: `src/server.rs` (mutation handlers — extract user_email from auth)

- [ ] **Step 1: Add `user_email` field to Tx**

In `src/brain.rs`, modify the `Tx` struct:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tx {
    pub tx_id: TxId,
    pub tx_time: String,
    pub user_email: Option<String>,  // NEW: authenticated user, server-set
    pub actor: String,               // existing: agent name or legacy actor
    pub action: TxAction,
    pub refs: Vec<EntityId>,
    pub note: String,
    pub parent_tx_id: Option<TxId>,
    pub branch_id: String,
    pub session: Option<String>,
}
```

Update all `Tx { ... }` construction sites in `brain.rs` to include `user_email: None` (will be populated from handler context).

- [ ] **Step 2: Add `tx/user-email` system attribute**

In `src/system_schema.rs`, add to the `tx` module:
```rust
pub mod tx {
    pub const ID: &str = "tx/id";
    pub const TIME: &str = "tx/time";
    pub const USER_EMAIL: &str = "tx/user-email";  // NEW
    pub const ACTOR: &str = "tx/actor";
    // ... rest unchanged
}
```

Add the attribute to the `system_attributes()` function where tx attributes are listed (around line 173-218). Follow the existing pattern for adding a new string attribute.

- [ ] **Step 3: Extract user_email in mutation handlers**

In `src/server.rs`, find mutation handlers (assert-fact, retract, eval, etc.). They already have `MaybeUser` available. Before building the `Tx`, extract user_email:

```rust
let user_email = maybe_user.0.as_ref().map(|u| u.email.clone());
```

Pass this into the Tx construction.

For the `actor` field: if request body has an `actor` field, use it. Otherwise fall back to `user_email` or "anonymous".

- [ ] **Step 4: Verify build and run tests**

Run: `cargo check && cargo test`
Expected: compiles, all tests pass (existing tests construct Tx with `user_email: None`)

- [ ] **Step 5: Commit**

```bash
git add src/brain.rs src/system_schema.rs src/server.rs
git commit -m "feat: add user_email to Tx struct and system schema"
```

---

## Task 10: Remove `_system/auth` and access control special case

**Files:**
- Modify: `src/auth/access.rs` (lines 35-45)
- Modify: `src/auth/store.rs` (bootstrap no longer creates `_system/auth`)
- Modify: `tests/auth_shares.rs` (remove/update `_system` tests)

- [ ] **Step 1: Remove `_system` access control**

In `src/auth/access.rs`, remove the `is_system` check block entirely (lines 35-45). The function should go straight from doc comment to admin check:

```rust
pub fn resolve_access(user: &User, path: &str, store: &AuthStore) -> AccessLevel {
    // Admins get full access
    if user.is_admin() {
        return AccessLevel::FullAccess;
    }
    // ... rest unchanged
}
```

Update the doc comment to remove references to `_system`.

- [ ] **Step 2: Remove `_system/auth` creation from bootstrap**

The old `AuthStore::bootstrap` created `_system/auth/exom.json`. The new `AuthStore::new` doesn't do this. Verify no other code creates `_system/auth`. Search for `_system` references in `src/` and remove any that create this directory.

- [ ] **Step 3: Skip `_system` in tree walk**

In `src/tree.rs`, in `walk_root()`, add a filter to skip the `_system` entry if it exists on disk from a previous install:

```rust
// In walk_root, when iterating top-level entries:
if entry_name == "_system" {
    continue;
}
```

This is exact match only — does NOT skip other underscore-prefixed paths.

- [ ] **Step 4: Update tests**

Remove or update `system_path_top_admin_can_read`, `system_path_regular_user_denied`, `system_path_denied_for_regular_admin`, `system_path_readonly_for_top_admin` tests. These test access control for a path that no longer has special handling.

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add src/auth/access.rs src/tree.rs tests/
git commit -m "fix: remove _system/auth exom and access control special case"
```

---

## Task 11: Conditional actor prompt in UI

**Files:**
- Modify: `ui/src/lib/actorPrompt.svelte.ts`
- Modify: `ui/src/lib/ActorIdentityDialog.svelte`
- Modify: `ui/src/routes/+layout.svelte`

- [ ] **Step 1: Make ActorIdentityDialog conditional on auth**

In `ui/src/routes/+layout.svelte`, wrap the `ActorIdentityDialog` component:

```svelte
{#if !auth.isAuthenticated}
    <ActorIdentityDialog />
{/if}
```

This preserves the dialog for no-auth JSONL mode but hides it when auth is active.

- [ ] **Step 2: Skip actor prompt when authenticated**

In `ui/src/lib/actorPrompt.svelte.ts`, modify the `run()` method to skip the prompt when authenticated:

```typescript
run(callback: () => Promise<void>) {
    if (auth.isAuthenticated) {
        // Authenticated mode: no actor prompt needed, user_email is server-set
        callback();
        return;
    }
    // Existing actor prompt logic for no-auth mode...
}
```

Import `auth` from `$lib/auth.svelte`.

- [ ] **Step 3: Verify UI builds**

Run: `cd ui && npm run check && npm run build`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add ui/src/lib/actorPrompt.svelte.ts ui/src/lib/ActorIdentityDialog.svelte ui/src/routes/+layout.svelte
git commit -m "feat(ui): skip actor prompt when auth is active"
```

---

## Task 12: Integration tests with Postgres

**Files:**
- Create: `tests/pg_auth.rs` (or extend existing test files)

- [ ] **Step 1: Write Postgres auth integration test**

Test requires a running Postgres instance. Gate with `#[cfg(feature = "postgres")]`.

```rust
#[cfg(feature = "postgres")]
#[tokio::test]
async fn pg_auth_round_trip() {
    let db_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost:5432/ray_exomem_test".to_string());
    let pool = db::create_pool(&db_url).await.unwrap();
    let auth_db = db::pg_auth::PgAuthDb::new(Arc::new(pool));

    // Create user
    auth_db.upsert_user("alice@co.com", "Alice", "google").await.unwrap();
    let user = auth_db.get_user("alice@co.com").await.unwrap().unwrap();
    assert_eq!(user.email, "alice@co.com");
    assert_eq!(user.role, UserRole::Regular);

    // Set role
    auth_db.set_role("alice@co.com", UserRole::TopAdmin).await.unwrap();
    let user = auth_db.get_user("alice@co.com").await.unwrap().unwrap();
    assert_eq!(user.role, UserRole::TopAdmin);

    // Session persistence
    auth_db.create_session("sess1", "alice@co.com", "2099-01-01T00:00:00Z").await.unwrap();
    let sess = auth_db.get_session("sess1").await.unwrap();
    assert!(sess.is_some());

    // Cleanup
    auth_db.cleanup_expired_sessions().await.unwrap();
    // sess1 is far future, should survive cleanup
    assert!(auth_db.get_session("sess1").await.unwrap().is_some());
}
```

- [ ] **Step 2: Write Postgres exom integration test**

```rust
#[cfg(feature = "postgres")]
#[tokio::test]
async fn pg_exom_round_trip() {
    // Test save_facts + load_facts, write_mutation atomicity, etc.
}
```

- [ ] **Step 3: Run integration tests (requires local Postgres)**

```bash
createdb ray_exomem_test 2>/dev/null || true
TEST_DATABASE_URL=postgres://localhost:5432/ray_exomem_test cargo test --features postgres pg_
```
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add tests/
git commit -m "test: add Postgres integration tests for auth and exom adapters"
```

---

## Task 13: Wire ExomDb into brain/storage persist paths

**Files:**
- Modify: `src/storage.rs`
- Modify: `src/brain.rs`
- Modify: `src/server.rs`

- [ ] **Step 1: Pass ExomDb to persist calls**

In `src/server.rs`, mutation handlers currently call brain functions that eventually call `storage::save_jsonl`. These need to be updated to call `exom_db.save_facts()` etc.

The cleanest approach: make `ExomDb` available to the persist layer. Either pass it through function params or store it on `AppState` (already there from Task 8).

Find all calls to `storage::save_jsonl` and `storage::load_jsonl` in `brain.rs` and replace with calls through `ExomDb`. This may require making some brain functions async (since ExomDb methods are async).

For functions that are currently sync (like `open_exom`), wrap the DB calls in `tokio::runtime::Handle::current().block_on()` as a temporary bridge, or refactor to async.

- [ ] **Step 2: Verify JSONL fallback still works**

Run the full test suite without `DATABASE_URL` set:
```bash
cargo test
```
Expected: all pass (JSONL path unchanged)

- [ ] **Step 3: Verify Postgres path works**

```bash
TEST_DATABASE_URL=postgres://localhost:5432/ray_exomem_test cargo test --features postgres
```

- [ ] **Step 4: Commit**

```bash
git add src/storage.rs src/brain.rs src/server.rs
git commit -m "feat: wire ExomDb into brain/storage persist paths"
```

---

## Task 14: Full build, UI build, deploy test

**Files:**
- Modify: `.github/workflows/deploy.yml` (add DATABASE_URL secret if needed for prod)

- [ ] **Step 1: Full release build**

```bash
cargo build --release
```
Expected: clean build

- [ ] **Step 2: UI build**

```bash
cd ui && npm run check && npm run build
```
Expected: no errors

- [ ] **Step 3: Full test suite**

```bash
cargo test
```
Expected: all pass

- [ ] **Step 4: Local smoke test (JSONL mode)**

```bash
ray-exomem serve --bind 127.0.0.1:9780
```
Open browser, verify tree works, create exom, assert fact. No Postgres needed.

- [ ] **Step 5: Local smoke test (Postgres mode)**

```bash
createdb ray_exomem_dev 2>/dev/null || true
ray-exomem serve --bind 127.0.0.1:9780 --database-url postgres://localhost:5432/ray_exomem_dev --auth-provider mock
```
Login, verify profile page, admin page, assert fact, check tx history shows user_email.

- [ ] **Step 6: Commit any final fixes**

```bash
git add -A
git commit -m "chore: final build verification and fixes"
```

---

## Dependency Graph

```
Task 1 (deps + migration SQL)
  └→ Task 2 (trait definitions)
       ├→ Task 3 (JsonlAuthDb)
       ├→ Task 4 (JsonlExomDb)
       ├→ Task 5 (PgAuthDb)
       └→ Task 6 (PgExomDb)
            └→ Task 7 (refactor AuthStore)
                 └→ Task 8 (wire into AppState/CLI)
                      ├→ Task 9 (user_email on Tx)
                      ├→ Task 10 (remove _system)
                      ├→ Task 11 (conditional actor UI)
                      ├→ Task 12 (integration tests)
                      └→ Task 13 (wire ExomDb into brain)
                           └→ Task 14 (full build + smoke test)
```

Tasks 3-6 can run in parallel. Tasks 9-12 can run in parallel after Task 8.
