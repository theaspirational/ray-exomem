# Auth, MCP & Admin Panel — Design Spec

**Date:** 2026-04-14
**Status:** Draft
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
- Domain restriction configurable via admin panel
- MCP server as 1:1 mirror of existing API surface
- Two-tier admin hierarchy: top-admin + admins

### Out of Scope (Future)

- Additional auth providers (email+password, generic OIDC)
- Moderator role with admin-grantable scoped access
- Group-based sharing (Google Workspace groups)
- Shareable links (anyone-with-link access)

---

## 2. Auth Provider Abstraction

### Provider Trait

```rust
trait AuthProvider: Send + Sync {
    /// Validate an external token and return user identity
    async fn validate_token(&self, token: &str) -> Result<AuthIdentity>;

    /// Provider name for storage/config ("google", "password", etc.)
    fn provider_name(&self) -> &str;

    /// Allowed email domains (None = any)
    fn allowed_domains(&self) -> Option<&[String]>;
}

struct AuthIdentity {
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    provider: String,
}
```

### Google Implementation

`GoogleAuthProvider` validates Google ID tokens (JWT):
- Verify JWT signature against Google's public JWKS keys (cached with TTL)
- Check `aud` matches configured client ID
- Check `hd` (hosted domain) against allowed domains
- Check `email_verified` is true

### Configuration

Daemon startup flags:
```
--auth-provider google
--google-client-id <GCP_CLIENT_ID>
--allowed-domains company.com
```

---

## 3. Identity Resolution & Middleware

### Two Auth Paths

**Browser path:** Google sign-in → backend receives ID token → validates via provider → creates session → sets `HttpOnly` cookie with session ID.

**API/MCP path:** `Authorization: Bearer <key>` header → lookup API key hash in system exom → resolve to user.

### User Type

```rust
struct User {
    email: String,
    display_name: String,
    provider: String,
    session_id: Option<String>,
}
```

Implemented as Axum `FromRequestParts` extractor. Every protected handler receives typed `User`.

`MaybeUser(Option<User>)` variant for routes that behave differently for authenticated vs unauthenticated requests.

### Middleware Behavior

- `/auth/*` routes — no auth required (login endpoint lives here)
- `/ray-exomem/api/*` routes — `User` extractor, 401 if missing
- Static UI assets (`/`) — no auth (SPA loads, handles login client-side)
- `/sse` — auth required (Bearer or cookie)
- `/mcp` — auth required (Bearer token)

### In-Memory Cache

`DashMap<String, User>` for both sessions and API keys. Invalidated on revocation — write to system exom triggers cache eviction via internal event.

---

## 4. User Model & API Keys

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

Generated from profile page. Stored in system exom:

```rayfall
(api-key <key-hash> <email> <label> <created-at>)
```

Key = random 32-byte token, base64url encoded. Only SHA-256 hash stored. Raw key shown once at creation, never retrievable.

### Profile API

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/auth/me` | GET | User | Current user profile |
| `/auth/api-keys` | POST | User | Generate new key; returns raw key + MCP config |
| `/auth/api-keys` | GET | User | List keys (label, created-at, prefix only) |
| `/auth/api-keys/<prefix>` | DELETE | User | Revoke key, evict from cache |

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

## 5. Namespaces, Visibility & Access Control

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
    └── auth                  (system exom, inaccessible to users)
```

### Ownership

User owns everything under `<their-email>/`. Full CRUD: create exoms/folders, assert, retract, branch, share.

### Access Resolution

```
resolve_access(user, path) → FullAccess | ReadWrite | ReadOnly | Denied

1. user is top-admin or admin → FullAccess
2. path starts with user.email → FullAccess (owner)
3. path starts with _system → Denied
4. lookup share grants for (path, user.email) → ReadWrite | ReadOnly
5. check parent paths for inherited grants → ReadWrite | ReadOnly
6. → Denied
```

### Grant Inheritance

Share on `alice@company.com/projects/` gives access to all exoms underneath. Deeper grant overrides shallower (e.g., read-write on subfolder overrides read-only on parent).

### Share Grants

Stored in system exom:
```rayfall
(share <share-id> <owner-email> <path> <grantee-email> <permission> <created-at>)
```

`<permission>` is `read` or `read-write`.

### Sharing API

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/auth/shares` | POST | User (owner) | Create grant: `{path, grantee_email, permission}` |
| `/auth/shares` | GET | User (owner) | List grants on a path |
| `/auth/shares/<share-id>` | DELETE | User (owner) | Revoke grant |
| `/auth/shared-with-me` | GET | User | List paths shared to current user |

UI shows "Shared with me" section in sidebar alongside user's own tree.

---

## 6. OAuth Login Flow

### Svelte UI Flow

1. SPA loads (no auth needed for static assets)
2. App checks `GET /auth/me` → 401 → show login screen
3. Login screen renders Google Sign-In button (GSI JS library)
4. User clicks → Google popup → consent → returns ID token to client
5. Client sends `POST /auth/login` with `{id_token, provider: "google"}`
6. Backend validates via `GoogleAuthProvider::validate_token()`
7. On success: create/update user in `_system/auth`, create session, set `HttpOnly` cookie
8. Return user profile → UI redirects to main app
9. On failure: return error → UI shows message

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

## 7. System Exom (`_system/auth`)

### Bootstrap

1. Daemon starts → check if `_system/auth` exom exists
2. If not → create with `ExomKind::Bare`
3. Load into memory, build auth cache from current facts
4. Auth middleware reads from cache; writes go through `brain.rs` mutation path

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
(api-key <key-hash> <email> <label> <created-at>)

;; Share Grants
(share <share-id> <owner-email> <path> <grantee-email> <permission> <created-at>)

;; Admin
(top-admin <email>)
(admin <email>)

;; Domain Restrictions
(allowed-domain <domain>)
```

### Cache Invalidation

When API key or session is retracted from system exom, internal event fires → auth cache listener evicts entry. Immediate invalidation.

### Access Restriction

`_system/` prefix hard-blocked in access check. No user-facing query access. Admin operations only through `/auth/*` and `/auth/admin/*` routes.

---

## 8. MCP Protocol Server

### Transport

Streamable HTTP (JSON-RPC over HTTP with SSE). Single endpoint: `POST /mcp`. Auth via Bearer token.

### Tool Surface

Full 1:1 mirror of existing API:

| MCP Tool | Maps to | Description |
|----------|---------|-------------|
| **Query & Explore** | | |
| `query` | `POST /api/query` | Run Rayfall query |
| `expand_query` | `POST /api/expand-query` | Debug query lowering |
| `explain` | `GET /api/explain` | Explain predicate/fact |
| `list_exoms` | `GET /api/tree` | List accessible exoms |
| `exom_status` | `GET /api/status` | Exom health/stats |
| `schema` | `GET /api/schema` | Exom ontology/schema |
| `graph` | `GET /api/graph` | Relation graph |
| **Facts** | | |
| `assert_fact` | `POST /api/actions/assert-fact` | Assert/replace fact |
| `retract` | via eval | Retract by fact ID |
| `fact_history` | `GET /api/facts/<id>` | Fact detail + history |
| `facts_valid_at` | `GET /api/facts/valid-at` | Bitemporal point query |
| `facts_bitemporal` | `GET /api/facts/bitemporal` | Bitemporal range query |
| `provenance` | `GET /api/provenance` | Fact provenance chain |
| **Beliefs & Observations** | | |
| `beliefs` | `GET /api/beliefs/<id>/support` | Belief support network |
| `clusters` | `GET /api/clusters` | Fact clusters |
| `derived` | `GET /api/derived/<pred>` | Derived facts by predicate |
| **Rules** | | |
| `eval` | `POST /api/actions/eval` | Execute raw Rayfall |
| **Branches** | | |
| `list_branches` | `GET /api/branches` | List branches |
| `create_branch` | `POST /api/actions/branch-create` | Create branch |
| `switch_branch` | `POST /api/branches/<id>/switch` | Switch branch |
| `diff_branch` | `GET /api/branches/<id>/diff` | Branch diff |
| `merge_branch` | `POST /api/branches/<id>/merge` | Merge branch |
| **Sessions** | | |
| `start_session` | `POST /api/actions/session-new` | Start session |
| `join_session` | `POST /api/actions/session-join` | Join session |
| **Lifecycle** | | |
| `create_exom` | `POST /api/actions/exom-new` | Create new exom |
| `export` | `POST /api/actions/export` | Export exom data |
| `import` | `POST /api/actions/import` | Import exom data |
| `logs` | `GET /api/logs` | Transaction logs |

### Access Control

Same rules as HTTP API. Each tool call scoped to user's accessible exoms via Bearer token identity.

### Implementation

`rmcp` crate or hand-rolled JSON-RPC router. Thin translation layer — each tool handler calls existing API logic. No resources or prompts in v1, tools only.

---

## 9. Admin Panel

### Role Hierarchy

- **Top-admin:** First user to login. Can create/delete admins. Cannot be demoted. Exactly one.
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
| `/auth/admin/api-keys/<prefix>` | DELETE | admin | Revoke any user's key |
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

## 10. Route Protection Summary

```
/ (static SPA assets — no auth)
├── /auth/
│   ├── POST /login              — no auth
│   ├── POST /logout             — requires User
│   ├── GET  /me                 — requires User
│   ├── POST /api-keys           — requires User
│   ├── GET  /api-keys           — requires User
│   ├── DELETE /api-keys/:id     — requires User
│   ├── POST /shares             — requires User (owner check)
│   ├── GET  /shares             — requires User
│   ├── DELETE /shares/:id       — requires User (owner check)
│   ├── GET  /shared-with-me     — requires User
│   └── /admin/*                 — requires admin (top-admin for admin mgmt)
│
├── /ray-exomem/api/             — ALL require User + access check
│   ├── /status                  — user-scoped
│   ├── /tree                    — filtered by visibility
│   ├── /query                   — access check on target exom
│   ├── /actions/*               — read-write required for mutations
│   └── ... (all routes)
│
├── /sse                         — requires User (scoped events)
└── /mcp                         — requires Bearer token
```

Mutations require `ReadWrite` or `FullAccess`. Queries require at least `ReadOnly`. SSE events filtered to accessible exoms only.

---

## 11. New Dependencies

### Rust (Cargo.toml)

- `jsonwebtoken` — JWT validation for Google ID tokens
- `reqwest` — fetch Google JWKS keys
- `dashmap` — concurrent auth cache
- `rmcp` or equivalent — MCP protocol server (evaluate at implementation time)
- `rand` — API key generation
- `sha2` — API key hashing
- `base64` — key encoding

### Svelte UI (package.json)

- Google Identity Services JS library (loaded via script tag or npm package)

---

## 12. Future Extension Points

Designed to accommodate without architectural changes:

- **Additional auth providers:** New `AuthProvider` trait implementations. Login UI shows provider picker.
- **Moderator role:** New `(moderator <email>)` fact + admin-grantable scoped access via `(moderator-grant <email> <path> <permission>)`. Access check adds step between admin check and owner check.
- **Group sharing:** `(share-group <share-id> <owner> <path> <group-id> <permission> <created-at>)` — resolve group membership via Google Groups API or local group definitions.
- **Shareable links:** `(share-link <link-id> <path> <permission> <created-at>)` — access check adds link-token lookup path.
