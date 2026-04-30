# Ch07 — Sessions (single, close, label validation, join)

Verify session creation flavors, the closed-session write block, and
session_join error surface.

## Steps

1. **Single-flavor session** —
   `mcp__ray-exomem__session_new { project_path: <scratch_project>, session_type: "single", label: "stress-single", agent: "claude-code-cli", model: "<your-model>" }`.
   Capture the returned exom path as `<single>`.

2. Verify single has only `main` branch:
   `mcp__ray-exomem__list_branches { exom: <single> }`. Expect **exactly 1**
   branch with label `main`. No pre-allocated agent branches.

3. **Close `<single>` and confirm writes blocked** —
   `mcp__ray-exomem__session_close { session_path: <single> }`.
   Then attempt `assert_fact { exom: <single>, fact_id: "post/close", predicate: "post/close", value: 1 }`.
   Expect MCP error with code `-32000` and message containing `session_closed`.

4. **Multi-session closed_at timeline** — for the main scratch session
   `<session>`:
   - Before any close: query `(query <session> (find ?ts) (where (?s 'session/closed_at ?ts)))`
     for the session entity. Expect 0 rows.
   - (Skip the close here — closing happens during teardown. Just record that
     `session/closed_at` is currently unset.)

5. **Bad label** —
   `session_new { project_path: <scratch_project>, session_type: "single", label: "bad/label", agent: "claude-code-cli", model: "<m>" }`.
   Expect an error mentioning `invalid label` or equivalent. The slash is the
   reserved tree-path separator.

6. **session_join with unknown agent_label** —
   `mcp__ray-exomem__session_join { session_path: <session>, agent_label: "ghost-agent" }`.
   Expect MCP error containing `BranchMissing` or `branch ghost-agent not in
   exom` (semantic equivalent — the joined branch must exist in `agents:`
   from `session_new`, which only seeded `agent-a` and `agent-b`).

7. **init scaffolds project** —
   Compose `<scratch_extra> = "<scratch_project>-extra"`.
   `mcp__ray-exomem__init { path: <scratch_extra> }`.
   Verify with `mcp__ray-exomem__tree { path: <scratch_extra> }` — expect a
   folder node containing `main` (an Exom node) and `sessions/` (a Folder
   node).

8. **exom_new creates bare exom** —
   Compose `<scratch_bare> = "<scratch_project>-bare"` (sits next to the
   scratch project under whichever `<scratch_root>` is in effect — private
   `{user_email}/test/...` by default, or `public/stress-test/...` with
   `--scratch public`).
   `mcp__ray-exomem__exom_new { path: <scratch_bare> }`.
   Verify with `mcp__ray-exomem__tree { path: <scratch_bare> }` — expect an
   Exom node (no sessions/ folder; bare exoms don't get session scaffolding).

## Pass criteria

- Step 2: `single` session has exactly `main` (no `agent-*` branches).
- Step 3: `session_closed` error returned. Verify error code is `-32000` if
  the MCP transport surfaces it (some transports drop the code; the message
  must still match).
- Step 4: closed_at unset on the live multi session.
- Step 5: bad-label error.
- Step 6: BranchMissing-style error.
- Step 7: tree shows `main` Exom + `sessions/` Folder.
- Step 8: tree shows a bare Exom node.

## Evidence

- `<single>` path.
- list_branches output for `<single>`.
- The exact error string from steps 3, 5, 6 (paste verbatim).

## Notes

- The single-flavor session can stay around — teardown doesn't need to clean
  it. Mark it as a known artifact in the report's "Notes" section.
- If step 3's error is `MCP error -32000: ...session_closed...` your skill
  passes. If the error is `permission denied` or generic `forbidden`, it's
  ambiguous — check that the session was actually closed (`session/closed_at`
  set) and re-run the assert.
- This chapter doesn't exercise multi-session creation again (the main scratch
  is multi). Skill could optionally add it if user requests — but it's
  redundant with setup.
