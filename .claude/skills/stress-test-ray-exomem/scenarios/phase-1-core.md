# Phase 1 — Single-user core (discovery + typed values + bitemporal + beliefs + observations + reads)

One MCP-driven phase against `<session>`. Touches every read tool, every typed-value path, every bitemporal transition, beliefs, observations, the required builtin views, attribution, explain, and export. Run as a tight sequence — fact_ids and tx_ids from earlier steps are referenced by later steps.

All calls go to `<session>` with the orchestrator's bearer (transport: MCP). No `branch:` arg → defaults to `main`.

> **Source of truth.** Behaviour is defined by `mcp__ray-exomem__guide` and the rustdoc comments in `src/{brain,mcp,fact_value,system_schema}.rs`. If a step here contradicts those, the spec is wrong — fix the spec.

## A. Read-tool surface (Ch01 surface)

1. `mcp__ray-exomem__guide` — capture byte length. Pass: ≥ 2 KB.
2. `mcp__ray-exomem__list_exoms` — pass: ≥ 1 entry; one entry's path equals `<session>`.
3. `mcp__ray-exomem__exom_status { exom: <session> }` — pass: no `current_branch` field, `facts == 0`, `beliefs == 0`, `transactions == 0`. (The genesis tx `tx/0` isn't counted.)
4. `mcp__ray-exomem__list_branches { exom: <session> }` — pass: 4 branches (`main` claimed by orchestrator; `agent-a`, `agent-b`, `probe-d` unclaimed). No branch row has `is_current`.

## B. Typed values (Ch02 surface)

Run five `assert_fact` calls on `<session>`, capture each `fact_id` + `tx_id`:

| # | predicate         | value (JSON)           | expected EDB |
|---|-------------------|------------------------|--------------|
| 1 | `test/n`          | `84` (JSON number)     | `facts_i64`  |
| 2 | `test/n_str`      | `"75"` (JSON string)   | `facts_str`  |
| 3 | `test/zerolead`   | `"007"`                | `facts_str`  |
| 4 | `test/plus`       | `"+5"`                 | `facts_str`  |
| 5 | `test/active`     | `{ "$sym": "active" }` | `facts_sym`  |

Then verify:
- `(query <session> (find ?e ?a ?v) (where (facts_i64 ?e ?a ?v)))` → exactly **1** row, value `84` as i64.
- `(query <session> (find ?e ?a ?v) (where (facts_str ?e ?a ?v)))` → exactly **3** rows: `"75"`, `"007"`, `"+5"`.
- `(query <session> (find ?e ?a ?v) (where (facts_sym ?e ?a ?v)))` → exactly **1** row; value sym `active`.
- `(query <session> (find ?e ?v) (where (facts_i64 ?e "test/n" ?v) (< ?v 100)))` → exactly **1** row, `84` (cmp filter on i64).

This locks in the **no-silent-coerce** invariant: a JSON string of digits (`"75"`) does NOT coerce to `I64`. The `#[serde(untagged)]` ordering in `src/fact_value.rs` is load-bearing — JSON shape strictly determines the variant. If `"75"` ever lands in `facts_i64`, the typed-value router regressed by gaining an unwanted coerce path. Conversely, if a JSON-number assert returns 0 rows from `facts_i64`, the I64 path itself regressed.

## C. Bitemporal lifecycle (Ch03 surface)

Use a single `fact_id` (`bitemp/sky-color`) to drive the full transition chain:

1. **Backfill assert** — `assert_fact { exom: <session>, fact_id: "bitemp/sky-color", predicate: "weather/sky_color", value: "blue", valid_from: "2020-01-01T00:00:00Z" }`. Capture `T1`.
2. **Supersede same fact_id** — `assert_fact { exom: <session>, fact_id: "bitemp/sky-color", predicate: "weather/sky_color", value: "gray", valid_from: "2025-06-01T00:00:00Z" }`. Capture `T2`.
3. **Explicit valid_to closure** — `assert_fact { exom: <session>, fact_id: "bitemp/sky-color", predicate: "weather/sky_color", value: "purple", valid_from: "2026-01-01T00:00:00Z", valid_to: "2027-01-01T00:00:00Z" }`. Capture `T3`.
4. **Retract** — via `mcp__ray-exomem__retract_fact { exom: <session>, fact_id: "bitemp/sky-color" }` or eval form `(retract-fact <session> "bitemp/sky-color" 'weather/sky_color "purple")`. Capture the retract tx `T4`.

Then `mcp__ray-exomem__fact_history { exom: <session>, fact_id: "bitemp/sky-color" }`. Pass criteria:
- 3 value-interval rows: `(blue, 2020-01-01 → T2-time)`, `(gray, 2025-06-01 → T3-time)`, `(purple, 2026-01-01 → T4.tx_time)`.
- Back-pointers: T1.superseded_by_tx = T2; T2.superseded_by_tx = T3; T3.revoked_by_tx = T4.
- `valid_to` chains: T1.valid_to ≤ T2.valid_from; T2.valid_to ≤ T3.valid_from; **T3.valid_to == T4.tx_time** — retract overrides the explicit `2027-01-01` projection. This is documented retract semantics (`src/brain.rs::retract_fact` and the `retract_overrides_explicit_future_valid_to` unit test): "Even rows with an explicit future valid_to lose that projection — the assertion terminates now."
- The retract tx appears in the tx-log: `(query <session> (find ?tx ?act) (where (?tx 'tx/action "retract-fact")))` → ≥ 1 row.
- Every history row carries the full attribution triple (`tx/user_email`, `tx/agent`, `tx/model`).

## D. Beliefs (Ch04 surface)

1. `mcp__ray-exomem__believe { exom: <session>, claim_text: "the sky is blue", supports: ["test/n"] }` (the `supports` references the i64 fact_id from B). Capture `belief_id_1`.
2. **Supersede same belief_id** — `mcp__ray-exomem__believe { exom: <session>, belief_id: <belief_id_1>, claim_text: "the sky is blue at noon" }`. The view collapses to current state — this is in-place, not a separate row.
3. **Revoke** — `mcp__ray-exomem__revoke_belief { exom: <session>, belief_id: <belief_id_1> }`.
4. **Believe v2 (fresh id)** — `mcp__ray-exomem__believe { exom: <session>, claim_text: "rain is wet", supports: ["test/n_str"] }`. Capture `belief_id_2`.

Then verify on `main`:
- `(query <session> (find ?b ?c ?s ?tx) (where (belief-row ?b ?c ?s ?tx)))` → exactly **2** rows. `belief_id_1` has `status=revoked`, `claim_text="the sky is blue at noon"`. `belief_id_2` has `status=active`, `claim_text="rain is wet"`.
- `belief/supports` link to the Ch02-B fact_ids: `(query <session> (find ?belief ?fact) (where (?belief 'belief/supports ?fact)))` → 2 rows linking each belief to its support fact.

## E. Observations (Ch05 surface)

1. `mcp__ray-exomem__observe { exom: <session>, source_type: "log", content: "first probe", tags: ["smoke", "probe", "v1"] }`. Capture `obs_id_1`.
2. `mcp__ray-exomem__observe { exom: <session>, source_type: "log", content: "second probe" }`. Capture `obs_id_2`.

Then:
- `(query <session> (find ?obs ?s ?c ?tx) (where (observation-row ?obs ?s ?c ?tx)))` → exactly **2** rows.
- `(query <session> (find ?obs ?tag) (where (?obs 'obs/tag ?tag)))` filtered to `obs_id_1` → exactly **3** tag rows.
- Both observations' `obs/tx` are recoverable via `(?obs 'obs/tx ?tx)`.
- If an HTTP/cookie context is available, `GET /api/observations?exom=<session>` returns both observations regardless of selected UI branch; the rows include source fields, tags, confidence/valid interval, and origin branch metadata from their tx.

## F. Builtin-view sweep (Ch10 surface)

For each required builtin_view below, run a `(find …)` over its full arity and assert the expected row count. If the schema advertises additional builtin views (`merge-row`, `claim-status-row`, `task-dependency-row`, etc.), run their full-arity query too and record the row count, but do not fail only because an optional view is empty.

| view              | arity | expected row count |
|-------------------|-------|--------------------|
| `fact-row`        | 3     | exactly 5 (typed values from B; `bitemp/sky-color` was retracted in C and is filtered from the current-fact view) |
| `fact-meta`       | 5     | matches `fact-row` (= 5) |
| `fact-with-tx`    | 8     | matches `fact-row` (= 5) |
| `tx-row`          | 8     | ≥ 6 (each write was a tx) |
| `observation-row` | 4     | exactly 2 |
| `belief-row`      | 4     | exactly 2 |
| `branch-row`      | 5     | exactly 4 (main, agent-a, agent-b, probe-d) |
| `claim-owner-row` | 2     | exactly 0 — `claim-owner-row` projects the `claim/owner` coordination attribute (`src/system_schema.rs::CLAIM_OWNER`); this phase asserts no `claim/*` facts. Phase 5's `--with-team` run grows this only if it asserts coordination claims. |

Then a separate probe to verify branch-ownership attribution (the orchestrator's `main` claim from Phase 0):

```scheme
(query <session> (find ?b ?u ?a ?m)
       (where (?b 'branch/claimed_by_user_email ?u)
              (?b 'branch/claimed_by_agent ?a)
              (?b 'branch/claimed_by_model ?m)))
```

Pass: ≥ 1 row with `?b == "branch/main"`, `?u == <user1_email>`, and non-empty `?a` / `?m`. This is the right attribute path for "main is claimed by orchestrator" — `branch/claimed_by_*`, not `claim/owner`. Phase 2.D4 reuses the same EAV path for the `probe-d` cache-staleness probe.

## G. Attribution (Ch08 surface)

For one of the asserts in B, query the full `tx-row` projection:
```
(query <session> (find ?tx ?id ?u ?a ?m ?act ?w ?br) (where (tx-row ?tx ?id ?u ?a ?m ?act ?w ?br)))
```
For at least one row matching a write the runner did, all of `?u`, `?a`, `?m` must be non-empty and equal to the values the runner sent (orchestrator email + `claude-code-cli` + the model). If any column is empty when it should have a value, attribution regressed.

## H. Explain + export

`explain` has **two distinct surfaces** (`src/mcp.rs::tool_explain`):

- With `predicate`: returns the **current** facts under that predicate from `facts_on_branch` — id / value / confidence. Retracted facts are not included.
- With `fact_id`: returns the **tx-history events** that touched that fact — every assert and the retract.

Don't conflate the two — the by-predicate surface is current-state, not history.

1. `mcp__ray-exomem__explain { exom: <session>, predicate: "weather/sky_color" }` — pass: returns `{"facts": []}`. Every fact under that predicate was retracted in C, so the current-state list is empty.
2. `mcp__ray-exomem__explain { exom: <session>, fact_id: "bitemp/sky-color" }` — pass: returns 4 `events`: 3 × `assert-fact` (T1, T2, T3) + 1 × `retract-fact` (T4).
3. `mcp__ray-exomem__export { exom: <session> }` — pass: returns ≥ 200 bytes (canonical Rayfall script).
4. `mcp__ray-exomem__export { exom: <session>, format: "jsonl" }` (or whatever the JSONL flag is in the current MCP surface) — pass: returns ≥ 5 newline-separated JSON lines.

## Pass criteria

- Every step's expected count / shape matches.
- All four `tx_id`s captured from C are distinct and monotonically increasing.
- Attribution triple is non-empty in G.
- Explain by `fact_id` returns 4 events; explain by `predicate` returns the documented current-state list (empty here, by design — see H).

## Evidence

Per step: short summary (counts, fact_ids, tx_ids). For F: row count per view. For G: the verbatim tuple. For H: byte counts.

## Notes

- This phase is single-context (orchestrator only). The `<session>` is private (`<user1_email>/test/...`), so no other user can see the asserts; stable ground for cross-user phases later.
- Steps C and D both use last-write-wins on the same `fact_id` / `belief_id`. If C's history shows fewer than 3 value-intervals, the bitemporal supersede-chain logic regressed.
- The retract at end of C is intentional and load-bearing for F/H: it exercises `fact-row` filtering of retracted facts (F drops to 5) and the by-predicate `explain` empty-state (H.1). If you reorder the steps, recompute the expected counts.
- Step F's branch-claim probe (`?b 'branch/claimed_by_user_email ?u`) covers the orchestrator's `main` claim. Phase 5's multi-agent run grows that to 3 rows (main + agent-a + agent-b after first writes). `claim-owner-row` is a different attribute (`claim/owner` in the coordination namespace) and stays 0 unless the run asserts coordination claims.
