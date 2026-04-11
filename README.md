# ray-exomem

A persistent, multi-knowledge-base daemon for agents and operators. Store facts,
observations, and beliefs; query them with Datalog rules; track everything
with bitemporal history, provenance, and branches for hypothetical reasoning.

Built on [rayforce2](https://github.com/RayforceDB/rayforce2) — a zero-dependency
C columnar engine with a native Datalog evaluator and Rayfall query language.

---

## What it is

ray-exomem gives LLM agents and operators a structured external memory store
(an "exomemory"). Each knowledge base is called an **exom**. Facts inside an exom
are EAV triples (entity / attribute / value) with confidence scores, valid-time
intervals, provenance, and a full transaction log. Datalog rules derive new facts
from existing ones at query time.

The daemon exposes an HTTP API and a Svelte web UI. The CLI talks to the daemon
or runs offline for scripting.

---

## Install

**One-liner** (requires Rust, Node.js, C compiler, git):
```bash
cargo install --git https://github.com/theaspirational/ray-exomem.git
```

This clones the repo, auto-clones rayforce2, builds the UI, compiles everything,
and installs a self-contained binary to `~/.cargo/bin/ray-exomem`.

**From a local checkout:**
```bash
cargo install --path .
# or build without installing:
cargo build --release
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
```

If rayforce2 is elsewhere:
```bash
RAYFORCE2_DIR=/path/to/rayforce2 cargo build --release
```

**1. Start the daemon**
```bash
ray-exomem daemon
# UI at http://127.0.0.1:9780
# Stop with: ray-exomem stop
```

**2. Explore all commands**
```bash
ray-exomem --help
ray-exomem assert --help
ray-exomem query --help
ray-exomem expand-query --help
```

**3. Bootstrap an agent session**
```bash
ray-exomem doctor --exom main --json
ray-exomem start-session --exom main --actor cli --json
```

**4. Assert facts**
```bash
# assert <predicate> <value>  [--exom <name>] [--confidence 0-1] [--source label]
ray-exomem assert sky-color blue
ray-exomem assert temperature 22   --confidence 0.8 --source sensor
ray-exomem assert location paris   --valid-from 2024-01-01T00:00:00Z \
                                   --valid-to   2024-06-01T00:00:00Z
```

When any metadata flag is provided (`--source`, `--confidence`, `--valid-from`, `--valid-to`),
the CLI uses the structured assert endpoint so provenance and confidence are preserved in history.

**5. List and query facts**
```bash
# Show all facts in the current exom
ray-exomem facts

# Query current logical facts with decoded JSON rows
ray-exomem query --exom main --json

# Inspect normalized + expanded query text
ray-exomem expand-query --exom main --request '(query (find ?fact ?pred ?value) (where (?fact '\''fact/predicate ?pred) (?fact '\''fact/value ?value)))'
```

**6. Add a Datalog rule and query derived facts**
```bash
# Define a rule: anything observed by a sensor is trusted
ray-exomem eval '(rule main (trusted ?p ?v) (?p :source sensor) (?p :value ?v))'

# Query the derived relation
ray-exomem eval '(query main (find ?p ?v) (where (trusted ?p ?v)))'
```

**7. Record an observation and export**
```bash
ray-exomem observe "humidity is 65%" --source-type sensor --source-ref "sensor-7" \
                                     --confidence 0.95 --tags "env,climate"
ray-exomem export > backup.json
```

**8. Full agent workflow in one eval call**
```bash
ray-exomem eval '
  (assert-fact main "alice" :knows "bob")
  (assert-fact main "bob"   :knows "carol")
  (rule main (connected ?x ?z) (?x :knows ?y) (?y :knows ?z))
  (query main (find ?x ?z) (where (connected ?x ?z)))
'
```

**9. Evaluate from a file**
```bash
ray-exomem eval --file examples/native_smoke.ray
```

**Offline (no daemon)**
```bash
ray-exomem run examples/native_smoke.ray
```

---

## Core concepts

### Exom
An isolated knowledge base. Multiple exoms coexist in one daemon with no
shared facts. The default exom is named `main`. Each exom has its own facts,
rules, observations, beliefs, transactions, and branches persisted to disk at
`~/.ray-exomem/exoms/<name>/`.

### Fact
An EAV triple `(entity, attribute, value)` with metadata:

| Field | Type | Meaning |
|---|---|---|
| `entity` | symbol or string | the subject — e.g. `alice`, `"sensor-42"` |
| `attribute` | symbol | the predicate — e.g. `:name`, `:location` |
| `value` | i64, symbol, or string | the object |
| `confidence` | f64 | 0.0–1.0 |
| `valid_from` / `valid_to` | timestamp | when this was true in the world |
| `provenance` | string | origin label — `"observation"`, `"api"`, `"rule"` |

Facts are **bitemporal**: valid-time (when the fact holds in the world) is
separate from transaction-time (when it was recorded). Retraction closes the
valid-time interval without erasing history.

### Observation
Raw evidence before it becomes a fact. Observations carry a source type,
source reference, confidence, and tags. They feed the fact-assertion pipeline
but are kept in full for audit.

### Belief
A higher-order claim with an explicit status (Active / Superseded / Revoked)
and a list of supporting evidence IDs. Beliefs aggregate observations and facts
into interpreted conclusions with a rationale.

### Datom encoding
Values in EAV columns are packed into tagged `i64` words, keeping the storage
schema uniform:

```
bits 63–62 = 00  →  plain I64 (integer or negative)
bits 63–62 = 01  →  SYM  (symbol intern ID in low 61 bits)
bits 63–62 = 10  →  STR  (string intern ID in low 61 bits)
```

Symbols and strings are interned in the rayforce2 global symbol table, persisted
alongside the exom data.

### Rules and Datalog
Rules are written in Rayfall list syntax and stored per-exom in `rules.ray`.
They are evaluated by the rayforce2 Datalog engine at query time via the
`(rules ...)` inline clause — no separate Datalog interpreter in ray-exomem.

```scheme
; Define a reachability rule
(rule main (reaches ?x ?z) (?x :edge ?y) (?y :edge ?z))

; Query derived facts
(query main (find ?x ?z) (where (reaches ?x ?z)))
```

### Branch
A named fork of an exom for hypothetical reasoning or parallel agent work.
Branches can be diffed and merged back with configurable conflict policies
(last-writer-wins, keep-target, manual).

---

## CLI reference

```
ray-exomem [OPTIONS] <COMMAND>
```

| Command | Description |
|---|---|
| `daemon` | Start daemon in background (forks, returns immediately) |
| `serve` | Start daemon in foreground (`--port`, `--no-persist`) |
| `stop` | Stop a running daemon |
| `status` | Show daemon health and exom stats |
| `eval <expr>` | Evaluate Rayfall source (inline or `--file`) |
| `assert <predicate> <value>` | Assert a fact (`--confidence`, `--valid-from`, `--valid-to`, `--source`) |
| `query` | Execute a read-only Rayfall `(query ...)` request with decoded rows |
| `expand-query` | Show normalized and expanded query text after exom/rule lowering |
| `doctor` | Health checks plus CLI/daemon build identity comparison |
| `start-session` | Bootstrap an exom for agents and print the session contract |
| `retract <fact-id>` | Retract a fact by stable fact id |
| `facts` | List current facts in an exom |
| `observe <content>` | Record an observation (`--source-type`, `--source-ref`, `--confidence`, `--tags`) |
| `export` | Export all data as lossless JSON (`--format rayfall` for human-readable) |
| `import <file>` | Import a JSON backup (replaces all data in the exom) |
| `exoms` | List all exoms |
| `log` | Show recent transaction log |
| `branch <sub>` | Manage branches: `list`, `create`, `switch`, `diff`, `merge`, `delete` |
| `run <file>` | Evaluate a `.ray` file offline (no daemon) |
| `version` | Print version and rayforce2 engine info |
| `coord <sub>` | Coordination helpers for claims, dependencies, and agent sessions |
| `history <fact-id>` | Show fact detail + touch history |
| `why <fact-id>` | Explain a fact or derived predicate |
| `why-not` | Check whether a matching active fact exists |
| `watch` | Stream mutation events over SSE |
| `lint-memory` | Run hygiene checks over exported facts |
| `guide [topic]` | Print operator reference (`overview`, `cli`, `http`, `env`, `limitations`) |

All commands that talk to the daemon accept `--exom <name>` (default: `main`)
and `--addr <host:port>` (default: `127.0.0.1:9780`).

Attribution for the transaction log:
```bash
ray-exomem assert alice :role engineer --actor "ops-bot" --session "s-42" --model "claude-4"
```

---

## Export and import

ray-exomem uses **lossless JSON** as its default export/import format. Every
entity type (facts, transactions, observations, beliefs, branches, rules) is
included with full metadata — confidence, provenance, valid-time intervals,
transaction IDs, actor/session info.

```bash
# Lossless backup (default)
ray-exomem export > backup.json

# Restore from backup (replaces all data in the exom)
ray-exomem import backup.json

# Human-readable Rayfall (facts + rules only, lossy)
ray-exomem export --format rayfall > snapshot.ray
```

The JSON format is also available via the HTTP API:

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/actions/export-json` | Lossless JSON export |
| `POST` | `/api/actions/import-json` | Lossless JSON import (replaces exom data) |
| `GET` | `/api/actions/export` | Human-readable Rayfall export |

---

## HTTP API

Base URL: `http://127.0.0.1:9780`
Exom selection: `?exom=<name>` query parameter (default: `main`)
Attribution: `X-Actor`, `X-Session`, `X-Model` request headers

### Status and discovery

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/status` | Engine health, KB load state, version, and build identity |
| `GET` | `/api/schema` | Relation catalog: arity, field types, row counts, samples |
| `GET` | `/api/exoms` | List all exoms |
| `POST` | `/api/exoms` | Create a new exom |
| `GET/POST/DELETE` | `/api/exoms/<name>/manage` | Rename, archive, unarchive, delete |
| `GET` | `/ray-exomem/events` | SSE event stream (live state changes) |

### Facts and queries

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/actions/eval` | Evaluate a Rayfall expression |
| `POST` | `/api/actions/assert-fact` | Assert a structured fact with interval metadata |
| `POST` | `/api/actions/retract` | Soft-retract a fact |
| `GET` | `/api/actions/export` | Export facts as Rayfall |
| `GET` | `/api/actions/export-json` | Lossless JSON export (all entities) |
| `POST` | `/api/actions/import-json` | Lossless JSON import (replaces exom data) |
| `GET` | `/api/facts/<id>` | Detail for a specific fact |
| `GET` | `/api/facts/valid-at` | Facts valid at a point in time (`?t=<iso8601>`) |
| `GET` | `/api/facts/bitemporal` | Bitemporal query (valid-time + transaction-time) |
| `GET` | `/api/derived/<predicate>` | All tuples for a derived (rule-produced) predicate |
| `POST` | `/api/actions/evaluate` | Trigger incremental re-evaluation |
| `POST` | `/api/actions/retract-all` | Retract all facts and clear rules (preserves tx history) |
| `POST` | `/api/actions/wipe` | True wipe: reset exom to empty (no history) |
| `POST` | `/api/actions/factory-reset` | Wipe ALL exoms + sym table, recreate empty `main` |

### Graph and provenance

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/graph` | Entity-relation graph (nodes + edges) |
| `GET` | `/api/provenance` | Provenance metadata for the exom |
| `GET` | `/api/explain` | Proof/derivation tree for a given tuple |
| `GET` | `/api/relation-graph` | Relation dependency graph |
| `GET` | `/api/clusters` | Cluster summaries |
| `GET` | `/api/clusters/<id>` | Detail for a specific cluster |
| `GET` | `/api/logs` | Recent transaction log entries |

### Branches

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/branches` | List branches for the selected exom |
| `POST` | `/api/branches` | Create a new branch |
| `POST` | `/api/branches/<id>/switch` | Switch the active branch |
| `GET` | `/api/branches/<id>/diff` | Diff a branch against base |
| `POST` | `/api/branches/<id>/merge` | Merge (`policy`: `last-writer-wins`, `keep-target`, `manual`) |
| `DELETE` | `/api/branches/<id>` | Archive or delete a branch |

---

## Web UI

The Svelte UI is served at `http://127.0.0.1:9780` when the daemon is running.

| Page | Description |
|---|---|
| Dashboard | Engine status, schema summary, cluster overview, recent logs |
| Facts | Browse, search, assert, retract, and edit facts; filter by predicate; import/export |
| Query | Rayfall REPL: enter expressions, inspect results |
| Rules | List, add, edit, delete Datalog rules; search by predicate |
| Graph | D3 force-directed entity–relation graph; zoom and filter |
| Provenance | Derivation tree viewer: select a fact, see its proof DAG |
| Timeline | Facts grouped by `valid_from` date |
| Exoms | Exom registry: create, rename, archive, wipe, export, factory reset |
| Branches | Branch manager: create, switch, diff, merge |

---

## Persistence

```
~/.ray-exomem/
  sym                     ← shared symbol table (rayforce2 intern table)
  exoms/
    main/
      fact.jsonl          ← JSONL sidecar (source of truth)
      tx.jsonl            ← transaction log
      observation.jsonl   ← observations
      belief.jsonl        ← beliefs
      branch.jsonl        ← branch metadata
      rules.ray           ← Datalog rule text
      datoms/             ← EAV triples (rayforce2 splayed columnar table)
      fact/               ← Fact columns (binary cache)
      tx/                 ← Tx columns (binary cache)
      observation/        ← Observation columns (binary cache)
      belief/             ← Belief columns (binary cache)
      branch/             ← Branch columns (binary cache)
```

### Dual persistence: JSONL + splay tables

Every mutation writes two representations:

1. **JSONL sidecars** (`.jsonl` files) — one JSON object per line, one file per
   entity type. Written first via atomic rename (`tmp` → final). These are the
   **source of truth** and are human-readable, portable, and format-stable.

2. **Splay tables** (rayforce2 columnar binary) — written second as a performance
   cache. These enable fast Datalog queries via the C engine.

### Upgrade resilience

The rayforce2 symbol table (`sym`) is a binary blob whose format may change
between engine versions. If the symbol table becomes unreadable after a binary
upgrade:

1. The daemon detects the incompatible `sym` file on startup
2. All exoms are **automatically recovered from JSONL sidecars** (zero data loss)
3. Splay tables and the symbol table are rebuilt from the recovered data
4. The daemon starts normally with a warning in the log

This means **binary upgrades never lose data** — the JSONL files are always
consistent and independent of the rayforce2 binary format.

### Manual backup and restore

```bash
# Full lossless backup
ray-exomem export > backup.json

# Restore (replaces all data in the target exom)
ray-exomem import backup.json
```

---

## Architecture

```
CLI / HTTP client
      │
      ▼
  ray-exomem daemon
  ┌─────────────────────────────────────────┐
  │  web.rs  (HTTP API, SSE)               │
  │  main.rs (CLI, clap)                    │
  │                                         │
  │  brain.rs  (Fact / Observation /        │
  │             Belief / Tx / Branch)       │
  │  exom.rs   (exom registry, disk I/O)    │
  │  rules.rs  (rule parsing, head/arity)   │
  │  context.rs (actor / session / model)   │
  │  storage.rs (splay I/O, JSONL, datom)   │
  │  backend.rs (RayforceEngine wrapper)    │
  │  ffi.rs    (C FFI declarations)         │
  └──────────────┬──────────────────────────┘
                 │  FFI  (librayforce.a)
                 ▼
         rayforce2 engine (C)
         ┌──────────────────────────────┐
         │  Rayfall parser + evaluator  │
         │  Datalog engine              │
         │  columnar storage            │
         │  query optimizer             │
         │  splayed table I/O           │
         └──────────────────────────────┘
```

**ray-exomem** owns: daemon lifecycle, CLI/HTTP transport, exom registry,
bitemporal fact model, transaction log, branch management, persistence
coordination, JSONL sidecar writes, and the datom encoding scheme.

**rayforce2** owns: Rayfall parsing and evaluation, Datalog fixpoint computation,
relation storage, query planning, column I/O, and the symbol intern table.

---

## How commands reach the engine

`assert`, `retract`, `observe`, and `rule` commands are **intercepted in Rust**
and handled by the Brain layer — they never reach the C engine via FFI. This
keeps the transaction log, bitemporal metadata, and provenance consistent.

`eval` and `query` forms pass through to `ray_eval_str()` in rayforce2. Before
calling into C, the server rewrites each `(query ...)` form to inject the exom's
stored rules as an inline `(rules ...)` clause, so Datalog derivation is always
scoped to the current exom.

```
ray-exomem assert sky-color blue
  └─ builds: (assert-fact main "sky-color" 'sky-color "blue")
  └─ POST /api/actions/eval
       └─ rayfall_parser classifies form as AssertFact
       └─ brain.assert_fact(...)   ← Rust only, no FFI

ray-exomem eval '(query main (find ?p) (where (?p :sky-color ?v)))'
  └─ POST /api/actions/eval
       └─ rayfall_parser classifies form as Query
       └─ rewrites to: (query main (find ?p) (where ...) (rules ...exom-rules...))
       └─ engine.eval(rewritten)   ← ray_eval_str() FFI → rayforce2
```

---

## Building

Requirements:
- Rust (stable)
- Node.js + npm (for UI build)
- C compiler (clang or gcc)
- git (to auto-clone rayforce2 if not present)

```bash
# Build everything (UI + rayforce2 + Rust — single command)
cargo build --release

# Run tests
cargo test

# Point at a different rayforce2 checkout
RAYFORCE2_DIR=/path/to/rayforce2 cargo build --release
```

The build script (`build.rs`) does three things in order:
1. Builds the SvelteKit UI (`npm install && npm run build` in `ui/`)
2. Builds rayforce2 (`make lib`) — auto-clones from GitHub if not at `../rayforce2`
3. Compiles the Rust binary with UI assets embedded via `include_dir!`

The resulting binary is fully self-contained (~3.5 MB).
