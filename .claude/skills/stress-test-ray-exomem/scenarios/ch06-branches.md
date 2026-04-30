# Ch06 — Branches (create / assert / isolation / list / merge / archive)

Verify branch lifecycle and isolation, plus merge and archive — all via MCP.

## Steps

1. **Create branch** —
   `mcp__ray-exomem__create_branch { exom: <session>, branch_name: "feature-x" }`.
   Capture the new branch row.

2. **Assert on `feature-x`** —
   `assert_fact { exom: <session>, branch: "feature-x", fact_id: "fx/marker", predicate: "fx/marker", value: "branched" }`.
   Capture `tx_id` as `T_fx`.

3. **Read on `feature-x`** —
   `query { exom: <session>, branch: "feature-x", query: "(query <session> (find ?id ?p ?v) (where (fact-row ?id ?p ?v)))" }`.
   Expect a row with `id="fx/marker"`, `value="branched"`.

4. **Read on main (default branch)** —
   same query, no `branch:` arg. The `fx/marker` row must **not appear**. (If
   it does, branch isolation regressed.)

5. **list_branches** —
   `mcp__ray-exomem__list_branches { exom: <session> }`. Expect `feature-x` in
   the list. After step 4 (which queried `main`), `feature-x.is_current` must
   be **false** and `main.is_current` must be **true**.

6. **Merge** —
   `mcp__ray-exomem__merge_branch { exom: <session>, branch: "feature-x", policy: "last-writer-wins" }`.
   Capture the response body's `tx_id` as `merge_tx_id` and `added` array.

7. **Verify merge** — query the tx-log for the merge row:
   `query { exom: <session>, query: "(query <session> (find ?tx ?act) (where (?tx 'tx/action ?act) (= ?act \"merge\")))" }`.
   Expect a row with `tx == merge_tx_id`. Also confirm `fact-row` on `main`
   now contains `fx/marker` (the merged-in fact).

8. **Archive** —
   `mcp__ray-exomem__archive_branch { exom: <session>, branch: "feature-x" }`.
   Verify with
   `query { exom: <session>, query: "(query <session> (find ?b ?a) (where (?b 'branch/archived ?a)))" }`
   — expect a row for `feature-x` with `a == "true"`.

## Pass criteria

- Step 1: branch creates without error.
- Step 3: returns the asserted fact.
- Step 4: returns 0 `fx/marker` rows on main (pre-merge).
- Step 5: list_branches has correct `is_current` flags after the cross-branch read.
- Step 6: merge returns `ok: true` with a non-empty `added` list and a
  `tx_id`.
- Step 7: merge tx exists in the tx-log; `fx/marker` is now visible on main.
- Step 8: archive flips the flag.

## Evidence

- Branch row from step 1.
- The `T_fx` tx id.
- Step 3 query result (with the marker row).
- Step 4 query result (verify empty wrt `fx/marker`).
- Step 5 list_branches output, focused on `is_current` per branch.
- Step 6 merge response.
- Step 7 merge-tx row from tx-log + fact-row scan on main.
- Step 8 archived flag.

## Notes

- The "cursor restoration" check after a cross-branch query is **also**
  exercised in Ch12; the duplication is intentional — Ch06 records "did the
  branch isolation work", Ch12 records "did the cursor restore".
- Cannot archive `main` — if the user-supplied test wiring tries to,
  `archive_branch` returns `MCP error -32000: cannot archive branch 'main'`.
  Out of scope for Ch06; handled in Ch11.
