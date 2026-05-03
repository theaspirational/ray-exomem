# ray-exomem

> **WIP / unstable:** this project is under active development. The HTTP API,
> CLI behavior, MCP tools, storage layout, auth flows, and UI surfaces can
> change unexpectedly until the project declares a stable compatibility
> contract. Treat this README as the current working state, not a promise of
> backward compatibility.

Persistent external memory for LLM agents and operators.

`ray-exomem` is a Rust daemon, CLI, HTTP API, MCP endpoint, and embedded
Svelte UI for storing and querying agent memory. The storage model is a tree of
folders and exoms; each exom is an isolated knowledge base with facts, rules,
branches, observations, beliefs, and transactions.

Rayfall parsing, Datalog evaluation, symbol interning, and columnar persistence
come from [rayforce2](https://github.com/RayforceDB/rayforce2). This crate adds
the daemon, tree/session workflow, auth/access layer, API, UI, and agent-facing
commands.

## Current Model

The active model is tree-first:

- Folders are grouping nodes. They do not store facts.
- Exoms are leaf knowledge bases. A directory is an exom when it contains
`exom.json`.
- Projects are scaffolded folders with a `main` exom and a `sessions/` folder.
- Session exoms live under `<project>/sessions/<session-id>`.
- Branches live inside one exom. They are used for parallel work, hypothetical
changes, and multi-agent coordination.

Fresh persistent state starts empty — `tree/` has no auto-created exoms.
Create exoms explicitly with `init` or `exom-new`. With auth enabled, the
first authenticated login also seeds the `public/*` exoms declared by
`bootstrap/*.json` fixtures (see [Bootstrap Seeds](#bootstrap-seeds)). The
daemon does **not** auto-create a `{email}/main` user-namespace exom; users
initialise their own private namespace by calling `init` or `exom-new` against
their own `{email}/...` path.

### Permissions and `acl_mode`

Each exom carries an `acl_mode` (`solo-edit` (default) or `co-edit`) that
combines with the namespace to produce the write policy:

- `public/...` solo-edit → Model A: read-for-all, write-for-creator. Forking
  is the contribution path.
- `public/...` co-edit → Wikipedia-style commons: any authenticated user
  in the allowed domain can write to the shared `main` trunk.
- `{email}/...` solo-edit → owner-only writes; sharing reads requires an
  explicit share grant.
- `{email}/...` co-edit → owner + grantees with a `read-write` share can
  write to the shared `main` trunk (co-edit only short-circuits TOFU on
  `main`; the auth layer still enforces the share's `read` vs `read-write`
  permission, and non-`main` branches keep first-writer-wins).

Auth and branch-claim (TOFU) are independent decisions: co-edit short-circuits
TOFU **only on `main`**; non-`main` branches keep first-writer-wins. Sessions
are always solo-edit and rejected by `exom-mode` flips
(`acl_mode_not_applicable`). Forking always produces a solo-edit exom owned
by the forker, regardless of the source's mode.

`init` and `exom-new` accept an optional `acl_mode` field at creation time;
the creator can flip the mode later via `POST /api/actions/exom-mode`
(creator-only — others get 403 `not_creator`).

### Forks

`POST /api/actions/exom-fork { source, target? }` (and the matching MCP
`exom_fork` tool) copies the currently-active facts of `source` into a new
exom owned by the forker, with `forked_from = { source_path, source_tx_id,
forked_at }` stamped on the target's meta and every replayed fact attributed
to the forker. The default `target` is `{your_email}/forked/<source-subpath>`:

- `public/X/Y/Z` → `{your_email}/forked/X/Y/Z`
- `{other_email}/X/Y` → `{your_email}/forked/{other_email}/X/Y`

When the default is taken, the leaf segment is auto-suffixed (`-2`, `-3`, …
up to 100). Session exoms cannot be forked (`fork_session_unsupported`); fork
the parent project's `main` instead.

Path forms:

```text
CLI:       work::team::project::repo::main
Disk/API:  work/team/project/repo/main
```

Default data directory:

```text
~/.ray-exomem/
```

Representative layout after creating a project and a session:

```text
~/.ray-exomem/
  sym
  sym.lk
  tree/
    work/
      team/
        project/
          repo/
            main/
              exom.json
              fact/
              tx/
              branch/
            sessions/
              20260411T143215Z_multi_agent_landing-page/
                exom.json
```

Memory data (facts, transactions, observations, beliefs, or  
branches) is stored in rayforce2 splay tables under each exom directory.  
Auth state is separate: JSONL by default, or Postgres when configured.

## Build

Requirements:

- Rust toolchain
- Node.js and npm for the embedded Svelte build
- C compiler and `make`
- git

Build the release binary:

```bash
cargo build --release --bin ray-exomem
```

Use `ln -f`, not `cp`, when deploying the binary on macOS. Copied binaries can
inherit `com.apple.provenance` metadata and hang silently.

```bash
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
```

`build.rs` also builds the Svelte UI and rayforce2 C library. It uses
`RAYFORCE2_DIR` when set, otherwise it looks for a sibling `../rayforce2`, and
falls back to fetching rayforce2 master.

The server base path is baked at compile time.

Specify one if planing to host ray-exomem somewhere different than root of `your-site.com `  
for example  
`your-site.com/<RAY_EXOMEM_BASE_PATH>`

```bash
# Default: mount at /
cargo build --release --bin ray-exomem

# Mount UI/API under /ray-exomem/
RAY_EXOMEM_BASE_PATH=/ray-exomem cargo build --release --bin ray-exomem
```

The same base path is passed to SvelteKit so UI asset URLs match the daemon.

## Quick Start

Start a foreground server for debugging:

```bash
ray-exomem serve --bind 127.0.0.1:9780
```

Start a background daemon for normal unauthenticated local use:

```bash
ray-exomem daemon
```

Open the UI at the base path baked into the binary:

```text
http://127.0.0.1:9780/
http://127.0.0.1:9780/ray-exomem/   # if built with RAY_EXOMEM_BASE_PATH=/ray-exomem
```

Create a project and inspect the tree:

```bash
ray-exomem init work::team::project::repo
ray-exomem inspect work::team::project::repo --depth 3
```

Create a multi-agent session exom under that project. With auth enabled, the
orchestrator's identity comes from the bearer token; the orchestrator's
`agent`/`model` are recorded via headers, and `agents` pre-allocates one
unclaimed branch per sub-agent label:

```bash
curl -s -X POST http://127.0.0.1:9780/api/actions/session-new \
  -H "Authorization: Bearer $RAY_EXOMEM_KEY" \
  -H "x-agent: orchestrator-cli" \
  -H "x-model: claude-opus-4-7" \
  -H 'Content-Type: application/json' \
  -d '{
        "project_path": "work/team/project/repo",
        "type": "multi",
        "label": "landing-page",
        "agents": ["agent-a", "agent-b"]
      }'
```

Sub-agents claim their branches via `POST /api/actions/session-join` with
their own `x-agent`/`x-model` headers. See the [Attribution Model](#attribution-model)
section for the full contract.

Query the project main exom:

```bash
ray-exomem query \
  --exom work::team::project::repo::main \
  --request '(query work/team/project/repo/main (find ?fact ?pred ?value) (where (fact-row ?fact ?pred ?value)))' \
  --json
```

Stop the daemon:

```bash
ray-exomem stop
```

The CLI and direct HTTP calls use the same compiled base path. By default that
is root (`/`); set `RAY_EXOMEM_BASE_PATH` before building if the daemon is
mounted under a subpath.

## CLI Surface

Offline/local commands:

- `ray-exomem run <file>` evaluates a Rayfall file in-process with no shared KB.
- `ray-exomem init <path>` creates `<path>/main` plus `<path>/sessions/`.
- `ray-exomem exom-new <path>` creates a bare exom.
- `ray-exomem inspect [path]` reads the local tree from disk.
- `ray-exomem guide` prints the long agent/operator reference.

Daemon-backed commands:

- `status`, `facts`, `query`, `expand-query`, `eval`
- `assert`, `retract`, `history`, `why`, `why-not`
- `branch <list|create|diff|merge|delete>`
- `coord <claim|release|depend|agent-session|...>`
- `session <new|join|rename|close|archive>`
- `export`, `import`, `watch`, `lint-memory`, `doctor`

Prefer declaring all participants up front with `session new --agents ...`.
The `session add-agent` CLI path is still not a reliable automation surface.

Asserting a fact against an authenticated daemon — `user_email` comes from
the bearer token, `agent`/`model` from headers (see
[Attribution Model](#attribution-model)):

```bash
curl -s -X POST http://127.0.0.1:9780/api/actions/assert-fact \
  -H "Authorization: Bearer $RAY_EXOMEM_KEY" \
  -H "x-agent: claude-code-cli" \
  -H "x-model: claude-opus-4-7" \
  -H 'Content-Type: application/json' \
  -d '{
        "exom": "work/team/project/repo/main",
        "fact_id": "project/status",
        "predicate": "project/status",
        "value": "active",
        "source": "kickoff-notes"
      }'
```

Fact values are typed at the API/brain layer. JSON numbers become `I64`,
JSON strings stay `Str`, and `{"$sym": "..."}` lands as `Sym`. Typed facts populate `facts_i64`,
`facts_str`, and `facts_sym` EDBs for native Datalog rules.

## HTTP, UI, SSE, and MCP

All daemon routes are mounted under `server::BASE_PATH`, compiled from
`RAY_EXOMEM_BASE_PATH`. With the default build, `BASE_PATH` is empty:

```text
GET http://127.0.0.1:9780/api/status
GET http://127.0.0.1:9780/events
POST http://127.0.0.1:9780/mcp
POST http://127.0.0.1:9780/mcp/sse
```

With `RAY_EXOMEM_BASE_PATH=/ray-exomem`:

```text
GET http://127.0.0.1:9780/ray-exomem/api/status
GET http://127.0.0.1:9780/ray-exomem/events
POST http://127.0.0.1:9780/ray-exomem/mcp
POST http://127.0.0.1:9780/ray-exomem/mcp/sse
```

Canonical API routes:

- `GET /api/status`
- `GET /api/tree`
- `GET /api/welcome/summary`
- `GET /api/guide`
- `POST /api/actions/init` (accepts `acl_mode`)
- `POST /api/actions/exom-new` (accepts `acl_mode`)
- `POST /api/actions/exom-fork` (Model A contribution path; default target `{user_email}/forked/<source-subpath>`, auto-suffixed on collision)
- `POST /api/actions/exom-mode` (creator-only `solo-edit` ↔ `co-edit` flip; rejected on session exoms)
- `POST /api/actions/folder-new` (idempotent; rejects with `already_exists_different` if the path is an exom)
- `POST /api/actions/rename` (folders + exoms; rejects `namespace_root_immutable`, `session_id_immutable`)
- `POST /api/actions/delete` (recursive subtree delete; rejects `namespace_root_immutable`, `not_found`)
- `POST /api/actions/session-new`
- `POST /api/actions/session-join`
- `POST /api/actions/branch-create`
- `POST /api/actions/assert-fact`
- `POST /api/query` (accepts `?branch=<branch_id>` to evaluate against a specific branch view)
- `POST /api/expand-query`
- `POST /api/actions/eval`
- `GET /api/facts` (accepts `?branch=<branch_id>`)
- `GET /api/facts/valid-at`
- `GET /api/facts/bitemporal`
- `GET /api/facts/{id}`
- `GET /api/beliefs` (accepts `?branch=<branch_id>`; response carries the resolved `branch`)
- `GET /api/observations` (exom-level; ignores `?branch` since observations are not branch-scoped in the read API)
- `GET|POST /api/branches`
- `GET|DELETE /api/branches/{id}`
- `GET /api/branches/{id}/diff`
- `POST /api/branches/{id}/merge` (JSON: `target_branch`, `policy`)
- `GET /api/explain`
- `GET /api/schema` (accepts `?branch=<branch_id>`)
- `GET /api/graph`
- `GET /api/relation-graph` (accepts `?branch=<branch_id>`)
- `GET /api/clusters`
- `GET /api/clusters/{id}`
- `GET /api/provenance`
- `GET /api/logs`
- `GET /api/actions/export`
- `GET /api/actions/export-json`
- `POST /api/actions/import-json`
- `POST /api/actions/retract-all`
- `POST /api/actions/wipe`
- `POST /api/actions/factory-reset`
- `GET /api/derived/{pred}`
- `GET /api/beliefs/{id}/support`

The embedded UI is served by the same daemon and includes tree, exom, query,
graph, guide, login, profile, and admin surfaces. The sidebar groups exoms
into three sections in concentric-circles order: **PERSONAL** (`{email}/*`),
**SHARED** (explicit share grants from other users, displayed as
`{owner_email}/path/...`), and **PUBLIC** (`public/*`). Co-edit exoms get a
`co-edit` badge in the tree and a header strip in the exom view; the
creator sees a "Switch to {mode}" button there. Server-Sent Events stream
from `/events`. The MCP Streamable HTTP endpoint is `/mcp`; `/mcp/sse` is
an alias for MCP clients whose configuration examples use an `/sse`-suffixed
URL.

## Auth and Local Development

Unauthenticated single-user mode is the default when no auth provider is set.
When auth is enabled, `/auth/info` and `/auth/login` stay public, while `/api`,
`/mcp`, and `/events` require a session cookie or bearer API key.

Google-authenticated local dev uses `serve`, not `daemon`, because the auth
provider flags are currently wired on `serve`:

```bash
set -a
source .env
set +a

ray-exomem serve --bind 127.0.0.1:9780 \
  --auth-provider google \
  --google-client-id "$GOOGLE_CLIENT_ID" \
  --allowed-domains "$ALLOWED_DOMAINS" \
  --database-url "$DATABASE_URL"
```

Auth persistence:

- Without `--database-url`, auth state lives in `_system/auth/auth.jsonl`.
- With `--database-url`, users, sessions, API keys, domains, and shares use
Postgres.
- Exom memory data always lives in local rayforce2 splay tables.

Share grants on private `{email}/...` paths are managed via `POST /auth/shares`
(`{ path, grantee_email, permission: "read" | "read-write" }`). Shares are
per-email today (no group-based access yet); combined with `acl_mode: co-edit`
on the granted path, a `read-write` share lets the grantee write to the shared
`main` trunk without hitting branch-TOFU.

`ray-exomem daemon` currently does not expose the auth provider flags. Use
foreground `serve` for authenticated development and deployment until that is
wired through.

### Multi-user dev via loopback dev-login

For two-user (or N-user) local testing without juggling separate OAuth
sessions, pass `--dev-login-email` (repeatable, or comma-separated, or set
`RAY_EXOMEM_DEV_LOGIN_EMAIL`) on `serve`. The flag is **only honoured on
loopback binds** (`127.0.0.1`, `[::1]`) and **requires `--auth-provider`**
(the route mints a real session against the auth store). Each listed email
becomes a permitted identity:

```bash
ray-exomem serve --bind 127.0.0.1:9780 \
  --auth-provider google --google-client-id "$GOOGLE_CLIENT_ID" \
  --allowed-domains "$ALLOWED_DOMAINS" --database-url "$DATABASE_URL" \
  --dev-login-email user1@example.com \
  --dev-login-email user2@example.com
```

Hitting `GET /auth/dev-login` (no params) logs in as the first allow-listed
email; `GET /auth/dev-login?email=user2@example.com` logs in as the named
user. Non-allow-listed emails return 400 `email_not_allowed`. Combined with
Chrome's `isolatedContext` (or two browser profiles), this gives clean
cookie-isolated sessions for cross-user permission/sharing/co-edit testing
without running multiple daemons.

## Attribution Model

Every mutation (fact assert/retract, observation, belief, branch create,
session join/close) records three independent attribution axes on the
underlying transaction:

| Axis | Source | Notes |
|---|---|---|
| `user_email` | DB-bound, from auth | The authenticated user. Load-bearing for permission checks; not caller-controlled. `None` only for system-internal writes. |
| `agent` | `x-agent` header (HTTP) or `agent` arg (MCP) | The tool/integration making the call (e.g. `cursor`, `claude-code-cli`). Falls back to the API key's label for Bearer auth. Cookie-auth UI writes leave it `None` unless explicitly set. An explicit value always wins over the label. |
| `model` | `x-model` header (HTTP) or `model` arg (MCP) | The LLM identity (e.g. `claude-opus-4-7`). Explicit only — no fallback. |

Branch ownership is captured the same way at TOFU claim time:
`branch/claimed_by_user_email`, `branch/claimed_by_agent`,
`branch/claimed_by_model`. All three are queryable EAV attributes and surface
in `list_branches`.

The canonical `tx-row` Datalog view exposes the full triple:

```scheme
(tx-row ?tx ?id ?email ?agent ?model ?action ?when ?branch)
```

Empty strings stand in for `None` (system writes have `?email = ""`,
cookie-auth UI writes have `?agent = ""`, writes without a `model` arg have
`?model = ""`). Filter empties at query time with `(not (= ?v ""))` if needed.

UI render format is `by {user_email} via {agent} using {model}`, with
`via`/`using` elided when those axes are `None`.

**CLI caveat.** The `--actor` flag on legacy CLI subcommands (e.g.
`assert --actor`, `session new --actor`, `branch create --actor`) predates
the three-axis model and is CLI-only. Authenticated HTTP/MCP/UI writes do
**not** read `actor` from the request body — they use the authenticated user
plus the `x-agent`/`x-model` headers (or `agent`/`model` MCP args). Until CLI
auth lands, treat the CLI's `--actor` as a `cli`-tier identity hint, not a
substitute for the three axes.

**Multi-subagent contract.** When a single MCP client (e.g. one Claude Code
CLI process) hosts many subagents authenticated by one API key, the client
must inject `agent: "<subagent-name>"` on every tool call to disambiguate.
Without it, every subagent's writes appear under the API key's label. The
daemon cannot infer this — it's a contract on the orchestrator.

## Bootstrap Seeds

`bootstrap/*.json` files are embedded into the binary at build time. On login,
each seed scaffolds `<seed.path>/main` and seeds it once. Numeric JSON values
become typed `I64` facts, which is required for numeric Datalog rules using
`facts_i64`.

See `bootstrap/README.md` and `bootstrap/example.json` for the fixture schema.

## Development

Useful checks:

```bash
cargo test
cd ui && npm run check && npm run build
```

For server, storage, auth, backend, or rayforce2 FFI changes, unit tests are not
enough. Follow the live daemon rebuild/redeploy loop in `CLAUDE.md` and exercise
the change against the running daemon.

Useful source files:

- `src/main.rs` - CLI and daemon lifecycle
- `src/server.rs` - HTTP API, SSE, MCP mount, UI hosting
- `src/brain.rs` - core memory model and mutations
- `src/scaffold.rs` - project and exom creation
- `src/tree.rs` - tree walking and node classification
- `src/path.rs` - tree path parsing and validation
- `src/auth/routes.rs` - auth/login/bootstrap routes
- `src/storage.rs` - splay persistence and decoded query tables
- `src/rayfall_ast.rs` - Rayfall parsing/lowering helpers
- `src/system_schema.rs` - builtin views and schema output
- `ui/src/lib/exomem.svelte.ts` - UI API client

## Operational Notes

- The project is Rayfall-native. Legacy `.dl` / Teide inputs are rejected.
- If `rayforce2` changed and behavior looks stale, run
`cargo clean && cargo build --release --bin ray-exomem`.
- The embedded UI is built into the binary at compile time.
- `ray-exomem daemon` forks and redirects output. Use `serve` when you need logs
in the terminal.
- The symbol table is part of persistent identity. Do not wipe `~/.ray-exomem/sym`
reflexively; persisted splay rows encode symbol IDs by slot.
- Startup runs a sym rewrite compatibility pass and an engine health probe over
loaded exoms to surface rayforce2 symbol-layout problems early.
