# rayforce2-backed storage design outline for ray-exomem brain layer

## 1) Goal
Move the brain layer from a JSONL append-only prototype to a native rayforce2-backed event/transaction store while preserving immutable history, time-travel reconstruction, branchable evolution, and provenance/explainability.

## 2) Core storage model in rayforce2
Use event sourcing as the durable source of truth, with transactions as the primary indexable unit.

Recommended native schema concepts:
- tx: tx_id, tx_time, actor, action, branch_id, parent_tx_id, note, refs
- event: event_id, tx_id, kind, payload, created_at
- entity tables or datoms for domain state: fact, observation, belief, branch
- provenance edges: derived_from / supported_by / retracted_by / superseded_by
- branch metadata: branch_id, parent_branch_id, created_tx_id, head_tx_id, name, status

Event kinds should be minimal and composable:
- ObservationAsserted
- FactAsserted
- FactRetracted
- BeliefRevised
- BranchCreated
- BranchSwitched or branch-head advancement only if needed for auditability

Each event must be immutable; updates are represented as new events, never in-place mutation.

## 3) Immutable history
History is append-only and never rewritten.
- Every assertion, retraction, revision, or branch change creates a new tx/event.
- Current state is a derived view over the full event log.
- Deleted data is represented by tombstones/retractions rather than physical removal.
- History queries should always be able to reconstruct the exact visible state at any tx or branch head.

## 4) Current vs as-of reconstruction
Expose two reconstruction modes:
- Current: compute state from the latest committed head of a branch.
- As-of: compute state as of a specific tx_id or tx_time cutoff.

Implementation approach:
- Current view = fold all events reachable from branch head.
- As-of view = fold only events with tx_id <= target (or tx_time cutoff with a stable tie-breaker).
- Derived caches may be materialized, but the event log remains authoritative.

## 5) Branch support
Branches are first-class and cheap.
- A branch is a named pointer to a head tx/event frontier.
- New branches fork from a parent branch and inherit its visible history.
- Branch-specific writes append new tx/events with branch_id.
- Branch merge can be modeled as a separate tx action or a higher-level orchestration policy, but should not require rewriting the source branch.

Recommended semantics:
- Branch head advances only by new tx on that branch.
- As-of reconstruction is branch-aware: state = events reachable from that branch plus ancestry.
- Cross-branch comparison is a diff over two reconstructed views.

## 6) Retraction semantics as events
Retraction should be a semantic event, not deletion.
- RetractFact means “this fact is no longer active from this tx onward.”
- The original assertion stays in history.
- A fact can carry revoked_by_tx / superseded_by_tx metadata in derived views.
- If a later event re-asserts the same claim, it becomes a new assertion with a new tx, not a resurrection of the old row.

Suggested rule:
- current facts = asserted and not retracted on the active branch ancestry
- as-of facts = asserted before cutoff and not yet retracted by cutoff

## 7) Provenance / explain
Provenance should be native, queryable, and explainable.
- Each derived belief or fact should reference supporting txs/events and upstream entities.
- Explain should return the minimal causal chain: who asserted it, when, from what inputs, and which txs produced or invalidated it.
- Prefer graph-shaped provenance over string blobs so explain can be rendered or queried.
- Support both “why is this true?” and “why is this absent now?” by including retraction and supersession edges.

## 8) Migration path from JSONL
Migrate in phases to avoid data loss.
1. Read-only compatibility: ingest existing JSONL log into rayforce2 on startup.
2. Dual-write period: write new tx/events to rayforce2 while optionally mirroring JSONL for rollback.
3. One-way replay: build a converter that replays JSONL into the native tx/event schema.
4. Cutover: stop JSONL writes once parity checks pass.
5. Cleanup: keep JSONL import tooling only; remove it from the hot path.

Important mapping:
- existing JSONL event lines -> rayforce2 tx + event records
- existing fact.revoked_by_tx -> retraction event + derived tombstone metadata
- existing branch records -> branch metadata rows/events
- existing explain/history behavior -> native provenance and as-of reconstruction

## 9) Risks and tradeoffs
- More schema complexity up front: tx/event/provenance/branch layers add design overhead.
- Query cost: reconstructing as-of/current views may be expensive without materialized caches.
- Branch explosion: many short-lived branches can increase storage and indexing cost.
- Retraction ambiguity: need clear semantics for “soft retract,” “supersede,” and “invalidate.”
- Provenance volume: fine-grained explain graphs can become large.
- Migration risk: JSONL replay must preserve ordering, tx identity, and branch ancestry exactly.
- Consistency tradeoff: strong immutability means derived views may lag unless cache invalidation is disciplined.

## 10) Recommended incremental implementation plan
Phase 1: Define native schema
- finalize tx/event/branch/provenance tables or relations in rayforce2
- codify event enums and payload contracts

Phase 2: Implement append-only tx writer
- write transactions and events natively
- keep current JSONL behavior only as an adapter if needed

Phase 3: Add reconstruction APIs
- current view
- as-of(tx_id)
- fact_history / tx_history / branch_history

Phase 4: Add branch semantics
- create branch
- switch branch
- branch head tracking
- ancestry-aware reconstruction

Phase 5: Add provenance/explain
- causal links for assertions, derivations, retractions, and supersessions
- structured explain output

Phase 6: Migrate JSONL
- replay importer
- parity tests against old outputs
- cutover and deprecate JSONL hot path

Phase 7: Optimize
- materialized current views
- incremental indexes
- compact/GC policies for derived caches only, never for source history

## 11) Success criteria
- All state changes are represented as immutable rayforce2-native events/txs.
- Current and as-of views are reproducible from history.
- Branching works without rewriting history.
- Retraction is auditable and explainable.
- JSONL data can be replayed faithfully into the new model.
