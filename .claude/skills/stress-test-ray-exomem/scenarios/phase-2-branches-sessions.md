# Phase 2 — Branches, sessions, scaffolding, regression probes

Single-context (orchestrator over MCP). All calls target `<session>` unless noted. Touches the branch lifecycle, session lifecycle, scaffolding tools (`init` / `exom_new` / `folder_new` / `rename` / `delete`), and the regression-class probes (Ch12 surface) that have repeatedly broken across releases.

## A. Branch lifecycle (Ch06 surface)

1. **Create** — `mcp__ray-exomem__create_branch { exom: <session>, branch_name: "feature-x" }`. Capture row.
2. **Assert on `feature-x`** — `assert_fact { exom: <session>, branch: "feature-x", fact_id: "fx/marker", predicate: "fx/marker", value: "branched" }`. Capture `T_fx`.
3. **Read on `feature-x`** — `query { exom: <session>, branch: "feature-x", query: "(query <session> (find ?id ?p ?v) (where (fact-row ?id ?p ?v)))" }`. Pass: row with `id="fx/marker"`, `value="branched"`.
4. **Read on main** — same query, no `branch:` arg. Pass: `fx/marker` **not** in result (branch isolation holds).
5. **list_branches** — pass: `feature-x` present. After step 4 (which queried `main`), `feature-x.is_current == false` and `main.is_current == true` (cross-branch cursor restoration; Ch12 regression).
6. **Merge** — `mcp__ray-exomem__merge_branch { exom: <session>, branch: "feature-x", policy: "last-writer-wins" }`. Capture `merge_tx_id` and `added`.
7. **Verify merge** — `(query <session> (find ?tx ?act) (where (?tx 'tx/action "merge")))` → row with `tx == merge_tx_id`. `fact-row` on main now contains `fx/marker`.
8. **Archive** — `mcp__ray-exomem__archive_branch { exom: <session>, branch: "feature-x" }`. Verify `(query <session> (find ?b ?a) (where (?b 'branch/archived ?a)))` → row for `feature-x` with `a == "true"`.

## B. Session lifecycle (Ch07 surface)

Most session-class probes need fresh sessions to avoid conflict with `<session>`. Drive against `<scratch_project>` for new sessions; close them as part of the test.

1. **Single-session** — `session_new { project_path: <scratch_project>, session_type: "single", label: "single-probe" }`. Capture `<single_session>`. List its branches: should be only `main` (no agent-* branches because no `agents` field).
2. **Bad label rejection** — `session_new { project_path: <scratch_project>, session_type: "multi", label: "bad/label", agents: ["a"] }`. Pass: error containing `invalid label`.
3. **Unknown agent rejection** — `session_join { session_path: <session>, agent_label: "ghost-agent" }`. Pass: error containing `BranchMissing`.
4. **Close blocks writes** — `session_close { session_path: <single_session> }`. Then `assert_fact { exom: <single_session>, fact_id: "post-close", predicate: "post/close", value: 1 }`. Pass: error containing `session_closed`.
5. **closed_at meta** — fetch `tree { path: <single_session> }`. Before close, `session.closed_at == null`. After close, `session.closed_at` is a timestamp. (Steps 1 and 4 above bracket this.)

## C. Scaffolding (Ch07 surface, init/exom_new/folder_new/rename/delete)

1. **`init` scaffolds project** — covered by Phase 0 step 4. Re-verify by reading `tree { path: <scratch_project> }`: must contain `main` (Exom) and `sessions/` (Folder).
2. **`exom_new` creates bare exom** — `exom_new { path: <scratch_project>/scratch_bare }`. Verify in tree: kind is `bare`, `created_by == <user1_email>`, `acl_mode == "solo-edit"`. Capture as `<scratch_bare>` (Phase 3 references this).
3. **`folder_new`** — `folder_new { path: <scratch_project>/empty-folder }`. Verify in tree: kind `folder`, no children.
4. **`rename`** — `rename { path: <scratch_project>/empty-folder, new_segment: "renamed-folder" }`. Verify the old path is gone and `<scratch_project>/renamed-folder` exists.
5. **`delete`** — `delete { path: <scratch_project>/renamed-folder }`. Verify gone from the tree. Then `delete` a non-existent path — pass: error containing `not found` or `bad_path`.

## D. Regression probes (Ch12 surface)

These have broken before and need explicit guards.

### D1. Hyphen attribute probe

`(query <session> (find ?tx ?u) (where (?tx 'tx/user-email ?u)))` — pass: returns **0 rows** (the canonical attr is `tx/user_email` with underscore; `tx/user-email` should not exist). If this returns rows, the symbol-name normalization regressed.

### D2. Default-fact-id supersede

`assert_fact { exom: <session>, predicate: "default-fid/probe", value: "v1" }` (no `fact_id` → default = predicate). Then `assert_fact { exom: <session>, predicate: "default-fid/probe", value: "v2" }`. Verify `fact_history` shows two value-intervals on the same default fact_id; the v1 row is superseded.

### D3. Sym health (no domain error on query)

`(query <session> (find ?e ?a ?v) (where (facts_sym ?e ?a ?v)))` after Phase 1 step B's sym assert. Pass: ≥ 1 row, no `RAY_ERROR code=domain` in the response. If the response is `RAY_ERROR code=domain` with empty msg, the rayforce2 sym-table layout regressed (see CLAUDE.md "__VM shadowing"); re-run after the canonical health probe at startup catches the regression.

### D4. Cache staleness post-join

`session_join { session_path: <session>, agent_label: "probe-d" }`. Then immediately `list_branches { exom: <session> }` — the `probe-d` row's `claimed_by_user_email` triple must be **populated** (with the orchestrator's email since the orchestrator is doing the join). If empty until a refresh, the session-cache eviction regressed (`tool_session_join` cache-invalidation).

### D5. Cross-branch cursor restoration

Already covered in A.5 — read on `main` after a `feature-x` query must restore `main.is_current == true`. Don't duplicate; just confirm the A.5 row passed and reference it here.

## Pass criteria

- A: branch lifecycle works including merge + archive; isolation + cursor correct.
- B: all four session error paths fire correctly (bad label, unknown agent, session_closed, closed_at meta).
- C: scaffolding tools all return ok and the resulting tree shape matches.
- D1: 0 rows for the wrong attr name.
- D2: 2 value-intervals on the default fact_id.
- D3: sym query returns rows without a domain error.
- D4: claim triple populated immediately, no second list call needed.

## Evidence

A: `T_fx`, `merge_tx_id`, archive flag value.
B: each session_new response, the bad-label and session_closed error strings verbatim.
C: `<scratch_bare>` path; rename roundtrip; delete error string.
D1: row count (must be 0).
D2: `fact_history` summary.
D3: row count (≥ 1) and a confirmation that the response was not RAY_ERROR.
D4: `list_branches` result for `probe-d` showing the claim triple.

## Notes

- `<scratch_bare>` is created here, not in Phase 0, because Phase 0 keeps setup minimal. Phase 3 references this path for fork probes.
- Step A.7's merge-tx query relies on the rule-call rewriter's literal-pin path. If A.7 returns 0 rows, the same probe in Phase 4 ("server-side arity error" or other Rayfall regressions) will likely also fail; check the `derive_rule_param_attrs` regression marker.
- D4's cache-staleness probe is the only place the `probe-d` agent slot is exercised; that's deliberate so Phase 5's `agent-a` / `agent-b` TOFU claims aren't disturbed.
