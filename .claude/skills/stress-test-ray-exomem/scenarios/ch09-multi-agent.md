# Ch09 — Multi-agent (gated `--with-team`)

Verify that two sub-agents on different branches each carry their own claim
triple, that cross-branch reads work, and that double-join is idempotent.

## Gate

If `--with-team` is not set, mark every step of this chapter `skipped` with
reason "--with-team not enabled". Don't spawn agents.

## Steps

1. **TeamCreate**. Spawn 2 sub-agents into the same team:
   - **agent-a:** general-purpose subagent posing as `cursor` /
     `claude-sonnet-4-6`. Prompt below.
   - **agent-b:** general-purpose subagent posing as `claude-code-cli` /
     `claude-opus-4-7`. Prompt below.

   Each gets the **scratch session path** in its prompt and is told to:
   1. `mcp__ray-exomem__session_join { session: <session>, agent_label: "<its-label>" }`.
      Capture and report back the response.
   2. `assert_fact { exom: <session>, branch: "<its-label>", fact_id: "ag/<label>/n", predicate: "ag/<label>/n", value: 1, agent: "<agent>", model: "<model>" }`.
   3. `assert_fact { ..., fact_id: "ag/<label>/m", predicate: "ag/<label>/m", value: 2, agent: "<agent>", model: "<model>" }`.
   4. Report the two fact_ids and tx_ids back as a single message.

   Use the same MCP-bearer the parent runner uses (sub-agents inherit). If
   sub-agents report any error from session_join, surface it verbatim — that
   is the BranchOwned/BranchMissing surface.

2. **After both agents report:** `mcp__ray-exomem__list_branches { exom: <session> }`.
   Each of `agent-a` and `agent-b` must show:
   - non-empty `claimed_by_user_email` (the parent's email — sub-agents share
     bearer)
   - `claimed_by_agent` matching the sub-agent's `agent` arg
   - `claimed_by_model` matching the sub-agent's `model` arg

   This is the residual-A regression check.

3. **Cross-branch query:**
   `query { exom: <session>, branch: "agent-a", rayfall: "(query <session> (find ?id ?p ?v) (where (fact-row ?id ?p ?v)))" }`.
   Expect both `ag/agent-a/n` and `ag/agent-a/m`. **Don't** expect agent-b's
   facts on agent-a's branch.

4. **Idempotent re-join (same user):** the parent runner calls
   `session_join { session: <session>, agent_label: "agent-a" }` itself.
   This is the **same user** that owns agent-a's branch, so it must succeed
   without error (idempotency contract).

5. **Cross-user collision:** if `--with-collision-user <bearer>` is set, swap
   that bearer in and call `session_join { session: <session>, agent_label: "agent-a" }`
   from a sub-agent that uses that other bearer. Expect an error containing
   `BranchOwned`. **Skip** if no second bearer; mark `skipped` with reason
   "no --with-collision-user".

6. **Teardown for this chapter:** send `shutdown_request` to both sub-agents,
   await approvals, then `TeamDelete`. Report any agent that did not approve
   shutdown within the timeout.

## Pass criteria

- Both sub-agents report 2 fact_ids each.
- list_branches shows full claim triples for both agent branches (no missing
  user_email/agent/model).
- Cross-branch query (step 3) returns exactly the 2 agent-a facts.
- Step 4 succeeds without error.
- Step 5 (if run) returns BranchOwned.

## Evidence

- Team id and 2 sub-agent ids.
- Per agent: 2 fact_ids and 2 tx_ids.
- list_branches output (claim triples in full).
- Step 3 query result.
- Step 4: ok / error string.
- Step 5: ok / error string (or `skipped`).

## Notes

- Run this chapter **last** so the agent branches and their facts persist
  through the report (they're visible evidence post-run).
- Sub-agents are general-purpose subagents, not the dedicated `Explore`
  type — Ch09 needs writes, not read-only.
- Don't confuse `agent_label` (the branch name to claim) with `agent` (the
  attribution agent string). They can be the same or different; the chapter
  uses `agent-a` / `agent-b` for both for legibility.
