---
name: stress-test-ray-exomem
description: Run a comprehensive stress test of ray-exomem covering every MCP write/read tool, branches, sessions, attribution, bitemporal facts, typed values, error surface, and multi-agent collision. Produces a pass/fail matrix per feature chapter. Use when the user says "stress test ray-exomem" / "regression-check exomem" / "/stress-test ray-exomem", or after any change touching brain.rs/storage.rs/mcp.rs/server.rs.
---

# stress-test-ray-exomem

Drive ray-exomem end-to-end through its MCP surface, verify every feature
chapter, and emit a single pass/fail matrix. Every probe runs through MCP —
there is no `--with-http` fallback; if a probe can't be expressed in MCP that
is itself a regression in the MCP surface and should fail the matrix.

## Invocation

The skill accepts these flags from the user prompt (parse from natural language
or trailing tokens):

| Flag                          | Meaning                                                                 |
|-------------------------------|-------------------------------------------------------------------------|
| `--with-team`                 | Run Ch09 (multi-agent collision/idempotency) by spawning a 2-agent team. Sub-agents inherit the parent's bearer, so they share `user_email` with the orchestrator and can read/write the parent's private namespace — Ch09 steps 1–4 work in private scratch unchanged. Auto-on for "full stress test". |
| `--with-collision-user <key>` | Bearer token of a *second* user — unlocks the cross-user write/fork probes in Ch13 step 7 (Model A's auth-layer 403), the BranchOwned residual in Ch09 step 5, and the corresponding Ch11 row. The collision user is a different `user_email` and so cannot see private scratch. Setting this flag implies `--scratch public`; if private scratch is forced, the cross-user steps auto-skip with reason "collision-user cannot see private scratch". For local dev without a second OAuth bearer, the daemon's `--dev-login-email` flag accepts multiple emails (loopback only) — use `GET /auth/dev-login?email=<addr>` to mint a session for any email in the allow-list. |
| `--with-admin-probes`         | Allow `/actions/factory-reset` / `/actions/wipe`. **DEFAULT OFF.** Never enable without explicit user opt-in. |
| `--cleanup`                   | After teardown, archive the scratch session (admin-gated).              |
| `--scratch public`            | Place scratch under `public/stress-test/...` instead of the **default** `{user_email}/test/...`. Use this when you intentionally want the run visible to other users in the allowed domain (cross-user collision probes, shared regression evidence). Default-private keeps probe noise out of the public tree and makes the test exoms invisible to other users by default. |
| `--base-url <url>`            | Base URL the MCP transport points at, recorded in the report header. Defaults to `https://mem.trydev.app`. Use `https://devmem.trydev.app` for local dev. |

## Rayfall query gotchas the runner keeps tripping over

These are not bugs — they're constraints of the engine's body-atom parser
and projection check. Past runs have lost time chasing them; consult this
list before authoring any cross-check query in a chapter file.

- **Pin literals in the body atom, not in `(= ?var "lit")`.** Rayfall does
  not accept assignment-style equality forms in `where`. The form
  `(where (?tx 'tx/action ?act) (= ?act "retract-fact"))` returns
  `MCP error -32000: rayforce2 err type: rule: cannot parse assignment expression`.
  Bind the literal directly: `(where (?tx 'tx/action "retract-fact"))`.
- **Every `find` var must be bound by a body atom.** Projecting a var that is
  pinned to a constant in the body — e.g.
  `(find ?id ?p ?v) (where (fact-row ?id "fx/marker" ?v))` — fails with
  `MCP error -32000: rayforce2 err domain: query: evaluation failed: dl_project: unset head-const type`.
  Either drop the unbound var from `find`, or write the body atom with a
  real variable in that slot and add a pinned join elsewhere.
- **Builtin-view arities don't match a dated mental model.** As of
  2026-04-29: `belief-row` is **arity 4** (`?belief ?claim ?status ?tx`) and
  `observation-row` is **arity 4** (`?obs ?source_type ?content ?tx`). The
  view does not project `belief/confidence` or `obs/source_ref` — those live
  on the entity and need a direct EAV join (`(?obs 'obs/source_ref ?ref)`)
  if you want them. Always cross-check arity from `schema.builtin_views`
  before authoring a query, especially when an old chapter file specifies
  an arity-5 form.
- **`belief-row` returns one row per `belief_id`, not one row per revision.**
  Supersede mutates the entity in place (replaces `claim_text` and
  `belief/created_by`); the row-view collapses the chain to current state.
  Don't expect a separate `superseded` row to land in the view — full
  history lives in the tx log (`tx/action = "revise-belief"`) and
  `belief_history`.
- **Don't confuse entity refs with predicate values.** `tx-row` projects
  `tx/N` for the `?tx` slot (the entity ref) and the bare numeric id for
  `?id` (the `tx/id` predicate value). So `(?tx 'tx/id "tx/22")` returns 0
  rows; the correct pin is `(?tx 'tx/id "22")`. Same trap with facts: a
  `fact_id` (`"test/n"`) and the underlying predicate name happen to look
  identical, but in `(?fact 'fact/predicate ?p)`, `?fact` binds to the
  entity ref and `?p` to the predicate name. When in doubt, run the
  unpinned form first to see what the value actually is.
- **Pinned literals in rule-call slots work** (post the `derive_rule_param_attrs`
  fix). `(fact-row ?id "predicate-literal" ?v)` and `(tx-row ?tx ?id ?u ?a ?m "merge" ?w ?br)`
  resolve correctly: the lowering layer derives each rule's head-param→
  attribute map and pins the literal at the call site before rayforce2
  expands the rule. If a future probe sees these returning 0 rows again,
  the rule-call rewriter (`src/rayfall_ast.rs::derive_rule_param_attrs`
  + `rewrite_body_literals_with_schema_and_rules`) regressed.

## Default invariants — DO NOT VIOLATE

- **Private by default, public on opt-in.** The dedicated test root is
  `{user_email}/test/`. All writes target one scratch session exom at
  `{user_email}/test/<UTC-timestamp>-<run-id>` unless `--scratch public` is
  given (then `public/stress-test/<UTC-timestamp>-<run-id>`). The agent guide's
  permission model makes `{user_email}/...` owner-private, so default scratch
  is invisible to every other user — no public-tree pollution from a
  successful or failed run. Use `--scratch public` only when the run *needs*
  cross-user visibility (Ch09 step 5).
- **Non-destructive by default.** Never call `/actions/factory-reset` or
  `/actions/wipe` unless `--with-admin-probes`.
- **Don't init a path that already exists.** The skill creates a *new* scratch
  path each run keyed by ISO-timestamp, so collisions shouldn't happen, but if
  they do, append a suffix.
- **Never share state between chapters via mutation.** Each chapter records the
  fact_ids / tx_ids / belief_ids it produced into the report row's evidence
  column. Later chapters reference those by id — they don't re-fetch by
  predicate name.
- **Single-agent path runs Ch01–Ch08, Ch10–Ch12 in this process.** Multi-agent
  Ch09 is the only chapter that spawns sub-agents.
- **A skipped chapter is not a failure.** Mark it `⏭ skipped` with the reason
  ("--with-http not provided", "no second bearer", etc).

## Setup (every run, before any chapter executes)

1. `mcp__ray-exomem__guide` — capture markdown size + first-line build identity
   if present. Used as Ch01 evidence; if this fails, abort with "MCP not connected".
2. `mcp__ray-exomem__list_exoms` — confirm read access; capture exom count.
3. Discover `<user_email>`: the runner's authenticated identity. Preferred:
   take it from the bearer's auth context. Fallback: call
   `mcp__ray-exomem__tree { path: "" }` and pick the top-level node that
   isn't `public/` and isn't `admin/` — every authenticated user has
   `<email>/main` seeded on first login, so its label is `<user_email>`.
4. Compose `<scratch_root>`. Default (private):
   `<user_email>/test`. With `--scratch public`: `public/stress-test`.
   Compose `<scratch_project>` = `<scratch_root>/<UTC-ISO>-<8-char-run-id>`.
5. `mcp__ray-exomem__tree { path: <scratch_root> }` — sanity-check the
   scratch namespace. If `<scratch_project>` already appears (extremely
   unlikely with a timestamped id), append `-2` / `-3` etc. to disambiguate.
   The first time the skill runs in private mode, `<user_email>/test` won't
   exist yet — that's fine; `init` creates it.
6. `mcp__ray-exomem__init { path: <scratch_project> }`. Creates
   `<scratch_project>/main` and `<scratch_project>/sessions/`. If this fails
   with `forbidden`, the user lacks write on `<scratch_root>/*` — abort
   with that error in the report. (Should be impossible under default
   private scratch — the user always owns their own `{email}/...` namespace.)
7. `mcp__ray-exomem__session_new`:
   ```
   { project_path: <scratch_project>,
     session_type: "multi",
     label: "stress",
     agents: ["agent-a", "agent-b", "probe-d"],
     agent: "claude-code-cli",
     model: "<the model you're running as>" }
   ```
   Capture the returned session exom path as `<session>`. This is the write
   target for every chapter except Ch07's "single" probe (which creates a
   second session). The `probe-d` slot is reserved for Ch12-D's cache-
   staleness probe so that probe doesn't pollute Ch09's `agent-a`/`agent-b`
   TOFU claims.
8. `mcp__ray-exomem__schema { exom: <session> }` — snapshot the schema. Used
   as Ch01 evidence and as the baseline for Ch12 regression probes.

If any setup step fails, abort and emit a report with all chapters marked
`⛔ blocked` plus the setup error.

## Chapter execution

For each `scenarios/chNN-*.md`, follow its concrete steps verbatim. Each
chapter file lists:
- The MCP tool calls (or HTTP calls for Tier 2 chapters)
- Pass criteria (what the response must contain)
- Evidence to capture for the report

Run chapters sequentially. Don't parallelize — later chapters depend on
fact_ids / tx_ids / branch names from earlier ones.

Order:
1. Ch01 — Discovery (read-only)
2. Ch02 — Typed values
3. Ch03 — Bitemporal
4. Ch04 — Beliefs
5. Ch05 — Observations
6. Ch06 — Branches  *(includes merge/archive via MCP)*
7. Ch07 — Sessions  *(includes init/exom-new via MCP)*
8. Ch08 — Attribution
9. Ch10 — Reads
10. Ch11 — Error surface  *(BranchOwned/Model A `forbidden` covered in Ch09 / Ch13)*
11. Ch12 — Regression probes
12. Ch13 — Model A: public ownership + fork  *(steps 1–6 always run; step 7
    needs `--with-collision-user`)*
13. Ch09 — Multi-agent  *(gated `--with-team`; runs LAST so its branches are
    visible to the rest of the report and aren't disturbed by single-agent
    chapters)*

## Teardown

1. `mcp__ray-exomem__session_close { session_path: <session> }`. Confirm
   subsequent `assert_fact` to the same session returns
   `MCP error -32000: session_closed` — but don't repeat this in Ch07 if
   Ch07 already covered it.
2. If `--with-team` was set: send `shutdown_request` to all agents, await
   approvals, then `TeamDelete`.
3. If `--cleanup`: there's no scratch-session delete tool yet; skip with a
   note. The closed session is harmless and gets cleaned up by
   `--with-admin-probes` factory-reset workflows if needed.
4. Render the report from `report-template.md`.

## Report

Render the matrix from `report-template.md`. Each chapter's status is one of:

- `pass` — every step in the chapter passed its check
- `fail` — at least one step failed; include the first failing step's
  evidence (response body, error string, or diff vs. expected)
- `skipped` — gated off by missing flag (e.g., `--with-team` not set)
- `blocked` — couldn't run due to setup or upstream chapter failure

Append a one-line summary: `N / M passed, K failed, S skipped`.

**Don't reformat or summarize evidence away — paste the raw error string or
returned tuple.** The whole point of the matrix is to be auditable after the
fact.

## Self-validation

The skill earns its keep when:

1. Run against current prod (`https://mem.trydev.app`) with `--with-team` and
   no other flags → all chapters pass except the `--with-http`-gated ones
   (which skip), and `--with-collision-user`-gated ones (which skip).
2. Locally regress one feature (e.g., revert
   `tool_session_join` cache eviction) → re-run against
   `https://devmem.trydev.app` → Ch12 "cache staleness" probe **fails loudly**
   with the stale `list_branches` payload as evidence.
3. Re-apply the fix → re-run → Ch12 passes.

If both regression and recovery flip the matrix correctly, the skill is doing
its job.
