# Ch02 — Typed values (assert_fact + facts_i64/str/sym)

Verify the I64/Str/Sym router (`src/fact_value.rs`) ingests the right type
for each shape, and that the parallel typed EDBs (`facts_i64`, `facts_str`,
`facts_sym`) carry the rows.

## Calls

All `assert_fact` calls go to `<session>` (no `branch:` arg → defaults to `main`).
Capture each returned `fact_id` and `tx_id`.

| # | predicate          | value                  | expected EDB |
|---|--------------------|------------------------|--------------|
| 1 | `test/n`           | `84` (JSON number)     | `facts_i64`  |
| 2 | `test/n_str`       | `"75"`                 | `facts_i64`  |
| 3 | `test/zerolead`    | `"007"`                | `facts_str`  |
| 4 | `test/plus`        | `"+5"`                 | `facts_str`  |
| 5 | `test/active`      | `{ "$sym": "active" }` | `facts_sym`  |

Use distinct `fact_id` for each (e.g., `test/n`, `test/n_str`, …). Don't reuse;
Ch12 covers default-id collision.

## Verification queries

After all 5 asserts:

1. `mcp__ray-exomem__query`:
   - `exom: <session>`
   - rayfall: `(query <session> (find ?e ?a ?v) (where (facts_i64 ?e ?a ?v)))`
   - Expect **exactly 2** rows. The values must be `84` and `75` (as i64s).

2. `(query <session> (find ?e ?a ?v) (where (facts_str ?e ?a ?v)))` — expect
   exactly 2 rows, values `"007"` and `"+5"`.

3. `(query <session> (find ?e ?a ?v) (where (facts_sym ?e ?a ?v)))` — expect
   exactly 1 row, value the sym `active`.

4. **Cmp filter test:** `(query <session> (find ?e ?v) (where (facts_i64 ?e "test/n" ?v) (< ?v 100)))`
   — expect exactly **1** row containing `84`. This proves that numeric
   comparisons on the i64 column run natively.

## Pass criteria

Every row count above matches exactly. Type routing is determined by the
fact_id's predicate-name + value shape; if `facts_i64` returns 0 rows the
typed-value router regressed.

## Evidence

- Each fact_id + tx_id (record into report row).
- The 4 query result tuples (paste verbatim).
- If `facts_i64` row-count is wrong, paste the full set of 5 returned
  values from a `(query <session> (find ?e ?a ?v) (where (fact-row ?e ?a ?v)))`
  scan to show what the router actually produced.

## Notes

- The fact_ids produced here are the seeds for Ch04 (`belief/supports`) and
  Ch10 (typed-EDB read coverage). Don't re-use predicates across chapters.
- The two i64 rows are the only "numeric" facts in the scratch session — Ch12's
  cmp probe relies on that.
