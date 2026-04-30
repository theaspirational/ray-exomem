# Ch03 — Bitemporal facts (valid_from / valid_to / supersede / retract)

Verify the bitemporal model: a fact's history is a chain of value-intervals
each carrying a `superseded_by` or `revoked_by` back-pointer, the chain joins
correctly (`prev.valid_to == next.valid_from`), and every revision carries
the full attribution triple (Gap E regression).

A retract is an EVENT (a tx, recorded in `tx-row` with `action = "retract-fact"`),
not a value-interval. So `fact_history` returns one row per asserted value;
the retract lives in the tx log.

## Setup

Use a dedicated fact_id for this chapter so it doesn't tangle with Ch02:
`bt/temp_c`.

## Steps

1. **Backfill assert** —
   `assert_fact { exom: <session>, fact_id: "bt/temp_c", predicate: "bt/temp_c", value: 21, valid_from: "2024-01-01T00:00:00Z" }`.
   Capture `tx_id` as `T1` and the row's `valid_from` (must equal the input)
   and `valid_to` (must be unset / null).

2. **Supersede** —
   `assert_fact { exom: <session>, fact_id: "bt/temp_c", predicate: "bt/temp_c", value: 22 }`.
   No explicit `valid_from`. Capture `tx_id` as `T2`.

3. **Explicit `valid_to`** —
   `assert_fact { exom: <session>, fact_id: "bt/temp_c", predicate: "bt/temp_c", value: 23, valid_to: "2030-01-01T00:00:00Z" }`.
   Capture `tx_id` as `T3`.

4. **Retract** —
   `retract_fact { exom: <session>, fact_id: "bt/temp_c" }`. Capture `tx_id`
   as `T4`.

5. **History** — `fact_history { exom: <session>, id: "bt/temp_c" }`.

6. **Tx-log retract event** —
   `query { exom: <session>, query: "(query <session> (find ?tx ?act) (where (?tx 'tx/action ?act) (= ?act \"retract-fact\")))" }`.

## Pass criteria

- `fact_history` returns **3 value-interval tuples** (NOT 4 — the retract is a
  tx event, not a value-interval).
- Tuple 1 (`T1`): `value == 21`, `valid_from == "2024-01-01T00:00:00Z"`,
  `valid_to == T2.valid_from`, `superseded_by == "tx/<T2>"`, `revoked_by == null`.
- Tuple 2 (`T2`): `value == 22`, `valid_to == T3.valid_from`,
  `superseded_by == "tx/<T3>"`, `revoked_by == null`.
- Tuple 3 (`T3`): `value == 23`, `valid_to == <T4.tx_time>`
  (NOT `2030-01-01T00:00:00Z` — retract overrides the explicit future
  projection), `superseded_by == null`, `revoked_by == "tx/<T4>"`.
- Chain integrity: `T1.valid_to == T2.valid_from` and `T2.valid_to == T3.valid_from`.
- **Every tuple** carries non-empty `tx_id`, `tx/user_email`, `tx/agent`, and
  `tx/model`. (Empty `tx/model` is acceptable only if the assert was made with
  `model: ""` or omitted — flag if any tuple has empty user_email or agent.)
- The cross-check query (step 6) returns **1 row**: `(<T4>, "retract-fact")`.
  Confirms the retract event lives in the tx log, not in `fact_history`.

## Evidence

- Tx ids `T1..T4`.
- Full `fact_history` response (paste verbatim).
- Chain-integrity check: list each `(prev.valid_to, next.valid_from)` pair.
- The retract-event row from the tx-log query.

## Failure modes

- 4 tuples → assert/retract is emitting a synthetic retract row into
  `fact_history`; the value-interval invariant is broken.
- Tuple 1's `valid_to == T4.tx_time` (the retract time) instead of
  `T2.valid_from` → supersede chain didn't close T1 when T2 landed (B1).
- Tuple 2's `valid_to == null` → supersede chain didn't close T2 when T3 landed.
- Tuple 3's `valid_to == "2030-01-01T00:00:00Z"` → retract did not override the
  explicit future projection (B2).
- Missing `superseded_by` / `revoked_by` back-pointers → fact_history is no
  longer surfacing the storage-layer pointers (B3 regressed).
- Missing `tx/model` on backfill tuple → Gap E regressed.
