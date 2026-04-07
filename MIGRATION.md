# ray-exomem Migration Blueprint

Status: working spec
Scope: migrate ray-exomem to a native rayforce2-only stack, using Rayfall list-style syntax as the canonical frontend specification and removing the custom Teide parser / translation path entirely.

## 1. Decision summary

1. Rayfall list-style syntax is the source of truth for all user-facing programs.
2. ray-exomem must not parse, rewrite, or translate Teide syntax on the frontend path.
3. All reasoning semantics must come from native rayforce2 integration, not a compatibility layer.
4. ray-exomem remains the product/orchestration layer: daemon, API, KB lifecycle, persistence coordination, and user-facing wrappers.
5. Any feature that cannot be expressed natively in rayforce2 must be moved into rayforce2 proper before it is exposed by ray-exomem; if not, it is blocked rather than emulated.

## 2. Current-state inventory from the codebase

ray-exomem today is already a thin rayforce2 client/orchestrator (`src/lib.rs`, `src/backend.rs`, `src/main.rs`) with a direct `ray_eval_str()` execution path. The remaining work is to restore the higher-level product surface that teide-exomem exposed, but only as native wrappers over rayforce2.

teide-exomem currently exposes:
- daemon + MCP + HTTP APIs
- multi-KB registry and KB lifecycle management
- assert / retract / load / evaluate / query / explain / schema / status / export
- persistence and restart behavior
- provenance / proof trees
- temporal intervals and temporal builtins / MTL-like behavior where possible
- cross-KB query and KB merge semantics
- admin UI, logs, relation graphs, clusters, fact detail, clear/drop-relation

## 3. Target architecture

### 3.1 Layering

rayforce2 proper owns:
- Rayfall parser and evaluator
- native Datalog / KB execution primitives
- relation catalog and storage primitives
- query planning / evaluation
- provenance plumbing
- temporal primitives and builtins
- serialization / persistence primitives for native KB state
- any engine-level introspection needed by higher layers

ray-exomem owns:
- daemon process lifecycle
- MCP and HTTP endpoints
- KB registry and multi-KB orchestration
- persistence coordination and backup/restore workflows
- wrapper commands for query/assert/retract/load/export/merge/status/schema/explain
- CLI entry points and compatibility UX
- formatting and presentation of native engine results

### 3.2 Canonical data model

Teide semantics map onto rayforce2-native concepts as follows:
- Teide fact -> native relation row / datom in rayforce2
- Teide rule -> Rayfall list-style rule form
- Teide query -> Rayfall query form
- Teide directive -> native relation or KB metadata
- Teide interval -> dedicated temporal column(s) or interval relation supported by rayforce2
- Teide provenance tree -> native provenance graph / derivation metadata
- Teide confidence -> metadata derived from provenance or stored as an explicit native fact attribute
- Teide KB -> native persisted KB directory / catalog entry
- Teide merge -> native KB merge operation over relation sets plus metadata policy

## 4. Feature decision matrix

Legend:
- Keep = keep the user-visible capability
- Drop = remove from product scope
- Wrapper = keep as a thin ray-exomem layer over native engine features
- Move to rayforce2 = implement in the engine proper, then expose

### 4.1 Language / frontend path

- Rayfall list-style syntax as the initial spec: Keep
  - Canonical user input format.
  - Example classes: facts, rules, queries, datoms, assert-fact, retract-fact, pull, sym-name.
- Teide parser / source rewriting / translation: Drop
  - No frontend parsing of Teide syntax.
  - No source-to-source compatibility layer.
- Teide AST / rule builder API: Drop from ray-exomem
  - Any equivalent builder should be native rayforce2 API, not Teide-shaped plumbing.

### 4.2 Core reasoning

- Facts / rules / queries / recursive fixpoint: Keep, Move to rayforce2
- Negation / stratification: Keep, Move to rayforce2
- Semi-naive evaluation: Keep, Move to rayforce2
- Provenance / proof trees: Keep, Move to rayforce2
- Temporal intervals: Keep, Move to rayforce2
- Temporal builtins (`before`, `overlaps`, `meets`, `duration_since`, `decay`): Keep, Move to rayforce2
- MTL-like operators (`eventually_within`, `always_during`, `since_within`): Keep if achievable natively; otherwise block until rayforce2 supports them
- Query optimization / magic sets: Keep if native; otherwise wrapper-level optional optimization only if it does not require source translation

### 4.3 KB lifecycle and persistence

- KB list / create / rename / archive / unarchive / delete: Keep, Wrapper
- KB merge: Keep, Wrapper initially; promote merge policy hooks into rayforce2 metadata if they become core semantics
- Persist on shutdown and on mutation: Keep, Wrapper around native persistence APIs
- Symbol table round-trip: Keep, Move to rayforce2
- Splayed / columnar persistence: Keep, Move to rayforce2
- Export / import / backup restore: Keep, Wrapper over native serialization

### 4.4 User-facing tools and endpoints

- CLI (`run`, `eval`, `version`): Keep, Wrapper
- MCP server: Keep, Wrapper
- HTTP admin API: Keep, Wrapper
- Embedded/disk UI: Keep if product still wants it, Wrapper
- SSE / event stream: Keep, Wrapper
- Logs, graph, clusters, fact detail, relation graph, clear, drop-relation: Keep only if backed by native metadata; otherwise re-scope as admin-only views and not core engine features

## 5. Exact semantic mappings

### 5.1 Query / assert / retract

- assert-fact
  - rayforce2-native equivalent: insert a row into a KB relation or datoms table.
  - Must preserve optional interval, source, and confidence metadata.
- retract-fact
  - rayforce2-native equivalent: remove row or mark tombstone, depending on persistence model.
  - soft retract should end the interval / mark inactive, not delete history.
- query
  - rayforce2-native equivalent: native query execution against the loaded KB.
  - direct relation dump is a convenience wrapper, not a separate execution model.

### 5.2 Load / export / merge

- load
  - canonical meaning: ingest native Rayfall source or structured KB payload into the active KB.
  - no Teide parsing, no conversion from Teide syntax.
- export
  - canonical meaning: emit a self-contained native Rayfall / KB snapshot that can be reloaded by rayforce2.
  - export should preserve rules, facts, metadata, provenance references, and intervals where applicable.
- merge
  - canonical meaning: union of KB state with explicit conflict resolution policy for metadata and temporal overlap.
  - conflict semantics should be explicit and deterministic.

### 5.3 Schema / status / explain

- schema
  - native meaning: relation catalog, arity, field types, row counts, interval coverage, provenance availability, and sample tuples.
- status
  - native meaning: engine health, KB load state, persistence state, evaluation state, and versioning.
- explain
  - native meaning: provenance tree for a derived tuple, rendered from engine provenance metadata.

### 5.4 Temporal behavior

- Temporal intervals stay first-class.
- Hard requirement: a tuple can still carry start/end metadata and participate in interval-aware rules.
- Native expression:
  - as relation columns, interval relation, or engine-native interval metadata; but not through frontend rewriting.
- If rayforce2 lacks a required temporal feature, the feature belongs in rayforce2 proper before ray-exomem exposes it.

### 5.5 Multi-KB semantics

- KB is the atomic persisted unit.
- Default KB remains supported for ease of use, but the registry must be explicit.
- Cross-KB query and merge are read-oriented orchestration features, not engine-level translation passes.
- KB archive/delete must update registry and persistence atomically.

## 6. What to keep, what to drop, what to re-implement as wrappers

### Keep
- Native rayforce2 execution path
- Rayfall list-style source as canonical input
- User-visible workflows: query, assert, retract, load, evaluate, explain, status, schema, export, merge, KB management
- CLI / daemon / MCP / HTTP surfaces

### Drop
- Teide parser
- Teide syntax compatibility mode
- Source rewriting / transpilation
- Any frontend dependency on Teide AST or Teide-specific rule builders
- Any “dual evaluator” behavior that falls back to a Teide-style engine path

### Re-implement as wrappers
- CLI commands
- MCP tools and JSON schemas
- HTTP routes and response formatting
- provenance tree formatting
- schema/status summaries
- KB registry management
- export-to-file convenience
- cross-KB orchestration and merge policies

### Move into rayforce2 proper
- Datalog core semantics
- relation catalog / base relation registry
- provenance store
- temporal operators / intervals
- persistence primitives
- query optimization needed for native KB workloads
- any native merge / import / export format support that is core to the runtime

## 7. Dependency order / implementation phases

### Phase 0: Contract freeze and inventory — DONE
Deliverables:
- freeze the canonical Rayfall syntax subset for exomem ✓ (this document)
- enumerate which teide-exomem behaviors are parity requirements ✓ (§2, §4)
- define the native KB metadata schema ✓ (§5)
- define compatibility boundaries: what is native, what is wrapper-only, what is deprecated ✓ (§4, §6)

Exit criteria:
- a single approved schema for KB metadata, provenance, and temporal fields ✓
- no remaining user-facing dependency on Teide syntax ✓

### Phase 1: rayforce2 engine capabilities — PARTIAL
Deliverables:
- native KB / relation API ✓ (datoms, assert-fact, retract-fact, query available via ray_eval_str)
- native query / rule / fact primitives ✓
- provenance plumbing — TODO: blocked on rayforce2 provenance storage format
- temporal interval support — TODO: blocked on rayforce2 native interval representation
- persistence read/write primitives — TODO: blocked on rayforce2 persistent KB catalog
- any required catalog and merge hooks — TODO: blocked on rayforce2

Exit criteria:
- all core Datalog semantics can be exercised without ray-exomem translation ✓
- sample KB round-trips through persistence and reload — blocked

### Phase 2: ray-exomem core wrappers — IN PROGRESS
Deliverables:
- thin Rust wrapper around native rayforce2 KB APIs ✓ (lib.rs: assert_fact, retract_fact, query, define_rule, create_kb)
- run / eval / version CLI commands ✓ (main.rs)
- load CLI command ✓ (alias for run)
- native load / query / assert / retract wrappers ✓
- export / schema / status wrappers — TODO: requires rayforce2 introspection APIs
- direct formatting for tables, proof trees, and summaries — partial (rayforce2 ray_fmt handles table formatting)

Exit criteria:
- every public command runs against native engine state only ✓
- no parser/translator modules in the frontend path ✓ (verified by guardrail tests)

### Phase 3: daemon and APIs — NOT STARTED
Deliverables:
- MCP server backed by native KB APIs
- HTTP API backed by native KB APIs
- SSE/event notifications from real engine state changes
- admin UI bindings to wrapper endpoints

Exit criteria:
- MCP and HTTP parity for the supported feature set
- tool responses are derived from native engine state, not re-parsed text

### Phase 4: multi-KB and persistence parity — NOT STARTED
Deliverables:
- registry-backed KB lifecycle
- create / rename / archive / unarchive / delete
- load-on-demand KBs
- save-all on shutdown
- merge semantics and conflict policies

Exit criteria:
- restart preserves KB registry and data
- merge/reload behavior matches the working spec

Note: blocked on rayforce2 persistent KB catalog and symbol round-tripping.

### Phase 5: temporal and provenance parity — NOT STARTED
Deliverables:
- interval-aware queries and exports
- proof-tree explainability
- confidence / source metadata mapping
- temporal builtins and any missing MTL operators moved to rayforce2 proper

Exit criteria:
- temporal example programs run natively
- explanations are stable and reproducible

Note: blocked on rayforce2 provenance storage format, temporal interval native representation, and MTL operators.

### Phase 6: migration cleanup — PARTIALLY DONE
Deliverables:
- remove stale Teide-only docs and code paths ✓ (no Teide code exists in ray-exomem)
- update examples to Rayfall-native programs only ✓ (all examples are native Rayfall)
- ensure the project tests fail if Teide translation sneaks back in ✓ (guardrail tests in lib.rs)

Exit criteria:
- no Teide parser references in production path ✓
- documentation matches native behavior ✓

## 8. Validation and test strategy

### Engine-level tests
- parse / evaluate / query / export / import round trips on native Rayfall programs
- recursive fixpoint regression tests
- negation and stratification tests
- provenance tree correctness tests
- interval reasoning tests
- merge conflict tests

### Wrapper tests
- CLI smoke tests
- MCP tool contract tests
- HTTP endpoint contract tests
- persistence restart tests
- multi-KB lifecycle tests
- cross-KB query tests

### Migration guardrails
- fail the build if the frontend path imports any Teide parser module
- fail the build if a public command accepts Teide syntax on the normal path
- require fixture-based golden outputs for query, schema, status, explain, and export

## 9. Product-level acceptance criteria

ray-exomem is done when:
- all public knowledge-base workflows work through native rayforce2 only
- the canonical syntax is Rayfall list-style syntax
- the daemon/API surface covers the old teide-exomem user workflows
- persistence and provenance survive restart
- temporal behavior is supported natively wherever promised
- no frontend translation layer remains

## 10. Recommended naming discipline

- Use rayforce2 terminology in new code and docs.
- Keep teide-exomem names only in migration notes or compatibility docs.
- Prefer “KB”, “relation”, “datom”, “provenance”, “interval”, and “native Rayfall” in user-facing wording.

## 11. Explicit non-goals

- No Teide syntax parser in ray-exomem.
- No source-to-source conversion between Teide and Rayfall.
- No hidden compatibility mode.
- No reintroduction of a parallel reasoning engine path.

## 12. Open items that must be resolved in rayforce2 proper

These cannot be solved correctly in ray-exomem alone:
- provenance storage format
- persistent KB catalog and symbol round-tripping
- temporal interval native representation
- any missing MTL operators
- merge semantics for conflicting facts and metadata
- query optimization hooks for KB-scale workloads

If a feature requires one of the above, it belongs in rayforce2 proper before ray-exomem advertises it.

## 13. Clarification answers for the working spec

These answers are the default implementation contract unless the team explicitly revises them later.

### 13.1 Canonical Rayfall subset

- Canonical input is Rayfall list-style syntax only.
- Supported user-facing forms for v1:
  - facts / datoms
  - rules
  - queries
  - assert-fact
  - retract-fact
  - load / export
  - explain / schema / status
  - pull only if rayforce2 already exposes it natively
- No Teide syntax, no parser compatibility mode, no source rewriting, no AST translation.
- If a program needs a construct that rayforce2 cannot represent natively, the feature is blocked until rayforce2 supports it.

### 13.2 Boundary between rayforce2 and ray-exomem

- rayforce2 owns semantics: parsing, evaluation, query planning, provenance, temporal operators, persistence primitives, and native relation/catalog support.
- ray-exomem owns orchestration: daemon lifecycle, CLI/MCP/HTTP transport, exom registry, persistence coordination, formatting, and admin workflows.
- ray-exomem must not implement a second reasoning engine or fallback evaluator.
- Any feature that changes truth maintenance, rule evaluation, provenance derivation, or interval semantics belongs in rayforce2.

### 13.3 Exom persistence model

- An exom is the atomic persisted unit.
- One exom maps to one persisted directory / catalog entry and one active engine state.
- Persistence is snapshot-based and atomic: write to a temp location, fsync if available, then rename/commit.
- Mutations are durable on explicit commit and on clean shutdown; the default operational mode should be write-through for user-facing commands.
- Soft retracts preserve history by closing or tombstoning facts rather than deleting historical evidence.
- Export/import must round-trip facts, rules, queries, metadata, provenance references, symbol identities, and temporal intervals.

### 13.4 Provenance contract

- Base facts record their origin metadata: source, actor, timestamp/commit id, and optional confidence.
- Derived facts record the rule id plus the input tuple ids / provenance ids that produced them.
- Provenance identifiers must be stable across restart and export/import.
- explain renders a provenance DAG/tree from the stored derivation graph; it is not a best-effort reconstruction.
- Confidence is metadata, not a truth value. It may be derived from provenance, but it must not silently alter rule semantics.

### 13.5 Temporal semantics

- Time is represented as an explicit validity interval [start, end), with open-ended end meaning "currently active".
- Query evaluation is snapshot-aware: by default it answers against the current active snapshot; as-of queries filter by interval containment.
- Retract closes the active interval; it does not erase historical provenance.
- Interval ordering is logical/commit-based for reasoning. Wall-clock timestamps are audit metadata only.
- Temporal builtins such as before, overlaps, meets, and duration_since are first-class engine semantics, not wrapper-level filters.

### 13.6 User-facing surface scope

- v1 user-facing surface is intentionally small: CLI, MCP, and HTTP wrappers over the same native engine operations.
- Keep: run, eval, version, query, assert, retract, load, export, explain, schema, status.
- Admin-only / optional in v1: list, create, rename, archive, unarchive, delete, merge.
- De-scope for v1 unless native metadata support exists: relation graphs, clusters, fact detail views, and any UI feature that depends on ad hoc reconstruction rather than native engine state.

### 13.7 Multi-exom behavior

- Multiple exoms may exist in one daemon, but each exom is isolated.
- There is no implicit sharing of facts, rules, or temporal state across exoms.
- Default operations act on exactly one selected exom.
- Cross-exom behavior is explicit only:
  - copy_from clones one exom into a new exom
  - merge creates a new target exom using a deterministic conflict policy
  - any future federated query must be an explicit feature, not an accidental side effect
- Exom names are user-friendly aliases; stable internal ids or paths should be the authoritative reference.
- Registry operations must be atomic with the underlying exom metadata changes.

### 13.8 Assumptions behind these answers

- rayforce2 will expose native primitives for persistence, provenance, temporal intervals, and relation catalogs before ray-exomem advertises them.
- No Teide compatibility requirement remains for the frontend path.
- exom means a collection of facts, rules, queries, and related metadata, and it is the unit of persistence, isolation, and administration.
