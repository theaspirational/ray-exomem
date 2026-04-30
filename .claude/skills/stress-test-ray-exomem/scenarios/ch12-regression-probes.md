# Ch12 — Regression probes (known historical bugs)

Each probe targets a specific historical regression and fails loudly if it
re-surfaces. Don't soften the criteria — these are the canaries.

## Probes

### A. Hyphen attr probe (tx/user_email rename)

The 2026-04 attribution rename made `tx/user_email` (underscore) the canonical
predicate. The hyphen form `tx/user-email` should not exist.

- `(query <session> (find ?tx ?e) (where (?tx 'tx/user_email ?e)))` — expect
  **>0 rows**. Any tx from Ch02–Ch08 will satisfy this.
- `(query <session> (find ?tx ?e) (where (?tx 'tx/user-email ?e)))` — expect
  **0 rows**. If this returns >0 rows, the rename regressed.

### B. Default-fact-id silent supersede

Documented behavior, not a bug — but verify it still holds so callers don't
accidentally lose data.

- `assert_fact { exom: <session>, predicate: "probe/dup", value: 1 }` (no
  `fact_id`). Capture the auto-generated id (it should default to predicate
  name, but verify whatever the API returns).
- `assert_fact { exom: <session>, predicate: "probe/dup", value: 2 }` (no
  `fact_id`).
- `fact_history { exom: <session>, id: "probe/dup" }` (or whatever id was
  returned).
- Expect **2 history tuples**, not 2 separate facts. The second supersedes
  the first by default-id collision. Document this as the documented
  behavior, not a bug.

If `fact_history` returns 1 tuple (only the second), the supersede dropped
the first row — that *is* a regression.

### C. Sym health (RAY_ERROR domain)

Per CLAUDE.md, a sym-table upgrade hazard surfaces as `RAY_ERROR code=domain`
with empty message on any query. Issue a no-op-ish query:

- `(query <session> (find ?x) (where (fact-row ?x ?p ?v)))`.

Pass if the response is OK (rows or empty rows are both fine). Fail loud if
the response carries `RAY_ERROR code=domain`. Capture the verbatim error
string in evidence.

### D. Cache staleness post-join

The `tool_session_join` cache eviction shipped in Ch08 era. Verify it still
runs on the live daemon.

- `mcp__ray-exomem__session_join { session: <session>, agent_label: "probe-d", agent: "stress-test-runner", model: "<runner-model>" }`.
- *Immediately* (no other calls in between):
  `mcp__ray-exomem__list_branches { exom: <session> }`.
- The `probe-d` row must show full claim triple (`claimed_by_user_email`,
  `claimed_by_agent` == `"stress-test-runner"`, `claimed_by_model` == the
  runner's model) — no nulls.

If the claim triple is null on the immediate call but populates a few seconds
later, the cache eviction regressed.

*Why a dedicated `probe-d` label and not `agent-a`:* Ch09 (which runs after
Ch12) needs `agent-a` and `agent-b` claim triples to reflect their *sub-
agent's* `agent`/`model` args. TOFU keeps the *first* claim's audit fields,
so if Ch12-D claimed `agent-a` first, Ch09's sub-agent join would be
idempotent and Ch09's "list_branches: full claim triple per agent" check
would assert against the wrong identity. Setup pre-allocates `probe-d` as
a third agent label specifically for this probe, isolating its TOFU claim
from Ch09's invariants.

### E. Cross-branch cursor restoration

After a query against a non-current branch, the daemon's cursor must restore
to the previously-current branch.

- `query { exom: <session>, branch: "agent-a", rayfall: "(query <session> (find ?id) (where (fact-row ?id ?p ?v)))" }`.
- `mcp__ray-exomem__list_branches { exom: <session> }`.
- The `main` row must show `is_current: true`. The `agent-a` row must show
  `is_current: false`.

## Pass criteria

A through E each have a single binary check above. Each independently
contributes one row to the matrix.

## Evidence

- Per probe: row count or full error string.
- For probe B: the auto-generated fact_id and the 2 history tuples.

## Notes

- This chapter runs **before** Ch09 (the multi-agent chapter), so probe D
  catches regressions even when Ch09 is gated off. Probe D claims the
  dedicated `probe-d` branch (pre-allocated at setup time), so its TOFU
  claim doesn't touch `agent-a`/`agent-b` and Ch09's per-agent claim-triple
  invariants stay clean.
