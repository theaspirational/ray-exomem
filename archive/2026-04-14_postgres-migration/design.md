# Postgres Storage Backend for ray-exomem

**Date:** 2026-04-14
**Status:** Design approved, pending implementation

## Overview

Replace JSONL file-based persistence with PostgreSQL via sqlx. Introduce trait-based storage adapters (`AuthDb`, `ExomDb`) with two implementations: Postgres and JSONL. Splay tables and rayforce2 FFI are unchanged — Postgres replaces JSONL as the durable source of truth, splay tables remain the query cache.

## Decisions

| Topic | Decision |
|---|---|
| Library | sqlx (async, runtime-checked queries, PgPool) |
| Schema | 10 tables: 5 auth + 5 core exom |
| Storage pattern | Trait adapters with Postgres + JSONL implementations |
| Backend selection | `--database-url` flag or `DATABASE_URL` env → Postgres; absent → JSONL fallback |
| Sessions | Persistent in Postgres mode; DashMap-only in JSONL mode |
| JSONL migration | Clean break — no import tool, fresh start |
| `_system/auth` | Removed entirely — no exom directory |
| Auth JSONL fallback | Standalone `auth.jsonl` file in data dir (not an exom) |
| Actor model | `user_email` server-set from auth; `actor` optional from MCP/CLI clients. In no-auth JSONL mode, actor prompt retained for browser. |
| UI cleanup | Remove ActorIdentityDialog + localStorage actor when auth enabled. Keep actor prompt in no-auth mode for browser provenance. |
| Cargo feature | `postgres` feature flag, default enabled |

## Database Schema

### Auth Tables

```sql
CREATE TABLE users (
    email        TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    provider     TEXT NOT NULL,
    role         TEXT NOT NULL DEFAULT 'regular',
    active       BOOLEAN NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login   TIMESTAMPTZ
);

CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    email      TEXT NOT NULL REFERENCES users(email),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_sessions_email ON sessions(email);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);

CREATE TABLE api_keys (
    key_id     TEXT PRIMARY KEY,
    key_hash   TEXT NOT NULL UNIQUE,
    email      TEXT NOT NULL REFERENCES users(email),
    label      TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_api_keys_email ON api_keys(email);

CREATE TABLE shares (
    share_id      TEXT PRIMARY KEY,
    owner_email   TEXT NOT NULL REFERENCES users(email),
    path          TEXT NOT NULL,
    grantee_email TEXT NOT NULL REFERENCES users(email),
    permission    TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_shares_grantee ON shares(grantee_email);
CREATE INDEX idx_shares_owner ON shares(owner_email);
CREATE INDEX idx_shares_path ON shares(path);

CREATE TABLE allowed_domains (
    domain TEXT PRIMARY KEY
);
```

### Core Exom Tables

All partitioned by `exom_path` column.

```sql
CREATE TABLE transactions (
    id           BIGSERIAL PRIMARY KEY,
    exom_path    TEXT NOT NULL,
    tx_id        BIGINT NOT NULL,
    tx_time      TIMESTAMPTZ NOT NULL,
    user_email   TEXT,
    actor        TEXT,
    action       TEXT NOT NULL,
    refs         TEXT[] NOT NULL DEFAULT '{}',
    note         TEXT NOT NULL DEFAULT '',
    parent_tx_id BIGINT,
    branch_id    TEXT NOT NULL DEFAULT 'main',
    session      TEXT,
    UNIQUE(exom_path, tx_id)
);
CREATE INDEX idx_tx_exom ON transactions(exom_path);

CREATE TABLE facts (
    id               BIGSERIAL PRIMARY KEY,
    exom_path        TEXT NOT NULL,
    fact_id          TEXT NOT NULL,
    predicate        TEXT NOT NULL,
    value            TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    created_by_tx    BIGINT NOT NULL,
    superseded_by_tx BIGINT,
    revoked_by_tx    BIGINT,
    confidence       DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    provenance       TEXT NOT NULL DEFAULT '',
    valid_from       TIMESTAMPTZ NOT NULL,
    valid_to         TIMESTAMPTZ,
    UNIQUE(exom_path, fact_id)
);
CREATE INDEX idx_facts_exom ON facts(exom_path);
CREATE INDEX idx_facts_predicate ON facts(exom_path, predicate);

CREATE TABLE observations (
    id          BIGSERIAL PRIMARY KEY,
    exom_path   TEXT NOT NULL,
    obs_id      TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_ref  TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    confidence  DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    tx_id       BIGINT NOT NULL,
    tags        TEXT[] NOT NULL DEFAULT '{}',
    valid_from  TIMESTAMPTZ NOT NULL,
    valid_to    TIMESTAMPTZ,
    UNIQUE(exom_path, obs_id)
);
CREATE INDEX idx_obs_exom ON observations(exom_path);

CREATE TABLE beliefs (
    id            BIGSERIAL PRIMARY KEY,
    exom_path     TEXT NOT NULL,
    belief_id     TEXT NOT NULL,
    claim_text    TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'active',
    confidence    DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    supported_by  TEXT[] NOT NULL DEFAULT '{}',
    created_by_tx BIGINT NOT NULL,
    valid_from    TIMESTAMPTZ NOT NULL,
    valid_to      TIMESTAMPTZ,
    rationale     TEXT NOT NULL DEFAULT '',
    UNIQUE(exom_path, belief_id)
);
CREATE INDEX idx_beliefs_exom ON beliefs(exom_path);

CREATE TABLE branches (
    id               BIGSERIAL PRIMARY KEY,
    exom_path        TEXT NOT NULL,
    branch_id        TEXT NOT NULL,
    name             TEXT NOT NULL,
    parent_branch_id TEXT,
    created_tx_id    BIGINT NOT NULL,
    archived         BOOLEAN NOT NULL DEFAULT false,
    claimed_by       TEXT,
    UNIQUE(exom_path, branch_id)
);
CREATE INDEX idx_branches_exom ON branches(exom_path);
```

## Storage Adapter Traits

```rust
#[async_trait]
pub trait AuthDb: Send + Sync {
    // Users
    async fn upsert_user(&self, email: &str, display_name: &str, provider: &str) -> Result<()>;
    async fn get_user(&self, email: &str) -> Result<Option<StoredUser>>;
    async fn list_users(&self) -> Result<Vec<StoredUser>>;
    async fn set_role(&self, email: &str, role: UserRole) -> Result<()>;
    async fn deactivate_user(&self, email: &str) -> Result<()>;
    async fn activate_user(&self, email: &str) -> Result<()>;
    async fn update_last_login(&self, email: &str) -> Result<()>;

    // Sessions
    async fn create_session(&self, session_id: &str, email: &str, expires_at: DateTime) -> Result<()>;
    async fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>>;
    async fn delete_session(&self, session_id: &str) -> Result<()>;
    async fn list_sessions(&self) -> Result<Vec<SessionRow>>;
    async fn cleanup_expired_sessions(&self) -> Result<u64>;

    // API Keys
    async fn store_api_key(&self, key_id: &str, key_hash: &str, email: &str, label: &str) -> Result<()>;
    async fn revoke_api_key(&self, key_id: &str) -> Result<bool>;
    async fn list_api_keys(&self) -> Result<Vec<StoredApiKey>>;
    async fn list_api_keys_for_user(&self, email: &str) -> Result<Vec<StoredApiKey>>;
    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyWithUser>>;

    // Shares
    async fn add_share(&self, grant: &ShareGrant) -> Result<()>;
    async fn revoke_share(&self, share_id: &str) -> Result<bool>;
    async fn shares_for_grantee(&self, email: &str) -> Result<Vec<ShareGrant>>;
    async fn shares_for_owner(&self, email: &str) -> Result<Vec<ShareGrant>>;
    async fn list_all_shares(&self) -> Result<Vec<ShareGrant>>;
    async fn update_share_paths(&self, old_prefix: &str, new_prefix: &str) -> Result<u64>;

    // Domains
    async fn add_domain(&self, domain: &str) -> Result<()>;
    async fn remove_domain(&self, domain: &str) -> Result<()>;
    async fn list_domains(&self) -> Result<Vec<String>>;
}

#[async_trait]
pub trait ExomDb: Send + Sync {
    async fn load_transactions(&self, exom_path: &str) -> Result<Vec<Tx>>;
    async fn save_transactions(&self, exom_path: &str, txs: &[Tx]) -> Result<()>;
    async fn append_transaction(&self, exom_path: &str, tx: &Tx) -> Result<()>;

    async fn load_facts(&self, exom_path: &str) -> Result<Vec<Fact>>;
    async fn save_facts(&self, exom_path: &str, facts: &[Fact]) -> Result<()>;

    async fn load_observations(&self, exom_path: &str) -> Result<Vec<Observation>>;
    async fn save_observations(&self, exom_path: &str, obs: &[Observation]) -> Result<()>;

    async fn load_beliefs(&self, exom_path: &str) -> Result<Vec<Belief>>;
    async fn save_beliefs(&self, exom_path: &str, beliefs: &[Belief]) -> Result<()>;

    async fn load_branches(&self, exom_path: &str) -> Result<Vec<Branch>>;
    async fn save_branches(&self, exom_path: &str, branches: &[Branch]) -> Result<()>;
}
```

Two implementations each:
- `JsonlAuthDb` / `JsonlExomDb` — wraps current JSONL logic
- `PgAuthDb` / `PgExomDb` — sqlx queries

## Code Structure

### New Files

- `src/db/mod.rs` — Trait definitions, PgPool init, migration runner
- `src/db/pg_auth.rs` — `PgAuthDb` implementation
- `src/db/pg_exom.rs` — `PgExomDb` implementation
- `src/db/jsonl_auth.rs` — `JsonlAuthDb` (extracted from current `auth/store.rs`)
- `src/db/jsonl_exom.rs` — `JsonlExomDb` (extracted from current `storage.rs`)
- `migrations/001_initial.sql` — Full schema creation

### Modified Files

- `Cargo.toml` — Add sqlx, async-trait dependencies
- `src/auth/store.rs` — Thin orchestrator over `Arc<dyn AuthDb>` + caches
- `src/storage.rs` — Persist/load delegates to `Arc<dyn ExomDb>`
- `src/brain.rs` — `open_exom()` uses ExomDb; Tx gains `user_email` field
- `src/main.rs` — `--database-url` flag on serve/daemon, construct adapters
- `src/server.rs` — Wire adapters into AppState, extract user_email on mutations
- `src/auth/access.rs` — Remove `_system` special case entirely

### Removed

- `_system/auth/` exom directory concept
- `_system` access control special case in `access.rs`
- ActorIdentityDialog / actorPrompt / localStorage actor **when auth is enabled** (retained for no-auth mode)
- Actor display in TopBar (already done)

### Additionally Modified

- `src/system_schema.rs` — add `tx/user-email` system attribute
- UI tx history views — show `user_email` + optional `actor`

## Runtime Behavior

### Startup (Postgres)

1. Parse `--database-url` or `DATABASE_URL`
2. Create `PgPool` (max 10 connections)
3. Run `sqlx::migrate!("./migrations")`
4. Construct `PgAuthDb` + `PgExomDb`
5. Build `AuthStore` with `PgAuthDb`:
   - Populate API key cache from DB (`SELECT * FROM api_keys JOIN users`)
   - Session cache starts empty — sessions are resolved on demand (see below)
6. Spawn session cleanup task (every 15 min)
7. Start Axum

**Session resolution (Postgres mode):** `get_user_by_session()` first checks DashMap cache. On cache miss, queries `SELECT s.*, u.* FROM sessions s JOIN users u ON s.email = u.email WHERE s.session_id = $1 AND s.expires_at > now()`. If found, populates cache and returns user. This means sessions survive restarts — first request after restart hits DB, subsequent requests hit cache.

### Startup (JSONL fallback)

1. No database URL found
2. Construct `JsonlAuthDb` (reads `<data-dir>/auth.jsonl`) + `JsonlExomDb`
3. Build `AuthStore` with `JsonlAuthDb`, replay JSONL
4. Start Axum (no session cleanup task — sessions are ephemeral)

### Mutation Flow

```
Client sends POST /api/actions/assert-fact
  { predicate: "x", value: "y", actor: "claude-desktop" }
  + Bearer API key (or session cookie)

Server:
  1. Authenticate → resolve user_email from key/session
  2. actor = request body "actor" field (optional)
  3. Build Tx { user_email, actor, action: "assert-fact", ... }
  4. Brain updates in-memory state
  5. ExomDb.write_mutation() — single atomic operation (see below)
  6. Rebuild splay table (unchanged)
```

**Atomic writes (Postgres mode):** `ExomDb` gains a `write_mutation()` method that wraps the transaction row + all affected table changes (facts, beliefs, observations, branches) in a single Postgres `BEGIN ... COMMIT` transaction. A crash between steps cannot leave a tx row without its matching data changes. The individual `append_transaction()` + `save_facts()` methods remain for bulk loads and JSONL fallback where atomicity across files isn't possible anyway.

```rust
#[async_trait]
pub trait ExomDb: Send + Sync {
    // ... existing methods ...

    /// Atomically persist a mutation: transaction + affected table state.
    /// Default implementation calls individual methods (non-atomic, for JSONL).
    /// Postgres overrides with a single DB transaction.
    async fn write_mutation(
        &self,
        exom_path: &str,
        tx: &Tx,
        facts: Option<&[Fact]>,
        observations: Option<&[Observation]>,
        beliefs: Option<&[Belief]>,
        branches: Option<&[Branch]>,
    ) -> Result<()>;
}
```

### Actor Model

**Authenticated mode (auth enabled):**
- `user_email`: Always server-set from authenticated session/API key. Not client-editable.
- `actor`: Optional client-provided string identifying the agent (e.g. "claude-desktop", "cursor", "my-script"). Browser UI sets actor to None — user_email provides provenance.
- Transaction history shows: "alice@co.com" or "alice@co.com via claude-desktop"
- ActorIdentityDialog and localStorage actor removed.

**No-auth mode (JSONL fallback, no auth provider):**
- `user_email`: None (no auth session).
- `actor`: Required — set via ActorIdentityDialog (existing behavior preserved). Used for provenance, TOFU branch claims, session ownership.
- ActorIdentityDialog, localStorage actor, and actor prompt retained.

**Transition:** UI checks `auth.isAuthenticated`. If true, actor prompt never shown and actor field is not sent from browser. If false, actor prompt works as today.

## AppState

```rust
pub struct AppState {
    pub engine: Engine,
    pub exoms: Mutex<HashMap<String, ExomState>>,
    pub auth_db: Arc<dyn AuthDb>,
    pub exom_db: Arc<dyn ExomDb>,
    pub auth_store: Option<AuthStore>,  // orchestrator with caches
    pub tree_root: Option<PathBuf>,
    pub sym_path: Option<PathBuf>,
    // ... existing fields
}
```

## CI / Build

- `sqlx` behind `features = ["postgres"]` cargo feature (default enabled)
- CI builds with feature enabled; no running Postgres needed (runtime-checked queries, not compile-time macros)
- Tests: integration tests that need Postgres use `#[cfg(feature = "postgres")]` gate + test database

## Scope Boundaries

Things this spec **does not change:**

- **`exom.json` metadata** — remains on-disk, per-exom. Source of truth for exom kind (bare/project/session), current branch, session metadata, format version. Not modeled in Postgres. The filesystem tree structure and `exom.json` files continue to define what exoms exist and their type.
- **Splay tables / rayforce2 FFI** — unchanged. Splay tables are rebuilt from Postgres (or JSONL) rows on startup, used as query cache.
- **Symbol table (`sym`)** — unchanged. Shared binary file, managed by rayforce2.
- **Tree walk / filesystem structure** — exom directories still exist on disk for splay tables and `exom.json`. Postgres stores the data that was in JSONL sidecars; directory structure remains.

**Additional files requiring updates:**

- `src/system_schema.rs` — tx views currently only expose `tx/actor`. Must add `tx/user-email` attribute and update ontology generation to reflect the new two-field provenance model.
- UI tx history/provenance views — must display `user_email` + optional `actor` instead of bare `actor` string.

## Removal of _system

- `_system` directory no longer created by auth bootstrap
- Auth JSONL fallback uses standalone `<data-dir>/auth.jsonl`
- Remove `_system` path checks from `access.rs` (no special-case access control)
- If `_system` exists on disk from previous installs, `walk_root()` skips the exact `_system` entry only (not other underscore-prefixed paths like `_foo`)

## Session Cleanup

Postgres mode only. Session TTL: 7 days from creation. Background Tokio task:

```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(900));
    loop {
        interval.tick().await;
        let _ = auth_db.cleanup_expired_sessions().await;
    }
});
```
