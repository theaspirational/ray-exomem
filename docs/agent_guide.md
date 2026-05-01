# Ray-exomem agent guide (MCP)

Ray-exomem is a persistent, bitemporal knowledge base for agents. This guide
describes the MCP interface — the canonical way for an agent to read, write,
and reason over an exom.

The CLI exists for human / dev workflows and is intentionally out of scope
here. Agents should not call CLI binaries.

---

## 1. Connecting

Ray-exomem speaks MCP over Streamable HTTP at `<base>/mcp`.
`<base>/mcp/sse` is also accepted as a Streamable HTTP alias for clients
whose examples or configuration UIs expect an `/sse`-suffixed endpoint.

For the hosted instance:

```
https://mem.trydev.app/mcp
Authorization: Bearer <api-key>
```

Equivalent alias:

```
https://mem.trydev.app/mcp/sse
Authorization: Bearer <api-key>
```

Issue an API key from the user's session (`/auth/api-keys`). Local dev runs at
`http://127.0.0.1:9780/mcp` or `http://127.0.0.1:9780/mcp/sse`. When local dev
is launched without an auth provider, those MCP endpoints work without a bearer
token and writes are attributed to `local@ray-exomem`.

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
its `valid_to` and stamping `superseded_by_tx` on it, then writing a new
tuple at the head of the chain. A retract closes every active tuple's
`valid_to` to retract-time and stamps `revoked_by_tx`. History is preserved
— `fact_history` returns the full chain with back-pointers intact. Retract
is an *event* (a tx in the tx log), not a value-interval.

### Permissions

Two layered access decisions, in order: the **auth layer** (does this user
have any write access at all?) and the **branch layer** (TOFU on whichever
branch the write targets). Failures at either layer surface different errors
(`forbidden` from auth, `branch_owned` from TOFU).

#### Auth layer — namespace × creator × `acl_mode`

| First path segment | Default access for an authenticated user |
|--------------------|------------------------------------------|
| `public/...` (creator)             | Full access. |
| `public/...` (other, `solo-edit`)  | Read-only — Model A: read-for-all, write-for-creator. To contribute, fork. |
| `public/...` (other, `co-edit`)    | Read + Write — anyone authenticated can write to the shared trunk. |
| `{email}/...` (owner)              | Full access. |
| `{email}/...` (other, share grant) | Per the share's `permission` field (`read` or `read-write`). |
| `{email}/...` (no share)           | Denied. |
| Anything else                      | Admin-only. |

`acl_mode` is `solo-edit` (default) or `co-edit`, set per-exom on creation
via `init` / `exom_new` (see §4) and flippable later by the creator via the
HTTP route `POST /api/actions/exom-mode` (no MCP tool yet — see §7).

#### Branch layer — TOFU on `claimed_by_user_email`

Independent of the namespace decision. Once auth says "yes, write," the brain
checks whether the target branch has been claimed by a different user. First
writer wins (TOFU); a colliding write returns `branch_owned`.

`co-edit` short-circuits the branch layer **only on `main`** — every co-editor
lands on the shared trunk. Non-`main` branches keep TOFU regardless of mode.
Flipping `co-edit → solo-edit` re-claims `main` deterministically for the
exom's `created_by`.

**Default to private. Write to `{user-email}/...` unless the user explicitly
named a `public/...` path or asked for a shared workflow.** Anything an agent
authors on behalf of one user — scratch, tests, drafts, in-progress notes,
agent-private state, regression probes — belongs in that user's private
namespace. `public/...` is reserved for bootstrap fixtures and intentionally
shared knowledge: a stray write there is visible (and writable) to every
authenticated user in the allowed domain. When in doubt, write private; the
user can graduate work to `public/...` later, but you can't undo public
exposure cleanly.

Conventional sub-roots inside `{user-email}/`:

- `{user-email}/main` — the user's primary private exom (seeded on first
  login).
- `{user-email}/test/...` — dedicated test/scratch root. Stress-test runs,
  regression probes, and other ephemeral test exoms live here so they don't
  pollute `{user-email}/main` or the public tree.
- `{user-email}/<topic>/...` — any other private project the user owns.

Sub-agents spawned by a parent runner inherit the parent's bearer token and
therefore share the parent's `user_email` — they have full access to the
parent's `{user-email}/...` namespace. Multi-agent workflows under a single
user do **not** require `public/...`; only cross-user collaboration does.

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
- a JSON string → stored as `Str`. So `"75"` remains `Str("75")`.
- `{"$sym": "active"}` → stored as a `Sym` (interned, identity-compared).

If you need numeric reasoning, send a JSON number. If you need symbol identity,
send `{"$sym": "..."}`.

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

### `tree` `{ path?, depth?, include_archived?, include_branches? }`

Walk the auth-aware tree for the calling user. Returns the user's own
namespace plus every namespace they have shares for plus the `public/*`
subtree. `path` narrows the walk to a sub-path (slash or `::` form);
`depth` caps how deep it descends. Use this before `init` / `session_new`
to confirm a path is/isn't already taken.

### `init` `{ path, acl_mode? }`

Scaffold a project at `<path>`: creates `<path>/main` (the project's main
exom) and an empty `<path>/sessions/` folder. Idempotent. Use this once
per project before issuing `session_new` calls under it. Permission-gated
the same way `assert_fact` is.

`acl_mode` is `"solo-edit"` (default) or `"co-edit"` and is stamped on the
project's `main` only — sessions are always solo-edit. Co-edit lets any
authenticated user write to `main` directly without forking; solo-edit
restricts writes to the creator (Model A in `public/*`, owner in
`{email}/*`). The mode can be flipped later via
`POST /api/actions/exom-mode` (no MCP tool yet).

### `exom_new` `{ path, acl_mode? }`

Create a free-standing bare exom at `<path>` (no `main`/`sessions`
scaffolding). Use for ad-hoc namespaces or scratch exoms that aren't part
of a project. For project setup use `init` instead. Idempotent.

`acl_mode` is `"solo-edit"` (default) or `"co-edit"`; same semantics as
`init`.

### `exom_fork` `{ source, target? }`

Fork an exom you can read into a new exom you'll own. Copies currently-active
facts as new tx records attributed to the forker, stamps `forked_from =
{ source_path, source_tx_id, forked_at }` on the target's meta, and gives
the forker `created_by` ownership. The fork is always created as
`solo-edit` regardless of the source's mode (the forker can flip it
afterward via `exom-mode`).

Refused on session exoms (`fork_session_unsupported` 400) — fork the parent
project's `main` instead.

**Default `target`** (when omitted) is `{your_email}/forked/<source-subpath>`,
where the namespace marker is stripped from the source so the lineage is
readable in the path:

- `public/X/Y/Z` → `{your_email}/forked/X/Y/Z`
- `{other_email}/X/Y` → `{your_email}/forked/{other_email}/X/Y`
  (preserves the source owner so the fork's path itself records who you
  forked from)
- `{your_email}/X/Y` (self-fork) → `{your_email}/forked/X/Y`

If the default target already exists, the leaf segment is auto-suffixed
with `-2`, `-3`, … up to 100 attempts, after which `fork_collision` is
returned and the caller must pass `target` explicitly. Passing `target`
explicitly always overrides the default (subject to write-access on the
target path).

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
`valid_from`, `valid_to`, `created_at`, plus the back-pointers `superseded_by`
(the tx that closed this interval with a re-assert) and `revoked_by` (the tx
that closed it with a retract). Use this to verify timestamps after a write,
or to read time-travel slices.

`fact_history` returns one row per asserted value-interval, **not** one row
per tx event. A retract is a tx — visible in the tx log as `tx/action =
"retract-fact"` — but it closes the existing interval rather than adding a
new one. So a sequence of three asserts followed by a retract returns three
rows: T1 carries `superseded_by = T2`, T2 carries `superseded_by = T3`, T3
carries `revoked_by = T4` and `valid_to = T4.tx_time`.

### `query` `{ exom, query, branch? }`

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

**Pin literals in the body atom, not in a separate `(= ?var "lit")`
clause.** Rayfall's `where` does not accept assignment-style equality forms:
`(where (?tx 'tx/action ?act) (= ?act "retract-fact"))` returns `rayforce2
err type: rule: cannot parse assignment expression`. Bind the literal
directly inside the body atom instead — `(where (?tx 'tx/action
"retract-fact"))`.

**Pinning a literal in a rule-call slot is supported.** Forms like
`(fact-row ?id "service/sla_p99_ms" ?v)` or `(tx-row ?tx ?id ?u ?a ?m
"merge" ?w ?br)` resolve correctly: the lowering layer derives each rule's
head-param→attribute map and tags the literal at the call site (so
`"service/sla_p99_ms"` becomes `'service/sla_p99_ms` for the sym-encoded
`fact/predicate` slot before the engine evaluates the rule). String-valued
slots (`tx/action`, `tx/branch`) are matched by rayforce2's tag-aware
compare without rewriting. Both the rule-call form and the direct EAV form
`(?id 'fact/predicate "service/sla_p99_ms")` work — pick whichever reads
more naturally.

**Don't confuse entity refs with predicate values.** `tx-row` projects
`tx/N` for `?tx` (the entity ref) and the bare numeric id for `?id` (the
`tx/id` predicate value). So `(?tx 'tx/id "tx/22")` returns 0 rows; the
correct pin is `(?tx 'tx/id "22")`. Same trap with facts: a `fact_id` and
the predicate name often look identical (e.g. `"test/n"`), but in
`(?fact 'fact/predicate ?p)`, `?fact` binds to the entity ref and `?p` to
the predicate name. When unsure, run the unpinned form first to see what
the value actually is.

**Every `find` variable must be bound by some body atom.** Projecting a var
that is pinned in the body to a constant — e.g. `(find ?id ?p ?v) (where
(fact-row ?id "fx/marker" ?v))` — fails with `dl_project: unset head-const
type` because `?p` never gets a binding. Either drop the unbound var from
`find`, or use a fully-variable body atom and add a join.

**Branch-scoped reads.** Pass `branch: "<name>"` to `query` or `eval` to
evaluate against a specific branch's view of facts/tx/observations/beliefs
without persistently changing the exom's cursor. Useful for inspecting
sub-agent branches in a multi-agent session:

```jsonc
{
  "exom": "public/work/x/y/sessions/<id>",
  "branch": "designer",
  "query": "(query public/work/x/y/sessions/<id> (find ?tx ?email ?agent ?model) (where (?tx 'tx/user_email ?email) (?tx 'tx/agent ?agent) (?tx 'tx/model ?model)))"
}
```

The brain's view is switched to `designer` for the duration of the query and
restored on the way out, so concurrent callers see the cursor unchanged.
Errors with `unknown_branch` if the branch doesn't exist or is archived.

Useful relations to remember (full list via `schema.builtin_views`):

- `fact-row(?fact ?pred ?value)`
- `fact-meta(?fact ?confidence ?prov ?valid_from ?tx)`
- `fact-with-tx(?fact ?pred ?value ?confidence ?prov ?vf ?tx ?when)` — join with `tx-row` if you also need attribution or branch. Capped at 8 columns by the engine's group/distinct op.
- `tx-row(?tx ?id ?email ?agent ?model ?action ?when ?branch)` — full three-axis attribution. Empty strings indicate no attribution recorded for that axis (system writes have `?email = ""`; cookie-auth UI writes have `?agent = ""`; writes without a model arg have `?model = ""`). Filter empties at query time with `(not (= ?v ""))` if needed.
- `observation-row(?obs ?source_type ?content ?tx)`
- `belief-row(?belief ?claim ?status ?tx)`
- `branch-row(?branch ?id ?name ?archived ?created_tx)`
- typed EDBs: `facts_i64`, `facts_str`, `facts_sym` — `(facts_i64 ?e ?a ?v)` etc.

### `eval` `{ source, exom?, branch? }`

Runs raw Rayfall (any form, not just `(query ...)`). Power tool. Bypasses the
canonical-query lowering, so it doesn't auto-inject rules — `query` is what
you want for ordinary reads.

When `branch` is supplied, the brain's view is temporarily switched to that
branch for the duration of the eval, then restored — same semantics as
`query`'s `branch` arg.

### `assert_fact` `{ exom, predicate, value, fact_id?, confidence?, source?, valid_from?, valid_to?, agent?, model?, branch? }`

Returns `{ ok, tx_id, fact_id, predicate, confidence, source }`. See §3 for
value typing and how the optional fields map onto the bitemporal model.

`predicate` is validated server-side: empty / whitespace-only / quote-bearing
names are rejected with `invalid 'predicate'`. A re-assert that hits an
existing `fact_id` closes every prior open interval (stamps
`superseded_by_tx`, clamps `valid_to` to `min(new_valid_from, tx_time)`)
and writes the new tuple as the head of the chain.

### `retract_fact` `{ exom, fact_id, agent?, model?, branch? }`

Closes every active tuple for `fact_id`: stamps `revoked_by_tx` and forces
`valid_to = now()`, overriding any explicit future projection (a tuple with
`valid_to = "2030-01-01"` becomes `valid_to = <retract_time>` — once
retracted, the projection no longer applies). History is preserved —
`fact_history` still returns each closed tuple with its `revoked_by`
back-pointer. Returns `{ ok, tx_id, fact_id }`.

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

### `merge_branch` `{ exom, branch, policy? }`

Merge `branch` into the exom's current branch. `policy` is one of
`"last-writer-wins"` (default — overwrites conflicting target facts),
`"keep-target"` (skips conflicts), or `"manual"` (returns the conflict
list without writing). Returns `{ ok, added, conflicts, tx_id }` where
`added` lists fact ids merged in and `tx_id` is the merge transaction.
The merge tx shows up in `tx-row` with `action = "merge"`.

### `archive_branch` `{ exom, branch }`

Soft-delete a branch — sets `branch/archived = "true"`. Cannot archive
`main`. Branch history is preserved; subsequent reads filter the branch
out. Returns `{ ok, exom, archived }`.

### `session_new` `{ project_path, session_type, label, agents?, agent?, model? }`

Create a new session exom under `<project>/sessions/<id>`. `session_type` is
`"single"` (only `main` branch) or `"multi"` (one branch per agent plus
`main` for the orchestrator). The authenticated user is the orchestrator
and gets `main`; the orchestrator's email plus the supplied `agent`/`model`
are recorded on the `main` branch as `branch/claimed_by_user_email`,
`branch/claimed_by_agent`, `branch/claimed_by_model` (queryable via the
EAV plane and surfaced in `list_branches`). Sub-agent labels in `agents`
get pre-allocated unclaimed branches (claimed later by `session_join`).
Returns `{ ok, session_path }`.

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
| `unknown_branch: unknown branch '<name>'` | Passed `branch:` arg that doesn't exist or is archived on the target exom | `list_branches` to enumerate; create with `create_branch` if you meant to. |
| `query missing database name` | Sent `(query (find ...) ...)` with no exom path inside the form | Add the exom path: `(query <exom> (find ...) (where ...))`. |
| `rule '<name>' expects N args, got M` | Server-side arity check rejected a body atom before evaluation | Look up the right arity in `schema.builtin_views` (`fact-row` is 3, `tx-row` is 8, typed EDBs are 3). |
| `rayforce2 err type: rule: cannot parse assignment expression` | Used `(= ?var "lit")` in a `where` clause — Rayfall doesn't accept assignment-style equality | Pin the literal directly into the body atom: `(?tx 'tx/action "retract-fact")` instead of `(?tx 'tx/action ?act) (= ?act "retract-fact")`. |
| `dl_project: unset head-const type` | A variable in `find` is pinned to a constant in the body (or otherwise never bound to a column) | Drop the unbound var from `find`, or rewrite the body atom with a real binding. |
| `unknown relation '<name>' in query body` | Body atom referenced something that isn't a typed EDB, builtin view, or user rule head | The error suggests the closest match. Use `schema.builtin_views` to enumerate. |
| `rayforce2 err domain: query: evaluation failed` | Engine rejected the query at runtime. Often a sym-shape incompatibility after a rayforce2 upgrade. | If it's reproducible across exoms, fall back to `explain`/`fact_history` and surface the issue to a human. |
| `missing required parameter: <name>` | MCP arg validation | Add the missing field. |
| `invalid 'predicate': must be non-empty` | Predicate name is empty / whitespace / contains quotes | Use a `<namespace>/<name>` form with no whitespace or quotes. |
| `invalid 'value'` | `value` JSON couldn't deserialize as a `FactValue` | Send a JSON number, string, or `{"$sym": "..."}`. Anything else (arrays, nested objects without `$sym`) is rejected. |
| `forbidden: write access denied to <path>` | Caller lacks write permission on the target path | See §2 permissions table; private paths need an explicit share. |
| `branch_owned` | Another user already claimed the branch under TOFU | Write to a branch you own (yours from `session_join`, or `main` if you're the orchestrator). On a `co-edit` exom this is bypassed for `main` only — writing to a non-`main` branch claimed by someone else still hits this. |
| `session_closed` | The session exom has `session/closed_at` set; writes are rejected | Retract `session/closed_at` to reopen, or pick a different session. |
| `cannot archive branch 'main'` | Tried to `archive_branch` on `main` | Archive a feature branch instead; `main` is the trunk and is permanent. |
| `fork_session_unsupported` | Tried to fork a session exom | Fork the parent project's `main` instead; sessions aren't durable knowledge artifacts. |
| `fork_collision` | The default fork target and 100 auto-suffixed variants are all taken | Pass `target` explicitly to choose a free path. |
| `not_creator` | Tried to flip `acl_mode` on an exom you didn't create | Mode flips are creator-only; ask the creator. (HTTP route only.) |
| `acl_mode_not_applicable` | Tried to flip `acl_mode` on a session exom | Sessions are always `solo-edit`; flip the parent project instead. |
| 0 rows from a pinned EAV query (no error, just empty result) | Often: pinned the wrong slot. `(?tx 'tx/id "tx/22")` returns 0 because `"tx/22"` is the entity ref; the `tx/id` *value* is `"22"`. Same with `fact_id` vs predicate name. | Run the unpinned form first (`(?tx 'tx/id ?id)`) to see what the value actually is, then pin against that. |

---

## 7. Out-of-scope today (file an issue or use HTTP)

- Session `archive` and `rename` (the `session/archived_at` and `session/label`
  predicates exist; you can write them through `assert_fact`, but there's no
  convenience tool yet).
- Branch `rename` and `diff` (create / list / merge / archive are MCP tools;
  rename and diff are HTTP-only today).
- `acl_mode` flip — `POST /api/actions/exom-mode { exom, mode }` is the
  only entry point today (creator-only, body `mode: "solo-edit" | "co-edit"`,
  rejected on session exoms with `acl_mode_not_applicable`). Use `exom_new`
  / `init` with `acl_mode` to set the mode at creation time from MCP.
- API-key issuance / rotation — issue keys via the UI at `/auth/api-keys`.
- Group-based access — sharing private `{email}/...` paths is per-email today.

When the MCP tool surface grows to cover any of these, this guide is the
right place to document it.
