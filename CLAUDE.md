# CLAUDE.md — ray-exomem

Project Philosophy (Greenfield Bias)

This project should be treated as a greenfield system, regardless of its current state. Do not assume the need for backward compatibility, legacy constraints, or incremental patching unless explicitly stated.

Prioritize correctness, simplicity, and architectural integrity over stability of existing patterns. Prefer clean, well-reasoned designs even if they require refactoring or replacing existing components partially or entirely.

Avoid introducing fallbacks, shims, or compatibility layers as a default strategy. These should only be considered when explicitly required. Instead, focus on identifying the most coherent and maintainable solution from first principles.


The role of this file is to describe common mistakes and confusion points that agents might encounter as they work in this project. If you ever encounter something in the project that surprises you, please alert the developer working with you and indicate that this is the case in the CLAUDE.md file to help prevent future agents from having the same issue.

When creating any artifacts during chat session, save them in archive/<date>_<session_name>/<artifact_name>.md

When adding or modifying any db/rayfall interactions test them against the running ray-exomem daemon.

## What this is

`ray-exomem` is a persistent memory daemon for LLM agents.

- Storage model: exoms, facts, observations, beliefs, transactions, branches
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

## Important gotchas

- Use `ln -f`, not `cp`, when deploying the binary on macOS. `com.apple.provenance` can make copied binaries hang silently.
- If `rayforce2` changed, run `cargo clean && cargo build --release` or Cargo may keep the old static library linked.
- The Svelte 5 UI is embedded in the binary at build time.
- `ray-exomem daemon` forks. Use `serve` if you want logs in the terminal.
- In authenticated UI mode, mutation actor attribution should fall back to the logged-in email. Do not require a separate `ray-exomem-actor` localStorage value for basic writes.
- JSONL auth replay must preserve `user.active` / `last_login` on repeated `user` entries. A naive replay that resets them on login makes deactivation appear to succeed in the UI while leaving the account effectively active.

## Current agent-facing workflow

```bash
ray-exomem doctor --exom main --json
ray-exomem start-session --exom main --actor cli --json
ray-exomem query --exom main --json
```

- Prefer `query --json` for reads.
- Use `expand-query` when debugging query lowering or injected rules.
- `assert <predicate> <value>` uses the structured assert path when `--source`, `--confidence`, `--valid-from`, or `--valid-to` is provided.
- `retract <fact-id>` resolves the current tuple for that fact id, then emits Rayfall retract.
- `history <fact-id>` and `why <fact-id>` both support slash-delimited fact ids.

## Key API surfaces

- `POST /api/query` — canonical read path for one Rayfall `(query ...)`
- `POST /api/actions/assert-fact` — structured assert / replace by `fact_id`
- `POST /api/actions/eval` — advanced mixed Rayfall execution
- `GET /api/facts/<id>?exom=...` — fact detail + history
- `GET /api/explain?exom=...&predicate=...` — explain by predicate or fact id
- `GET /api/status` — health, stats, rules, current branch, `server.build.identity`

## Architecture notes

- Mutations go through `brain.rs`, not directly through the C engine.
- Queries are lowered/re-written before eval so exom-scoped rules are injected correctly.
- JSONL sidecars are the source of truth; splay tables are the cache.
- After state changes, runtime bindings must be refreshed.

## Files worth knowing

- `src/main.rs` — CLI and daemon lifecycle
- `src/web.rs` — HTTP API, query/eval routing, SSE
- `src/brain.rs` — core memory model and mutations
- `src/storage.rs` — persistence and decoded query-table handling
- `src/rayfall_ast.rs` — Rayfall parsing/lowering
- `src/system_schema.rs` — builtin views and ontology/schema generation
- `ui/src/lib/exomem.svelte.ts` — UI API client

## Verification

```bash
cargo test
cd ui && npm run check && npm run build
```
