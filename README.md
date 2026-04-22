# ray-exomem

Persistent external memory for agents and operators.

`ray-exomem` stores facts, observations, beliefs, rules, transactions, and
branches inside a tree of folders and exoms, then exposes that state through a
native Rayfall/Datalog CLI, HTTP API, and embedded Svelte UI.

Built on [rayforce2](https://github.com/RayforceDB/rayforce2), which provides
the Rayfall evaluator, Datalog engine, symbol table, and columnar storage.

## Current State

The repo is currently centered on the newer tree/session model:

- Tree paths on disk and in the UI: `work/ath/lynx/orsl/main`
- CLI tree paths: `work::ath::lynx::orsl::main`
- Projects scaffold to `main` plus `sessions/`
- Sessions are exoms created under `<project>/sessions/<id>`

Some legacy flat-exom helpers still exist in the CLI for compatibility, but
they are not the recommended path anymore. In particular:

- Prefer `ray-exomem inspect`, `init`, `exom-new`, and `session ...`
- Prefer `GET /api/tree` over `/api/exoms`
- `POST /api/actions/start-session` is removed

## Quick Start

Build and install locally:

```bash
cargo build --release
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
```

Start the daemon:

```bash
ray-exomem daemon
```

Open the UI at:

```text
http://127.0.0.1:9780/ray-exomem/
```

Stop it with:

```bash
ray-exomem stop
```

Inspect the local tree:

```bash
ray-exomem inspect
```

Scaffold a project and query its main exom:

```bash
ray-exomem init work::ath::lynx::orsl
ray-exomem query --exom work::ath::lynx::orsl::main --json
```

Create a session exom under that project:

```bash
ray-exomem session new work::ath::lynx::orsl \
  --name landing-page \
  --multi \
  --actor orchestrator \
  --agents agent-a,agent-b
```

Useful entry points:

```bash
ray-exomem --help
ray-exomem guide
ray-exomem guide --topic cli
```

## Conceptual Model

### Folder

A grouping node in the tree. Folders contain other folders and exoms but do not
store facts themselves.

### Exom

A leaf knowledge base with facts, rules, branches, transactions, observations,
and beliefs. Exoms are identified by tree path.

### Project

A scaffolded folder with:

- `main/` as the project's main exom
- `sessions/` as a folder for per-session exoms

Created with:

```bash
ray-exomem init <path>
```

### Session Exom

An exom under `<project>/sessions/<session-id>` used for isolated work, handoff,
or multi-agent coordination. Created with:

```bash
ray-exomem session new <project-path> --name <label> --actor <name> ...
```

### Branch

A branch inside one exom. Branches are used for hypothetical reasoning,
parallel work, and session ownership.

## Paths and Persistence

CLI paths use `::` separators:

```text
work::ath::lynx::orsl::main
```

Disk and UI paths use `/` separators:

```text
work/ath/lynx/orsl/main
```

The default data root is:

```text
~/.ray-exomem/
```

High-level layout:

```text
~/.ray-exomem/
  sym
  sym.lk
  tree/
    main/
      exom.json
    work/
      ath/
        lynx/
          orsl/
            main/
              exom.json
            sessions/
              20260411T143215Z_multi_agent_landing-page/
                exom.json
```

Notes:

- A directory is an exom when it contains `exom.json`
- A fresh persistent store auto-creates a bare `main` exom
- `rules.ray` and the `fact/`, `tx/`, `observation/`, `belief/`, and `branch/`
  splay directories appear lazily as data is written
- Auth state is separate from exom storage

## CLI Overview

Tree and session commands:

- `ray-exomem inspect [path]`
- `ray-exomem init <path>`
- `ray-exomem exom-new <path>`
- `ray-exomem session new ...`
- `ray-exomem session rename ...`
- `ray-exomem session close ...`
- `ray-exomem session archive ...`

Daemon-backed knowledge commands:

- `ray-exomem status`
- `ray-exomem query --exom <path>`
- `ray-exomem expand-query --request ...`
- `ray-exomem eval ...`
- `ray-exomem assert ...`
- `ray-exomem retract ...`
- `ray-exomem history <fact-id>`
- `ray-exomem why <fact-id>`
- `ray-exomem why-not --predicate ...`
- `ray-exomem branch <subcommand> ...`
- `ray-exomem coord <subcommand> ...`
- `ray-exomem export`
- `ray-exomem import`
- `ray-exomem watch`
- `ray-exomem lint-memory`

Examples:

```bash
ray-exomem assert project/status active \
  --exom work::ath::lynx::orsl::main \
  --source kickoff-notes

ray-exomem branch list --exom work::ath::lynx::orsl::main

ray-exomem query \
  --exom work::ath::lynx::orsl::main \
  --request '(query work/ath/lynx/orsl/main (find ?fact ?pred ?value) (where (fact-row ?fact ?pred ?value)))' \
  --json
```

Two CLI caveats worth knowing:

- Some older commands still use `--addr 127.0.0.1:9780`
- Newer tree/session commands route through the global `--daemon-url`

## HTTP API

Base prefix:

```text
/ray-exomem/api
```

Common endpoints:

- `GET /api/status`
- `GET /api/tree`
- `GET /api/guide`
- `POST /api/actions/init`
- `POST /api/actions/exom-new`
- `POST /api/actions/session-new`
- `POST /api/actions/rename`
- `POST /api/actions/assert-fact`
- `GET|POST /api/query`
- `POST /api/expand-query`
- `POST /api/actions/eval`
- `GET /api/facts`
- `GET /api/facts/{id}`
- `GET /api/branches`
- `POST /api/branches`
- `POST /api/branches/{id}/switch`
- `GET /api/branches/{id}/diff`
- `POST /api/branches/{id}/merge`
- `GET /api/explain`
- `GET /api/schema`
- `GET /api/graph`
- `GET /api/provenance`
- `GET /api/logs`
- `GET /api/actions/export`
- `GET /api/actions/export-json`
- `POST /api/actions/import-json`

Removed/legacy endpoints:

- `/api/exoms` returns `410 gone`; use `/api/tree`
- `/api/actions/start-session` returns `410 gone`; use `/api/actions/session-new`

SSE endpoints:

- `GET /ray-exomem/events`
- `GET /sse`

## Web UI

The embedded UI is served under `/ray-exomem/`.

Current top-level surfaces include:

- Tree browser under `/ray-exomem/tree/...`
- Exom view with facts, branches, history, graph, and rules
- Full-page query editor at `/ray-exomem/query`
- Full-page graph view at `/ray-exomem/graph`
- Guide at `/ray-exomem/guide`

When auth is enabled, the app also exposes login, profile, and admin surfaces.

## Architecture

`ray-exomem` owns:

- daemon lifecycle
- tree/exom/session scaffolding
- HTTP API and UI serving
- mutation orchestration through `brain.rs`
- persistence wiring for exom splay tables
- auth, API keys, shares, and admin flows

`rayforce2` owns:

- Rayfall parsing and evaluation
- Datalog fixpoint computation
- symbol interning
- columnar relation storage
- query planning and execution

Key files:

- `src/main.rs` — CLI surface and daemon startup
- `src/server.rs` — HTTP API, SSE, UI hosting
- `src/brain.rs` — mutation model and history
- `src/scaffold.rs` — `init` and bare-exom creation
- `src/tree.rs` — tree traversal and node classification
- `src/path.rs` — tree path parsing and validation
- `src/system_schema.rs` — builtin derived views and ontology/schema output
- `ui/src/routes/tree/[...path]/` — tree, folder, and exom UI

## Build and Dev Notes

Requirements:

- Rust
- Node.js + npm
- C compiler
- git

Build:

```bash
cargo build --release
```

Run tests:

```bash
cargo test
cd ui && npm run check && npm run build
```

Use foreground mode for debugging:

```bash
ray-exomem serve --bind 127.0.0.1:9780
```

Authenticated local dev currently uses `serve`, not `daemon`, because the auth
flags are wired there:

```bash
set -a; source .env; set +a
ray-exomem serve --bind 127.0.0.1:9780 \
  --auth-provider google \
  --google-client-id "$GOOGLE_CLIENT_ID" \
  --allowed-domains "$ALLOWED_DOMAINS" \
  --database-url "$DATABASE_URL"
```
