# Ch10 — Reads (every builtin view + typed EDB + explain + export)

Sweep every read surface ray-exomem advertises. Each subcheck must return at
least one row (zero rows is a fail unless explicitly noted).

## Builtin views

For each, run a `(query <session> ...)` against the scratch session.

| View              | Query                                                                          | Min rows |
|-------------------|--------------------------------------------------------------------------------|----------|
| `fact-row`        | `(find ?id ?p ?v) (where (fact-row ?id ?p ?v))`                                | ≥ 5 (Ch02 + Ch03 + Ch08) |
| `fact-meta`       | `(find ?id ?meta) (where (fact-meta ?id ?meta))` (or whatever shape it exposes)| ≥ 1      |
| `fact-with-tx`    | `(find ?id ?p ?v ?tx) (where (fact-with-tx ?id ?p ?v ?tx))`                    | ≥ 5      |
| `tx-row`          | `(find ?tx ?u ?a ?m ?c ?br ?k ?x) (where (tx-row ?tx ?u ?a ?m ?c ?br ?k ?x))`  | ≥ 5      |
| `observation-row` | `(find ?o ?st ?b ?tx) (where (observation-row ?o ?st ?b ?tx))` (arity **4**: `?obs ?source_type ?content ?tx`; `obs/source_ref` not projected) | 2 (Ch05) |
| `belief-row`      | `(find ?bid ?text ?status ?tx) (where (belief-row ?bid ?text ?status ?tx))` (arity **4**; `belief/confidence` not projected) | 2 (Ch04: v1 revoked + v2 active — supersede is in-place, not a new row) |
| `branch-row`      | `(find ?b ?...) (where (branch-row ?b ?...))`                                  | ≥ 5 (main + agent-a + agent-b + probe-d + feature-x) |
| `claim-owner-row` | `(find ?fact ?owner) (where (claim-owner-row ?fact ?owner))`                   | ≥ 0 (depends on Ch09 — at least populated post-Ch09) |

If a view's exact arity drifts, capture the shape in evidence — don't fail
the whole chapter on arity unless `tx-row` ≠ 8 (already covered in Ch01).

## Typed EDBs

| EDB         | Min rows | Source chapter |
|-------------|----------|----------------|
| `facts_i64` | 2        | Ch02 #1, #2    |
| `facts_str` | 2        | Ch02 #3, #4    |
| `facts_sym` | 1        | Ch02 #5        |

## Explain

- **By predicate:** `mcp__ray-exomem__explain { exom: <session>, predicate: "test/n" }`.
  Expect a non-empty result that mentions `test/n` and the lowering rules.
- **By fact_id:** `mcp__ray-exomem__explain { exom: <session>, fact_id: "test/n" }`.
  Expect a non-empty result citing the tx that asserted it.

## Export

- `mcp__ray-exomem__export { exom: <session>, format: "json" }`. Verify the
  returned payload is parseable JSON and has at least the facts array.
  Capture byte length.
- `mcp__ray-exomem__export { exom: <session>, format: "jsonl" }`. The MCP
  tool emits one fact per line. Capture line count.

## Pass criteria

- Each view returns its `Min rows` floor.
- Each typed EDB returns its `Min rows` floor.
- Both explain calls return non-empty results.
- Both exports return parseable output.

## Evidence

- Per view: row count + first row (truncated).
- Per typed EDB: row count.
- Explain results (truncated).
- Export sizes.

## Notes

- This chapter runs **after** Ch02–Ch08 because it depends on the rows seeded
  there. If any row count is below the floor, the upstream chapter likely
  failed too — cross-reference before declaring this a real regression.
