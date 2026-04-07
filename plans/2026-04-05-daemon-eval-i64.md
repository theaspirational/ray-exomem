# Daemon-Backed `ray-exomem eval` with Named Exoms and Per-DB Rules

## Goal

Implement the greenfield design:

- `ray-exomem eval` always talks to the daemon
- Rayfall references exoms directly:
  - `(query research ...)`
  - `(assert-fact research entity attr value)`
  - `(retract-fact research entity attr value)`
  - `(rule research (head ...) ...)`
- `rayforce2` keeps the original `i64` datom direction
- the daemon keeps one long-lived runtime plus one cached datoms table per exom
- rules are per-exom, not global
- rules persist across daemon restarts
- one eval request may read multiple exoms but may mutate only one exom

## Product decisions locked in

- drop legacy compatibility; this is greenfield
- do not add a separate `rayfall` command
- use `ray-exomem eval`
- `eval` should go through the daemon only
- direct exom syntax, no `--exom` for `eval`
- rules use `(rule research ...)`
- rules persist across daemon restarts
- multi-exom reads are allowed in one eval request
- mutations in one eval request must target exactly one exom

## Current implementation state

### `rayforce2`

Patched files:

- `include/rayforce.h`
- `src/lang/eval.h`
- `src/lang/eval.c`

Implemented:

- public declarations for:
  - `ray_env_get`
  - `ray_env_set`
  - tagged datom helpers
  - eval mutation log accessors
  - `ray_rule_reset`
- tagged datom encoding in `eval.c`
  - `I64` stays raw
  - `SYM` and `STR` are encoded into reserved positive `i64` ranges
- `assert-fact` and `retract-fact` now operate as special forms over a named exom binding
- `assert-fact` and `retract-fact` auto-update the named env binding
- eval-scoped mutation log exists and records:
  - assert-fact
  - retract-fact
  - rule
- one-write-exom enforcement exists in the evaluator
- rules are now stored per exom binding instead of globally
- `query` now requires a named exom binding and only loads rules for that binding
- query result decoding exists for tagged `SYM` / `STR` columns
- `scan-eav` / `pull` decode tagged values on output
- `resolve` now delegates to the tagged-column decoder
- `ray_lang_destroy` resets per-db rules and the eval mutation log
- `ray_eval_str` resets the mutation log at the start of each eval

Validation already done:

- `make lib` in `rayforce2` succeeded after these changes

Important note:

- `rayforce2` also has unrelated dirty user changes in:
  - `src/datalog/datalog.c`
  - `src/datalog/datalog.h`
  - `test/test_main.c`
  - `test/test_datalog.c`
- those provenance-related changes were intentionally not touched

### `ray-exomem`

Patched files:

- `src/ffi.rs`
- `src/backend.rs`
- `src/storage.rs`
- `src/brain.rs`
- `src/exom.rs`
- `src/web.rs`

Implemented:

- `ffi.rs`
  - added FFI for env access
  - added FFI for mutation log access
  - added FFI for tagged datom helpers
- `backend.rs`
  - added `EvalMutationKind`
  - added `EvalMutation`
  - added `eval_with_mutations`
  - added `bind_named_db`
  - added `get_named_db`
  - added `reset_rules`
  - added `eval_mutations`
- `storage.rs`
  - added `RayObj::try_clone`
  - added `encode_string_datom`
  - added `decode_datom_to_string`
  - added `build_datoms_table`
- `brain.rs`
  - `Brain` now derives `Clone`
  - added `retract_fact_exact(fact_id, predicate, value)`
- `exom.rs`
  - added Rayfall-symbol validation for create/rename
- `web.rs`
  - partially refactored
  - new types added:
    - `ExomState`
    - `DaemonState`
  - startup helpers added:
    - rule file load/save
    - datoms load/build
    - runtime restore/rebind
    - mutation application helper
    - exom binding refresh helper
  - `serve()` has already been switched to:
    - create one `RayforceEngine`
    - load exoms into `ExomState`
    - restore the runtime from cached exom state
    - store everything behind `Mutex<DaemonState>`

Important note:

- `web.rs` refactor is incomplete
- the file likely does not compile yet
- handlers still contain old `brains`-based logic in some sections

## Exact next steps

### 1. Finish `web.rs`

Continue from the partially refactored file and complete these edits:

- replace all remaining `get_brain_map(...)` / `brains` usage with `get_daemon(...)`
- finish handler migration for:
  - `api_clear`
  - `api_retract`
  - `api_import`
  - `api_exom_create`
  - `api_exom_manage`
- add `api_eval(state, body)`:
  - call `daemon.engine.eval_with_mutations(source)`
  - on eval error:
    - call `restore_runtime(&daemon)`
    - return an error JSON response
  - on success:
    - call `apply_eval_mutations(...)`
    - return JSON:
      - `ok`
      - `output`
      - `mutated_exom`
      - `mutation_count`
- keep `/api/actions/evaluate` as the current no-op route
- for now, `api_import` can simply forward to `api_eval`
  - greenfield means no need to preserve the old line parser
- after `api_clear` / `api_retract`, rebuild datoms with `refresh_exom_binding(...)`
- after exom create / rename / delete:
  - update the `DaemonState`
  - call `restore_runtime(...)`

### 2. Clean `web.rs` leftovers

After switching `api_import`, remove unused legacy helpers if they are no longer referenced:

- `RayLine`
- `parse_rayfall_line`
- `extract_quoted_strings`

### 3. Patch `main.rs`

Change the CLI so:

- `Eval` gets:
  - `source: String`
  - `--addr` with default `127.0.0.1:9780`
- `Commands::Eval` does not call `run_source`
- instead it should:
  - POST raw text to `/api/actions/eval`
  - parse returned JSON
  - print only `output`
- update `Assert`, `Observe`, and `Import` to use the new eval route too
  - example:
    - `Assert` should send something like:
      - `(assert-fact default "sky" 'color "blue")`
      - or whatever exact entity/value shape is chosen for those commands
- `Run` may stay local for now unless we explicitly choose to daemonize it too

### 4. Verify `client.rs`

`client.rs` already has `post_text`, so no structural change is required.

Only verify the CLI callers are using:

- `POST /api/actions/eval`

## Validation checklist after resuming

### Build

Run:

- `make lib` in `rayforce2`
- `cargo build` in `ray-exomem`

### Smoke tests

Manual daemon flow:

1. start the daemon
2. `ray-exomem eval "(+ 1 2)" --addr 127.0.0.1:9780`
3. `ray-exomem eval "(assert-fact research 'f1 'color \"blue\")" --addr ...`
4. `ray-exomem eval "(query research (find ?e ?v) (where (?e :color ?v)))" --addr ...`
5. define a rule:
   - `(rule research (same-color ?e ?v) (?e :color ?v))`
6. restart the daemon
7. re-run the query and verify:
   - facts survived
   - rules survived

### Safety checks

Verify these cases:

- `(do (query research ...) (query notes ...))` succeeds
- `(do (assert-fact research ...) (assert-fact notes ...))` fails
- after that failure:
  - runtime still serves the previous cached state
  - no persisted partial rule/fact changes exist

## Likely compile/runtime issues to check first when resuming

- `web.rs` still has old `brains` references
- `api_eval` is not added yet
- `main.rs` still uses local `run_source` for `Eval`
- `main.rs` still routes `Assert` / `Import` through `/api/actions/import`
- some `rayforce2` tests in `ray-exomem/src/lib.rs` still assume legacy syntax like:
  - `(rule (path ?x ?y) ...)`
  - `(set db (assert-fact db ...))`
- since this is greenfield, update or remove those tests rather than reintroducing compatibility

## Resume point

Resume directly in:

- `/Users/aspirational/Documents/code/lynx/Teide/ray-exomem/src/web.rs`

Complete the handler migration and add `api_eval` before touching anything else.
