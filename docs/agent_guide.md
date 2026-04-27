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

What MCP `assert_fact` does **not** currently expose (use the HTTP API for
these, or wait for the tool surface to grow):

- caller-supplied `valid_from`, `valid_to`, `confidence`, `source/provenance`,
  `actor`, `branch`. The tool hardcodes `actor=mcp`, `source=mcp`,
  `confidence=1.0`, and stamps `valid_from = now()`.

When you need to record uncertain or time-bounded knowledge through MCP, encode
it in your predicates (`belief/...`, `observation/...`) until per-call
confidence/provenance lands in the tool schema.

### Re-asserting and retracting

Re-asserting with the same `fact_id` supersedes — the engine closes the prior
tuple's `valid_to` and writes a new tuple. There is no MCP `retract` tool yet;
to mark a fact gone, write a tombstone value (`"deleted"`, `{"$sym": "void"}`,
…) under a convention your queries respect, or use the HTTP
`/api/actions/retract-fact` endpoint.

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
match `exom`. Examples:

```scheme
; All current facts as (id, predicate, value) triples
(query public/work/ath/lynx/theplatform/concepts/main
       (find ?fact ?pred ?value)
       (where (fact-row ?fact ?pred ?value)))

; Names of language concepts
(query public/work/ath/lynx/theplatform/concepts/main
       (find ?id ?name)
       (where (entity/type ?id "language-concept")
              (entity/name ?id ?name)))

; Numeric filter — works only when value is I64
(query public/work/ath/.../main
       (find ?id ?ms)
       (where (service/sla_p99_ms ?id ?ms) (< ?ms 500)))
```

Useful relations to remember (full list via `schema.builtin_views`):

- `fact-row(?fact ?pred ?value)`
- `fact-meta(?fact ?confidence ?prov ?valid_from ?tx)`
- `fact-with-tx(?fact ?pred ?value ?confidence ?prov ?vf ?tx ?actor ?when)`
- `tx-row(?tx ?id ?actor ?action ?when ?branch)`
- `observation-row(?obs ?source_type ?content ?tx)`
- `belief-row(?belief ?claim ?status ?tx)`
- `branch-row(?branch ?id ?name ?archived ?created_tx)`
- typed EDBs: `facts_i64`, `facts_str`, `facts_sym` — `(facts_i64 ?e ?a ?v)` etc.

### `eval` `{ source, exom? }`

Runs raw Rayfall (any form, not just `(query ...)`). Power tool. Bypasses the
canonical-query lowering, so it doesn't auto-inject rules — `query` is what
you want for ordinary reads.

### `assert_fact` `{ exom, predicate, value, fact_id? }`

Returns `{ ok, tx_id, fact_id, predicate }`. See §3 for value typing and the
parameters MCP currently does not expose.

### `list_branches` `{ exom }`

Returns each branch with `branch_id`, `name`, `parent_branch_id`,
`is_current`. Branches are light copy-on-write namespaces; most agent work
stays on `main`.

### `create_branch` `{ exom, branch_name }`

Creates a new branch off the current one. Note: the MCP write tools do not
let you target a specific branch — they always write to the exom's current
branch. To do branch-isolated work today, drive the daemon via HTTP.

### `start_session` `{ project_path, session_type?, label? }`

**Currently a stub.** Returns `{ status: "stub" }`. Don't depend on this.
Sessions are created via HTTP `POST /api/actions/session-new` for now.

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
| `branch_owned` (writes via HTTP) | Another actor already claimed the branch under TOFU | Write to your own branch, or via MCP, which is locked to the exom's current branch. |

---

## 7. Out-of-scope today (file an issue or use HTTP)

- Asserting with `confidence`, `provenance`, explicit `valid_from`/`valid_to`,
  or a non-`mcp` actor → use `POST /api/actions/assert-fact`.
- Retract → use `POST /api/actions/retract-fact`.
- Branch-targeted writes → use `POST /api/actions/eval` or
  `assert-fact` with the `branch` field.
- Real session lifecycle (create / close / archive / join) → use the
  `/api/actions/session-*` endpoints.
- Observations and beliefs as first-class entities → only readable through
  Datalog views currently.

When the MCP tool surface grows to cover any of these, this guide is the
right place to document it.
