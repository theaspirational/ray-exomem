# CLAUDE.md — ray-exomem

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
Files **copied** by sandboxed processes (Claude Code, VS Code extensions) get the `com.apple.provenance` extended attribute. macOS silently refuses to execute them — the binary appears to hang with zero output. **Use `ln -f` (hard link) instead of `cp` to deploy the binary.** Hard links share the original inode and don't acquire the provenance xattr.

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

## API endpoints (key ones)

- `POST /api/actions/eval` — evaluate Rayfall source
- `POST /api/actions/assert-fact` — assert a structured fact
- `POST /api/actions/retract` — retract facts by predicate
- `POST /api/actions/retract-all` — retract all facts + clear rules (keeps tx history)
- `POST /api/actions/wipe?exom=<name>` — true wipe: reset Brain to empty, delete disk state
- `POST /api/actions/factory-reset` — wipe ALL exoms and sym table, recreate empty `main`
- `GET /api/status` — health check
- `GET /api/exoms` — list exoms
- `POST /api/exoms` — create exom
- `POST /api/exoms/<name>/manage` — rename, archive, unarchive, delete

All mutation endpoints accept `?exom=<name>` (default: `main`) and `X-Actor`, `X-Session`, `X-Model` headers.

## Architecture rules

- **Mutations go through Brain (Rust), not FFI.** `assert`, `retract`, `observe` are handled in `brain.rs` to keep the transaction log, bitemporal metadata, and provenance consistent. Only `eval` and `query` call into the C engine.
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
