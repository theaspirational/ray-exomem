# Ch13 — Model A: public ownership + fork

`public/*` is read-by-all, write-by-creator-only, fork-to-contribute. This
chapter verifies the auth↔storage layers agree (no more "auth says yes,
branch-TOFU says no" drift), the `created_by` stamp lands on every new exom,
and `exom_fork` mirrors the source faithfully with lineage stamped.

This chapter has two tiers:

- **Single-bearer probes (always run)** — Steps 1–6 use the runner's bearer
  alone; they cover the "creator can write, fork their own exom, lineage
  surfaces, sessions refuse fork" surface.
- **Cross-user probe (gated)** — Step 7 needs `--with-collision-user
  <bearer>` to confirm the auth-layer 403 (the *user-facing* Model A change).
  Skip with reason `no collision-user` otherwise.

When `--with-collision-user` is provided, the skill auto-implies
`--scratch public` (per existing setup) — Model A only changes behavior
inside `public/*`, so the cross-user probe is meaningless under private
scratch.

## Steps

1. **`created_by` stamp on a fresh exom** —
   Compose `<scratch_bare>` (already created in Ch07 step 8). Read it back:
   `mcp__ray-exomem__tree { path: <scratch_bare> }`. The returned Exom node
   must have `created_by == <user_email>`. If absent or empty, the
   ownership stamp regressed at scaffold time.

   Same check on `<scratch_project>/main` and `<session>` (both scaffolded
   during setup): both must show `created_by == <user_email>`.

2. **`forked_from` absent on non-fork** —
   The same three nodes from step 1 must NOT include `forked_from` (the
   field is `skip_serializing_if = "Option::is_none"`, so its absence in
   the JSON is the pass condition — don't accept `null` either; absence is
   the canonical state).

3. **Fork own exom into an explicit target** —
   `mcp__ray-exomem__exom_fork { source: <scratch_bare>, target: "<scratch_root>/<run-id>-fork-explicit" }`.
   Capture the returned `target` as `<fork_explicit>`. Response must
   include `ok: true`, `copied_facts: <count>` (matching the source's
   `current_facts()` length — typically 0 for a freshly-created bare exom
   from Ch07; if Ch07 wrote to it, the count must match), and a
   `forked_from` block containing `source_path == <scratch_bare>` and
   `source_tx_id` matching the source's tip.

   Then read it back: `mcp__ray-exomem__tree { path: <fork_explicit> }`.
   The Exom node must have `created_by == <user_email>` (the forker, not
   the source's creator) and `forked_from.source_path == <scratch_bare>`.

4. **Default-target auto-suffix** —
   `mcp__ray-exomem__exom_fork { source: <scratch_bare> }` (no `target`).
   The default is `{user_email}/{basename of <scratch_bare>}`. If that path
   already exists from a prior run, the daemon must auto-suffix
   (`...-2`, `...-3`, ...). Capture the returned `target`. Verify the
   path's basename matches one of the expected forms.

   Re-fork once more without target: must yield a different target path
   (next free suffix).

5. **Fork refuses session exoms** —
   `mcp__ray-exomem__exom_fork { source: <session> }`. Expect MCP error
   with code `-32000` and message containing `fork_session_unsupported`.
   A success here is a regression — sessions are time-bounded multi-agent
   contexts, not knowledge artifacts.

6. **Replayed facts attribution** —
   For `<fork_explicit>` (or any fork from step 3 with non-zero
   `copied_facts`), query
   `(query <fork_explicit> (find ?fact_id ?value ?u) (where (fact-row ?fact_id ?p ?value) (?fact_id 'fact/created_by ?tx) (?tx 'tx/user_email ?u)))`.
   Every row's `?u` must be `<user_email>` (the forker). The original
   source's tx attribution does NOT carry over — the fork is a new tx
   stream attributed to whoever ran fork.

7. **Cross-user 403 (gated `--with-collision-user`)** —
   With `--scratch public` in effect (auto-implied by `--with-collision-user`):

   a. **Setup:** runner creates `public/stress-test/<run-id>-modela-target`
      and asserts a fact under it (this exom now has
      `created_by = <user_email>`).

   b. **Collision user reads** — using the collision bearer, issue
      `mcp__ray-exomem__query { exom: <runner's public exom>, rayfall: "(query <path> (find ?p ?v) (where (fact-row ?f ?p ?v)))" }`.
      Must succeed with the runner's fact in the result.

   c. **Collision user writes** — using the collision bearer, attempt
      `mcp__ray-exomem__assert_fact { exom: <runner's public exom>, predicate: "intruder/mark", value: 1, fact_id: "intruder-1" }`.
      **Must error with `-32000` and message containing `forbidden` and
      `write access denied` — NOT `branch_owned`.** This is the
      headline Model A change: the auth layer rejects, not the branch
      layer. A `branch_owned` error here means the auth-layer
      `resolve_access` regressed back to `FullAccess` for `public/*`.

   d. **Collision user forks** — using the collision bearer, call
      `mcp__ray-exomem__exom_fork { source: <runner's public exom> }`.
      Default target is `{collision_user_email}/<basename>` and the call
      must succeed. The collision user becomes the owner of the fork;
      `forked_from.source_path` points back to the runner's exom.

   e. **Collision user writes to their fork** — using the collision
      bearer, `assert_fact { exom: <collision fork path>, predicate: "my/note", value: 1, fact_id: "note-1" }` must succeed.

## Pass criteria

- Steps 1–6 all pass with the runner's bearer alone. A failure here means
  Model A's *server-side* invariants regressed.
- Step 7 (with collision user) verifies the *cross-user* invariants. The
  most important sub-step is 7c — that's the contract Model A established.

## Evidence

- Per step: the path involved, the response body (or error string verbatim
  for negative tests), and any `forked_from` blocks.
- Step 7c specifically: paste the verbatim error message from the failed
  write attempt. It must include the substring `forbidden` AND
  `write access denied`. If it contains `branch_owned`, paste that too —
  it's the marker of the regression.

## Notes

- This chapter depends on `<scratch_bare>` from Ch07 step 8. If Ch07 was
  blocked, mark Ch13 as blocked too.
- Step 4's auto-suffix has a hard cap (100 retries in the daemon). On the
  unlikely chance both the bare suffix and `-2`...`-100` are taken, the
  daemon returns `fork_collision`; record that as the evidence and treat
  it as a pass for the negative path.
- Under `--scratch public`, the run leaves `public/stress-test/...` exoms
  visible to other users in the allowed domain. That's deliberate (the
  cross-user probe needs visibility); document the residual paths in the
  report's Notes section.
- The MCP `exom_fork` tool was added at the same time as Model A. If the
  tool doesn't appear in `tools/list`, fail step 1 with that as evidence.
