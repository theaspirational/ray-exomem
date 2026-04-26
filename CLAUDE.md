# CLAUDE.md — ray-exomem

Project Philosophy (Greenfield Bias)

This project should be treated as a greenfield system, regardless of its current state. Do not assume the need for backward compatibility, legacy constraints, or incremental patching unless explicitly stated.

Prioritize correctness, simplicity, and architectural integrity over stability of existing patterns. Prefer clean, well-reasoned designs even if they require refactoring or replacing existing components partially or entirely.

Avoid introducing fallbacks, shims, or compatibility layers as a default strategy. These should only be considered when explicitly required. Instead, focus on identifying the most coherent and maintainable solution from first principles.


The role of this file is to describe common mistakes and confusion points that agents might encounter as they work in this project. If you ever encounter something in the project that surprises you, please alert the developer working with you and indicate that this is the case in the CLAUDE.md file to help prevent future agents from having the same issue.

When creating any artifacts during chat session, save them in archive/<date>_<session_name>/<artifact_name>.md

When adding or modifying any db/rayfall interactions test them against the running ray-exomem daemon.

### Live-test loop (mandatory for db/rayfall/server changes)

Unit tests (`cargo test`) are not a substitute for this — the bug classes that matter here (engine error surfacing, EDB auto-register, sym-table load, typed-fact binding) only show up against a live daemon. After making changes touching `src/server.rs`, `src/brain.rs`, `src/storage.rs`, `src/backend.rs`, `src/auth/**`, or rayforce2 FFI, always:

1. **Rebuild** — `cargo build --release --bin ray-exomem` (not just `cargo test`; `cargo test --lib --release` compiles a test binary, not the daemon binary).
2. **Kill the old daemon** — `pgrep -lf "ray-exomem serve" | awk '{print $1}' | xargs -r kill`. Note: `ray-exomem stop` only finds daemons started via `ray-exomem daemon`; the dev-workflow `serve` invocation needs `kill` by PID.
3. **Redeploy** — `ln -f target/release/ray-exomem ~/.local/bin/ray-exomem` (must be `ln -f`, not `cp` — see the macOS `com.apple.provenance` gotcha below).
4. **Relaunch with env** — `set -a; source .env; set +a; nohup ~/.local/bin/ray-exomem serve --bind 127.0.0.1:9780 --auth-provider google --google-client-id "$GOOGLE_CLIENT_ID" --allowed-domains "$ALLOWED_DOMAINS" --database-url "$DATABASE_URL" > /tmp/ray-exomem.log 2>&1 &`.
5. **Verify liveness** — `curl -s http://127.0.0.1:9780/ray-exomem/api/status` should return `ok: true`.
6. **Exercise the change** — hit the specific endpoint/query the change affects via `curl`, confirm the expected HTTP status and error shape. Logs tail at `/tmp/ray-exomem.log`.

The `server.build.identity` in `/api/status` is cached across rebuilds when `HEAD` hasn't moved; rely on binary mtime / size for "did the new code ship", not on the build-identity string.

## What this is

`ray-exomem` is a persistent memory daemon for LLM agents.

- Storage model: tree of folders and exoms; facts, observations, beliefs, transactions, branches
- Query model: Rayfall / Datalog
- Runtime: Rust daemon + rayforce2 via FFI + embedded Svelte UI

## Build and run

```bash
cargo build --release
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem

ray-exomem daemon
ray-exomem stop
```

Use `ray-exomem serve --bind 127.0.0.1:9780` for foreground debugging.

## Local dev with Cloudflare tunnel

Developer runs local daemon + Cloudflare tunnel, reached at **https://devmem.trydev.app/ray-exomem/**. Assume this is the live test surface, not a bare `localhost`.

Components:

- **Postgres**: container `ddd-postgres-1` (`postgres:17-alpine`, port 5432). ray-exomem uses role `ray_exomem` / db `ray_exomem`. Superuser for admin: `rapidcrm`.
- **`.env` at repo root** (gitignored) holds `GOOGLE_CLIENT_ID`, `ALLOWED_DOMAINS`, `DATABASE_URL`. No dotenv loader in Rust — export with `set -a; source .env; set +a` before launching.
- **Build**: Postgres backend is feature-gated → `cargo build --release --features postgres`.
- **Daemon** (foreground, backgrounded via shell):
  ```bash
  set -a; source .env; set +a
  ray-exomem serve --bind 127.0.0.1:9780 \
    --auth-provider google --google-client-id "$GOOGLE_CLIENT_ID" \
    --allowed-domains "$ALLOWED_DOMAINS" --database-url "$DATABASE_URL"
  ```
  Note: the `daemon` subcommand lacks the auth flags, so local dev uses `serve`.
- **Cloudflare tunnel**: named `devmem` (id `1bc8be11-197d-4f57-8115-4af051fa626a`), config at `~/.cloudflared/devmem.yml`, ingress → `http://localhost:9780`. Run with `cloudflared --config ~/.cloudflared/devmem.yml tunnel run devmem`. The older `ridtech` tunnel is unrelated.
- **Google OAuth console**: `https://devmem.trydev.app` must be listed under *Authorized JavaScript origins* for the client ID. GSI id_token flow → no redirect URI needed.

Gotchas specific to this setup:

- `cloudflared tunnel route dns <name> <host>` can silently target whichever tunnel appears first in `~/.cloudflared/config.yml`. Always pass `--config ~/.cloudflared/devmem.yml` (and `--overwrite-dns` when re-routing) so the CNAME points at the intended tunnel.
- Fresh persistent state auto-creates a bare `main` exom on startup, so `/api/status` without `?exom=` should succeed. Authenticated login may additionally provision user-scoped exoms such as `{email}/main`.
- When changing auth/postgres flags, fully stop the daemon (`ray-exomem stop`) before restarting — the new `serve` invocation will not take over a daemonised instance bound to the same port.

## Important gotchas

- Use `ln -f`, not `cp`, when deploying the binary on macOS. `com.apple.provenance` can make copied binaries hang silently.
- If `rayforce2` changed, run `cargo clean && cargo build --release` or Cargo may keep the old static library linked.
- The Svelte 5 UI is embedded in the binary at build time.
- `ray-exomem daemon` forks. Use `serve` if you want logs in the terminal.
- The repo is mid-migration from the old flat-exom flow to the tree/session model. Prefer `ray-exomem inspect`, `init`, `exom-new`, `session ...`, and `GET /api/tree`. `/api/exoms` and `POST /api/actions/start-session` are removed, and the legacy `start-session` / `exoms` CLI helpers should not be treated as the primary path.
- In authenticated UI mode, mutation actor attribution should fall back to the logged-in email. Do not require a separate `ray-exomem-actor` localStorage value for basic writes.
- JSONL auth replay must preserve `user.active` / `last_login` on repeated `user` entries. A naive replay that resets them on login makes deactivation appear to succeed in the UI while leaving the account effectively active.
- The bootstrap health rules (`src/auth/routes.rs::health_bootstrap_rules`) use `<`/`>=`/`not` cmp bodies, constant-string rule heads (`(rule ... (health/water-band "small") ...)`), and body atoms against the typed `facts_i64` EDB. These require rayforce2 master ≥ `dda2b98` (PR #7 merged: head-const projection + auto-registered env-bound EDBs). If the sibling `../rayforce2` checkout is on a commit older than `dda2b98`, bootstrap rule registration fails with unstratifiable-negation / missing-relation errors.
- The runtime uses `ray_runtime_create_with_sym_err` so persisted user symbol IDs keep their slots across binary upgrades — builtins are appended afterwards, not interleaved. This is the correct design. **Caveat:** if a rayforce2 update changes the _shape_ of an existing builtin's interning (e.g. master commit `7db37e4` made `.sys.gc` a dotted sym backed by a `.sys` dict where it used to be a flat interned name), the old sym file's flat-interned `.sys.gc` slot will conflict with the new dotted registration path and queries fail with `RAY_ERROR code=domain` (empty msg — see the `__VM` shadowing bug below). Startup runs a canonical health probe (`engine_health_probe` in `src/server.rs`) to surface this loudly instead of failing silently at first query time. **Do not wipe `~/.ray-exomem/sym` as a reflex** — that strands every persisted RAY_SYM column on disk (fact ids, predicates, etc.) because splay tables encode sym IDs by slot. The forward path is either (a) file an upstream issue asking for a sym-compat contract across such refactors, or (b) implement the rewrite-on-startup migration spec'd in `archive/2026-04-24_sym-rewrite-migration/design.md`.
- rayforce2 has duplicate `static _Thread_local ray_vm_t *__VM = NULL;` declarations in both `src/core/runtime.c` and `src/lang/eval.c` — they shadow each other instead of sharing storage. As a result, `ray_error_msg()` (which reads runtime.c's `__VM`) returns NULL on any thread that didn't call `ray_runtime_create`, including every tokio worker thread. The eval's RAY_ERROR object still carries the 8-byte ASCII code in `sdata` (read via `ray_err_code`, see `src/backend.rs::eval_raw`), so we get the label but lose the explanatory string. Worth filing upstream.
- Fact values are typed at the API/brain layer via `FactValue { I64 | Str | Sym }` (`src/fact_value.rs`). Splay emits parallel `facts_i64` / `facts_str` / `facts_sym` EDBs so Datalog rule bodies can run cmp/agg against numeric columns natively. Only typed asserts populate `facts_i64`. Bootstrap seeds numeric profile predicates (weight_kg, height_cm, age) as `FactValue::I64` — if a pre-typed-values exom still has them as `Str`, the derivation rules won't fire until those facts are re-asserted typed.

## Current agent-facing workflow

```bash
ray-exomem daemon
ray-exomem inspect
ray-exomem init work::team::project::repo
ray-exomem session new work::team::project::repo --name landing-page --multi --actor orchestrator --agents agent-a,agent-b
ray-exomem query --exom work::team::project::repo::main --json
```

- Prefer tree paths (`work::team::project::repo::main`) over the old flat `main` mental model.
- Prefer `query --json` for reads.
- Use `expand-query` when debugging query lowering or injected rules.
- `assert <predicate> <value>` uses the structured assert path when `--source`, `--confidence`, `--valid-from`, or `--valid-to` is provided.
- `retract <fact-id>` resolves the current tuple for that fact id, then emits Rayfall retract.
- `history <fact-id>` and `why <fact-id>` both support slash-delimited fact ids.

## Key API surfaces

- `GET /api/tree` — canonical tree/discovery path; use instead of `/api/exoms`
- `POST /api/actions/init` — scaffold `<path>/main` plus `<path>/sessions/`
- `POST /api/actions/exom-new` — create a bare exom at a tree path
- `POST /api/actions/session-new` — create a session exom under a project
- `POST /api/query` — canonical read path for one Rayfall `(query ...)`
- `POST /api/actions/assert-fact` — structured assert / replace by `fact_id`
- `POST /api/actions/eval` — advanced mixed Rayfall execution
- `GET /api/facts/<id>?exom=...` — fact detail + history
- `GET /api/explain?exom=...&predicate=...` — explain by predicate or fact id
- `GET /api/status` — health, stats, rules, current branch, `server.build.identity`

## Architecture notes

- Mutations go through `brain.rs`, not directly through the C engine.
- Queries are lowered/re-written before eval so exom-scoped rules are injected correctly.
- Splay tables under each `tree/<exom-path>/` directory are the source of truth on disk. There are no JSONL sidecars for facts/txs/observations/beliefs/branches; `auth.jsonl` is a separate subsystem and still exists.
- The daemon nests API routes under `/ray-exomem/api`, with a small `/api/status` compatibility shim at the root.
- A fresh persistent data dir auto-creates bare `tree/main`; projects and sessions are additional tree nodes layered on top.
- After state changes, runtime bindings must be refreshed.

## Files worth knowing

- `src/main.rs` — CLI and daemon lifecycle
- `src/server.rs` — HTTP API, query/eval routing, tree routes, SSE, UI hosting
- `src/brain.rs` — core memory model and mutations
- `src/scaffold.rs` — project scaffolding and bare-exom creation
- `src/tree.rs` — folder/exom classification and tree walking
- `src/path.rs` — tree path parsing/validation (`::` and `/`)
- `src/auth/routes.rs` — auth/login flows and current bootstrap seeding
- `src/storage.rs` — persistence and decoded query-table handling
- `src/rayfall_ast.rs` — Rayfall parsing/lowering
- `src/system_schema.rs` — builtin views and ontology/schema generation
- `ui/src/lib/exomem.svelte.ts` — UI API client

## Verification

```bash
cargo test
cd ui && npm run check && npm run build
```
