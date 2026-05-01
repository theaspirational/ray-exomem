# Phase 4 — Error surface

Trigger every documented error class intentionally. **All steps here expect a failure** — a success is itself a failure. Single-context (orchestrator over MCP).

## Steps

1. **`unknown_exom`** — `query { exom: "public/does-not-exist/<run-id>", rayfall: "(query public/does-not-exist/<run-id> (find ?x) (where (fact-row ?x ?p ?v)))" }`. Pass: error message contains `unknown_exom` (or `not found`).

2. **`unknown_branch`** — `query { exom: <session>, branch: "ghost", rayfall: "(query <session> (find ?x) (where (fact-row ?x ?p ?v)))" }`. Pass: contains `unknown_branch`.

3. **`query missing database name`** — `query { exom: <session>, rayfall: "(query (find ?x) (where (fact-row ?x ?p ?v)))" }` (note: no exom name inside the rayfall body). Pass: contains `missing` or `database`.

4. **Server-side arity error** — `query { exom: <session>, rayfall: "(query <session> (find ?x) (where (fact-row ?x)))" }`. `fact-row` is arity 3; calling it with 1 var must be rejected by `validate_body_atom`. Pass: error verbatim `rule 'fact-row' expects 3 args, got 1`. Also try `(query <session> (find ?e ?p) (where (facts_i64 ?e ?p)))` — must error with `rule 'facts_i64' expects 3 args, got 2`. The MCP boundary stops the query before it reaches rayforce2 — no `__VM`-shadowing-era empty-message ambiguity.

5. **Invalid value (array)** — `assert_fact { exom: <session>, fact_id: "bad/array", predicate: "bad/array", value: [1,2,3] }`. Pass: contains `invalid` and `value`.

6. **Missing required parameter** — `assert_fact { exom: <session>, fact_id: "no/predicate", value: 1 }` (no `predicate`). Pass: contains `missing required parameter: predicate`.

7. **Empty-string predicate rejected** — `assert_fact { exom: <session>, fact_id: "empty/predicate", predicate: "", value: 1 }`. Pass: code `-32602`, contains `invalid 'predicate'` and `non-empty`.

8. **Cannot archive `main`** — `archive_branch { exom: <session>, branch: "main" }`. Pass: contains `cannot archive branch 'main'`.

9. **Cross-references already covered earlier** — these aren't repeated here, just cross-marked:
    - **`session_closed`** → covered Phase 2 step B.4.
    - **`BranchMissing`** → covered Phase 2 step B.3.
    - **`branch_owned` (cross-user, public/* or rw-share)** → covered Phase 3 steps A.3, D.2, F.5.
    - **`fork_session_unsupported`** → covered Phase 3 step B.5.
    - **`acl_mode_not_applicable`** → covered Phase 3 step C.8.
    - **`not_creator` (mode flip)** → covered Phase 3 step C.7.

## Pass criteria

- Every step that's expected to error **does** error.
- The error message contains the canonical substring listed above.
- A success response on any of these steps is a `fail` for that step.
- For step 4 specifically: the error message is **non-empty** (regression marker for the `__VM` shadowing era).

## Evidence

- Per step: the verbatim error string returned (truncate at 500 chars).
- For step 4: explicit confirmation that the error message was non-empty.

## Notes

- Some MCP transports map error codes differently (`-32000` server error, `-32602` invalid params). Record the code as evidence but the message substring is the canonical check.
- If the daemon **stops responding** after step 4 (the rayforce2 error path used to crash on certain inputs), abort the run and report. Don't continue.
- Cross-references in step 9 mean this phase is shorter than the legacy Ch11 — duplicate probes were absorbed by the phases that already exercise the same surface in context.
