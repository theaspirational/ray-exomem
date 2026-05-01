# Phase 3 — Cross-user (Model A + fork + co-edit/solo-edit)

The cross-user phase. Drives both Chrome contexts opened in Phase 0 (`<page_user1>` + `<page_user2>`). Touches every cross-user invariant in one coordinated dance: Model A's auth-layer 403 in `public/*`, the `exom_fork` flow including session refusal and replayed attribution, both directions of the `acl_mode` flip, the auth-layer co-edit elevation, the branch-layer co-edit short-circuit on `main`, non-`main` TOFU preservation, audit-trail facts, mode persistence across daemon restart, and the share-grant + co-edit interaction in `{email}/*`.

Skip the entire phase if `--no-cross-user`. Otherwise, every step is mandatory.

**Pattern reminder.** Every step calls `mcp__chrome-devtools__select_page` to switch contexts, then `mcp__chrome-devtools__evaluate_script` with a `fetch()` that uses `credentials: 'include'`. Pack as much logic as possible into a single `evaluate_script` per step — round trips are the dominant cost.

## A. Model A baseline (single-user creator + multi-user denial)

A.1. **Setup (user1):** create `<public_scratch>/wiki` as a bare exom (default solo-edit). Assert one fact `wiki/topic = "rays"` with `fact_id = topic-1`.

```
select_page user1
evaluate_script:
  await fetch('/api/actions/exom-new', {
    method: 'POST', credentials: 'include',
    headers: {'content-type': 'application/json'},
    body: JSON.stringify({ path: '<public_scratch>/wiki' })
  });
  await fetch('/api/actions/assert-fact', {
    method: 'POST', credentials: 'include',
    headers: {'content-type': 'application/json'},
    body: JSON.stringify({ exom: '<public_scratch>/wiki', predicate: 'wiki/topic', value: 'rays', value_type: 'str', fact_id: 'topic-1' })
  });
```

A.2. **Confirm `created_by` + absent `forked_from`:** fetch `/api/tree?path=<public_scratch>/wiki` from user1. Pass: `created_by == <user1_email>`, `acl_mode == "solo-edit"`, no `forked_from` field in the JSON (must be **absent**, not `null`).

A.3. **(Model A 403)** user2 attempts write — `select_page user2` → POST `/api/actions/assert-fact` with `predicate: "wiki/intruder", value: "no", fact_id: "intruder-1"`. Pass: `403 forbidden`, message contains `write access denied`. **Must NOT contain `branch_owned`** — that would mean the auth-layer regression where `public/*` returned `FullAccess`.

A.4. user2 reads (`/api/query` with `(in-exom <public_scratch>/wiki (query (find ?p ?v) (where (fact-row ?f ?p ?v))))`). Pass: row `["wiki/topic", "rays"]` returned (Model A allows read-by-all in `public/*`).

## B. Fork flow

The default-target rule (introduced 2026-05-01) lands every fork under
`{user.email}/forked/<source-without-namespace-marker>`:

- `public/X/Y/Z` → `<user2_email>/forked/X/Y/Z` (drop `public/` prefix)
- `{other_email}/X/Y` → `<user2_email>/forked/{other_email}/X/Y` (preserve owner email)
- `<user2_email>/X/Y` (self-fork) → `<user2_email>/forked/X/Y`

The `forked/` prefix is the canonical organisation point. Auto-suffix `-2` / `-3` on the leaf segment if the path is taken.

B.0. **(Default-target shape — public source)** user2 forks `<public_scratch>/wiki` with **no `target` argument**. Pass: response `target` equals `<user2_email>/forked/stress-test/<UTC-ISO>-<run_id>/wiki` (the `public/` prefix is dropped, the rest of the path is preserved verbatim under `forked/`). A target like `<user2_email>/wiki` (the legacy basename-only shape) is a **fail** — that's the regression marker for the default-target rule. Capture as `<fork_default>`.

B.1. **(Fork explicit target)** user2 forks `<public_scratch>/wiki` again, this time with an explicit `target: "<user2_email>/wiki-explicit"`. Pass: response `ok: true`, `copied_facts: 1`, `forked_from.source_path == "<public_scratch>/wiki"`, `forked_from.source_tx_id` matches the source's tip, `target == "<user2_email>/wiki-explicit"` (the explicit override takes priority over the default).

B.2. **(Fork lineage in tree)** user2 fetches tree for `<fork_default>` and for `<user2_email>/wiki-explicit`. Pass: both have `created_by == <user2_email>` (the forker, not user1), `acl_mode == "solo-edit"` (forks are always solo-edit per Q6), `forked_from.source_path == "<public_scratch>/wiki"`.

B.3. **(Replayed attribution)** user2 queries `<fork_default>`:
```
(query <fork_default> (find ?fact_id ?value ?u)
  (where (fact-row ?fact_id ?p ?value)
         (?fact_id 'fact/created_by ?tx)
         (?tx 'tx/user_email ?u)))
```
Pass: every row's `?u == <user2_email>` (the forker) — the original tx attribution does NOT carry over.

B.4. **(Default-target auto-suffix)** user2 forks `<public_scratch>/wiki` a third time with no `target`. The default path `<fork_default>` is taken from B.0, so the daemon must suffix the leaf segment. Pass: response `target` ends in `/wiki-2` (or `-3`/`-4`/etc. on accumulated runs); the rest of the path matches `<fork_default>` minus the leaf. Fork once more — must yield the next free suffix.

B.5. **(Default-target shape — `{other_email}/*` source)** if user2 has a share grant on a `<user1_email>/...` exom from Phase 0 setup or an earlier step, fork it without `target`. Pass: target equals `<user2_email>/forked/<user1_email>/<rest-of-source-path>` (the source owner's email is preserved as a sub-path so lineage is readable from the path alone). If no such grant exists, skip with reason "no `{other_email}/*` source available" — the explicit-target probes (B.1) cover the rest of the fork code path.

B.6. **(Fork refuses session exoms)** user2 attempts `exom_fork { source: <session> }`. Pass: error containing `fork_session_unsupported`. (Use the orchestrator's session from Phase 0 — user2 won't have access, so this might 403 first; if so, set up via user1 a session under `<public_scratch>` and retry as user1 — fork-of-own-session must also fail.)

## C. Solo-edit → co-edit flip + auth-layer elevation in `public/*`

C.1. **(Flip endpoint, creator-only)** user1 flips `<public_scratch>/wiki` to co-edit:
```
POST /api/actions/exom-mode { exom: '<public_scratch>/wiki', mode: 'co-edit' }
```
Pass: `200 ok`, `changed: true`, `previous_mode: "solo-edit"`, `mode: "co-edit"`.

C.2. **(Branch claim cleared)** user1 reads `/api/branches?exom=<public_scratch>/wiki`. Pass: the `main` row's `claimed_by_user_email == null` (cleared by the flip).

C.3. **(Co-edit auth elevation)** user2 attempts the same write that failed in A.3 — `assert_fact { exom: <public_scratch>/wiki, predicate: "wiki/note", value: "tester contributed", fact_id: "note-tester" }`. Pass: `200 ok`, `tx_id` returned. Auth elevated to ReadWrite + branch-layer short-circuited TOFU on `main`.

C.4. **(Symmetric retract — co-editor retracts creator's fact)** user2 retracts user1's `topic-1`:
```
POST /api/actions/eval body: (retract-fact <public_scratch>/wiki "topic-1" 'wiki/topic "rays")
```
Pass: `200 ok`, then `/api/facts?exom=<public_scratch>/wiki` no longer contains `topic-1`.

C.5. **(Symmetric retract — creator retracts co-editor's fact)** user1 retracts user2's `note-tester` via the same eval form. Pass: `200 ok`, then the fact is gone.

C.6. **(Audit-trail fact)** user1 queries `/api/facts?exom=<public_scratch>/wiki` and filters for `_meta/acl_mode`. Pass: row exists with `value: "co-edit"`, `actor: <user1_email>`, `provenance: "exom-mode-flip"`.

C.7. **(Non-creator flip rejected)** user2 attempts `POST /api/actions/exom-mode { exom: <public_scratch>/wiki, mode: "solo-edit" }`. Pass: `403 not_creator`.

C.8. **(Session refuses flip)** user1 attempts `POST /api/actions/exom-mode { exom: <session-path>, mode: "co-edit" }` (use the Phase-0 session). Pass: `400 acl_mode_not_applicable`, message contains `session exoms use orchestrator-allocated branches`.

## D. Co-edit non-`main` TOFU preservation

D.1. user1 creates a feature branch on the co-edit exom: `POST /api/branches { exom: <public_scratch>/wiki, branch_id: "feat-1", name: "feat-1", parent_branch_id: "main" }`. user1 asserts on `feat-1`: `assert_fact { exom: <public_scratch>/wiki, branch: "feat-1", fact_id: "feat-1-mark", predicate: "feat/mark", value: "user1" }`. Pass: 200; the assert TOFU-claims `feat-1` for user1.

D.2. user2 attempts a write on `feat-1`: `assert_fact { exom: <public_scratch>/wiki, branch: "feat-1", fact_id: "feat-1-intruder", predicate: "feat/intruder", value: "user2" }`. Pass: `400 branch_owned by <user1_email>`. Co-edit only short-circuits TOFU on `main`; non-`main` branches preserve ownership.

## E. Co-edit child session under co-edit parent (Q7)

E.1. user1 inits a co-edit project: `POST /api/actions/init { path: <public_scratch>/proj, acl_mode: "co-edit" }`. user1 asserts a fact on `<public_scratch>/proj/main`.

E.2. user2 spawns a session under that project: `POST /api/actions/session-new { project_path: <public_scratch>/proj, type: "multi", label: "tester-spawn", agents: ["agent-x"] }`. Pass: `200 ok`, returned `session_path` is under `<public_scratch>/proj/sessions/`.

E.3. user1 reads tree for the new session. Pass: `acl_mode == "solo-edit"`, `session.initiated_by == <user2_email>`, `created_by == <user2_email>`. Sessions are always solo-edit even when their parent is co-edit.

## F. `{email}/*` co-edit flow (share-grant + co-edit + branch-bypass)

F.1. user1 creates `<priv_coedit>` (a `{user1_email}/coedit-<run_id>` path) via `exom-new`. Asserts a fact `secret/origin = "user1"`.

F.2. **(Pre-grant: user2 denied)** user2 attempts read. Pass: `403`. user2 attempts write. Pass: `403`. Tree fetch from user2 doesn't include `<priv_coedit>`.

F.3. **(Grant + flip combined)** user1 grants `read-write` share to user2 (`POST /auth/shares { path: <priv_coedit>, grantee_email: <user2_email>, permission: "read-write" }`) and flips to co-edit. Both must `200 ok`.

F.4. **(Post-grant + co-edit: user2 writes successfully)** user2 writes to `<priv_coedit>/main`. Pass: `200 ok`. The combination of the grant (auth ReadWrite) and co-edit (branch TOFU bypassed on main) lets user2 actually land facts on user1's namespace path.

F.5. **(Flip back: BranchOwned re-enabled)** user1 flips `<priv_coedit>` back to `solo-edit`. user2 attempts write. Pass: `400 branch_owned by <user1_email>`. The grant still says ReadWrite at the auth layer, but the brain layer's TOFU re-claimed `main` for user1 — the canonical legacy `branch_owned` error path. (This is the path that surfaced as the "rw-share auth allows but branch-ownership blocks" finding pre-co-edit.)

## G. Persistence across daemon restart

G.1. From within the runner (out of band — instruct the operator, or use a Bash MCP tool if available): kill the daemon and restart with the same `--dev-login-email` flags. Wait for `/auth/info` to respond.

G.2. user1 re-fetches tree for: `<public_scratch>/wiki` (co-edit), `<public_scratch>/proj/main` (co-edit, init-stamped), `<priv_coedit>` (solo-edit, post-flip-back), `<user2_email>/wiki-explicit` (solo-edit fork, owned by user2 — fetched from user2 instead, since user1 cannot read user2's namespace).

Pass: every exom's `acl_mode` matches the value set in the run, post-restart.

## Pass criteria

- A: Model A's 403 fires correctly with message containing `forbidden` + `write access denied` (NOT `branch_owned`).
- B: forks succeed with lineage stamped, attribution flips to forker, session refusal fires.
- C: every flip transition behaves correctly; audit fact lands; non-creator and session flip both reject with the right error code.
- D: non-`main` TOFU preserved under co-edit.
- E: child session under co-edit parent is solo-edit owned by the spawner.
- F: share + co-edit unblocks `{email}/*` writes; flip-back re-locks via brain-layer `branch_owned`.
- G: every exom's mode persists across restart.

## Evidence

- A.3: verbatim error string from user2's write attempt (must be `forbidden`, not `branch_owned`).
- B.1: copied_facts count + forked_from block.
- B.5: verbatim error string (`fork_session_unsupported`).
- C.1, C.7, C.8: verbatim flip-endpoint responses (success, then `not_creator`, then `acl_mode_not_applicable`).
- D.2: verbatim `branch_owned` error from user2's non-`main` write attempt.
- E.3: tree node showing the child session's `initiated_by`, `created_by`, `acl_mode`.
- F.5: verbatim `branch_owned` error from user2's post-flip-back write.
- G.2: per-exom `acl_mode` post-restart, side by side with the value set during the run.

## Notes

- Phase 3 is the only phase that runs the daemon-restart probe. Don't re-do this in another phase — the restart cost is real.
- The `<public_scratch>` exoms remain visible in the public tree after the run; the report's Notes section should list them so the operator can clean up manually.
- A failure in C.2 (main claim not cleared after flip) is a strong signal the `exom-mode` endpoint's `save_branches_to_disk` step regressed; check the handler in `src/server.rs::api_exom_mode`.
- A failure in C.3 (user2 still 403 after flip) but C.2 passed (claim is cleared) means the **auth layer** didn't pick up the new mode — `lookup_owner` may be reading stale `ExomState.acl_mode`. The handler updates that field under the exoms lock; if it's stale, the in-memory propagation regressed.
- F.4 → F.5 is the cleanest demonstration of "auth-layer write permission" vs "brain-layer branch ownership" — they're independent, and co-edit is the bridge that sometimes makes them agree.
