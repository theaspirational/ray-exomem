# Ch11 — Error surface

Trigger every documented error class intentionally and check the daemon's
response. **All steps here expect a failure** — a success is itself a failure.

## Steps

1. **`unknown_exom`** —
   `query { exom: "public/does-not-exist/<run-id>", rayfall: "(query public/does-not-exist/<run-id> (find ?x) (where (fact-row ?x ?p ?v)))" }`.
   Expect MCP error with message containing `unknown_exom` (or `not found`).

2. **`unknown_branch`** —
   `query { exom: <session>, branch: "ghost", rayfall: "(query <session> (find ?x) (where (fact-row ?x ?p ?v)))" }`.
   Expect MCP error containing `unknown_branch`.

3. **`query missing database name`** —
   `query { exom: <session>, rayfall: "(query (find ?x) (where (fact-row ?x ?p ?v)))" }`.
   (No exom inside the rayfall body — should fail.) Expect error containing
   `missing` or `database` per current message.

4. **arity error (server-side validation)** —
   `query { exom: <session>, query: "(query <session> (find ?x) (where (fact-row ?x)))" }`.
   `fact-row` is declared arity 3; calling it with 1 var must be rejected by
   the server's `validate_body_atom`. Expect MCP error verbatim
   `rule 'fact-row' expects 3 args, got 1`. The MCP boundary stops the query
   before it reaches rayforce2 — no `__VM`-shadowing-era empty-message
   ambiguity.

   Also try: `query { exom: <session>, query: "(query <session> (find ?e ?p) (where (facts_i64 ?e ?p)))" }`.
   `facts_i64` is arity 3; this must also error with
   `rule 'facts_i64' expects 3 args, got 2`.

5. **`invalid 'value'`** —
   `assert_fact { exom: <session>, fact_id: "bad/array", predicate: "bad/array", value: [1,2,3] }`.
   Expect error containing `invalid` and `value`. Arrays aren't a supported
   value type.

6. **Missing / empty predicate** — two sub-cases:

   a. **Missing:** `assert_fact { exom: <session>, fact_id: "no/predicate", value: 1 }`.
      No `predicate` field. Expect MCP error containing `missing required
      parameter: predicate`.

   b. **Empty string:** `assert_fact { exom: <session>, fact_id: "empty/predicate", predicate: "", value: 1 }`.
      Expect MCP error with code `-32602` and message containing
      `invalid 'predicate'` and `non-empty`. (Brain layer is the trust
      boundary; server.rs and mcp.rs both validate eagerly.)

7. **`BranchOwned` *(legacy)* / Model A `forbidden`** *(gated
   `--with-collision-user`)* —
   The original probe expected the brain layer's `branch_owned` error when
   a collision user wrote to the runner's branch. Under **Model A**
   (shipped 2026-04-30) the auth layer intercepts first for `public/*`
   paths and returns `forbidden` with `write access denied`, *before* the
   branch layer runs. Two outcomes are now valid:

   - **Cross-user write to a `public/*` exom** → expect `forbidden` /
     `write access denied`. This is the canonical Model A path; covered
     in **Ch13 step 7c**. Mark this row `pass (covered Ch13)` if Ch13
     ran, or `skipped` with reason "no collision-user".
   - **Cross-user write to a *shared* `{owner_email}/...` exom** with a
     `read-write` share but the runner having claimed `main` → still
     hits the original brain-layer `branch_owned` error. Covered in
     **Ch09 step 5** under `--with-team` + `--with-collision-user`.
     Mark `pass (covered Ch09)` / `skipped`.

   If both Ch09 and Ch13 ran, take the more specific evidence (the verbatim
   error string from each) and record it here.

8. **`session_closed`** — already covered in Ch07 step 3. Mark this row
   as `pass (covered Ch07)` if Ch07 passed, or `fail (Ch07 step 3 failed)`.

9. **`BranchMissing`** — already covered in Ch07 step 6. Same handling.

## Pass criteria

- Every step that's expected to error **does** error.
- The error message contains the canonical substring listed above (or, for
  step 4, *any* non-OK response).
- A success response on any of these steps is a `fail` for that step.

## Evidence

- Per step, the verbatim error string returned (truncated at 500 chars).
- For step 4 specifically: indicate whether the error msg was empty (the
  `__VM` shadowing surfacing) or non-empty.

## Notes

- Some MCP transports map error codes — `-32000` (server error), `-32602`
  (invalid params). The skill records the code as evidence but doesn't
  fail on the specific number; the message substring is the canonical check.
- If the daemon **stops responding** after step 4 (the rayforce2 error path
  used to crash on certain inputs), abort the run and report. Don't continue.
