# Ch05 — Observations (observe + obs/tag + obs/tx)

Verify the observation write path and that observations link back to tags +
their originating transaction.

## Steps

1. **Tagged observation** —
   `observe { exom: <session>, body: "First-pass review of API surface; everything except merge is MCP-exposed.", source_type: "github-pr", source_ref: "https://github.com/lynx/ray-exomem/pull/123", tags: ["a", "b", "c"] }`.
   Capture the returned observation id as `O1` and `tx_id`.

2. **Same source_type, different ref** —
   `observe { exom: <session>, body: "Follow-up — confirmed merge needs HTTP path.", source_type: "github-pr", source_ref: "https://github.com/lynx/ray-exomem/pull/124", tags: ["follow-up"] }`.
   Capture `O2`.

## Verification queries

- `observation-row` arity **4** (scope to scratch session):
  `(query <session> (find ?obs ?stype ?content ?tx) (where (observation-row ?obs ?stype ?content ?tx)))`.
  Expect **2 rows**. The view does NOT project `obs/source_ref` — that
  predicate lives on the entity (queryable via `(?obs 'obs/source_ref ?ref)`)
  but isn't part of the row-view tuple.

- Tag triples for `O1`:
  `(query <session> (find ?t) (where (?obs 'obs/tag ?t)) (where (= ?obs <O1>)))`
  — or, if your equality syntax differs, just project all `obs/tag` rows and
  filter client-side. Expect **3 distinct tags** for `O1`: `"a"`, `"b"`, `"c"`.

- Tx recoverability:
  `(query <session> (find ?obs ?tx) (where (?obs 'obs/tx ?tx)))`.
  Expect both `O1` and `O2` linked to non-empty `tx_id`s.

## Pass criteria

- `observation-row` returns exactly 2 rows.
- 3 `obs/tag` rows for `O1`.
- Both `obs/tx` rows present and non-empty.

## Evidence

- `O1`, `O2`, both tx_ids.
- Tag rows for `O1`.
- Tx-link rows for both observations.

## Notes

- `source_type` and `source_ref` are free-form strings — the test just
  requires they round-trip. Pick distinguishable but realistic values.
- Tags are a sub-EAV (`obs/tag` is a tuple-per-tag) — if `O1`'s tag count is 1,
  the multi-tag write was flattened to a single string somewhere.
