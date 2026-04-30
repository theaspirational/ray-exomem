# Ch01 — Discovery (read-only)

Confirm every read-side MCP tool returns plausibly-shaped output for the
scratch session.

## Calls

1. `mcp__ray-exomem__guide` — no args.
2. `mcp__ray-exomem__list_exoms` — no args.
3. `mcp__ray-exomem__exom_status { exom: <session> }`.
4. `mcp__ray-exomem__schema { exom: <session> }`.
5. `mcp__ray-exomem__list_branches { exom: <session> }`.

## Pass criteria

- `guide` returns a markdown payload **> 2 KB**. Capture byte length.
- `list_exoms` returns **≥ 1 entry**, and one entry's path equals `<session>`.
- `exom_status` returns:
  - `current_branch == "main"`
  - `facts == 0`
  - `beliefs == 0`
  - `transactions == 0` (the genesis tx is `tx/0` and isn't counted by
    `exom_status`; the first counted tx will be the first user-initiated
    write)
- `schema` lists at minimum these system attrs (case-sensitive):
  - `tx/user_email`
  - `tx/agent`
  - `tx/model`
  - `branch/claimed_by_user_email`
  - `branch/claimed_by_agent`
  - `branch/claimed_by_model`
  - `belief/supports`
  - `obs/tag`
  - `obs/tx`
  - `session/closed_at`
  And lists these `builtin_views` with these arities:
  - `fact-row` arity 3 (`?fact ?pred ?value`)
  - `fact-meta` arity 5 (`?fact ?confidence ?prov ?vf ?tx`)
  - `fact-with-tx` arity 8
  - `tx-row` arity 8 (`?tx ?id ?email ?agent ?model ?action ?when ?branch`)
  - `observation-row` arity 4 (`?obs ?source_type ?content ?tx`) — the
    `obs/source_ref` predicate exists on the entity but is **not** projected
    into the view; query it via direct EAV when needed
  - `belief-row` arity 4 (`?belief ?claim ?status ?tx`) — `belief/confidence`
    lives on the entity but is not projected
  - `branch-row` arity 5 (`?branch ?id ?name ?archived ?created_tx`)
- `list_branches` returns **4 branches**: `main` (current, claimed by the
  orchestrator), `agent-a` (unclaimed), `agent-b` (unclaimed), `probe-d`
  (unclaimed; reserved for Ch12-D's cache-staleness probe).

## Evidence to record

- guide bytes
- list_exoms count + first 3 paths
- exom_status JSON
- schema attr names found vs. expected (set diff)
- list_branches JSON (truncated to label + claim triple per branch)

## Failure modes

- guide returns **0 bytes** or `Method not found` → MCP server rebuilt without
  the guide tool; report and abort the run.
- `tx-row` arity ≠ 8 → schema regression; this is the load-bearing view for
  Ch08 attribution and Ch12 hyphen probe.
- list_branches missing `agent-a` / `agent-b` → `session_new` didn't honor
  `agents: [...]`; report and continue, marking Ch09 likely-blocked.
