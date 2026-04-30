# Ch08 — Attribution (full triple, agent fallback, model omission, tx-row arity 8)

The original three-axis attribution battery, kept here so the matrix is
complete. Re-confirms what shipped at 2026-04-27.

## Steps

1. **Full triple** —
   `assert_fact { exom: <session>, fact_id: "attr/full", predicate: "attr/full", value: 1, agent: "stress-test-runner", model: "claude-opus-4-7" }`.
   Capture `T_full`.

2. **No `agent` arg → API-key-label fallback** —
   `assert_fact { exom: <session>, fact_id: "attr/no-agent", predicate: "attr/no-agent", value: 2, model: "claude-opus-4-7" }`.
   Capture `T_no_agent`.

3. **No `model` arg** —
   `assert_fact { exom: <session>, fact_id: "attr/no-model", predicate: "attr/no-model", value: 3, agent: "stress-test-runner" }`.
   Capture `T_no_model`.

## Verification queries

For each tx, run **two** views — strict `tx-row` and direct EAV — to know
exactly what's stored:

A. `tx-row` strict view (arity 8: tx, user_email, agent, model, created_at, branch, mutation_kind, source-or-payload-tail):
   `(query <session> (find ?tx ?u ?a ?m ?c ?br ?k ?x) (where (tx-row ?tx ?u ?a ?m ?c ?br ?k ?x)))`.
   Capture all rows for `T_full`, `T_no_agent`, `T_no_model`.

B. Direct EAV per tx:
   `(query <session> (find ?p ?v) (where (?tx ?p ?v)) (where (= ?tx <T_no_model>)))`
   — or equivalent. Inspect whether `tx/model` is missing or empty-string.

## Pass criteria

- **`T_full`:** appears in `tx-row` with full triple; `?u == <user_email>`,
  `?a == "stress-test-runner"`, `?m == "claude-opus-4-7"`.
- **`T_no_agent`:** appears in `tx-row` with `?a` == the API-key label that
  the bearer/MCP key was registered under (NOT empty). The runner must be
  able to read that label from the response or `/auth/api-keys` — if it
  can't, fall back to "non-empty string" as the criterion.
- **`T_no_model`:**
  - **Strict tx-row behavior (load-bearing):** the row is **kept** in the
    strict view with `?m == ""` (empty-string sentinel). It does NOT drop
    out. Every downstream consumer — UI render (`by … using {model}` with
    `using` elided when empty), Ch12-A hyphen-attr probe, attribution
    audits — depends on this. If the row drops out of `tx-row`, that's a
    regression: fail Ch08.
  - **Direct EAV** confirms `tx/model` is the empty-string sentinel `""`,
    not absent.
- All three rows in the strict view have arity **8**.

## Evidence

- `T_full`, `T_no_agent`, `T_no_model`.
- Verbatim `tx-row` rows for each tx (or note "not present in strict view").
- Verbatim direct-EAV rows for `T_no_model`.
- API-key-label fallback target value (record it explicitly).

## Notes

- This chapter is a **regression** chapter for the 2026-04-27 attribution
  ship. If you discover `tx/user_email` returning empty when full-bearer auth
  was used, that's the regression target.
- `mutation_kind` in the 8-tuple distinguishes assert from retract from
  belief from observe — `T_full` should show `assert` (or whatever the codename
  is in `system_schema.rs`).
