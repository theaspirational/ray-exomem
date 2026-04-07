# Bitemporal Follow-Up: 12-Feature Roadmap

Post-bitemporal implementation. Covers safety, performance, data integrity, observation parity, UI, and Datalog integration.

## Phase 1: Safety & Correctness (S each, no deps)

All three are independent — can be parallelized.

### F1: Timestamp Validation at Ingestion

Bitemporal queries use `valid_from <= timestamp` lexicographic comparison. Breaks silently on wrong format.

**Changes:**
- `brain.rs`: Add `fn validate_timestamp(s: &str) -> Result<()>` checking `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$` with valid ranges. Call at top of `assert_fact`, `revise_belief`, `assert_observation` on the `valid_from`/`valid_to` params.
- `web.rs`: Validate in `api_assert_fact_direct` and `api_facts_valid_at`/`api_facts_bitemporal` query params. Return 400 on bad format.
- `main.rs`: Validate `--valid-from`/`--valid-to` in Assert handler before hitting the daemon.

**Decision:** Validation function now, defer newtype `Iso8601(String)` to later.

### F6: `now_iso()` Unit Tests

Hand-rolled Hinnant calendar algorithm — zero test coverage.

**Changes:**
- `brain.rs`: Extract `fn unix_secs_to_iso(total_secs: u64) -> String` from `now_iso()`. Add tests:
  - Epoch zero → `1970-01-01T00:00:00Z`
  - `1704067200` → `2024-01-01T00:00:00Z`
  - Leap year Feb 29: `1709164800` → `2024-02-29T00:00:00Z`
  - Year boundary: `1735689599` → `2024-12-31T23:59:59Z`
  - Century leap: `946684800` → `2000-01-01T00:00:00Z`

### F12: Agent Guide Update

`agent_guide.rs` is missing the new bitemporal endpoints and CLI flags.

**Changes:**
- `agent_guide.rs` CLI section: Add `--valid-from`, `--valid-to` to `assert` entry. Note which `observe` flags are wired.
- HTTP section: Add `POST /api/actions/assert-fact`, `GET /api/facts/valid-at`, `GET /api/facts/bitemporal`.
- LIMITATIONS section: Update stale "flags not propagated" bullet. Note export annotations are comments.

---

## Phase 2: Performance (M + S, no deps)

### F2: Batch Retract / Bulk Operations

`api_clear` does 2N disk writes for N facts. `apply_eval_mutations` persists per-fact.

**Changes:**
- `brain.rs`: Add private `alloc_tx_deferred` that appends tx but skips persist. Add:
  - `pub fn retract_all(&mut self) -> Result<TxId>`: single tx, mark all active facts revoked, persist once.
  - `pub fn assert_facts_batch(&mut self, facts: &[...]) -> Result<TxId>`: single tx, push all, persist once.
- `web.rs`: `api_clear` calls `brain.retract_all()`. `apply_eval_mutations` batches assertions.
- Tests: verify `retract_all` creates one tx, all facts revoked.

**Decision:** Single transaction for batch ops (matches Datomic semantics).

### F5: Cache `current_facts()`

Every GET handler does linear scan + filter + Vec alloc.

**Changes:**
- `brain.rs`: Add `active_fact_indices: Option<Vec<usize>>` to `Brain`. Set to `None` on any mutation. `current_facts()` checks cache first. Rebuild from indices on cache hit.

**Depends on:** F2 (new `retract_all` must invalidate cache).

---

## Phase 3: Data Integrity (S + M + S)

### F3: Confidence + Provenance Through Eval Path

Eval path hardcodes `confidence: 1.0`, `provenance: "rayfall-eval"`.

**Changes (Option B — pragmatic):**
- `web.rs`: Add `default_confidence` and `default_provenance` query params to `api_eval`/`api_import` endpoints. Apply to all `AssertFact` mutations in `apply_eval_mutations`.
- `agent_guide.rs`: Document that `(assert-fact ...)` uses defaults, direct endpoint for full control.

**Option A (deferred):** Extend Rayfall syntax with `:confidence` `:provenance` kwargs — requires rayforce2 C work.

### F11: Export Round-Trip Fidelity

`;; @valid[from, to]` annotations are comments — lost on reimport.

**Changes:**
- `web.rs`: Make `api_import` a "smart import" instead of delegating to `api_eval`:
  1. Parse each line. For `(assert-fact ...) ;; @valid[...] @confidence=... @provenance=...`, extract metadata.
  2. Call `brain.assert_fact()` directly with extracted validity/confidence/provenance.
  3. Non-fact lines (rules, expressions) continue through eval.
- `web.rs` export: Extend annotation format to include confidence and provenance: `;; @valid[from, to] @confidence=0.9 @provenance=sensor`.
- `exomem.svelte.ts`: Update `parseFactsFromExport` to parse extended annotations.

### F9: Rule Editing Flow Validation

Edit-rule UI may strip exom name, producing a confusing parse error.

**Changes:**
- `exomem.svelte.ts`: In `parseRulesFromExport`, make exom name group optional in regex but flag it.
- `rules/+page.svelte`: In `parseDraftRule`, if 0 rules parsed, try lenient parse and return specific error: "Rule must include the exom name, e.g., (rule main (...) (...))".
- Alternative: auto-prepend `app.selectedExom` if missing.

---

## Phase 4: Observation Parity & UI (M + M, no external deps)

### F7: Observation API Parity

Observations have bitemporal fields but no UI, no query endpoint, CLI ignores its own flags.

**Changes:**
- `main.rs`: Fix `Observe` handler (currently ignores `source_type`, `source_ref`, `confidence`, `tags`). Add `--valid-from`/`--valid-to` flags. Use new direct endpoint instead of eval workaround.
- `web.rs`: Add `POST /api/actions/assert-observation` and `GET /api/observations` endpoints. Include validity in observation schema tuples.
- `ui/src/lib/types.ts`: Add `ObservationEntry` interface.
- `ui/src/lib/exomem.svelte.ts`: Add `fetchObservations()`, `assertObservation()`.
- `ui/src/routes/observations/+page.svelte`: New page — table of observations with source, content, validity, tags.
- `+layout.svelte`: Add nav entry.

### F8: Date-Range Filter on Validity Timeline

Timeline shows all temporal facts. Need "what was true on date X" query UI.

**Changes:**
- `timeline/+page.svelte`: Add `<input type="date">` above the timeline. When set, fetch from `/api/facts/valid-at?timestamp=<T>` instead of full export. Toggle between "All temporal facts" and "Facts valid at date".
- `exomem.svelte.ts`: Add `fetchFactsValidAt(timestamp, exom)` calling the existing endpoint.
- Visual: render selected date as a vertical line intersecting active validity bars.

---

## Phase 5: Datalog Integration (L + M, requires rayforce2)

### F10: Cross-Exom Rule Test

`restore_runtime` two-pass is designed for cross-exom rules but untested.

**Changes:**
- `tests/cross_exom.rs` (or web.rs test module): Integration test that:
  1. Creates two exoms ("alpha", "beta").
  2. Asserts facts in alpha.
  3. Adds rule in beta referencing alpha's DB.
  4. Calls `restore_runtime`, verifies derived facts in beta.
  5. Uses `global_test_lock()`.

### F4: Bitemporal Queries in Datalog Rules

No Datalog temporal predicates — `facts_valid_at` is Rust-only.

**Approach (materialized columns):**
- `storage.rs`: Modify `build_datoms_table` to include `valid_from` and `valid_to` as additional columns (5-col datoms instead of 3-col EAV).
- `web.rs`: Update `build_or_load_datoms` and schema to reflect extra columns.
- `rayforce2` (`feature/datalog-ops`): Add string comparison builtins (`string<=`, `string>`) so rules can filter: `(rule main (was-in-paris ?e) (main (location ?e "paris" ?vf ?vt)))` with comparison guards.
- Alternative: built-in `(valid-at ...)` predicate in C engine — more work, cleaner semantics.

**Decision needed:** Materialized columns vs built-in temporal predicate. Depends on whether rayforce2 has or can get string comparison builtins.

---

## Sequencing

```
Phase 1 ─── F1, F6, F12 (parallel, S each)
  │
Phase 2 ─── F2 → F5 (sequential, M+S)
  │
Phase 3 ─── F3, F11, F9 (mostly parallel, S+M+S)
  │
Phase 4 ─── F7, F8 (parallel, M each)
  │
Phase 5 ─── F10 → F4 (sequential, M+L, rayforce2 required)
```

Phases 1-4 are self-contained in ray-exomem. Phase 5 needs rayforce2 `feature/datalog-ops`.

## Open Design Decisions

| Feature | Question | Recommendation |
|---------|----------|----------------|
| F1 | Newtype vs validation function? | Validation function first |
| F2 | Single tx for batch or one-per-fact? | Single transaction |
| F3 | Extend Rayfall syntax or document limitation? | Document + query-param defaults now |
| F4 | Materialized columns vs built-in temporal predicate? | Materialized columns (less C work) |
| F11 | Smart import parser or extended Rayfall syntax? | Smart import parser |
