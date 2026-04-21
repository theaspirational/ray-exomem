# Drop JSONL as Source of Truth — Implementation Plan

**Date drafted:** 2026-04-21
**Target branch:** `feature/declarative-derivations` (or a fresh branch off it)
**Status:** Plan only — no implementation started.

---

## Why this plan exists

ray-exomem currently uses JSONL sidecars as the disk source of truth and splayed rayforce2 tables as a derived cache. The motivation for dropping JSONL and making splay authoritative is:

- Faster startup via mmap (`ray_read_splayed`) instead of JSONL parse + splay rebuild
- Less disk usage (no redundant representation)
- Cleaner data model: one disk format, one in-memory representation
- Unblocks rayforce2 as a real persistent-memory backend (the persistent-daemon use case it wasn't originally designed for)

Three prerequisite rayforce2 features already landed (on branch `feature/datalog-aggregates`, PR [RayforceDB/rayforce2#7](https://github.com/RayforceDB/rayforce2/pull/7)):

1. **`ray_runtime_create_with_sym(path)` / `ray_runtime_create_with_sym_err(path, &err)`** — loads persisted symbol table **before** `ray_lang_init` registers ~175 builtins, so persisted user sym IDs keep their slots across restarts and rayforce2 upgrades.
2. **Zero-copy mmap reads** — `ray_read_splayed` returns a MAP_PRIVATE-backed table; startup becomes O(1) instead of O(data size).
3. **Atomic splay directory swap** — `save_table` in ray-exomem writes to `dir.new/`, then renames `dir → dir.old → dir.new → dir`, with `recover_splay_dirs` on startup cleaning up any interrupted swap.

ray-exomem already consumes all three (see commits `21dfca8`, `467b638`, `fc24a0f` on `feature/declarative-derivations`).

## Current state audit (2026-04-21)

### Data parity between JSONL and splay

Audited all 5 Brain tables (facts, txs, observations, beliefs, branches). Result: **splay can round-trip 5/5 tables**, with ONE gap:

| Table | Gap? |
|---|---|
| fact | None. `value_kind` column correctly disambiguates `FactValue::I64`/`Str`/`Sym`. Legacy splay (`ncols < 12`) falls back to `Str`. |
| tx | None. Graceful fallback for missing `session`/`user_email` cols. |
| observation | None. |
| belief | None. |
| **branch** | **`claimed_by` field is NOT persisted to splay.** Hard-coded to `None` on load. Comment in `src/storage.rs::load_branches` says "splay tables don't store this; JSONL is the source of truth". |

### Current code paths

- **Happy path read**: `Brain::open_exom()` (src/brain.rs:252) loads splay only — no JSONL touched.
- **Happy path write**: `Brain::persist_table` (src/brain.rs around :475) writes splay; `Brain::write_jsonl` (src/brain.rs:496) writes JSONL. Both are called after every mutation.
- **Recovery path**: `Brain::open_exom_from_jsonl` exists for "sym/splay corrupt after binary upgrade" scenarios. Called from `server.rs` only when splay load returns error.
- **Startup**: `server.rs::from_data_dir` loads sym via `ray_runtime_create_with_sym_err`, logs + wipes on corrupt, then opens each exom via `Brain::open_exom`.

### What works without JSONL today

- Reading an exom that has splay + sym on disk: ✅ works
- Crash recovery via `recover_splay_dirs`: ✅ works
- Binary upgrades where rayforce2 adds builtins: ✅ works (user sym IDs stable)
- Branch `claimed_by` round-trip: ❌ broken — currently masked by JSONL rebuild

## Phase 0 — Close the branch `claimed_by` gap (prerequisite)

**Scope:** single PR, small, low risk, mergeable immediately.

Required changes (all in `src/storage.rs`):

1. In `build_branch_table` (around line 1856–1872): add a 6th column `claimed_by` (sym, nullable). Use `TableBuilder` with the `encode_optional_string_as_sym`-style helper already used for `valid_to` in other tables.
2. In `load_branches` (around line 1875–1902): read the 6th column, guard with `if ncols >= 6` for backward compat, populate `Branch.claimed_by` from the sym value.
3. Remove the hard-coded `claimed_by: None` with its "JSONL is the source of truth" comment.

Test additions (in `tests/` or an existing integration test file):

- Round-trip test: create brain, set `Branch.claimed_by = Some("operator@x")`, save splay, clear in-memory, load splay, assert `claimed_by == Some("operator@x")`.
- Backward compat test: load a fixture splay dir created WITHOUT the claimed_by column (simulate by writing a 5-column branch.d), verify loader succeeds with `claimed_by = None`.

Verification:

```bash
cargo test  # all existing tests must still pass
cargo test branch_claimed_by  # new test
```

## Phase 1 — Make splay authoritative, keep writing JSONL

**Scope:** separate PR after Phase 0 lands.

Goal: at runtime, splay is the only thing we trust. JSONL is still written but never read on happy path. Removes the "JSONL is source of truth" invariant without yet removing JSONL I/O.

Changes:

1. **In `Brain::open_exom` (src/brain.rs:252)** — add a post-load verification step:
   - After loading all 5 splay tables into Vecs, verify internal consistency (e.g., every `Fact.created_by_tx` references an existing `Tx.tx_id`). This already partially exists; make it strict and return Err on violation.
   - On verification failure: if JSONL exists, log loudly and fall back to `open_exom_from_jsonl`. If JSONL missing too, fail the exom open.

2. **Add `src/migration.rs`** (new file):
   - `migrate_jsonl_to_splay(exom_dir, sym_path) -> Result<MigrationReport>` — reads JSONL, reconstructs Vecs, calls `brain.save()`, then loads splay and diffs against the original Vecs. Returns counts of facts/txs/obs/beliefs/branches migrated, plus any field-level mismatches.
   - Call this automatically on exom open ONLY when splay is missing OR splay version is older than current code version (tag splay dirs with a `.version` file).

3. **Gate `save_all_jsonl()` behind a config flag** — `RAY_EXOMEM_WRITE_JSONL=1` (default true for now). Phase 2 will flip the default.

Tests:

- Simulated corruption: delete `fact/.d`, call `open_exom`, verify error is surfaced cleanly (not a silent JSONL rebuild).
- Migration test: start with JSONL only (no splay), call `migrate_jsonl_to_splay`, verify splay matches JSONL byte-for-field.
- Verification test: seed splay with a dangling `Fact.created_by_tx`, verify `open_exom` returns Err.

## Phase 2 — Stop writing JSONL

**Scope:** separate PR after Phase 1 has baked in production. "Baked" = real write traffic against Phase 1 code with zero JSONL-fallback triggers for a reasonable window.

Changes:

1. Flip default of `RAY_EXOMEM_WRITE_JSONL` to false.
2. Remove all `write_jsonl` call sites on mutation paths — search `src/brain.rs` and `src/server.rs` for `write_jsonl(`.
3. Remove `save_all_jsonl()` from post-load hook.
4. Keep `load_jsonl`/`save_jsonl` functions — they become emergency recovery tools only (Phase 3).

Before merging: run `ray-exomem verify --all-exoms` (Phase 3 CLI) against every exom in a staging environment. Zero diffs required.

## Phase 3 — Migration and verification tooling

**Scope:** can overlap with Phase 1/2; useful standalone.

New CLI subcommands (extend `src/main.rs`):

- `ray-exomem migrate --exom <name>` — explicitly rewrite splay from JSONL for one exom. Idempotent.
- `ray-exomem migrate --all-exoms` — bulk variant.
- `ray-exomem verify --exom <name>` — load splay, load JSONL, diff field-by-field, report mismatches without writing.
- `ray-exomem recover-from-jsonl --exom <name>` — emergency tool: deletes splay dir, rebuilds from JSONL. Requires `--yes-i-mean-it`.

Tests: unit tests for the diff logic in `verify`, integration test for `migrate` round-trip.

## Phase 4 — Delete JSONL code

**Scope:** final PR after Phase 2 is stable.

Changes:

1. Delete `save_jsonl`, `save_all_jsonl`, `write_jsonl`, all callers.
2. Delete `Brain::open_exom_from_jsonl` — recovery is now via Phase 3 CLI.
3. Delete JSONL scaffolding in `src/scaffold.rs::new_bare_exom`.
4. Delete `load_jsonl` functions except the ones used by the `recover-from-jsonl` CLI command.
5. Update CLAUDE.md: remove "JSONL sidecars are the source of truth; splay tables are the cache" — reverse the statement.
6. Update docstrings and comments throughout referencing JSONL as source of truth.

## What this plan does NOT change

- **Postgres backend** (feature-gated, `src/db/pg_exom.rs`) — in pg mode, splay is a read cache rebuilt from pg. JSONL in pg mode can be dropped too, but that's orthogonal; no coordination required.
- **Auth JSONL** (`auth.jsonl`, session logs) — separate subsystem, not covered here.
- **Sym persistence format** — already stable via `ray_runtime_create_with_sym`.

## Gotchas and things NOT to do

These are errors a fresh agent might make if not warned:

1. **Do not change `Brain::open_exom_from_jsonl`'s call sites preemptively.** It's the existing recovery fallback. Phases 1–3 preserve it; Phase 4 removes it. Out-of-order removal breaks the safety net.

2. **Do not bundle Phase 0 with anything else.** The `claimed_by` gap is a correctness bug visible even without this plan — it deserves its own PR for reviewability and reversibility.

3. **Do not add a new sym table per exom.** The global sym table is shared across all exoms in a single process and across restarts via `ray_runtime_create_with_sym`. Per-exom sym tables would fragment the runtime.

4. **Do not write the sym file mid-mutation.** Current code calls `storage::sym_save(sym_path)` in `Brain::persist_table` (src/brain.rs:491). Keep that pattern — sym_save is append-only and idempotent. Do NOT try to "optimize" by only saving sym on shutdown; a crash would leave column data referencing un-persisted sym IDs.

5. **Do not rely on file mtime to decide migration.** Use a `.version` marker file written atomically alongside `.d`.

6. **Do not forget the `Postgres mode` path in `Brain::open_exom_from_db`.** Verify your changes don't break the pg-as-source-of-truth deployment. It uses splay as a cache rebuilt on open; anything that changes the save/load contract affects it.

7. **Do not change the atomic swap protocol** in `storage::save_table` / `recover_splay_dirs` without updating the other. They are symmetric.

## Key file pointers

| What | Where |
|---|---|
| Brain mutation flow | `src/brain.rs::persist_table` (~line 475) and `::write_jsonl` (~line 496) |
| Splay I/O | `src/storage.rs::save_table`, `::load_table`, `::recover_splay_dirs` (lines 173–230) |
| Brain table loaders | `src/storage.rs::load_facts/load_txs/load_observations/load_beliefs/load_branches` |
| Brain table builders | `src/storage.rs::build_fact_table/build_tx_table/etc.` |
| Branch `claimed_by` gap | `src/storage.rs::build_branch_table` (line ~1866) and `::load_branches` (line ~1875) |
| Runtime init & sym load | `src/backend.rs::RayforceEngine::new_with_sym` |
| Startup flow | `src/server.rs::from_data_dir` (line ~108) |
| JSONL load/save | `src/storage.rs::save_jsonl`, `::load_jsonl`, scattered `write_jsonl` calls |
| Recovery path | `src/brain.rs::open_exom_from_jsonl` |
| rayforce2 sym API | `src/core/runtime.c::ray_runtime_create_with_sym_err` (rayforce2 repo, branch `feature/datalog-aggregates`) |

## Recent relevant commits (on `feature/declarative-derivations`)

```
fc24a0f fix: surface rayforce2 sym_load errors via new _err API
21dfca8 feat(storage): atomic splay directory swap for crash-safe writes
467b638 feat: stable sym IDs via ray_runtime_create_with_sym + mmap splay load
547abff feat(admin): factory-reset action in System tab (top-admin only)
aabeb74 docs: note rayforce2 feature dependency + typed FactValue splay invariants
eafd05d refactor(server): delete hardcoded known_derived_samples lookup
```

`feature/declarative-derivations` is ahead of `main` by ~19 commits; rayforce2 `feature/datalog-aggregates` is open as PR#7 upstream. Do not merge this plan's Phase 4 until rayforce2 PR#7 is merged upstream and ray-exomem's `build.rs` is updated (it currently clones rayforce2 at `--branch master` — Phase 4 depends on the sym features being in master).

## Verification checklist per phase

Each phase must pass before moving to the next:

- [ ] `cargo test` green
- [ ] `cargo build --release --features postgres` green
- [ ] Local daemon starts against an existing exom directory
- [ ] Create fact, retract fact, create branch, query — all work
- [ ] Stop daemon, restart, verify state preserved
- [ ] For Phase 2+: verify JSONL files are not modified by normal operation
- [ ] For Phase 4: verify `ray-exomem recover-from-jsonl` still works on an old JSONL-only exom

## Open questions for the implementing agent

These are decisions the plan deliberately does NOT make — the implementing agent should resolve them with the user:

1. Should Phase 2 be a hard cut (flip flag, remove writes) or a staged rollout (feature flag, then remove)?
2. Should `ray-exomem verify` have a `--fix` mode that rewrites splay from JSONL on mismatch?
3. Should we version splay dirs with a `.version` file or rely on column-count detection (the current pattern)?
4. For Phase 4, what's the concrete "baked long enough" signal? Time-based, exom-reopen-count-based, or an explicit user go-ahead?
