# ray-exomem Datalog-Native Enhancement Plan

Date: 2026-04-09

## North Star

`ray-exomem` should remain a Datalog-first persistent memory daemon:

- Rayfall/Datalog is the canonical write and query model.
- HTTP, CLI, UI, and future MCP layers compile to that model instead of defining parallel memory semantics.
- Facts, provenance, valid-time, tx-time, branches, and coordination state should be representable as relations or as queryable facts derived from one substrate.
- Structured helper endpoints may exist, but only as ergonomic adapters over the same semantics.

## What Was Confirmed In Live Testing

The current rebuilt daemon was exercised in background mode against a disposable exom:

- `start-session` works.
- plain `assert` works through Rayfall eval.
- structured bitemporal `assert` works.
- `status`, `export`, `retract`, and `why-not` work.
- `query` works again from the CLI once an actor header is supplied.

Two server regressions were found and fixed during this session:

1. mutation requests could deadlock because SSE emission tried to re-lock the daemon mutex.
2. `POST /api/actions/assert-fact` had been removed but the CLI still depended on it for valid-time asserts.

## Remaining Confirmed Issues

### 1. Query results are still engine-oriented, not agent-usable

`ray-exomem query` currently succeeds, but the response still contains encoded `i64` datoms in the `output` string rather than decoded symbols/strings.

This is the main remaining usability gap for the Datalog-native direction.

### 2. `query` is using `eval` transport rather than a query-specialized Rayfall path

The semantics are still acceptable because the source of truth is Rayfall, but the transport and output contract are too generic.

### 3. Export/read surfaces are still split awkwardly

Today the most reliable agent-readable surfaces are:

- `status`
- `export`
- `schema`
- `facts/<id>`
- `why-not`
- raw Rayfall `query`

These work, but they do not yet compose into one coherent Datalog-native read story.

### 4. Branch semantics are more correct than before, but still under-modeled

Branch state is durable enough to use, but branches are not yet fully exposed as first-class queryable graph data.

### 5. Observations and beliefs still lag behind facts in the public interface

The data model exists, but the public Datalog-facing ergonomics are still fact-centric.

## Design Rule For All Future Work

Before adding any feature, require all of the following:

1. The feature must have a Rayfall-level representation.
2. The feature must be queryable through the same substrate.
3. Helper APIs must compile to that representation rather than bypass it.
4. Advanced users must be able to reproduce the helper behavior directly with Rayfall and existing relations.

If a feature fails that test, it is architectural drift.

## Phase 1: Finish The Datalog-Native Core

### 1. Decoded query output over the existing engine

Goal:

- keep query semantics in Datalog
- keep `/api/actions/eval` valid
- decode result values before returning them to CLI/UI when the result is a query table

Implementation direction:

- add a result decoding layer at the HTTP boundary
- detect table-shaped query results
- map tagged datoms back into stable JSON scalars or typed cells
- preserve a raw mode for debugging

Deliverables:

- `ray-exomem query --json` returns decoded rows, not encoded `i64`
- UI query page renders decoded values without custom hacks

### 2. Separate “mutation eval” from “read eval” without splitting semantics

Goal:

- keep one Rayfall model
- stop forcing read-only queries through the same server-side path classification as mutations

Implementation direction:

- keep `/api/actions/eval` but classify forms early
- treat pure queries/rules inspection as read operations
- require `X-Actor` only for actual mutations

This is not a second API model. It is cleaner routing over the same Rayfall substrate.

### 3. Restore and harden all documented Datalog helper endpoints

Keep only the helper endpoints that are still compatible with the Datalog-first design:

- `POST /api/actions/assert-fact`
- `GET /api/facts/<id>`
- `GET /api/facts/valid-at`
- `GET /api/facts/bitemporal`
- `GET /api/actions/export`

Rules:

- every helper must map to existing Brain/Rayfall semantics
- helpers must not invent replacement semantics like canonical upsert layers

### 4. Remove deadlock classes around global state

Goal:

- no mutation path may hold the daemon mutex and then attempt re-entry through SSE, status, or helper calls

Implementation direction:

- keep SSE branch lookup non-blocking or precomputed
- audit all mutation handlers for “lock daemon -> call helper that re-locks daemon”
- add regression tests for concurrent status/export during mutation

## Phase 2: Make Querying Good Without Leaving Datalog

### 1. Add a proper query contract on top of Rayfall

Introduce a query-oriented CLI shape:

```bash
ray-exomem query --request '(query myexom (find ?e ?a ?v) (where (?e ?a ?v)))' --json
```

But improve the returned structure:

- `columns`
- `types`
- `rows`
- optional `pretty_output`

This remains Rayfall-native because the request is still Rayfall.

### 2. Add generated query helpers, not canonical data APIs

Examples:

- `why-not --predicate P --value V`
- `history <fact-id>`
- `facts --predicate P`

These should generate Rayfall queries or use existing Datalog-compatible helpers internally.

Do not add JSON-first memory query DSLs.

### 3. Make export/import the audit surface

Goal:

- export should be a reliable textual representation of current active state
- export-json should be the full audit/history surface

Refinements:

- `export` stays current-state Rayfall
- `export-json` stays full-state/history
- clearly document this difference in CLI and guide text

## Phase 3: Represent More Of The System As Queryable Data

### 1. Expose provenance and transaction structure more directly

Current provenance exists, but richer queryability is still weak.

Add queryable relations or generated Datalog views for:

- fact creation tx
- fact retraction tx
- actor
- session
- provenance/source
- valid interval
- branch membership/visibility

### 2. Make branch state queryable

Branches should be visible not only through bespoke endpoints but through queryable relations or generated views.

Examples:

- current branch
- branch ancestry
- branch creation tx
- archived state
- merge events

### 3. Promote observations and beliefs without inventing a second API model

Expose them through:

- Rayfall forms
- queryable relations/views
- helper commands that compile to those forms

Do not add separate “canonical belief APIs” that bypass the Datalog layer.

## Phase 4: Multi-Agent Memory Features That Preserve The Model

### 1. Coordination facts

Represent multi-agent coordination as facts:

- task claims
- ownership
- lease expiry
- handoff markers
- review requests

These should be stored as ordinary memory facts or as first-class queryable relations, not hidden server state.

### 2. Triggering and subscriptions

Allow agents to subscribe to changes, but base the trigger model on observed relation changes rather than opaque app-only events.

### 3. Scoped memory tiers using exoms plus queryable ancestry

Recommended structure:

- global exom
- project exom
- task exom
- speculative branch state

But the inheritance and visibility rules should be explicit and inspectable.

## Immediate Next Fixes

These should be the next concrete implementation tasks:

1. Decode query result tables for `ray-exomem query --json`.
2. Refine mutation-context enforcement so read-only queries do not look like anonymous mutations.
3. Add regression tests for:
   - daemon mode after fork
   - eval mutation plus concurrent status/export
   - valid-time assert via CLI
   - retract followed by why-not/export
4. Audit all mutation handlers for daemon-lock re-entry.
5. Document current helper endpoint semantics precisely in the operator guide.

## Acceptance Criteria

The next milestone is complete when all of the following are true:

- background daemon mode answers HTTP reliably after restart
- `query --json` returns decoded rows
- `assert`, valid-time `assert`, `retract`, `why-not`, and `export` all work from the CLI without special-case failures
- no remaining public route depends on a parallel canonical-memory JSON model
- branch, provenance, and temporal metadata are moving toward queryable Datalog-visible structure instead of hidden app-only state
