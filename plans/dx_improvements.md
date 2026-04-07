# ray-exomem DX improvement plan

## Current state (2026-04-05)

ray-exomem is a self-contained daemon with:
- Native rayforce2 splayed table persistence (~/.ray-exomem/)
- Multi-exom support with directory-per-exom layout
- CLI commands for agent interaction (assert, retract, facts, import, export, etc.)
- Rayfall s-expression syntax for import/export
- Brain layer with event-sourced facts, observations, beliefs, branches, time-travel

## Known gaps

### 1. Brain and rayforce2 engine are disconnected
The Brain stores string-keyed facts in Rust Vec<T>. The rayforce2 datoms engine (rules, queries) sits alongside but isn't connected. Facts asserted through the Brain API can't be queried with `(query db ...)` Rayfall expressions.

### 2. Import parser is minimal
Hand-rolled s-expression reader only handles `(assert-fact "..." "...")`. No nested expressions, no `:keyword` arguments, no `(do ...)` blocks. Agents can't send full Rayfall scripts.

### 3. CLI flags are incomplete
`--confidence` and `--source` flags on `assert` exist but are ignored. No metadata passthrough.

### 4. Observation/belief CLI commands are missing or hacky
`observe` creates a fact with predicate "observation" rather than using Brain's `assert_observation`. No `believe`, `explain`, `history` commands.

### 5. No query CLI
Agents can list facts but can't run structured queries or search.

### 6. RAY_STR serialization gap
All strings go through the global symbol table (RAY_SYM) because rayforce2's `col_save` doesn't support RAY_STR. Long unique text permanently bloats the sym file.

## Near-term: DX essentials

### Wire confidence and source through the stack
- Extend import format: `(assert-fact "pred" "val" :confidence 0.9 :source "agent-name")`
- Update import parser to extract `:keyword value` pairs
- Pass through to `Brain::assert_fact`
- Wire CLI `--confidence` and `--source` flags

### Add `ray-exomem query <predicate>` CLI
- Returns matching facts as human-readable or JSON output
- Supports `--json` flag for machine-parseable output
- Agents need to read, not just write

### Add belief and observation CLI commands
- `ray-exomem believe "the sky is blue" --confidence 0.9 --rationale "direct observation"`
- `ray-exomem observe "temperature is 22C" --source-type sensor --tags env,temp`
- `ray-exomem explain <entity-id>` — show all transactions referencing an entity
- `ray-exomem history <fact-id>` — show all versions of a fact including retractions

### Add `--json` flag to all read commands
- `ray-exomem facts --json` → raw API response
- `ray-exomem status --json` → raw status
- `ray-exomem log --json` → raw events
- Agents parsing human-readable output is fragile; JSON is reliable

### Connect Brain to rayforce2 eval
- When `ray-exomem eval "(query db ...)"` runs, `db` should be the Brain's current state
- Build a datoms table from Brain facts, inject into the eval context
- This unifies the Brain and rayforce2 worlds

## Medium-term: multi-agent DX

### Agent identity
- Add `--agent <name>` global flag to all mutation commands
- Brain already stores `actor` on every transaction
- `ray-exomem log` shows who did what: `tx5 [agent-alpha] assert-fact — "sky-color = blue"`
- Default actor: hostname or "cli"

### Watch mode
- `ray-exomem watch` tails the SSE event stream
- Prints mutations as they happen in real-time
- Lets one agent monitor what others are doing
- `ray-exomem watch --exom research --type assert` for filtered streams

### Exom-scoped eval
- `ray-exomem eval --exom research "(query db (find ?x ?y) (where (?x :edge ?y)))"`
- Evaluates Rayfall against a specific exom's facts
- Bridge between Brain facts and rayforce2's rule/query engine
- Enable derived facts via rules that run over Brain data

## Longer-term: from the storage plan

### Fix RAY_STR serialization in rayforce2
- Add RAY_STR support to `ray_col_save` / `ray_col_load` in rayforce2's `src/store/col.c`
- This would allow Brain to use RAY_STR for long text (belief rationales, observation content)
- Avoids permanent sym table bloat for unique strings
- Alternative: add sym table compaction/GC for unused entries

### Branch support in API and CLI
- `ray-exomem branch create <name>` — fork from current branch
- `ray-exomem branch switch <name>` — change active branch
- `ray-exomem branch list` — show all branches
- `ray-exomem branch diff <a> <b>` — compare two branch states
- Multi-agent workflow: speculative branches, merge-on-consensus
- Already defined in Brain struct, just not exposed

### Provenance graph queries
- `ray-exomem why <fact-id>` — causal chain from fact to original assertions
- `ray-exomem why-not <predicate>` — explain why a fact is absent (retraction/supersession trace)
- Graph-shaped provenance output for rendering in UI or agent consumption
