---
name: stress-test-ray-exomem
description: Drive ray-exomem end-to-end through MCP + a pair of isolated browser contexts, exercise every write/read surface, branches, sessions, attribution, bitemporal facts, typed values, fork lineage, co-edit/solo-edit mode flips, error surface, and cross-user permission. Produces a single pass/fail matrix. Use when the user says "stress test ray-exomem" / "regression-check exomem" / "/stress-test-ray-exomem", or after any change touching brain.rs / storage.rs / mcp.rs / server.rs / auth/access.rs / exom.rs / scaffold.rs / ui (sidebar/exom-view) / Rayfall AST.
---

# stress-test-ray-exomem

Drive ray-exomem end-to-end and emit a single pass/fail matrix. Probes are batched into **six phases** so a full run is ~6 top-level tasks instead of one-per-chapter; each phase touches dozens of features in a single round trip.

**Two transports, both required:**

1. **MCP** (`mcp__ray-exomem__*`) — single-user surface. The orchestrator's bearer drives discovery, asserts, queries, branches, sessions, etc. Fast and concise.
2. **Two isolated Chrome contexts** (`mcp__chrome-devtools__*`) — cross-user surface. Two cookie jars, two distinct `user_email` identities, minted from the daemon's loopback `--dev-login-email` allow-list. Used by Phase 3 (Model A + fork + co-edit) and the optional Phase 5 multi-agent collisions.

The Chrome path is **mandatory** when the matrix includes cross-user probes (default: yes). It replaces the legacy `--with-collision-user <bearer>` flag — no second OAuth bearer is needed; the daemon mints both sessions for any allow-listed email.

## Preconditions

A run will not start unless **all of these hold**:

1. `mcp__ray-exomem__guide` returns markdown (`> 2 KB`). If `Method not found`, the MCP server isn't connected; abort with that as evidence.
2. `mcp__chrome-devtools__list_pages` returns successfully. If the Chrome MCP isn't connected, abort with "Chrome MCP not connected — install the chrome-devtools MCP server, or pass `--no-cross-user` to skip Phase 3 cross-user steps."
3. The daemon is configured with **at least two `--dev-login-email` entries** (or a comma-separated `RAY_EXOMEM_DEV_LOGIN_EMAIL`). Verify by issuing `GET /auth/dev-login?email=<expected-second-email>` from a fresh isolated Chrome context — a 303 redirect with `Set-Cookie: ray_exomem_session=...` is pass; 400 `email_not_allowed` or 404 `dev-login is not enabled` aborts the run with that as evidence.
4. The daemon is bound to a loopback address (`127.0.0.1` or `localhost`). The dev-login route is loopback-only at the route layer; remote runs (e.g., against `https://mem.trydev.app`) cannot use Phase 3's cross-user surface and must be invoked with `--no-cross-user`.

## Invocation flags

| Flag | Meaning |
|---|---|
| `--no-cross-user` | Skip Phase 3 cross-user probes. Use against remote (non-loopback) deployments where `--dev-login-email` isn't available. Phases 1, 2, 4 still run via MCP. |
| `--with-team` | Run Phase 5 (multi-agent collision via TeamCreate). Sub-agents inherit the orchestrator's bearer, so they share `user_email` and exercise the **same-user, multi-agent** branch-TOFU path — a different invariant from Phase 3's cross-user path. Both can run in the same matrix. |
| `--with-admin-probes` | Allow `/actions/factory-reset` / `/actions/wipe`. **DEFAULT OFF.** Never enable without explicit user opt-in. |
| `--scratch public` | Place the cross-user scratch under `public/stress-test/<run-id>` (default for Phase 3). The single-user Phases 1+2 default to private (`<user1_email>/test/<run-id>`); pass `--scratch public` to put those public too. |
| `--cleanup` | After teardown, archive the scratch session(s). |
| `--base-url <url>` | Base URL recorded in the report header. Defaults to `https://devmem.trydev.app`; use `http://127.0.0.1:9780` for local-only loopback. |

## Default invariants — DO NOT VIOLATE

- **Private-by-default for single-user phases.** Phase 1+2 scratch lives at `<user1_email>/test/<UTC-ISO>-<8-char-run-id>` so probe noise is invisible to other users. Phase 3 must use `public/stress-test/<run-id>` to exercise Model A's auth-layer 403 (Model A only changes behavior in `public/*`). Phase 3 also touches `<user1_email>/...` for the `{email}/*` co-edit path — those exoms remain private to user1 unless explicitly shared.
- **Non-destructive by default.** Never call `/actions/factory-reset` or `/actions/wipe` without `--with-admin-probes`.
- **Don't reuse paths across runs.** Every run keys off `<UTC-ISO>-<run-id>`; collisions are unlikely but if they happen, append `-2` / `-3` etc.
- **No state-sharing across phases via mutation.** Each phase records the fact_ids / tx_ids / branch names it produced into the report row's evidence column; later phases that need a value reference it by name from the run's evidence dict, not by re-querying.
- **A skipped phase is not a failure.** Mark it `⏭ skipped` with the reason ("--no-cross-user", "Chrome MCP not connected", "no second dev-login-email").

## Rayfall query gotchas the runner keeps tripping over

These are not bugs — they're constraints of the engine's body-atom parser and projection check. Past runs have lost time chasing them; consult this list before authoring any cross-check query in a phase file.

- **Pin literals in the body atom, not in `(= ?var "lit")`.** Rayfall does not accept assignment-style equality forms in `where`. The form `(where (?tx 'tx/action ?act) (= ?act "retract-fact"))` returns `MCP error -32000: rayforce2 err type: rule: cannot parse assignment expression`. Bind the literal directly: `(where (?tx 'tx/action "retract-fact"))`.
- **Every `find` var must be bound by a body atom.** Projecting a var that is pinned to a constant in the body — e.g. `(find ?id ?p ?v) (where (fact-row ?id "fx/marker" ?v))` — fails with `MCP error -32000: rayforce2 err domain: query: evaluation failed: dl_project: unset head-const type`. Either drop the unbound var from `find`, or write the body atom with a real variable in that slot and add a pinned join elsewhere.
- **Builtin-view arities don't match a dated mental model.** As of 2026-04-29: `belief-row` is **arity 4** (`?belief ?claim ?status ?tx`) and `observation-row` is **arity 4** (`?obs ?source_type ?content ?tx`). The view does not project `belief/confidence` or `obs/source_ref` — those live on the entity and need a direct EAV join (`(?obs 'obs/source_ref ?ref)`) if you want them. Always cross-check arity from `schema.builtin_views` before authoring a query.
- **`belief-row` returns one row per `belief_id`, not one row per revision.** Supersede mutates the entity in place (replaces `claim_text` and `belief/created_by`); the row-view collapses the chain to current state. Don't expect a separate `superseded` row to land in the view — full history lives in the tx log (`tx/action = "revise-belief"`) and `belief_history`.
- **Don't confuse entity refs with predicate values.** `tx-row` projects `tx/N` for the `?tx` slot (the entity ref) and the bare numeric id for `?id` (the `tx/id` predicate value). So `(?tx 'tx/id "tx/22")` returns 0 rows; the correct pin is `(?tx 'tx/id "22")`. Same trap with facts: a `fact_id` (`"test/n"`) and the underlying predicate name happen to look identical, but in `(?fact 'fact/predicate ?p)`, `?fact` binds to the entity ref and `?p` to the predicate name. When in doubt, run the unpinned form first.
- **Pinned literals in rule-call slots work** (post the `derive_rule_param_attrs` fix). `(fact-row ?id "predicate-literal" ?v)` and `(tx-row ?tx ?id ?u ?a ?m "merge" ?w ?br)` resolve correctly: the lowering layer derives each rule's head-param→attribute map and pins the literal at the call site before rayforce2 expands the rule. If a future probe sees these returning 0 rows again, the rule-call rewriter (`src/rayfall_ast.rs::derive_rule_param_attrs` + `rewrite_body_literals_with_schema_and_rules`) regressed.

## Cross-user transport — the two-context pattern

Phase 3 uses two isolated Chrome contexts, each with a cookie jar minted by the loopback dev-login route. The pattern is reusable wherever cross-user behaviour matters:

```
# Phase 0 sets these up once, all later phases reuse them.
mcp__chrome-devtools__new_page {
  url: "<base_url>/auth/dev-login?email=<user1_email>",
  isolatedContext: "user1"
}
mcp__chrome-devtools__new_page {
  url: "<base_url>/auth/dev-login?email=<user2_email>",
  isolatedContext: "user2"
}

# Drive each context with mcp__chrome-devtools__select_page + evaluate_script.
# Inside evaluate_script, use fetch() with credentials: 'include' so cookies
# go along. Confirm identity with a one-liner: await fetch('/auth/me') first.
```

**Per-context cookies:** Same-origin pages in the same `isolatedContext` share cookies; pages in different `isolatedContext` values are fully isolated. Never use `select_page` to a `pageId` outside the intended user — it's the most common test-runner footgun.

**Batching inside a context:** Pack as much logic into a single `evaluate_script` as possible. Each round trip costs a model↔API hop; one script that performs ten asserts + a query is a single round trip, ten separate calls is ten round trips.

**MCP vs fetch transport choice:** MCP for single-user, fetch-via-Chrome for cross-user. Don't mix transports in the same probe — it's hard to attribute a discrepancy ("did MCP and HTTP normalize the body differently?").

## Setup (Phase 0)

See `scenarios/phase-0-setup.md` for the concrete steps. Summary:

1. Verify preconditions (MCP, Chrome MCP, dev-login allow-list, loopback bind).
2. Discover `<user1_email>` from the orchestrator's MCP bearer (the runner's identity).
3. Discover `<user2_email>` — the second allow-listed email. Default heuristic: `GET /auth/info` doesn't expose it, so the user passes it explicitly via `--user2 <email>`, or the runner falls back to the first non-`<user1_email>` entry in the daemon's allow-list (which it can probe by trying common test addresses + reading the `400 email_not_allowed` failures). Practical path: when the runner is launched in auto mode by an operator, the operator provides `<user2_email>` in the prompt; otherwise the runner asks.
4. Open both Chrome contexts, dev-login each, verify `/auth/me` returns the right email per context.
5. Compose `<scratch_root_priv>` (`<user1_email>/test`) and `<scratch_root_pub>` (`public/stress-test`).
6. Compose `<run_id>` = 8-char random tag; `<scratch_project>` = `<scratch_root_priv>/<UTC-ISO>-<run_id>`; `<public_scratch>` = `<scratch_root_pub>/<UTC-ISO>-<run_id>`.
7. `mcp__ray-exomem__init { path: <scratch_project> }` — Phase 1+2 scaffolding.
8. `mcp__ray-exomem__session_new { project_path: <scratch_project>, session_type: "multi", label: "stress", agents: ["agent-a","agent-b","probe-d"] }` — capture as `<session>`.
9. `mcp__ray-exomem__schema { exom: <session> }` — snapshot for the report.

If any step fails, abort and emit the matrix with all phases marked `⛔ blocked`.

## Phase execution

Each phase is **one top-level task** that batches many feature checks. Run sequentially — later phases reference evidence from earlier ones.

| Phase | File | Touches | Transport |
|---|---|---|---|
| 0 | `phase-0-setup.md` | preconditions, discovery, scratch init, two-context dev-login | MCP + Chrome |
| 1 | `phase-1-core.md` | guide / list_exoms / exom_status / schema / list_branches / tree (read), typed values + facts_i64/str/sym, bitemporal (assert+supersede+retract+history+back-pointers), beliefs (believe+supersede+revoke), observations (observe+tags), builtin views, attribution (tx-row triple), explain, export | MCP (user1) |
| 2 | `phase-2-branches-sessions.md` | create_branch / switch / merge / archive, session_new (single+multi) / close / join unknown, init scaffolds, exom_new bare, rename, delete, regression probes (hyphen attr / default-fact-id supersede / sym health / cache staleness / cursor restoration) | MCP (user1) |
| 3 | `phase-3-cross-user.md` | Model A 403, exom_fork (explicit + auto-suffix + session refusal + replayed attribution + lineage), co-edit ↔ solo-edit flip endpoint (creator-only, session-rejected, no-op detection), public/* co-edit auth elevation, {email}/* co-edit branch-TOFU bypass + flip-back BranchOwned, co-edit non-`main` TOFU preservation, co-edit child session under co-edit parent, audit-trail `_meta/acl_mode` fact, mode persistence across daemon restart, cross-user attribution on retract | Chrome (both contexts) |
| 4 | `phase-4-error-surface.md` | unknown_exom, unknown_branch, missing database name, server-side arity error, invalid value, missing/empty predicate, archive `main` rejection | MCP (user1) |
| 5 | `phase-5-multi-agent.md` | (`--with-team` only) TeamCreate, agent-a + agent-b TOFU claims on their allocated branches, second-join idempotency, cross-branch claim triple in tx-log | MCP (orchestrator + sub-agents) |

## Teardown

1. `mcp__ray-exomem__session_close { session_path: <session> }`. Confirm subsequent `assert_fact` to that session returns `MCP error -32000: session_closed` — but only if Phase 2 didn't already cover it.
2. If `--with-team` ran: send `shutdown_request` to all agents, await approvals, then `TeamDelete`.
3. Close Chrome pages opened in Phase 0 (both contexts). The browser process keeps running.
4. If `--cleanup`: there's no scratch-session delete tool yet; skip with a note. Phase 3's public-namespace exoms remain visible to other users in the allowed domain — document them in the report's Notes section so the operator knows what to clean up manually.
5. Render the report from `report-template.md`.

## Report

Render the matrix from `report-template.md`. Each row's status is one of:

- `pass` — every step in the row's group passed.
- `fail` — at least one step failed; include the first failing step's evidence (response body, error string, or diff vs. expected) verbatim.
- `skipped` — gated off (e.g., `--no-cross-user` set, `--with-team` not set).
- `blocked` — couldn't run due to setup or upstream phase failure.

Append a one-line summary: `N / M passed, K failed, S skipped`. **Don't reformat or summarize evidence away — paste the raw error string or returned tuple.** The whole point of the matrix is to be auditable after the fact.

## Self-validation

The skill earns its keep when:

1. **Healthy run** against current local dev (`http://127.0.0.1:9780`) with default flags → all phases pass; no skips except `--with-team` and `--with-admin-probes`.
2. **Regression flip:** revert one feature (e.g., remove the `if matches!(meta.acl_mode, AclMode::CoEdit) && branch == "main"` short-circuit in `precheck_write`) → re-run → Phase 3's "co-edit write to main" row **fails loudly** with `branch_owned` as evidence.
3. **Recovery:** re-apply the fix → re-run → Phase 3 passes. The matrix reflects the fix.

If the matrix flips correctly under each regression, the skill is doing its job.

## Notes for skill maintainers

- **When a new feature ships, pick the phase that already touches the relevant brain surface and add the probe there.** Don't create a new phase unless the surface is genuinely orthogonal. Phase 3 currently absorbs Model A, fork, AND co-edit because all three live at the auth↔storage seam; that's correct density.
- **Two-transport split is load-bearing.** MCP-only phases run faster (no Chrome round-trip per probe). Don't move single-user probes into Chrome unless they have a cookie-specific dependency. Keep cross-user probes in Chrome — they need real cookies, not a synthetic bearer.
- **Per-phase run time on a healthy daemon should be < 30s.** If a phase regularly takes longer, it's probably issuing too many round trips; batch into fewer `evaluate_script` calls or fewer MCP calls.
