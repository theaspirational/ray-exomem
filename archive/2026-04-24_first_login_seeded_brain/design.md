# First-Login Seeded Brain Redesign

## Critique

The previous first-login state created a narrow domain demo plus generic work/example rows. It felt clinical because the first visible namespace was not the product's actual domain, the facts were tiny and static, and the rules looked like a demo of threshold math rather than durable memory. The tree did not imply a lived-in workspace, the graph had predicate nodes without edges, and history/branches had too little temporal or relational material to demonstrate why exomem matters.

## Proposal

Land users directly on `{email}/main`, a native exom dashboard that acts as a cross-project brain index. Seed it with technical/work memory: active projects, archived imports, decisions, incidents, constraints, commands, docs, open questions, observations, beliefs, branch alternatives, and rules. The data should teach concepts by being useful:

- Facts: current claims with stable ids like `project/ray-exomem#status`.
- Observations: evidence from code review, docs, incidents, query logs, and imports.
- Beliefs: revised interpretations with support links and superseded history.
- Rules: derived working sets such as `high_priority`, `at_risk`, `stale_open_question`, and `decision_review_due`.
- Branches: named alternatives and follow-up branches with branch-local facts.
- Graph: entity-to-entity edges inferred from fact id prefixes and fact values.

## Exact Bootstrap Content

Tree structure:

```text
{email}/main
{email}/work/main
{email}/work/platform/main
{email}/work/platform/memory-daemon/main
{email}/work/platform/native-ui/main
{email}/work/platform/rayfall/main
{email}/work/operations/main
{email}/work/operations/incidents/main
{email}/research/main
{email}/research/agent-memory/main
{email}/research/retrieval-eval/main
{email}/knowledge/main
{email}/knowledge/architecture/main
{email}/archive/main
{email}/archive/2025-import/main
```

Dashboard facts include:

- `brain/home#purpose`, `brain/home#memory-model`, `brain/home#branch-policy`, `brain/home#retention-policy`, `brain/home#first-login-contract`
- project facts for `project/ray-exomem`, `project/native-ui`, `project/rayfall-engine`, `project/retrieval-eval`, `project/ops-runbooks`
- decision facts for `decision/entity-ids`, `decision/valid-time`, `decision/graph-shape`, `decision/no-wizard`
- incident facts for `incident/auth-replay`, `incident/symbol-table`
- document facts for `doc/ui-polish-spec`, `doc/onboarding-template-plan`, `doc/live-test-loop`
- open questions for `question/graph-density`, `question/branch-merge`, `question/rule-errors`
- constraints, preferences, commands, ownership, dependencies, supports, and provenance links

Representative observations:

- `obs/first-run-mismatch`
- `obs/graph-predicate-only`
- `obs/native-tabs-ready`
- `obs/entity-id-collisions`
- `obs/typed-rules-work`
- `obs/auth-replay`
- `obs/open-questions-stale`

Representative beliefs:

- Superseded: `belief/welcome-template`
- Active: `belief/native-first-run`
- Active: `belief/entity-graph`
- Active: `belief/stable-ids`
- Active: `belief/rules-value`
- Active: `belief/branch-value`

Rules:

```text
high_priority(?id)        :- project/priority >= 8
at_risk(?id)              :- risk/score >= 7
stale_open_question(?id)  :- question/age_days >= 14
decision_review_due(?id)  :- decision/review_due_days < 14
recent_incident(?id)      :- incident/days_since < 30
mature_memory(?id)        :- memory/age_days >= 180
```

Important derived facts:

- `project/ray-exomem#priority` and `project/native-ui#priority` derive as `high_priority`
- `project/native-ui#risk` and `incident/symbol-table#severity` derive as `at_risk`
- `question/branch-merge#age` derives as `stale_open_question`
- `decision/valid-time#review` derives as `decision_review_due`
- `incident/symbol-table#days` derives as `recent_incident`

Timestamp strategy:

- Seeded transaction times span `2025-09-02T08:30:00Z` through `2026-04-23T11:40:00Z`.
- Valid-time mirrors real-world claim dates; superseded facts have `valid_to`.
- Beliefs include superseded and active versions.
- Branch-local facts are dated on their branch creation/follow-up dates.

Relationship strategy:

- Use stable `entity#attribute` fact ids.
- Relation graph subject is the fact id prefix before `#`.
- Relation graph target is the fact value when it looks like an entity reference; otherwise it is a typed literal node scoped by predicate.
- Shared values such as docs, decisions, owners, questions, incidents, and projects create connected graph hubs.

## Native UI Recommendations

- Default route should open `{email}/main`, not a folder root.
- Graph should show entity edges by default and allow selecting entities to inspect facts.
- Facts should remain dense and filterable; seed predicates should be readable without explanatory copy.
- History should emphasize valid-time and branch-origin badges.
- Branches should show meaningful names and fact counts, not only internal ids.
- Rules should surface authored rules alongside system views, with derived samples visible in schema/facts.

## Implementation Plan

1. Replace `src/auth/routes.rs` hardcoded seed with a rich `BootstrapSeed` builder.
2. Seed `{email}/main` plus nested project/folder exoms on login.
3. Change UI defaults in `ui/src/lib/stores.svelte.ts` and `ui/src/routes/+page.svelte` to land on `{email}/main`.
4. Change `src/server.rs::api_relation_graph` to emit entity-to-entity edges.
5. Adjust entity fact lookup in `ui/src/lib/exomem.svelte.ts` to match `entity#attribute` ids.
6. Update auth tests to assert the new bootstrap contract, derived rules, and graph summary.
7. Run Rust/UI checks and live daemon verification.

## Implementation Status

Implemented in this branch:

- Rich first-login seed in `src/auth/routes.rs`
- Entity relation graph in `src/server.rs`
- Default native dashboard landing in Svelte UI
- Entity panel lookup support for `entity#attribute` ids
- Updated auth/bootstrap tests
- Agent guide test fix for the existing guide expectation
