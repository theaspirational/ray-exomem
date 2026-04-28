# Ray-exomem agent guide (MCP)

Ray-exomem is a persistent, bitemporal knowledge base for agents. This guide
describes the MCP interface — the canonical way for an agent to read, write,
and reason over an exom.

The CLI exists for human / dev workflows and is intentionally out of scope
here. Agents should not call CLI binaries.

---

## 1. Connecting

Ray-exomem speaks MCP over Streamable HTTP at `<base>/mcp`.

For the hosted instance:

```
https://mem.trydev.app/mcp
Authorization: Bearer <api-key>
```

Issue an API key from the user's session (`/auth/api-keys`). Local dev runs at
`http://127.0.0.1:9780/mcp` with the same auth contract.

The server advertises tools via standard MCP `tools/list`. The current toolset
is fixed at compile time and listed in §4.

---

## 2. The model

```
Tree:        public/work/<team>/<project>/<topic>/main      (project main exom)
             public/work/.../<project>/sessions/<id>        (per-session exom)
             {email}/...                                    (private user namespace)
```

Two node kinds, no others:

- **Folder** — grouping. Holds child folders and child exoms. No facts.
- **Exom** — leaf knowledge base. Holds facts, observations, beliefs, branches,
  rules, transactions. Marked on disk by `exom.json`. **Cannot nest anything
  inside an exom.**

Facts are **bitemporal**:

- `valid_from` / `valid_to` — the wall-clock window the fact is true in the
  modelled world.
- `created_at` (tx-time) — when the daemon recorded the fact.

A new assert with the same `fact_id` supersedes the previous tuple by closing
its `valid_to` and creating a new tuple. History is preserved.

### Permissions

| First path segment | Default access for an authenticated user |
|--------------------|------------------------------------------|
| `public/...`       | Read + Write for everyone in the allowed domain. |
| `{email}/...`      | Owner has full access; everyone else denied unless explicitly shared. |
| Anything else      | Admin-only. |

Bootstrap fixtures seed under `public/...`; private agent state belongs under
`{user-email}/...`.

### Fact identity

A fact is a tuple `(fact_id, predicate, value)`. `fact_id` is the addressable
key that supports replace / retract. Convention used in seeds:

```
<entity>#<property>           e.g. concept/verb#name
```

Pick one and stick with it within a topic. If you omit `fact_id` on assert it
defaults to the predicate name — fine for singletons, dangerous when you have
multiple instances.

### Typed values

Values are `I64 | Str | Sym`. The MCP `assert_fact` tool accepts:

- a JSON number → stored as `I64` (enables `<` / `>` / `sum` in Datalog).
- a JSON string → run through `FactValue::auto` (parses to `I64` if it
  round-trips, else `Str`). So `"75"` lands as `I64(75)`, `"green"` as
  `Str("green")`.
- `{"$sym": "active"}` → stored as a `Sym` (interned, identity-compared).

Strings that should remain strings even though they look numeric (`"007"`,
`"+5"`, `"7.5"`) auto-detect to `Str` because they don't round-trip. If you
need numeric reasoning, send a JSON number.

---

## 3. Authoring patterns

### Predicates

User predicates are free-form `<namespace>/<name>`. Common namespaces seen in
seeds:

- `entity/name`, `entity/type` — universal handles.
- `concept/summary`, `concept/docs_url` — for definitional knowledge.
- domain-specific (`feature/...`, `service/...`, `task/status`, etc.).

Reserved namespaces (you query them, you don't assert into them):
`fact/*`, `tx/*`, `obs/*`, `belief/*`, `branch/*`, `session/*`, `claim/*`,
`task/*`, `agent/*`. Use `schema` to enumerate the full list for the running
build.

### Writing knowledge

Use `assert_fact`. Each call is one tuple. Patterns:

```jsonc
// canonical: entity/property
{ "exom": "public/work/...", "predicate": "entity/name",   "fact_id": "service/auth#name", "value": "auth-gateway" }
{ "exom": "public/work/...", "predicate": "entity/type",   "fact_id": "service/auth#type", "value": "service" }
{ "exom": "public/work/...", "predicate": "service/owner", "fact_id": "service/auth#owner", "value": "platform-team" }

// numeric, becomes I64 — supports cmp/agg in Datalog
{ "exom": "public/work/...", "predicate": "service/sla_p99_ms", "fact_id": "service/auth#sla", "value": 250 }

// symbol — for status enums you'll join on
{ "exom": "public/work/...", "predicate": "service/status", "fact_id": "service/auth#status", "value": {"$sym": "active"} }
```

Optional fields on `assert_fact`:

- `confidence` (0.0..1.0, default `1.0`)
- `source` — provenance tag (default `"mcp"`)
- `valid_from` / `valid_to` — ISO-8601 wall-clock bounds (default
  `valid_from = now()`, open-ended `valid_to`)
- `agent`, `model` — three-axis attribution (see below)

`created_at` (the daemon's tx-time) is always set independently to wall-clock
now — that's the bitemporal split. Backfilling `valid_from` to a historical
date is supported and recommended when seeding from older sources.

All write tools (`assert_fact`, `retract_fact`, `observe`, `believe`,
`revoke_belief`, `create_branch`, `session_close`, `session_new`,
`session_join`) accept an optional `branch` argument. The exom's current
branch is restored after the write, so other callers see the cursor
unchanged. Branch ownership is TOFU-enforced — writing to a branch claimed
by a different user returns `branch_owned`.

### Attribution: three independent axes

Every write records `(user_email, agent, model)` on the underlying tx:

- `user_email` — DB-bound identity from auth. Always the authenticated user;
  load-bearing for permission checks; not caller-controlled.
- `agent` — the tool/integration making the call (e.g. `cursor`,
  `claude-code-cli`). Falls back to the API key's label for Bearer auth;
  cookie-auth UI writes leave it `None`. An explicit `agent` arg always
  wins over the key label.
- `model` — the LLM identity (e.g. `claude-opus-4-7`). Explicit only — no
  fallback.

UI render: `by {user_email} via {agent} using {model}`, with `via`/`using`
elided when those fields are `None`. System-internal writes (no user) render
as `by system`.

**Multi-subagent contract (important).** When a single MCP client (e.g. one
Claude Code CLI process) hosts many subagents (`general-purpose`,
`code-reviewer`, `ui-sketcher`, etc.) authenticated by one API key, the
client must inject `agent: "<subagent-name>"` on every tool call to
disambiguate. Without it, all subagent writes appear under the API key's
label. The daemon cannot infer this — it's a contract on the orchestrator.

### Re-asserting and retracting

Re-asserting with the same `fact_id` supersedes — the engine closes the prior
tuple's `valid_to` and writes a new tuple, preserving history.

`retract_fact { exom, fact_id, agent?, model?, branch? }` closes the active
tuple's `valid_to = now()` and marks the fact revoked. `fact_history` still
returns the closed tuple, so retract is non-destructive.

---

## 4. Tool reference

All tools live under the MCP namespace `ray-exomem` (e.g.
`mcp__ray-exomem__query` from a Claude client). Argument names match the
schemas the server exposes.

### `list_exoms`

No args. Returns every exom the authenticated user can see. Cheap; use it as
the first call in a fresh session.

### `exom_status` `{ exom }`

Returns `{ exom, current_branch, facts, beliefs, transactions }`. Lazy-loads
the exom into memory on first call.

### `schema` `{ exom }`

Returns the full ontology: `system_attributes`, `coordination_attributes`,
`builtin_views` (with the rule body of each — handy for query authoring), and
`user_predicates` (the deduplicated list of free-form predicates currently
asserted in this exom). This is the right call to discover what's already
modeled before you assert.

### `explain` `{ exom, predicate? | fact_id? }`

Spot-checking surface — does **not** route through Rayfall, so it works even
when the engine query path is being upgraded.

- With `predicate`: returns every current fact under that predicate (id,
  value, confidence).
- With `fact_id`: returns the tx-history events that touched that fact.

### `fact_history` `{ exom, id }`

Bitemporal history for a single `fact_id`: each tuple's `value`, `confidence`,
`valid_from`, `valid_to`, `created_at`. Use this to verify timestamps after a
write, or to read time-travel slices.

### `query` `{ exom, query }`

Run one Rayfall (Datalog) form. The form must be a single `(query <exom-path>
(find ?vars) (where (<relation> ...)))`. The exom path inside the query must
match `exom`.

**Predicate names are values, not relations.** Storage is EAV: a fact like
`entity/name = "verb"` lives as two triples (`?fact 'fact/predicate
"entity/name"`, `?fact 'fact/value "verb"`). The string `"entity/name"` is
*data*, not a registered relation. Querying `(entity/name ?id ?v)` directly
is a category error — the engine has no such relation and returns an
"unknown relation" error suggesting `fact-row`. Project through one of the
builtin views instead.

```scheme
; All current facts as (id, predicate, value) triples
(query public/work/ath/lynx/theplatform/concepts/main
       (find ?fact ?pred ?value)
       (where (fact-row ?fact ?pred ?value)))

; Filter by predicate — pin ?pred to a string literal
(query public/work/ath/lynx/theplatform/concepts/main
       (find ?id ?value)
       (where (fact-row ?id "entity/name" ?value)))

; Names of language concepts (two predicates, joined on ?id)
(query public/work/ath/lynx/theplatform/concepts/main
       (find ?id ?name)
       (where (fact-row ?id "entity/type" "language-concept")
              (fact-row ?id "entity/name" ?name)))

; Numeric filter — values typed as I64 land in the typed EDB
(query public/work/ath/lynx/theplatform/concepts/main
       (find ?id ?ms)
       (where (facts_i64 ?id "service/sla_p99_ms" ?ms) (< ?ms 500)))
```

Useful relations to remember (full list via `schema.builtin_views`):

- `fact-row(?fact ?pred ?value)`
- `fact-meta(?fact ?confidence ?prov ?valid_from ?tx)`
- `fact-with-tx(?fact ?pred ?value ?confidence ?prov ?vf ?tx ?when)` — join with `tx-row` if you also need `?agent` or `?branch`. Capped at 8 columns by the engine's group/distinct op.
- `tx-row(?tx ?id ?agent ?action ?when ?branch)`
- `observation-row(?obs ?source_type ?content ?tx)`
- `belief-row(?belief ?claim ?status ?tx)`
- `branch-row(?branch ?id ?name ?archived ?created_tx)`
- typed EDBs: `facts_i64`, `facts_str`, `facts_sym` — `(facts_i64 ?e ?a ?v)` etc.

### `eval` `{ source, exom? }`

Runs raw Rayfall (any form, not just `(query ...)`). Power tool. Bypasses the
canonical-query lowering, so it doesn't auto-inject rules — `query` is what
you want for ordinary reads.

### `assert_fact` `{ exom, predicate, value, fact_id?, confidence?, source?, valid_from?, valid_to?, agent?, model?, branch? }`

Returns `{ ok, tx_id, fact_id, predicate, confidence, source }`. See §3 for
value typing and how the optional fields map onto the bitemporal model.

### `retract_fact` `{ exom, fact_id, agent?, model?, branch? }`

Closes the active tuple's `valid_to` to now and marks the fact revoked.
History is preserved — `fact_history` still returns the closed tuple.
Returns `{ ok, tx_id, fact_id }`.

### `observe` `{ exom, obs_id, source_type, source_ref?, content, confidence?, tags?, valid_from?, valid_to?, agent?, model?, branch? }`

Record an observation — raw evidence captured from a source. Cheaper than a
fact: an observation doesn't claim truth, it records what was seen. Use it
when you've read a doc, a chat, or a code snippet and want to remember the
quote/summary without committing to the claim being true.

- `source_type`: `notion-page`, `github-pr`, `chat`, `manual`, etc.
- `source_ref`: the stable id within that source (page id, PR number, ...).
- `content`: the observed material (a quote or summary).
- `confidence` defaults to 0.8.

Returns `{ ok, tx_id, obs_id }`. Read back with the `observation-row` builtin
view via `query`.

### `believe` `{ exom, belief_id, claim_text, confidence?, rationale?, supports?, valid_from?, valid_to?, agent?, model?, branch? }`

Record (or revise) a belief — a claim the agent considers true, with rationale
and confidence. Re-believing the same `claim_text` supersedes the prior active
belief (the prior one transitions to status `superseded`, history preserved).

- `claim_text`: natural-language claim.
- `supports`: list of fact ids or observation ids that back the claim.
- `confidence` defaults to 0.7.

Returns `{ ok, tx_id, belief_id }`. Read back with `belief-row` via `query`.

### `revoke_belief` `{ exom, belief_id, agent?, model?, branch? }`

Withdraw an active belief without supplying a replacement claim. Sets status
to `revoked`, closes `valid_to` to now, drops the belief from `current_beliefs`
but keeps it visible via `belief-row` (with `status="revoked"`) and
`belief_history`. Errors if the belief id isn't currently active. Use re-`believe`
with a new `claim_text` instead when you do have a replacement — that emits a
`superseded` transition.

### `list_branches` `{ exom }`

Returns each branch with `branch_id`, `name`, `parent_branch_id`,
`is_current`. Branches are light copy-on-write namespaces; most agent work
stays on `main`.

### `create_branch` `{ exom, branch_name }`

Creates a new branch off the current one. All write tools (`assert_fact`,
`retract_fact`, `observe`, `believe`, `revoke_belief`) accept an optional
`branch` argument that targets the write at a specific branch without
disturbing the exom's current-branch cursor for other callers — the switch
is held only for the duration of the write under an exclusive exom lock.
Branch ownership is enforced by TOFU: writes to a branch claimed by a
different user return `branch_owned`.

### `session_new` `{ project_path, session_type, label, agents?, agent?, model? }`

Create a new session exom under `<project>/sessions/<id>`. `session_type` is
`"single"` (only `main` branch) or `"multi"` (one branch per agent plus
`main` for the orchestrator). The authenticated user is the orchestrator
and gets `main`; the supplied `agent`/`model` are recorded on the `main`
branch as `claimed_by_agent` / `claimed_by_model`. Sub-agent labels in
`agents` get pre-allocated unclaimed branches (claimed later by
`session_join`). Returns `{ ok, session_path }`.

`label` must be non-empty and contain no `/`, `::`, or whitespace.

### `session_join` `{ session_path, agent_label, agent?, model? }`

Claim a pre-allocated sub-agent branch under TOFU on behalf of the
authenticated user. `agent_label` is the branch name (must match one of
the labels passed to `session_new`). The supplied `agent`/`model` are
recorded on the branch's audit fields. Returns the branch claimed.
Idempotent for the same `user_email`; different users colliding on the
same branch get `branch_owned`.

### `session_close` `{ session_path, agent?, model? }`

Asserts `session/closed_at = now`. Subsequent writes to the session exom
fail with `session_closed`. Reverse by retracting `session/closed_at`.

### `export` `{ exom, format? }`

`format = "json"` (default) or `"jsonl"`. Dumps current facts. Useful for
audits, not for incremental sync.

---

## 5. Discoverability flow for a fresh agent

1. `list_exoms` → which exom am I working in?
2. `exom_status { exom }` → does it have data already? what branch am I on?
3. `schema { exom }` → what predicates and views are modeled? what do I have to
   join against?
4. Read with `query` (or `explain` for quick lookups).
5. Write with `assert_fact`. Use stable `fact_id`s so future agents (and
   future-you) can supersede in place.
6. Verify with `fact_history { exom, id }`.

---

## 6. Errors you'll see

| Code / message | Cause | Fix |
|---|---|---|
| `unknown exom '<path>'` | Path not loaded / doesn't exist | Check `list_exoms`; the path is case-sensitive and uses `/`. |
| `query missing database name` | Sent `(query (find ...) ...)` with no exom path inside the form | Add the exom path: `(query <exom> (find ...) (where ...))`. |
| `rayforce2 err type` / `err arity` | Malformed Rayfall (wrong arity for a relation, mismatched parens) | Check `schema.builtin_views` for the right arity; `query` requires `(find ...) (where ...)`. |
| `rayforce2 err domain: query: evaluation failed` | Engine rejected the query at runtime. Often a sym-shape incompatibility after a rayforce2 upgrade. | If it's reproducible across exoms, fall back to `explain`/`fact_history` and surface the issue to a human. |
| `missing required parameter: <name>` | MCP arg validation | Add the missing field. |
| `invalid 'value'` | `value` JSON couldn't deserialize as a `FactValue` | Send a JSON number, string, or `{"$sym": "..."}`. Anything else (arrays, nested objects without `$sym`) is rejected. |
| `branch_owned` | Another user already claimed the branch under TOFU | Write to a branch you own (yours from `session_join`, or `main` if you're the orchestrator). |
| `session_closed` | The session exom has `session/closed_at` set; writes are rejected | Retract `session/closed_at` to reopen, or pick a different session. |

---

## 7. Out-of-scope today (file an issue or use HTTP)

- Session `archive` and `rename` (the `session/archived_at` and `session/label`
  predicates exist; you can write them through `assert_fact`, but there's no
  convenience tool yet).
- Branch-level operations beyond create/list (rename, diff, merge).
- Group-based access — sharing private `{email}/...` paths is per-email today.

When the MCP tool surface grows to cover any of these, this guide is the
right place to document it.
