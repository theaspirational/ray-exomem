# Phase 0 — Setup

Verify preconditions, discover identities, open the two isolated browser contexts, and scaffold scratch. Every later phase depends on this one — if any step fails, abort with all phases marked `blocked`.

## Preconditions

1. **MCP available** — `mcp__ray-exomem__guide` must return markdown ≥ 2 KB. If it returns `Method not found`, the MCP server isn't connected; abort with that as evidence.

2. **Chrome MCP available (unless `--no-cross-user`)** — `mcp__chrome-devtools__list_pages` must succeed. If it errors with "browser is already running for ...", kill the stale `chrome-devtools-mcp` processes (`pkill -9 -f chrome-devtools-mcp`) and retry once. If still broken, abort.

3. **Daemon bound to loopback** — fetch `<base_url>/auth/info`. If `<base_url>` resolves to a non-loopback address, the dev-login route is unreachable; pass `--no-cross-user` to skip Phase 3 cross-user steps.

4. **Dev-login allow-list ≥ 2 emails** — open a fresh isolated context (`isolatedContext: "user2-probe"`) and `GET <base_url>/auth/dev-login?email=<expected_user2_email>`. A 303 redirect with `set-cookie: ray_exomem_session=...` confirms the email is in the allow-list. A 400 `email_not_allowed` means the daemon was launched with a different second email; ask the operator for the right address. A 404 `dev-login is not enabled` means the daemon has no allow-list at all; abort with "configure --dev-login-email twice on the daemon for cross-user testing." Close the probe context after.

## Steps

1. **Discover `<user1_email>`** — the runner's authenticated identity from MCP. Prefer the bearer's auth context. Fallback: `mcp__ray-exomem__tree { path: "" }` and pick the top-level node that matches the orchestrator's email if that namespace exists. Do not assume login auto-seeds `<email>/main`; private scratch is created explicitly in step 4.

2. **Discover `<user2_email>`** — passed by the operator in the run prompt, or discovered by probe (preconditions step 4). Record both emails as evidence.

3. **Compose paths**:
   - `<run_id>` = 8-char random tag (alphanumeric, lowercase).
   - `<scratch_project>` = `<user1_email>/test/<UTC-ISO>-<run_id>` (private; Phase 1+2 lives here).
   - `<public_scratch>` = `public/stress-test/<UTC-ISO>-<run_id>` (public; Phase 3 lives here).
   - `<priv_coedit>` = `<user1_email>/coedit-<run_id>` (private; Phase 3 `{email}/*` co-edit probe).

4. **Init private scratch** — `mcp__ray-exomem__init { path: <scratch_project> }`. Creates `<scratch_project>/main` and `<scratch_project>/sessions/`. Pass criterion: response `ok: true`. Failure here means the user lacks write on their own namespace — abort.

5. **Open multi-session for Phase 1+2** — `mcp__ray-exomem__session_new`:
   ```json
   {
     "project_path": "<scratch_project>",
     "session_type": "multi",
     "label": "stress",
     "agents": ["agent-a", "agent-b", "probe-d"],
     "agent": "claude-code-cli",
     "model": "<the model you're running as>"
   }
   ```
   Capture as `<session>`. The `probe-d` slot is reserved for Phase 2's cache-staleness probe so it doesn't pollute `agent-a`/`agent-b` TOFU claims.

6. **Snapshot baseline schema** — `mcp__ray-exomem__schema { exom: <session> }`. Record: list of attrs, builtin_views with arities, total relation count. Used as Phase 1 evidence and the baseline for Phase 2 regression probes.

7. **Open user1 Chrome context** (skip if `--no-cross-user`):
   ```
   mcp__chrome-devtools__new_page {
     url: "<base_url>/auth/dev-login?email=<user1_email>",
     isolatedContext: "user1"
   }
   ```
   Capture the returned `pageId` as `<page_user1>`. Verify identity:
   ```
   mcp__chrome-devtools__select_page { pageId: <page_user1> }
   mcp__chrome-devtools__evaluate_script {
     function: "async () => (await fetch('/auth/me', {credentials:'include'})).json()"
   }
   ```
   The returned `email` must equal `<user1_email>`. Pass criterion confirms cookies + dev-login wired correctly.

8. **Open user2 Chrome context** (skip if `--no-cross-user`):
   ```
   mcp__chrome-devtools__new_page {
     url: "<base_url>/auth/dev-login?email=<user2_email>",
     isolatedContext: "user2"
   }
   ```
   Capture the returned `pageId` as `<page_user2>`. Verify identity the same way; `email` must equal `<user2_email>`.

## Pass criteria

- All preconditions hold.
- Both `init` and `session_new` return `ok: true`.
- Both Chrome contexts return the expected `email` from `/auth/me`.
- Schema snapshot has `tx/user_email`, `tx/agent`, `tx/model`, `branch/claimed_by_user_email`, and at least these builtin_views with matching arity: fact-row 3; fact-meta 5; fact-with-tx 8; tx-row 8; observation-row 4; belief-row 4; branch-row 5.

## Evidence

- `<user1_email>`, `<user2_email>`, `<run_id>`, all four scratch paths.
- `init` and `session_new` response bodies.
- Schema snapshot — abbreviate to attr count + view arities.
- Page IDs of both Chrome contexts.

## Failure modes

- **`guide` returns 0 bytes / `Method not found`** → MCP server rebuilt without the guide tool; abort.
- **Chrome MCP "already running" after retry** → instruct operator to restart their Claude Code session and retry.
- **`init` fails with `forbidden`** → the runner's identity doesn't own `<user1_email>/...` (shouldn't happen unless the user namespace was migrated). Abort with the error.
- **`/auth/me` in Chrome returns the wrong email** → the dev-login allow-list got swapped; daemon needs restart with the right `--dev-login-email` order. Abort.
