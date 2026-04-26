# bootstrap/ — drop-in seed fixtures

Each `*.json` file in this directory is embedded into the daemon binary at
build time and replayed on first login as a one-time seed of the public
tree. Files are gitignored (except this README and `example.json`) so
extension developers can ship private/proprietary seed data without
committing it.

## How it works

1. `build.rs` scans `bootstrap/*.json` and emits an `include_str!()` table
   into `OUT_DIR/bootstrap_seeds.rs`.
2. `src/auth/routes.rs::bootstrap_public_tree` iterates that table on every
   successful login. For each fixture it scaffolds `<seed.path>/main`
   (idempotent) and seeds it once via `seed_bootstrap_exom`.
3. Re-login is a no-op per exom (`exom_is_bootstrapped`).

An empty `bootstrap/` directory ships a daemon with no seed data — that
is a valid configuration. A fresh `tree/main` is auto-created on first
hit either way.

## Authoring a fixture

Minimal shape:

```json
{
  "path": "public/work/<team>/<project>/<topic>",
  "branches": [],
  "facts": [],
  "observations": [],
  "beliefs": [],
  "rules": []
}
```

`path` is required. The seed is materialized at `<path>/main`.

See `example.json` for the full schema, and `src/auth/routes.rs` for the
authoritative `BootstrapSeed` types (`BootstrapFactSpec`,
`BootstrapObservationSpec`, `BootstrapBeliefSpec`, `BootstrapBranchSpec`,
`BootstrapRuleSpec`).

### Numeric vs string values

Fact values follow `FactValue { I64 | Str | Sym }` — JSON numbers become
`I64` (so `facts_i64` rules work natively), JSON strings become `Str`.
If you need a numeric predicate (`age`, `weight_kg`, …) make sure the
JSON value is a number, not a quoted string.

### Rules and `path`

If your fixture defines rules, the rule heads must reference the exom
path that the fixture seeds (e.g. `(rule public/work/team/project/main
…)`). The test in `bootstrap_fixture_tests` enforces this so a moved
fixture doesn't silently misroute.

## Verifying a new fixture

After dropping a JSON file in:

```bash
cargo build --release --bin ray-exomem
cargo test --lib bootstrap_fixture_tests
```

The fixture parses if `cargo test` passes. To watch it actually seed,
follow the live-test loop in `CLAUDE.md` — first login from a fresh
`~/.ray-exomem/` will materialize `<path>/main`.
