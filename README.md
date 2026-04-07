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

## Quick start

```bash
# Build (requires rayforce2 checked out alongside this repo)
cargo build

# Start the daemon (http://127.0.0.1:9780)
cargo run -- serve

# Assert a fact into the default exom
cargo run -- assert alice :name "Alice" --confidence 0.9 --source "observation"

# Query facts
cargo run -- facts

# Evaluate Rayfall directly
cargo run -- eval '(query main (find ?e ?n) (where (?e :name ?n)))'

# Offline: evaluate a .ray file without the daemon
cargo run -- run examples/native_smoke.ray
```

If rayforce2 is not in the default location:

```bash
RAYFORCE2_DIR=/path/to/rayforce2 cargo build
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
| `serve` | Start daemon in foreground (`--port`, `--no-persist`) |
| `daemon` | Start daemon in background |
| `stop` | Stop a running daemon |
| `status` | Show daemon health and exom stats |
| `eval <expr>` | Evaluate Rayfall source via the daemon |
| `assert <entity> <attr> <value>` | Assert a fact (`--confidence`, `--valid-from`, `--valid-to`, `--source`) |
| `retract <predicate>` | Soft-retract a fact (closes valid-time interval) |
| `facts` | List current facts in an exom |
| `observe <content>` | Record an observation (`--source-type`, `--source-ref`, `--confidence`, `--tags`) |
| `import [file]` | Import a `.ray` file (or stdin) into an exom |
| `export` | Export all facts as Rayfall source |
| `exoms` | List all exoms |
| `log` | Show recent transaction log |
| `branch <sub>` | Manage branches: `list`, `create`, `switch`, `diff`, `merge`, `delete` |
| `run <file>` | Evaluate a `.ray` file offline (no daemon) |
| `load <file>` | Alias for `run` |
| `version` | Print version and rayforce2 engine info |
| `guide [topic]` | Print operator reference (`overview`, `cli`, `http`, `env`, `limitations`) |

All commands that talk to the daemon accept `--exom <name>` (default: `main`)
and `--url <base>` (default: `http://127.0.0.1:9780`).

Attribution for the transaction log:
```bash
ray-exomem assert alice :role engineer --actor "ops-bot" --session "s-42" --model "claude-4"
```

---

## HTTP API

Base URL: `http://127.0.0.1:9780`  
Exom selection: `?exom=<name>` query parameter (default: `main`)  
Attribution: `X-Actor`, `X-Session`, `X-Model` request headers

### Status and discovery

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/status` | Engine health, KB load state, version |
| `GET` | `/api/schema` | Relation catalog: arity, field types, row counts, samples |
| `GET` | `/api/exoms` | List all exoms |
| `POST` | `/api/exoms` | Create a new exom |
| `GET/POST/DELETE` | `/api/exoms/<name>/manage` | Rename, archive, unarchive, delete |
| `GET` | `/ray-exomem/events` | SSE event stream (live state changes) |

### Facts and queries

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/actions/eval` | Evaluate a Rayfall expression |
| `POST` | `/api/actions/import` | Import a Rayfall source body |
| `POST` | `/api/actions/assert-fact` | Assert a structured fact with interval metadata |
| `POST` | `/api/actions/retract` | Soft-retract a fact |
| `GET` | `/api/actions/export` | Export all facts as Rayfall |
| `GET` | `/api/facts/<id>` | Detail for a specific fact |
| `GET` | `/api/facts/valid-at` | Facts valid at a point in time (`?t=<iso8601>`) |
| `GET` | `/api/facts/bitemporal` | Bitemporal query (valid-time + transaction-time) |
| `GET` | `/api/derived/<predicate>` | All tuples for a derived (rule-produced) predicate |
| `POST` | `/api/actions/evaluate` | Trigger incremental re-evaluation |
| `POST` | `/api/actions/clear` | Clear all facts and rules from an exom |

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
| Exoms | Exom registry: create, rename, archive, export |
| Branches | Branch manager: create, switch, diff, merge |

---

## Persistence

```
~/.ray-exomem/
  sym                   ← shared symbol table (rayforce2 intern table)
  exoms/
    main/
      datoms/           ← EAV triples (rayforce2 splayed columnar table)
      facts/            ← Fact rows
      observations/     ← Observation rows
      beliefs/          ← Belief rows
      transactions/     ← Tx log
      branches/         ← Branch metadata
      rules.ray         ← Datalog rule text
```

All tables use rayforce2's splayed format — one file per column, typed binary
vectors, no external dependencies. The shared symbol table maps interned strings
to integer IDs across all exoms.

Writes are write-through for interactive commands and snapshot-based (temp →
fsync → atomic rename) on daemon shutdown.

---

## Architecture

```
CLI / HTTP client
      │
      ▼
  ray-exomem daemon
  ┌─────────────────────────────────────────┐
  │  web.rs  (HTTP API, SSE, actix-web)     │
  │  main.rs (CLI, clap)                    │
  │                                         │
  │  brain.rs  (Fact / Observation /        │
  │             Belief / Tx / Branch)       │
  │  exom.rs   (exom registry, disk I/O)    │
  │  rules.rs  (rule parsing, head/arity)   │
  │  context.rs (actor / session / model)   │
  │  storage.rs (splay I/O, datom codec)    │
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
coordination, and the datom encoding scheme.

**rayforce2** owns: Rayfall parsing and evaluation, Datalog fixpoint computation,
relation storage, query planning, column I/O, and the symbol intern table.

---

## Building

Requirements:
- Rust (stable)
- C compiler (clang or gcc)
- rayforce2 checked out at `../rayforce2` (relative to this repo), or set `RAYFORCE2_DIR`

```bash
# Build everything (compiles rayforce2 lib then ray-exomem)
cargo build

# Run tests
cargo test

# Release build
cargo build --release
```

The build script (`build.rs`) calls `make lib` in the rayforce2 directory to
produce `librayforce.a`, then links it statically into the ray-exomem binary.
