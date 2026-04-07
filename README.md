# ray-exomem

Native rayforce2 knowledge-base front-end using Rayfall list-style syntax as the canonical input format.

**No Teide parser, Teide AST, or Teide-to-Rayfall translation layer is present.**

## Quick start

```bash
# Run a Rayfall source file
cargo run -- run examples/native_smoke.ray

# Load (alias for run)
cargo run -- load examples/recursive_reach.ray

# Evaluate inline Rayfall source
cargo run -- eval "(+ 1 2)"
cargo run -- eval "(do (set db (datoms)) (set db (assert-fact db 1 'edge 2)) (query db (find ?x ?y) (where (?x :edge ?y))))"

# Serve the standalone UI
cargo run -- serve

# Version and backend info
cargo run -- version
```

If your rayforce2 checkout is elsewhere:

```bash
RAYFORCE2_DIR=/path/to/rayforce2 cargo run -- version
```

## Architecture

ray-exomem is a thin wrapper/orchestration layer over native rayforce2:

- **rayforce2** owns: Rayfall parser, evaluator, Datalog semantics, relation storage, query planning, and all engine primitives.
- **ray-exomem** owns: CLI entry points, wrapper helpers (`assert_fact`, `retract_fact`, `query`, `define_rule`), daemon lifecycle (future), MCP/HTTP endpoints (future), KB registry (future), and formatting/presentation.

See [MIGRATION.md](MIGRATION.md) for the full migration blueprint.

## Library API

The `ray_exomem` crate exposes typed Rust wrappers that construct Rayfall source and delegate to native rayforce2:

- `run_source(source)` / `run_file(path)` — evaluate Rayfall source
- `create_kb(engine)` — create a fresh datoms KB
- `assert_fact(engine, db, entity, attr, value)` — assert a fact
- `retract_fact(engine, db, entity, attr, value)` — retract a fact
- `query(engine, db, find_vars, where_clauses)` — run a query
- `define_rule(engine, head, body)` — define a Datalog rule

Developer note: the helper return values are formatted rayforce2 output, not reusable DB handles. If you want to chain operations, keep the Rayfall source itself in a variable and pass that source back into later wrapper calls.

## Examples

- `examples/native_smoke.ray` — basic datoms, rules, transitive closure query
- `examples/assert_retract.ray` — assert/retract round-trip
- `examples/recursive_reach.ray` — longer transitive closure chain (5 nodes, 10 reachable pairs)
- `examples/multi_relation.ray` — multiple relations joined in a rule

## Tests

```bash
cargo test
```

Tests include:
- Smoke tests for native Rayfall execution (datoms, rules, queries, assert/retract)
- Recursive transitive closure correctness
- All example files execute without error
- **Migration guardrails**: verify no Teide parser/AST/translation references in `src/` or `Cargo.toml`

## Current status (Phase 2 of migration)

Implemented:
- Native rayforce2 execution via `ray_eval_str` FFI
- CLI: `run`, `load`, `eval`, `version` with `--version` and `--help`
- Library wrappers for assert/retract/query/rule
- Migration guardrail tests
- Rayfall example programs

Blocked on rayforce2 (see MIGRATION.md §12):
- Provenance storage and proof-tree retrieval
- Persistent KB catalog and symbol round-tripping
- Temporal interval native representation and builtins
- MTL operators
- Native merge semantics
- Query optimization hooks

Not yet implemented (future phases):
- MCP server and HTTP API beyond the static UI server
- Multi-KB registry and lifecycle management
- Persistence coordination
- Export/import/backup workflows
