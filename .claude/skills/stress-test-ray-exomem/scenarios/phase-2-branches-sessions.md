# Phase 2 — Branches, sessions, scaffolding, regression probes

Single-context (orchestrator over MCP). All calls target `<session>` unless noted. Touches the branch lifecycle, session lifecycle, scaffolding tools (`init` / `exom_new` / `folder_new` / `rename` / `delete`), and the regression-class probes (Ch12 surface) that have repeatedly broken across releases.

## A. Branch lifecycle (Ch06 surface)

1. **Create from explicit parent** — `mcp__ray-exomem__create_branch { exom: <session>, branch_name: "feature-x", parent_branch_id: "main" }`. Capture row. Pass: `feature-x.parent_branch_id == "main"`.
2. **Assert on `feature-x`** — `assert_fact { exom: <session>, branch: "feature-x", fact_id: "fx/marker", predicate: "fx/marker", value: "branched" }`. Capture `T_fx`.
3. **Read on `feature-x`** — `query { exom: <session>, branch: "feature-x", query: "(query <session> (find ?id ?p ?v) (where (fact-row ?id ?p ?v)))" }`. Pass: row with `id="fx/marker"`, `value="branched"`.
4. **Read on main by omission** — same query, no `branch:` arg. Pass: `fx/marker` **not** in result (omitted branch defaults to `main`; branch isolation holds).
5. **Read on main explicitly** — same query with `branch: "main"`. Pass: same result as step 4. This is the regression marker that reads are branch-param driven, not cursor driven.
6. **list_branches** — pass: `feature-x` present, no row contains `is_current`, and `claimed_by_user_email` is populated for `feature-x` after step 2's write.
7. **Merge into explicit target** — `mcp__ray-exomem__merge_branch { exom: <session>, branch: "feature-x", target_branch: "main", policy: "last-writer-wins" }`. Capture `merge_tx_id` and `added`.
8. **Verify merge** — `(query <session> (find ?tx ?act ?target) (where (?tx 'tx/action "merge") (?tx 'tx/merge_target ?target)))` → row with `tx == merge_tx_id` and `target == "main"`. `fact-row` on main now contains `fx/marker`.
9. **Query feature after merge** — run the feature query again. Pass: `feature-x` still has `fx/marker`, confirming merge did not mutate selected branch state globally.
10. **Archive** — `mcp__ray-exomem__archive_branch { exom: <session>, branch: "feature-x" }`. Verify `(query <session> (find ?b ?a) (where (?b 'branch/archived ?a)))` → row for `feature-x` with `a == "true"`.

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

### D5. No branch cursor state

Covered by A.3-A.6: a `feature-x` read followed by omitted-branch and explicit-`main` reads must not leak feature rows into main, and `list_branches` must not expose `is_current`. Also verify `exom_status` does not expose `current_branch`.

### D6. Branch-param API + UI smoke

If a Chrome context is available, run one batched `evaluate_script` as user1:

1. Create a fresh branch `ui-feature` from `main`, assert `ui/branch-marker = "ui-feature"` on `branch: "ui-feature"`, and leave it unmerged/unarchived for this smoke.
2. Visit `/tree/<session>?branch=ui-feature`.
3. Fetch `/api/facts?exom=<session>&branch=main` and `/api/facts?exom=<session>&branch=ui-feature`. Pass: the branch-local fact appears only in the branch view, while inherited main facts appear in the branch view.
4. Fetch `/api/beliefs?exom=<session>&branch=ui-feature`. Pass: response has `branch == "ui-feature"` and only branch-visible active beliefs.
5. Fetch `/api/observations?exom=<session>`, then fetch it again after changing the page URL's `branch` search param. Pass: same observation ids in both responses; observations are exom-level.
6. Fetch `/api/schema?exom=<session>&branch=ui-feature` and `/api/relation-graph?exom=<session>&branch=ui-feature`. Pass: both reflect the selected branch.
7. On the page, clicking a branch in the Branches section updates `window.location.search` to `?branch=<branch_id>` and does **not** call any `/switch` endpoint. The visible section order is `Branches -> Connections -> Facts -> Beliefs -> Timeline`, then an exom-level band containing `Observations | Rules`.

## Pass criteria

- A: branch lifecycle works including explicit parent, explicit branch reads/writes, explicit-target merge, archive, and no cursor fields.
- B: all four session error paths fire correctly (bad label, unknown agent, session_closed, closed_at meta).
- C: scaffolding tools all return ok and the resulting tree shape matches.
- D1: 0 rows for the wrong attr name.
- D2: 2 value-intervals on the default fact_id.
- D3: sym query returns rows without a domain error.
- D4: claim triple populated immediately, no second list call needed.
- D5/D6: no `current_branch` / `is_current`; branch-param API/UI smoke behaves as branch-view vs exom-level split.

## Evidence

A: `T_fx`, `merge_tx_id`, target branch, archive flag value, and a sample branch row proving `is_current` is absent.
B: each session_new response, the bad-label and session_closed error strings verbatim.
C: `<scratch_bare>` path; rename roundtrip; delete error string.
D1: row count (must be 0).
D2: `fact_history` summary.
D3: row count (≥ 1) and a confirmation that the response was not RAY_ERROR.
D4: `list_branches` result for `probe-d` showing the claim triple.
D5/D6: status/list branch field absence; URL after branch click; facts/beliefs/observations/schema/relation-graph response summaries.

## Notes

- `<scratch_bare>` is created here, not in Phase 0, because Phase 0 keeps setup minimal. Phase 3 references this path for fork probes.
- Step A.7's merge-tx query relies on the rule-call rewriter's literal-pin path. If A.7 returns 0 rows, the same probe in Phase 4 ("server-side arity error" or other Rayfall regressions) will likely also fail; check the `derive_rule_param_attrs` regression marker.
- D4's cache-staleness probe is the only place the `probe-d` agent slot is exercised; that's deliberate so Phase 5's `agent-a` / `agent-b` TOFU claims aren't disturbed.
