# Auth, MCP & Admin Panel — Design Spec

**Date:** 2026-04-14
**Status:** Draft (rev 2)
**Scope:** Authentication, authorization, MCP server, admin panel for ray-exomem

---

## 1. Overview

Add authentication, per-user namespaces with visibility/sharing, an MCP protocol server mirroring the full API, and an admin panel — transforming ray-exomem from a single-user local daemon into a multi-user internal service.

### Decisions

- Google OAuth (GSI/OIDC) as first auth provider; pluggable provider interface for future providers
- Browser login via Svelte UI; profile page shows copyable MCP config with embedded API key
- Long-lived API keys, manually revoked from UI
- User namespace = full email (e.g., `alice@company.com/projects/main`)
- Per-path sharing to specific users with read-only or read-write permission
- Auth state stored in system exom (`_system/auth`), dog-fooding the storage engine
- Domain restriction configurable via admin panel (seeded from CLI on first boot only)
- MCP server as 1:1 mirror of existing API surface
- Two-tier admin hierarchy: top-admin + admins

### Out of Scope (Future)

- Additional auth providers (email+password, generic OIDC)
- Moderator role with admin-grantable scoped access
- Group-based sharing (Google Workspace groups)
- Shareable links (anyone-with-link access)

---

## 2. Path Validator Extension

The current `validate_segment` (`src/path.rs:78`) rejects `@` in path segments. Email-based namespace roots require `@` to be valid.

### Change

Extend the character set for non-first characters to include `@`:

```
Current: [_A-Za-z0-9.-]
New:     [_A-Za-z0-9.@-]
```

First-character rule unchanged: `[_A-Za-z0-9-]` (no `@` or `.` as first char).

This makes `alice@company.com` a valid single path segment. The full path `alice@company.com/projects/main` parses as three segments: `["alice@company.com", "projects", "main"]`.

### Rationale

Alternatives considered:
- **URL-encode `@` as `%40`:** Introduces encoding/decoding throughout the stack, confuses disk paths.
- **Replace with separator (e.g., `alice--company.com`):** Lossy, collision risk, ugly.
- **Extending the validator** is simplest: one-line change, no encoding layer, readable on disk and in UI.

### Impact

Existing exoms use `[_A-Za-z0-9.-]` only — no breakage. The `@` character is unused in current trees. Disk paths with `@` are valid on all target filesystems (macOS HFS+/APFS, Linux ext4/btrfs).

---

## 3. Auth Provider Abstraction

### Provider Trait

```rust
trait AuthProvider: Send + Sync {
    /// Validate an external token and return user identity
    async fn validate_token(&self, token: &str) -> Result<AuthIdentity>;

    /// Provider name for storage/config ("google", "password", etc.)
    fn provider_name(&self) -> &str;
}

struct AuthIdentity {
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    provider: String,
}
```

The provider trait does NOT own allowed-domain policy. Domain restriction is a cross-cutting concern managed by the auth module and stored in `_system/auth` (see Section 8: Bootstrap & Domain Policy).

### Google Implementation

`GoogleAuthProvider` validates Google ID tokens (JWT):
- Verify JWT signature against Google's public JWKS keys (cached with TTL)
- Check `aud` matches configured client ID
- Check `hd` (hosted domain) is present and `email_verified` is true

Domain filtering happens in the auth module after provider validation, not inside the provider.

### Configuration

Daemon startup flags:
```
--auth-provider google
--google-client-id <GCP_CLIENT_ID>
```

Domain restriction is configured separately (see Section 8).

---

## 4. Identity Resolution & Middleware

### Two Auth Paths

**Browser path:** Google sign-in → backend receives ID token → validates via provider → creates session → sets session cookie (see Section 7 for cookie hardening).

**API/MCP path:** `Authorization: Bearer <key>` header → lookup API key hash in system exom → resolve to user.

### User Type

```rust
struct User {
    email: String,
    display_name: String,
    provider: String,
    session_id: Option<String>,
    role: UserRole,  // Regular | Admin | TopAdmin
}

enum UserRole {
    Regular,
    Admin,
    TopAdmin,
}
```

Implemented as Axum `FromRequestParts` extractor. Every protected handler receives typed `User`.

`MaybeUser(Option<User>)` variant for routes that behave differently for authenticated vs unauthenticated requests.

### Actor Identity Mapping

**Critical change:** Authenticated `User.email` becomes the canonical actor for all mutations. Client-supplied `actor` (body field) and `X-Actor` header are ignored on authenticated requests.

Current code trusts `body.actor` / `X-Actor` directly (`server.rs:674`, `server.rs:700`, `server.rs:736`, `server.rs:870`, `server.rs:1208`). After auth lands:

```rust
// MutationContext is always built from the authenticated User
fn mutation_context_from_user(user: &User) -> MutationContext {
    MutationContext {
        actor: user.email.clone(),
        session: user.session_id.clone(),
        model: None,  // X-Model header still accepted (advisory, non-security)
    }
}
```

- `X-Model` header remains advisory (useful for agents to tag which LLM model is calling).
- `X-Actor` and `body.actor` are removed from all handlers.
- Branch TOFU ownership (`Branch.claimed_by`) now uses `User.email`, preventing spoofing.

### Middleware Behavior

- `/auth/*` routes — no auth required (login endpoint lives here)
- `/ray-exomem/api/*` routes — `User` extractor, 401 if missing
- Static UI assets (`/`) — no auth (SPA loads, handles login client-side)
- `/sse` — auth required (Bearer or cookie)
- `/mcp` — auth required (Bearer token)

### In-Memory Cache

`DashMap<String, User>` for both sessions and API keys. Invalidated on revocation — write to system exom triggers cache eviction via internal event.

---

## 5. User Model & API Keys

### User Registration

Automatic on first successful login. System exom stores:

```rayfall
(user <email> <display-name> <provider> <created-at>)
(user-avatar <email> <avatar-url>)
(user-status <email> active)
(user-last-login <email> <timestamp>)
```

User namespace `<email>/` created automatically as root folder in tree.

### API Keys

Generated from profile page. Each key gets a stable UUID (`key-id`). Stored in system exom:

```rayfall
(api-key <key-id> <key-hash> <email> <label> <created-at>)
```

Key = random 32-byte token, base64url encoded. Only SHA-256 hash stored. Raw key shown once at creation, never retrievable. `key-id` is a UUID used for listing and revocation (no prefix-matching).

### Profile API

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/auth/me` | GET | User | Current user profile |
| `/auth/api-keys` | POST | User | Generate new key; returns raw key + MCP config |
| `/auth/api-keys` | GET | User | List keys (key-id, label, created-at) |
| `/auth/api-keys/<key-id>` | DELETE | User | Revoke key by stable UUID, evict from cache |

### MCP Config Snippet

Copyable from profile page:
```json
{
  "mcpServers": {
    "ray-exomem": {
      "url": "http://localhost:9780/mcp",
      "headers": {
        "Authorization": "Bearer <raw-key>"
      }
    }
  }
}
```

---

## 6. Namespaces, Visibility & Access Control

### Namespace Structure

```
tree/
├── alice@company.com/
│   ├── projects/
│   │   └── main              (exom, private by default)
│   └── scratch/
│       └── experiment        (exom)
├── bob@company.com/
│   └── research/
│       └── ml-notes          (exom)
└── _system/
    └── auth                  (system exom, inaccessible via data API)
```

### Ownership

User owns everything under `<their-email>/`. Full CRUD: create exoms/folders, assert, retract, branch, share.

### Access Resolution

```
resolve_access(user, path) → FullAccess | ReadWrite | ReadOnly | Denied

1. path starts with _system → Denied (always, regardless of role)
2. user is top-admin or admin → FullAccess
3. path starts with user.email → FullAccess (owner)
4. lookup share grants for (path, user.email) → ReadWrite | ReadOnly
5. check parent paths for inherited grants → ReadWrite | ReadOnly
6. → Denied
```

**`_system` is hard-denied at step 1, before any role check.** No user — including admins — can access `_system/` through the data API (`/ray-exomem/api/*`). Admin operations on auth state go exclusively through `/auth/*` and `/auth/admin/*` routes, which call internal auth module functions that do not pass through `resolve_access`.

### Grant Inheritance

Share on `alice@company.com/projects/` gives access to all exoms underneath. Deeper grant overrides shallower (e.g., read-write grant on subfolder overrides read-only on parent).

### Share Grants

Stored in system exom:
```rayfall
(share <share-id> <owner-email> <path> <grantee-email> <permission> <created-at>)
```

`<permission>` is `read` or `read-write`.

### Shares and Rename

When an exom or folder is renamed via `/actions/rename` (`server.rs:202`), all share grants whose `<path>` is a prefix match of the old path are updated atomically to reflect the new path. This happens inside the rename handler after the tree rename succeeds.

Example: renaming `alice@company.com/projects` to `alice@company.com/work` updates all shares matching `alice@company.com/projects` or `alice@company.com/projects/*` to use `alice@company.com/work` prefix.

### Sharing API

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/auth/shares` | POST | User (owner) | Create grant: `{path, grantee_email, permission}` |
| `/auth/shares` | GET | User (owner) | List grants on a path |
| `/auth/shares/<share-id>` | DELETE | User (owner) | Revoke grant |
| `/auth/shared-with-me` | GET | User | List paths shared to current user |

UI shows "Shared with me" section in sidebar alongside user's own tree.

---

## 7. OAuth Login Flow & Session Security

### Svelte UI Flow

1. SPA loads (no auth needed for static assets)
2. App checks `GET /auth/me` → 401 → show login screen
3. Login screen renders Google Sign-In button (GSI JS library)
4. User clicks → Google popup → consent → returns ID token to client
5. Client sends `POST /auth/login` with `{id_token, provider: "google"}`
6. Backend validates via `GoogleAuthProvider::validate_token()`
7. Auth module checks email domain against `(allowed-domain ...)` facts in `_system/auth`
8. On success: create/update user in `_system/auth`, create session, set session cookie
9. Return user profile → UI redirects to main app
10. On failure: return error → UI shows message

### Session Cookie Hardening

Session cookies use these attributes:

| Attribute | Value | Rationale |
|-----------|-------|-----------|
| `HttpOnly` | `true` | Prevent JS access to session token |
| `Secure` | `true` when not localhost | Require HTTPS in production |
| `SameSite` | `Lax` | Block cross-site POST requests (CSRF baseline) |
| `Path` | `/` | Cookie available to all routes |
| `Max-Age` | 30 days (configurable) | Session lifetime |

### CSRF Protection

State-changing requests (`POST`, `PUT`, `DELETE`) on cookie-authenticated routes check the `Origin` header:

1. If `Origin` header present: must match the daemon's own origin. Reject if mismatch.
2. If `Origin` absent and `Referer` present: `Referer` host must match. Reject if mismatch.
3. If neither present: reject (browsers always send `Origin` on cross-origin requests; absence with a cookie means a same-origin request from a non-browser client, which would use Bearer auth instead).

Bearer-token-authenticated requests skip CSRF checks (tokens are not auto-attached by browsers).

### Session Lifecycle

- `POST /auth/login` — create session
- `POST /auth/logout` — delete session, clear cookie, evict cache
- Sessions are long-lived (configurable, default 30 days)
- Google ID tokens used only at login to establish session; no refresh token dance

### Dependencies

- `jsonwebtoken` crate — JWT validation
- `reqwest` — fetch Google JWKS keys (cached with TTL)
- Google GSI JS library in Svelte UI

---

## 8. System Exom, Bootstrap & Domain Policy

### System Exom (`_system/auth`)

Special exom bootstrapped at daemon startup. Not visible in user tree. Only accessible by auth module internals — never through data API (hard-denied in `resolve_access` step 1).

### Fact Schema

```rayfall
;; Users
(user <email> <display-name> <provider> <created-at>)
(user-avatar <email> <avatar-url>)
(user-status <email> active|deactivated)
(user-last-login <email> <timestamp>)

;; Sessions
(session <session-id> <email> <created-at> <expires-at>)

;; API Keys
(api-key <key-id> <key-hash> <email> <label> <created-at>)

;; Share Grants
(share <share-id> <owner-email> <path> <grantee-email> <permission> <created-at>)

;; Admin
(top-admin <email>)
(admin <email>)

;; Domain Restrictions
(allowed-domain <domain>)
```

### Bootstrap Flow

1. Daemon starts → check if `_system/auth` exom exists in tree
2. If not → create with `ExomKind::Bare`
3. **First boot only:** if CLI flags `--allowed-domains` provided AND no `(allowed-domain ...)` facts exist yet, seed them into `_system/auth`
4. Load into memory, build auth cache from current facts
5. Auth middleware reads from cache; writes go through `brain.rs` mutation path

### Domain Policy: Single Source of Truth

After first-boot seeding, `_system/auth` is the sole authority for allowed domains. The CLI `--allowed-domains` flag is **ignored on subsequent boots** — it only seeds empty state. Runtime changes go through the admin panel (`POST /auth/admin/allowed-domains`).

The `AuthProvider` trait does NOT own domain policy. The auth module checks domains after provider validation:

```
login flow:
  1. provider.validate_token(id_token) → AuthIdentity
  2. auth_module.check_domain(identity.email) → Ok/Denied
  3. auth_module.create_or_update_user(identity) → User
```

### Top-Admin Bootstrap

- **First boot:** First user to successfully log in is auto-promoted to top-admin. `(top-admin <email>)` asserted in `_system/auth`.
- **Recovery:** If the top-admin account is lost (employee leaves, email changes), the daemon supports a `--bootstrap-admin <email>` CLI flag. This flag:
  - Only takes effect if no `(top-admin ...)` fact exists (original top-admin was explicitly removed by a prior recovery, or data was wiped)
  - If a `(top-admin ...)` already exists, the flag is ignored with a warning log
  - This prevents accidental takeover while providing a recovery path

### Cache Invalidation

When API key or session is retracted from system exom, internal event fires → auth cache listener evicts entry. Immediate invalidation.

---

## 9. Rayfall Body Authorization

### Problem

The current server resolves the target exom inside Rayfall bodies via `query`, `in-exom`, `assert-fact`, `retract-fact`, and `rule` forms (`server.rs:966`, `server.rs:997`). A simple `?exom=` param check is insufficient — the Rayfall body itself can reference arbitrary exoms.

### Authorization Pass

After lowering Rayfall forms and before execution, an authorization pass collects and checks all referenced exom paths:

```
authorize_rayfall(user, lowered_forms) → Ok | Denied(path, reason)

For each CanonicalForm in lowered_forms:
  1. Extract the target exom path (from Query.exom, AssertFact.exom, RetractFact.exom, Rule.exom)
  2. resolve_access(user, exom_path)
  3. For Query forms: require ReadOnly or higher
  4. For AssertFact/RetractFact/Rule forms: require ReadWrite or higher
  5. If any path is Denied → reject entire request with 403, identifying the denied path

Fail-closed: if an exom path cannot be resolved or is missing, deny.
```

### `_system` Hard-Reject

`_system/` paths are rejected at step 2 (`resolve_access` returns `Denied` for any `_system/` path). This applies regardless of how the path entered the request — query param, body field, or Rayfall form.

### `eval` Endpoint

The `POST /actions/eval` endpoint accepts mixed Rayfall (queries + mutations in one body). The authorization pass runs over ALL lowered forms before ANY are executed. If one form references a denied path, the entire eval is rejected.

### Default Exom Resolution

When a Rayfall form does not specify an exom explicitly, the lowering pass applies `default_query_exom` or `default_rule_exom`. The authorization pass sees the resolved (post-lowering) exom path, so defaults are checked too.

---

## 10. MCP Protocol Server

### Transport

Streamable HTTP (JSON-RPC over HTTP with SSE). Single endpoint: `POST /mcp`. Auth via Bearer token.

### Tool Surface

Full 1:1 mirror of existing API. All tools go through the same authorization pass (Section 9) as HTTP routes.

| MCP Tool | Maps to | Description |
|----------|---------|-------------|
| **Query & Explore** | | |
| `query` | `POST /ray-exomem/api/query` | Run Rayfall query |
| `expand_query` | `POST /ray-exomem/api/expand-query` | Debug query lowering |
| `explain` | `GET /ray-exomem/api/explain` | Explain predicate/fact |
| `list_exoms` | `GET /ray-exomem/api/tree` | List accessible exoms (filtered by user visibility) |
| `exom_status` | `GET /ray-exomem/api/status` | Exom health/stats |
| `schema` | `GET /ray-exomem/api/schema` | Exom ontology/schema |
| `graph` | `GET /ray-exomem/api/graph` | Relation graph |
| `relation_graph` | `GET /ray-exomem/api/relation-graph` | Full relation graph |
| **Facts** | | |
| `assert_fact` | `POST /ray-exomem/api/actions/assert-fact` | Assert/replace fact |
| `retract` | via eval | Retract by fact ID |
| `fact_history` | `GET /ray-exomem/api/facts/<id>` | Fact detail + history |
| `facts_list` | `GET /ray-exomem/api/facts` | List facts |
| `facts_valid_at` | `GET /ray-exomem/api/facts/valid-at` | Bitemporal point query |
| `facts_bitemporal` | `GET /ray-exomem/api/facts/bitemporal` | Bitemporal range query |
| `provenance` | `GET /ray-exomem/api/provenance` | Fact provenance chain |
| **Beliefs & Observations** | | |
| `beliefs` | `GET /ray-exomem/api/beliefs/<id>/support` | Belief support network |
| `clusters` | `GET /ray-exomem/api/clusters` | Fact clusters |
| `cluster_detail` | `GET /ray-exomem/api/clusters/<id>` | Cluster detail |
| `derived` | `GET /ray-exomem/api/derived/<pred>` | Derived facts by predicate |
| **Rules** | | |
| `eval` | `POST /ray-exomem/api/actions/eval` | Execute raw Rayfall (rules, retractions, etc.) |
| **Branches** | | |
| `list_branches` | `GET /ray-exomem/api/branches` | List branches |
| `branch_detail` | `GET /ray-exomem/api/branches/<id>` | Branch detail |
| `create_branch` | `POST /ray-exomem/api/actions/branch-create` | Create branch |
| `delete_branch` | `DELETE /ray-exomem/api/branches/<id>` | Delete branch |
| `switch_branch` | `POST /ray-exomem/api/branches/<id>/switch` | Switch branch |
| `diff_branch` | `GET /ray-exomem/api/branches/<id>/diff` | Branch diff |
| `merge_branch` | `POST /ray-exomem/api/branches/<id>/merge` | Merge branch |
| **Sessions** | | |
| `start_session` | `POST /ray-exomem/api/actions/session-new` | Start session |
| `join_session` | `POST /ray-exomem/api/actions/session-join` | Join session |
| **Lifecycle** | | |
| `create_exom` | `POST /ray-exomem/api/actions/exom-new` | Create new exom |
| `rename` | `POST /ray-exomem/api/actions/rename` | Rename exom or folder |
| `export` | `GET /ray-exomem/api/actions/export` | Export exom (Rayfall format) |
| `export_json` | `GET /ray-exomem/api/actions/export-json` | Export exom (JSON format) |
| `import_json` | `POST /ray-exomem/api/actions/import-json` | Import exom (JSON format) |
| `logs` | `GET /ray-exomem/api/logs` | Transaction logs |
| **Dangerous** | | |
| `retract_all` | `POST /ray-exomem/api/actions/retract-all` | Retract all facts |
| `wipe` | `POST /ray-exomem/api/actions/wipe` | Wipe exom |
| `factory_reset` | `POST /ray-exomem/api/actions/factory-reset` | Factory reset |

### Access Control

Same rules as HTTP API. MCP tool calls go through identity resolution (Bearer token → User) then Rayfall body authorization (Section 9) for any tool that accepts Rayfall input.

### Implementation

`rmcp` crate or hand-rolled JSON-RPC router. Thin translation layer — each tool handler calls existing API logic. No resources or prompts in v1, tools only.

---

## 11. Admin Panel

### Role Hierarchy

- **Top-admin:** First user to login (see Section 8 bootstrap). Can create/delete admins. Cannot be demoted except via recovery flow. Exactly one.
- **Admin:** Can manage users, sessions, keys, shares, domains. Cannot manage other admins.

```rayfall
(top-admin <email>)
(admin <email>)
```

### Admin API

| Endpoint | Method | Requires | Description |
|----------|--------|----------|-------------|
| `/auth/admin/users` | GET | admin | List all users |
| `/auth/admin/users/<email>` | DELETE | admin | Deactivate user (revoke sessions + keys, block login) |
| `/auth/admin/users/<email>/activate` | POST | admin | Reactivate user |
| `/auth/admin/admins` | POST | top-admin | Grant admin role |
| `/auth/admin/admins/<email>` | DELETE | top-admin | Revoke admin role |
| `/auth/admin/sessions` | GET | admin | List all active sessions |
| `/auth/admin/sessions/<id>` | DELETE | admin | Force-kill session |
| `/auth/admin/api-keys` | GET | admin | List all API keys (no raw values) |
| `/auth/admin/api-keys/<key-id>` | DELETE | admin | Revoke any user's key by stable UUID |
| `/auth/admin/shares` | GET | admin | List all share grants |
| `/auth/admin/allowed-domains` | POST | admin | Add allowed email domain |
| `/auth/admin/allowed-domains/<domain>` | DELETE | admin | Remove allowed domain |

### Deactivation Behavior

- User's data preserved (namespaces, exoms intact)
- Access blocked — middleware checks `user-status`
- Shares from deactivated user stay active (grantees keep access)
- All sessions and API keys revoked immediately
- Reactivation restores login ability

### UI

Admin section in Svelte UI — table views for users, sessions, keys, shares, domains. Only visible to admin/top-admin users. Simple CRUD, no complex dashboards.

---

## 12. Route Protection Summary

Aligned with current `server.rs` router structure:

```
/ (static SPA assets — no auth, spa_fallback)
│
├── /auth/                          — auth management routes
│   ├── POST /login                 — no auth (this IS the login)
│   ├── POST /logout                — requires User
│   ├── GET  /me                    — requires User
│   ├── POST /api-keys              — requires User
│   ├── GET  /api-keys              — requires User
│   ├── DELETE /api-keys/:key-id    — requires User
│   ├── POST /shares                — requires User (owner check in handler)
│   ├── GET  /shares                — requires User
│   ├── DELETE /shares/:share-id    — requires User (owner check in handler)
│   ├── GET  /shared-with-me        — requires User
│   └── /admin/*                    — requires admin (top-admin for /admins routes)
│
├── /ray-exomem/api/                — ALL require User + access check
│   ├── GET  /status                — user-scoped (only show accessible exoms in stats)
│   ├── GET  /tree                  — filtered by visibility
│   ├── GET  /guide                 — user-scoped
│   ├── POST /actions/init          — access check (owner or write)
│   ├── POST /actions/exom-new      — access check (owner of parent path)
│   ├── POST /actions/session-new   — access check + actor from User
│   ├── POST /actions/session-join  — access check + actor from User
│   ├── POST /actions/branch-create — access check (write) + actor from User
│   ├── POST /actions/rename        — access check (owner) + update share paths
│   ├── POST /actions/assert-fact   — Rayfall body authz (write) + actor from User
│   ├── GET|POST /query             — Rayfall body authz (read)
│   ├── POST /expand-query          — Rayfall body authz (read)
│   ├── POST /actions/eval          — Rayfall body authz (mixed read/write per form)
│   ├── GET  /facts                 — access check (read)
│   ├── GET  /facts/valid-at        — access check (read)
│   ├── GET  /facts/bitemporal      — access check (read)
│   ├── GET  /facts/:id             — access check (read)
│   ├── GET  /branches              — access check (read)
│   ├── GET  /branches/:id          — access check (read)
│   ├── DELETE /branches/:id        — access check (write)
│   ├── POST /branches/:id/switch   — access check (write)
│   ├── GET  /branches/:id/diff     — access check (read)
│   ├── POST /branches/:id/merge    — access check (write)
│   ├── GET  /explain               — access check (read)
│   ├── GET  /actions/export        — access check (read)
│   ├── GET  /actions/export-json   — access check (read)
│   ├── POST /actions/import-json   — access check (write)
│   ├── POST /actions/retract-all   — access check (owner)
│   ├── POST /actions/wipe          — access check (owner)
│   ├── POST /actions/factory-reset — access check (owner)
│   ├── POST /actions/consolidate-propose — access check (write)
│   ├── GET  /schema                — access check (read)
│   ├── GET  /graph                 — access check (read)
│   ├── GET  /clusters              — access check (read)
│   ├── GET  /clusters/:id          — access check (read)
│   ├── GET  /logs                  — access check (read)
│   ├── GET  /provenance            — access check (read)
│   ├── GET  /relation-graph        — access check (read)
│   ├── GET  /derived/:pred         — access check (read)
│   └── GET  /beliefs/:id/support   — access check (read)
│
├── /sse                            — requires User (events filtered to accessible exoms)
├── /api/status                     — compat shim, requires User
└── /mcp                            — requires Bearer token + Rayfall body authz
```

Mutations require `ReadWrite` or `FullAccess`. Queries require at least `ReadOnly`. Destructive operations (`retract-all`, `wipe`, `factory-reset`) require `FullAccess` (owner only). SSE events filtered to accessible exoms only.

---

## 13. New Dependencies

### Rust (Cargo.toml)

- `jsonwebtoken` — JWT validation for Google ID tokens
- `reqwest` — fetch Google JWKS keys
- `dashmap` — concurrent auth cache
- `rmcp` or equivalent — MCP protocol server (evaluate at implementation time)
- `rand` — API key generation
- `sha2` — API key hashing
- `base64` — key encoding
- `uuid` — stable key-id and share-id generation

### Svelte UI (package.json)

- Google Identity Services JS library (loaded via script tag or npm package)

---

## 14. Future Extension Points

Designed to accommodate without architectural changes:

- **Additional auth providers:** New `AuthProvider` trait implementations. Login UI shows provider picker. Domain policy remains in auth module, not per-provider.
- **Moderator role:** New `(moderator <email>)` fact + admin-grantable scoped access via `(moderator-grant <email> <path> <permission>)`. Access check adds step between admin check and owner check in `resolve_access`.
- **Group sharing:** `(share-group <share-id> <owner> <path> <group-id> <permission> <created-at>)` — resolve group membership via Google Groups API or local group definitions.
- **Shareable links:** `(share-link <link-id> <path> <permission> <created-at>)` — access check adds link-token lookup path.
