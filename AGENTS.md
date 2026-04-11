# AGENTS.md — ray-exomem

## What this is

ray-exomem is a persistent knowledge-base daemon for LLM agents. It wraps rayforce2 (a C columnar engine) via FFI to provide bitemporal facts, Datalog rules, provenance, and branches. The daemon serves an HTTP API and a SvelteKit web UI.

## Build & deploy

```bash
# Install from git (auto-clones rayforce2, builds UI, compiles everything)
cargo install --git https://github.com/theaspirational/ray-exomem.git

# Or build from source (requires rayforce2 alongside at ../rayforce2)
cargo build --release
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem

# Start daemon (forks into background)
ray-exomem daemon

# Foreground (useful for debugging)
ray-exomem serve --bind 127.0.0.1:9780

# Stop
ray-exomem stop
```

`cargo build --release` does everything in one command: builds the SvelteKit UI (`npm run build`), compiles rayforce2 (`make lib`), then compiles and links the Rust binary with the UI assets embedded. If rayforce2 isn't found at `../rayforce2`, it auto-clones from GitHub. The resulting binary is fully self-contained.

## Critical gotchas

### macOS `com.apple.provenance` blocks execution silently
Files **copied** by sandboxed processes (Codex, VS Code extensions) get the `com.apple.provenance` extended attribute. macOS silently refuses to execute them — the binary appears to hang with zero output. **Use `ln -f` (hard link) instead of `cp` to deploy the binary.** Hard links share the original inode and don't acquire the provenance xattr.

```bash
# Deploy (hard link — no provenance issues)
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem

# If the binary hangs with zero output, diagnose with:
xattr ~/.local/bin/ray-exomem
# If it shows com.apple.provenance, re-link:
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
```

### Rebuilding rayforce2 requires `cargo clean`
Cargo does not detect changes to the static C library (`librayforce.a`) after `make lib`. If you rebuild rayforce2, you must `cargo clean && cargo build --release` to force relinking. Otherwise the old library stays linked.

### UI is embedded in the binary
The SvelteKit UI (`ui/build/`) is embedded at compile time via `include_dir!`. The binary is fully self-contained — no external `ui/build/` directory needed at runtime. For development, use `--ui-dir path/to/ui/build` to serve from disk instead (supports HMR workflow).

### The `daemon` command forks
`ray-exomem daemon` forks via `libc::fork()`, prints the child PID, and the parent exits immediately. The child calls `setsid()` to detach from the terminal. Use `ray-exomem serve` for foreground operation (blocks the terminal).

### Signal handling
The daemon spawns a thread that blocks on `sigwait(SIGTERM | SIGINT)`. When a signal arrives, it removes the PID file and calls `process::exit(0)`. The PID file lives at `~/.ray-exomem/daemon.pid`.

## Project layout

```
src/
  main.rs        — CLI (clap), daemon lifecycle, fork
  web.rs         — HTTP server, all API endpoints, SSE
  brain.rs       — Fact / Observation / Belief / Tx / Branch (Rust-only, no FFI)
  backend.rs     — RayforceEngine wrapper (FFI calls to rayforce2)
  ffi.rs         — C FFI declarations (ray_runtime_create, ray_eval_str, etc.)
  storage.rs     — Splay table I/O, JSONL sidecars, datom encoding
  exom.rs        — Exom directory manager (~/.ray-exomem/exoms/)
  rules.rs       — Rule parsing, head/arity extraction
  context.rs     — MutationContext (actor, session, model)
  client.rs      — HTTP client for CLI→daemon communication
  rayfall_parser.rs — Classifies Rayfall forms (AssertFact, Query, Rule, etc.)
  agent_guide.rs — Built-in operator reference text
  datom.rs       — Datom type (tagged i64 encoding for EAV columns)
  lib.rs         — Public API surface
ui/              — SvelteKit 5 + Tailwind 4 web UI
  src/lib/exomem.svelte.ts  — Client API wrappers (all fetch calls)
  src/lib/stores.svelte.ts  — Svelte 5 reactive app state
  src/routes/               — Page components (facts, query, rules, exoms, etc.)
examples/        — .ray files for smoke tests
```

## CLI (thin HTTP client)

The CLI speaks HTTP to the daemon; paths are under `/ray-exomem` (e.g. `http://127.0.0.1:9780/ray-exomem/api/...`). Full detail: `ray-exomem guide` or `ray-exomem guide --topic cli`.

- Global `--json` — minified JSON on stdout when set; several commands also default to JSON when stdout is not a TTY (pipes).
- `query` — `POST /api/query` with a plain-text Rayfall `(query ...)` form. Optional `--request '<RAYFALL>'`; default lists current facts.
- `assert <predicate> <value>` — zero-metadata assertions use `POST /api/actions/eval`; metadata-bearing assertions (`--source`, `--confidence`, `--valid-from`, `--valid-to`) use `POST /api/actions/assert-fact`.
- `retract <fact-id>` — resolves the active tuple via `GET /api/facts/<id>`, then emits Rayfall `(retract-fact …)` through `POST /api/actions/eval`.
- `history <fact-id>` — `GET /api/facts/<id>?exom=` (percent-encode the id if needed).
- `why <fact-id>` — `GET /api/explain?predicate=…&exom=…` (predicate parameter may be a `fact_id` or predicate name).
- `why-not --predicate P [--value V]` — uses a filtered Rayfall query and reports whether a matching current fact exists.
- `watch` — `GET /ray-exomem/events` (SSE: mutation events as `event: memory` JSON, heartbeats).
- `lint-memory` — hygiene report (duplicates, oversized values, missing provenance, bad/empty identifiers, time-related heuristics); non-zero exit if any issue.
- `doctor` — status, branches, export, `eval` smoke, and CLI/daemon build identity comparison.
- `start-session` — JSON contract for agents; creates the exom with `POST /api/exoms` if missing (`--actor`, default `cli`).
- Also: `eval`, `observe`, `export`, `import` (→ `POST /api/actions/import-json`), `exoms`, `log`, `branch`, `status`, `stop`, `daemon`, `serve`, `run`, `version`, `brain-demo`.

## API endpoints (key ones)

- `POST /api/query` — canonical read endpoint for a single Rayfall `(query ...)` form; returns decoded JSON rows plus formatted output; `X-Actor` not required
- `POST /api/actions/assert-fact` — assert/replace by `fact_id` with explicit valid-time/provenance payload
- retract is handled via the Brain layer and current CLI flow resolves the tuple via `GET /api/facts/<id>`, then emits Rayfall `(retract-fact …)` through `POST /api/actions/eval`
- `GET /api/facts/<id>?exom=` — fact detail + history (URL-encode `<id>` if needed)
- `GET /api/explain?exom=&predicate=` — match by predicate name or `fact_id`
- `POST /api/observations` / `POST /api/beliefs/revise`
- `POST /api/actions/eval` — Rayfall engine (advanced; not the default agent read path)
- `POST /api/actions/import-json` — lossless JSON import (requires `X-Actor`)
- `POST /api/actions/retract-all` — retract all facts + clear rules (keeps tx history)
- `POST /api/actions/wipe?exom=<name>` — true wipe
- `POST /api/actions/factory-reset` — wipe ALL exoms + sym, recreate `main`
- `GET /api/status` — includes `stats.sym_entries`, rule counts and `derived_predicates`, plus `server.build.identity`
- `GET /api/exoms`, `POST /api/exoms`, `POST /api/exoms/<name>/manage`
- `GET /api/beliefs/<id>/support?exom=` — belief `supported_by` resolved to facts/observations
- `POST /api/actions/consolidate-propose` — **501** (future consolidation job)
- `GET /ray-exomem/events?exom=&branch=&actor=&predicate=&since=` — SSE (`event: memory` + JSON; heartbeats)
- Binary **`ray-exomem-mcp`** — MCP over stdio (`RAY_EXOMEM_ADDR`, `RAY_EXOMEM_ACTOR`)

Mutations require a non-empty `X-Actor` (and typically `X-Session`, `X-Model`), except `POST /api/query` and read-only `GET`s.

## Architecture rules

- **Mutations go through Brain (Rust), not FFI.** Assert/retract/observations/beliefs use `brain.rs`. Canonical read is `POST /api/query`, which accepts a Rayfall `(query ...)` form and returns decoded JSON. `POST /api/actions/eval` hits the C engine for mixed Rayfall, rules, and advanced ad-hoc queries.
- **Query rewriting.** Before calling `ray_eval_str()`, the server injects the exom's stored rules as an inline `(rules ...)` clause so Datalog derivation is scoped correctly.
- **Dual persistence.** Every mutation writes JSONL first (atomic rename), then splay tables. JSONL is the source of truth; splay tables are a performance cache.
- **`restore_runtime()` after state changes.** Any endpoint that modifies exom state must call `refresh_exom_binding()` or `restore_runtime()` to update the C engine's bindings.
- **Borrow checker pattern in web.rs.** When you need mutable access to an exom (`daemon.exoms.get_mut`) followed by an immutable call to `daemon.engine`, scope the mutable borrow in a block first, then access the engine.

## Testing

```bash
cargo test                    # Rust unit tests
ray-exomem run examples/native_smoke.ray   # Offline smoke test
```

## UI development

```bash
cd ui
npm install
npm run dev      # Dev server with HMR
npm run build    # Production build to ui/build/
```

The UI uses shadcn-svelte components, Tailwind 4, and D3 for the graph view.
