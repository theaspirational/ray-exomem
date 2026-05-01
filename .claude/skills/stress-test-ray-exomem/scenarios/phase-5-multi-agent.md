# Phase 5 — Multi-agent collision (gated `--with-team`)

Gated phase. Spawn a 2-agent team and exercise the **same-user, multi-agent** branch-TOFU path: two sub-agents inheriting the orchestrator's bearer share `user_email` but use different `agent` labels, claim different branches via `session_join`, write concurrently, and the brain-layer attribution reflects each agent's identity per branch.

This is a different invariant from Phase 3's cross-user path:
- Phase 3 = different `user_email`, different cookie jar, exercises **auth layer**.
- Phase 5 = same `user_email`, different `agent` label, exercises **brain-layer attribution + branch TOFU**.

Both can fail independently and both need coverage.

## Setup (when `--with-team` is set, after Phase 0 completes)

1. `TeamCreate` — name a 2-agent team (`agent-a`, `agent-b`). Both inherit the orchestrator's MCP bearer.
2. Pass each agent the `<session>` path from Phase 0 plus their `agent_label` (`agent-a` / `agent-b`). The session was opened with `agents: ["agent-a", "agent-b", "probe-d"]` so both branches exist as unclaimed.

## Steps

1. **Agents join their branches** — each agent calls `session_join { session_path: <session>, agent_label: <self> }`. Pass: both joins return `ok: true`. After both joins, `list_branches { exom: <session> }` shows `agent-a.claimed_by_user_email == <user1_email>` (the orchestrator's email — agents inherit it) and `claimed_by_agent == "agent-a"` (the label distinguishes them). Same for `agent-b`.

2. **Each agent asserts 2 facts on its branch** — agent-a: `assert_fact { exom: <session>, branch: "agent-a", fact_id: "a/n", predicate: "team/agent", value: "a" }` plus a second fact. agent-b: same on `agent-b`. Pass: 4 distinct `tx_id`s, all on different branches.

3. **list_branches: full claim triple per branch** — pass: `agent-a` and `agent-b` each have non-null `claimed_by_user_email`, `claimed_by_agent`, `claimed_by_model`. Main retains the orchestrator's claim (separate from sub-agents).

4. **Cross-branch query: agent-a's tx visible** — orchestrator runs `(query <session> (find ?tx ?u ?a) (where (?tx 'tx/user_email ?u) (?tx 'tx/agent ?a) (?tx 'tx/branch ?br) (= ?br "agent-a")))`. Wait — `(= ?br ...)` is the assignment-form trap; **don't use it**. Use the pinned-literal form: `(query <session> (find ?tx ?u ?a) (where (tx-row ?tx ?id ?u ?a ?m ?act ?w "agent-a")))`. Pass: 2 rows (agent-a's two asserts), each with `?u == <user1_email>`, `?a == "agent-a"`.

5. **Second join is idempotent** — agent-a calls `session_join { session_path: <session>, agent_label: "agent-a" }` again. Pass: `ok: true`, no error, claim triple unchanged.

6. **(Cross-user collision via Chrome — bonus)** if Phase 3 ran (cross-user is on), the orchestrator and user2 both have an interest in `<session>`. user2 cannot see `<session>` (it lives under `<user1_email>/test/...`), but if `--scratch public` was passed, `<session>` lives under `public/stress-test/...` and is readable by user2. Probe the boundary: from user2's Chrome context, attempt `assert_fact { exom: <session>, branch: "agent-a", fact_id: "intruder", predicate: "intruder", value: 1 }`. Under `--scratch public`: pass criterion is `400 branch_owned by <user1_email>` (auth ReadOnly because user2 isn't the creator → so actually 403 first; this is a 403, not branch_owned). Under default-private scratch: pass is `403 forbidden`. Either way, **the write does not land**.

## Teardown for Phase 5

After all probes pass, the runner sends `shutdown_request` to each agent and waits for approval, then `TeamDelete`. The branches remain claimed; that's part of the persistent evidence in the report.

## Pass criteria

- Both agents joined their allocated branches.
- Each branch's claim triple is fully populated post-join.
- Cross-branch query returns the right rows attributed to the right agent.
- Second join is idempotent (no error, no state change).
- Bonus step 6 (if cross-user is on): cross-user write is rejected.

## Evidence

- TeamCreate response (team id + agent ids).
- Per-agent fact_ids and tx_ids from step 2.
- list_branches output focused on `claimed_by_*` triples per branch.
- Cross-branch query result (must be 2 rows for agent-a).
- Second-join response (must be ok, no error).
- Bonus: verbatim error string from cross-user write attempt.

## Notes

- Sub-agent message format: keep messages plain text. Don't send structured JSON status messages — they pollute the team chat without informing the orchestrator. The agents mark tasks complete via TaskUpdate.
- Agents go idle after each turn. That's normal and expected; idle ≠ done. The orchestrator can send messages to idle agents to wake them.
- If Phase 5 is skipped (`--with-team` not set), the matrix records "skipped — --with-team not provided" for every step. That's not a failure.
- The `probe-d` slot from Phase 0 is **not** joined here; it was used by Phase 2's cache-staleness probe (D4). If Phase 2 ran, `probe-d.claimed_by_user_email` is already set; that's fine and doesn't conflict with Phase 5.
