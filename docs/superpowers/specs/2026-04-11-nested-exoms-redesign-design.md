# Nested Exoms Redesign — Design

**Date:** 2026-04-11
**Status:** Approved design, ready for implementation plan
**Scope:** Persistence layout, CLI surface, HTTP API, UI structure, agent guidance

## 1. Summary

Ray-exomem today stores memory as a flat namespace of exoms. This design replaces it with a tree of folders and exoms, shipped with one fixed schema (project with `main` + `sessions/`), and reworks the CLI, HTTP API, and UI around that tree. It introduces a unified path-based addressing model (`work::team::project::repo::main` on the CLI, `work/team/project/repo/main` on disk and in the UI), TOFU branch ownership for multi-agent sessions, and an agent-guidance surface (`--help` blurb plus a `ray-exomem guide` subcommand) that teaches the workflow on first contact.

## 2. Goals

- A mental model that scales past a handful of exoms by giving them structure.
- A CLI surface agents can learn in one pass via `--help` and `guide`.
- Multi-agent session support with real write isolation (not just metadata tagging).
- A UI that can show the tree, the current focus, and the parallel work of multiple agents in the same session.
- Keep the greenfield posture: no migration, no legacy shims, no user-defined schema DSL.

## 3. Non-goals (v1)

- Branch merging. `session close` freezes writes; it does not merge.
- Cross-exom queries. Each query targets one exom, one branch.
- Agent authentication beyond TOFU. No tokens, no keys.
- User-defined schema DSL. Shipping one fixed schema.
- Remote daemons, multi-host trees.
- CLI parity for rename. Rename is UI-only in v1.

## 4. Conceptual model

### 4.1 Two node kinds

- **Folder** — pure grouping. Holds child folders and child exoms. No facts, no branches, no metadata beyond its name.
- **Exom** — leaf that holds facts and branches. Cannot contain other nodes.

### 4.2 Addressing

- **CLI:** `::`-separated path. Example: `work::team::project::repo::main`.
- **Disk and UI:** `/`-separated path. Example: `work/team/project/repo/main`.
- **One unified address flag:** `--exom <path>`. No `--project`, no `--session` on the addressing side. Paths must end at an exom, except on commands that accept folder paths (`inspect`, `init`).
- **Branch is separate:** `--branch <name>`. Defaults to literal `main`. Not part of the path.
- **Actor is separate:** `--actor <name>`. Required on every write.

### 4.3 Path segment syntax

- `[_A-Za-z0-9-][_A-Za-z0-9.-]*` — alphanumerics, underscore, hyphen, dot. No slashes, no `::`, no whitespace inside segments.
- **Reserved segment:** `sessions`. Can only exist as a folder created by `init`. `exom new <path>::sessions` is rejected.
- Default branch name is literally `main`. Not configurable.

### 4.4 Fixed schema

Ray-exomem ships **one** schema. Users do not define their own.

- `init <path>` — creates any missing folder segments along `<path>`, then creates `main` (exom) and `sessions/` (folder) inside the leaf. Idempotent. Safe at any depth. Projects nest freely: `init work::ath` and `init work::team::project::repo` coexist, each is a project, facts can live at multiple depths in the same chain.
- `exom new <path>` — creates a bare exom at the given path with no scaffolding. Parent folders are auto-created. Escape hatch for arbitrary memory spaces that don't need `main`/`sessions`.

## 5. On-disk persistence layout

### 5.1 Directory tree mirrors logical paths

```
~/.ray-exomem/
├── sym                              # global symbol table (shared across all exoms, unchanged)
├── sym.lk
└── tree/                            # renamed from today's `exoms/`
    └── work/                        # folder (no exom.json)
        └── ath/                     # folder
            └── team/                # folder
                └── orsl/            # folder (also a project)
                    ├── main/        # exom (has exom.json + splay tables)
                    │   ├── exom.json
                    │   ├── tx/       (.d .i .o .s)
                    │   ├── fact/
                    │   ├── observation/
                    │   ├── belief/
                    │   └── branch/
                    └── sessions/    # folder
                        └── 20260411T143215Z_multi_agent_landing-page/
                            ├── exom.json
                            ├── tx/ fact/ observation/ belief/ branch/
                            └── …
```

- **Folder vs exom detection:** a directory is an exom iff it contains an `exom.json`. Otherwise it's a folder. Empty folders are real empty dirs. No sidecar files needed for folders.
- **Folders cannot exist inside exoms.** Exoms are leaves. Any scaffold operation (`init`, `exom new`, `session new`) that would place a segment under an existing exom is rejected with `cannot_nest_inside_exom`. `init` additionally rejects if the target leaf path is already an exom with a different `kind` (e.g. attempting to `init` on top of a bare exom).
- **Splay tables unchanged.** Every exom still has its own `tx/ fact/ observation/ belief/ branch/` columnar layout.
- **Global `sym` symbol table** stays at the root and is still shared across every exom in the tree.

### 5.2 `exom.json` schema (format_version 2)

```json
{
  "format_version": 2,
  "current_branch": "main",
  "kind": "project-main" | "session" | "bare",
  "created_at": "2026-04-11T14:32:15Z",
  "session": {
    "type": "multi" | "single",
    "label": "landing-page",
    "initiated_by": "orchestrator",
    "agents": ["orchestrator", "agent_a", "agent_b"],
    "closed_at": null,
    "archived_at": null
  }
}
```

- `session` is present only when `kind == "session"`.
- `session.label` is mutable (rename updates it). `session.type`, `session.initiated_by`, and the directory name are immutable.
- `agents` is a denormalized list for quick startup; the authoritative list is the set of branches in the exom's `branch/` splay table.

### 5.3 Session id format

- Directory name: `<YYYYMMDDTHHMMSSZ>_<multi|single>_agent_<label>`.
- `YYYYMMDDTHHMMSSZ` is compact ISO 8601 basic (UTC). Filesystem-safe on every platform, sortable, unambiguous.
- Example: `20260411T143215Z_multi_agent_landing-page`.
- Session id (= directory name) is **immutable**. Rename mutates `session.label` only.

### 5.4 Session metadata mirroring

Session metadata writes flow through the normal fact path (asserts against reserved system attributes), and the server mirrors them into `exom.json` on write so daemon startup is cheap and consistent:

| Attribute | Meaning |
|---|---|
| `session/label` | Mutable display label |
| `session/closed_at` | Non-null ⇒ writes rejected to this exom |
| `session/archived_at` | Non-null ⇒ hidden from default `inspect` |
| `branch/claimed_by` | TOFU owner of a branch |

### 5.5 Migration

None. Existing `~/.ray-exomem/exoms/` directories are ignored by the new daemon. Users coming from the old model run `rm -rf ~/.ray-exomem` and start fresh. `ray-exomem guide` will call this out in its first section.

## 6. Branch ownership and multi-agent sessions

### 6.1 Rules

- **Orchestrator allocates branches.** `session new --agents a,b,c` pre-creates branches `a`, `b`, `c`. The orchestrator itself always claims `main` (TOFU on session create, using the `--actor` passed at `session new`).
- **Non-orchestrator agents cannot create branches.** Only orchestrator can — via `session new --agents` up front, or `ray-exomem session add-agent` mid-session (which hits `POST /api/actions/branch-create` server-side).
- **TOFU ownership.** First write from `<actor>` to an unclaimed branch claims it. Subsequent writes with a different actor are rejected.
- **Read any branch, write only your own.** Reads require no actor.
- **Session close.** `session/closed_at` non-null ⇒ all writes to that exom rejected. Reads continue to work.
- **Single-agent sessions.** One `main` branch by default. The agent can create extra branches for exploration via `branch-create` (in a single-agent session, the initiator is trivially the orchestrator).

### 6.2 Server enforcement

- `branch/claimed_by` stored as a regular fact on the branch entity.
- Every `assert-fact` / `retract` / mutating `eval` call checks:
  1. Exom exists → else `no such exom`.
  2. `session/closed_at` null → else `session closed`.
  3. Branch exists → else `branch not in exom`.
  4. Branch claim matches `--actor` → else `branch owned by <other>`.
- All four checks happen before touching storage.

## 7. CLI surface

### 7.1 Commands

```
# Scaffolding
ray-exomem init <path>                                 # folders + main + sessions/
ray-exomem exom new <path>                             # bare exom, no scaffolding

# Sessions
ray-exomem session new <project-path> \
    (--multi | --single) --name <label> \
    --actor <orchestrator> [--agents a,b,c]
ray-exomem session add-agent <session-path> --agent <name>
ray-exomem session join     <session-path> --actor <name>
ray-exomem session rename   <session-path> --label <new-label>
ray-exomem session close    <session-path>
ray-exomem session archive  <session-path>

# Inspection
ray-exomem inspect [path] [--depth N] [--branches] [--archived] [--json]
ray-exomem guide

# Reads
ray-exomem query --exom <path> [--branch <name>] [--json] <rayfall>
ray-exomem history <fact-id>
ray-exomem why     <fact-id>
ray-exomem expand-query --exom <path> <rayfall>

# Writes (TOFU enforced)
ray-exomem assert <pred> <val> --exom <path> [--branch <name>] --actor <name> \
    [--source ...] [--confidence ...] [--valid-from ...] [--valid-to ...]
ray-exomem retract <fact-id> --exom <path> --actor <name>

# Daemon
ray-exomem daemon | stop | serve [--bind ...]
ray-exomem doctor --exom <path>
```

### 7.2 Global flags

| Flag | Meaning |
|---|---|
| `--exom <path>` | Required on almost every command. Unified addressing. `::` separator. Must end at an exom except on `inspect`/`init` which accept folder paths. |
| `--branch <name>` | Optional. Defaults to `main`. |
| `--actor <name>` | Required on writes. TOFU-claims on first touch. |

### 7.3 Rayfall body forms

Any command that takes a rayfall body accepts three forms:

- **Inline string:** `'(query …)'`
- **File:** `@path/to/query.ray` — rayfall file extension is `.ray`.
- **Stdin:** `-` — read until EOF.

### 7.4 Multi-line convention

All example output in `--help`, `guide`, and `inspect --help`-style text renders commands on multiple lines with `\` continuations. The `guide` doctrine explicitly tells agents: "write multi-line commands for readability; use `@file` or stdin for non-trivial rayfall." This is a convention, not a parser rule — shells already handle continuations.

### 7.5 Defaults

| Decision | Default |
|---|---|
| `inspect` depth cap | `2` |
| `inspect` archived visibility | hidden (use `--archived`) |
| `inspect` branches visibility | hidden (use `--branches`) |
| Default branch name | `main` |
| Session id date format | `YYYYMMDDTHHMMSSZ` (UTC, compact ISO 8601 basic) |
| Orchestrator branch | always `main` |
| Daemon tree root | `~/.ray-exomem/tree/` |
| Rayfall file extension | `.ray` |
| `--exom` targeting a folder | rejected except on `inspect`/`init` |
| Intermediate folders on any scaffold | auto-created silently |

### 7.6 Gone

- Flat `--exom <name>`. Every address is path-based.
- Any command that assumes a flat exom namespace.

## 8. HTTP API

### 8.1 New endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/tree?path=&depth=&archived=&branches=&activity=` | Folder/exom tree — mirrors `inspect`. `activity=true` returns per-exom `recent_activity` (timestamp of last write) for the rename confirmation modal. |
| `POST` | `/api/actions/init` | `{ path }`. Scaffolds folders + `main` + `sessions/`. |
| `POST` | `/api/actions/exom-new` | `{ path }`. Bare exom. |
| `POST` | `/api/actions/session-new` | `{ project_path, type, label, actor, agents? }`. Creates session exom + pre-allocates branches. |
| `POST` | `/api/actions/session-join` | `{ session_path, actor }`. TOFU-claims branch. |
| `POST` | `/api/actions/branch-create` | `{ exom, branch }`. Orchestrator-gated generic branch creation. |
| `POST` | `/api/actions/rename` | `{ path, new_segment }`. Renames last segment of a folder or exom. Rejects session exoms. Emits SSE `tree-changed`. |
| `GET` | `/api/guide` | Agent doctrine as markdown. |

### 8.2 Cut vs merged-into-assert

Session metadata mutations (rename-label, close, archive) do **not** get dedicated endpoints. They go through the normal `POST /api/actions/assert-fact` path using reserved attributes:

- `session/label` — mutable string.
- `session/closed_at` — timestamp; non-null ⇒ writes rejected.
- `session/archived_at` — timestamp; non-null ⇒ hidden from default `inspect`.

The server watches these on assert and mirrors into `exom.json`.

### 8.3 Changed endpoints

| Endpoint | Change |
|---|---|
| `GET /api/status?exom=<path>` | `exom` accepts a path. `storage.exom_path` shows full path. `server.tree_root` added. |
| `POST /api/query` | `exom` in body is a path. `branch` required on writes, not reads. TOFU enforced. |
| `POST /api/actions/assert-fact` | Requires `actor`. Rejects on TOFU mismatch or `session.closed_at`. |
| `POST /api/actions/eval` | Requires `actor` on writes. |
| `GET /api/facts/<id>?exom=<path>` | Path-based. |
| `GET /api/explain?exom=<path>&…` | Path-based. |
| `GET /api/branches?exom=<path>` | Path-based. |
| `GET /api/exoms` | 410 Gone. Hint: use `/api/tree`. |

### 8.4 SSE events

- `tree-changed` — fires on `init`, `exom-new`, `session-new`, `branch-create`, `rename`. UI listens and refreshes the drawer tree.
- Existing `tx-committed` / `fact-changed` events keep their shape but add `exom_path` instead of flat name.

## 9. UI structure

### 9.1 Shell

- **Top bar:** breadcrumb path, current branch indicator, current actor. Always visible.
- **Left drawer (32px collapsed rail):** expands over the content on hover/click, not squeezing it. Icons for tree, recents, search, settings.
- **Main area:** focus view for the currently selected node.
- **Status bar:** daemon health, loaded exoms, tree root path.

### 9.2 Drawer tree

- Full tree with lazy-loaded folder children.
- Exom nodes color-coded by kind: project-main green, session blue, bare gray, archived dim.
- Right-click menu: `init here`, `exom new`, `session new`, `rename`, `close`, `archive`.
- Root is `~/.ray-exomem/tree/`.

### 9.3 Focus view shapes

- **Folder selected:** grid of children (exoms first, folders after). Inline quick actions: `init here`, `exom new`, `session new`.
- **Project-main / bare exom selected:** header (path, fact count, current branch, kind), tabs `Facts | Branches | History | Graph | Rules`.
- **Session exom selected:** same as project-main, *plus* a **Mode** toggle in the Facts tab header: `Switcher (default) | Kanban | Timeline`.
  - **Switcher:** pill list of branches, one branch at a time. Default.
  - **Kanban:** one column per branch, all visible in parallel.
  - **Timeline:** all branches interleaved chronologically, color-coded, filter pills.
- **Archived exom selected:** read-only and dim, with an "Unarchive" action.

### 9.4 Rename modal

Triggered from the drawer right-click menu. Behaves differently based on node kind:

- **Folders and non-session exoms (project-main, bare):** full directory rename, affects all descendants. Uses the modal below.
- **Session exoms:** right-click shows *"Rename label…"*, which opens a simple single-field modal that writes to `session/label`. The session directory name never changes.

For folders and non-session exoms, the modal is:

```
Rename "work/team/project/repo" → "work/team/project/repo2"

This will change 14 descendant paths:
  work/team/project/repo/main                    → work/team/project/repo2/main
  work/team/project/repo/sessions/20260411…      → work/team/project/repo2/sessions/20260411…
  …

⚠ Running agents targeting the old path will fail on their next write.
   2 sessions have activity in the last 15 minutes:
     · sessions/20260411T143215Z_multi_agent_landing-page (branches: main, agent_a)
     · sessions/20260411T150002Z_single_agent_refactor

[ Cancel ]   [ I understand — rename ]
```

The "recent activity" list comes from `GET /api/tree?path=<x>&activity=true`.

### 9.5 Routes

```
/                          → redirect to /tree/
/tree/<path>               → focus view for that node (folder or exom)
/tree/<path>?branch=&mode= → query params for branch selection and session view mode
/facts/<id>                → fact detail (unchanged)
/guide                     → renders /api/guide as markdown
```

### 9.6 Cross-cutting

- **Command palette (⌘K):** fuzzy go-to-path, "switch branch", "open guide", "init here". Replaces half the right-click menu.
- **Actor identity:** stored in `localStorage`. Prompted on first write if unset. Sent on every API call as a header. UI uses it for its own TOFU behavior.

### 9.7 Cut from today's UI

- `/exoms` flat list page.
- `/branches/[id]` standalone page (folds into exom Branches tab).
- `/dependencies` as its own route (merged into Graph).
- Dashboard `+page.svelte` (replaced by tree root focus view).

### 9.8 Implementation tooling (mandatory for the UI refactor)

Any work on `.svelte` / `.svelte.ts` files in this refactor **must** use the following skills:

- **`svelte:svelte-file-editor`** / **`svelte:svelte-core-bestpractices`** — and the Svelte MCP server tools (`mcp__plugin_svelte_svelte__*`) for documentation lookup, autofix, and validation. Every Svelte component touched in this refactor is validated via the MCP autofixer before being considered done.
- **`shadcn`** — for adding, searching, and composing shadcn/ui components. The new drawer, focus view tabs, rename modal, and command palette should use shadcn primitives where they exist (sheet/drawer, tabs, dialog, command, context-menu, tree).
- **Impeccable skills, as appropriate for the step:**
  - `impeccable:shape` — at the start of the UI refactor, to produce a design brief grounded in this spec before any components are written.
  - `impeccable:layout` — for the drawer + focus shell, spacing, hierarchy, rhythm.
  - `impeccable:typeset` — for the breadcrumb, path segments, branch pills, and metadata rows.
  - `impeccable:clarify` — for the rename modal warning copy, error messages, empty states, and all microcopy agents will read.
  - `impeccable:harden` — for empty states, missing-daemon state, loading states, error toasts, long-path overflow, offline/closed session states.
  - `impeccable:polish` — as the last pass before declaring the UI refactor done.
  - `impeccable:critique` and/or `impeccable:audit` — for a final quality check covering accessibility, responsive behavior, and anti-patterns.

The plan phase should schedule the Impeccable skills explicitly as plan steps, not as optional polish.

## 10. Agent guidance: `--help` and `guide`

### 10.1 `ray-exomem --help` top-of-output blurb

```
Ray-exomem persists memory as a tree of folders and exoms.

  Tree:        work/team/project/repo/main              (the project's main exom)
               work/team/project/repo/sessions/<id>     (per-session exoms)
  CLI paths:   work::team::project::repo::main          (`::` == `/`)
  Branches:    per-exom; write only to your own (TOFU + orchestrator-allocated)
  Writes:      always require --actor <name>
  Full agent workflow:   ray-exomem guide
```

### 10.2 `ray-exomem guide` doctrine

Printed to stdout as markdown; also served at `GET /api/guide` and rendered at `/guide` in the UI. Structure:

1. **Model** — folders vs exoms, fixed schema, nesting rules.
2. **Addressing** — `::` vs `/`, `--exom`, `--branch`, `--actor`.
3. **Starting work** — `init`, `exom new`.
4. **Sessions** — id format, multi vs single, lifecycle (close, archive), label rename.
5. **Branching rules** — orchestrator allocates, TOFU, read-any-write-own.
6. **Multi-line commands** — always render with `\`; use `@file.ray` or stdin for rayfall bodies.
7. **Reading peer branches** — examples with `--branch`.
8. **Common errors** — error taxonomy with suggested fixes.
9. **Cheat sheet** — copyable canonical examples for every major verb.

### 10.3 Error taxonomy

All server rejections return `{code, message, path?, actor?, branch?, suggestion?}`. The `suggestion` field gives the agent the exact fix command.

| Code | Message | Suggestion |
|---|---|---|
| `no_such_exom` | `no such exom <path>` | `ray-exomem init <path>` or `ray-exomem exom new <path>` |
| `branch_owned` | `branch owned by <other>` | Write with a different `--branch` you own, or ask orchestrator to allocate one |
| `session_closed` | `session closed` | Retract `session/closed_at` to reopen |
| `branch_not_in_exom` | `branch <name> not in exom` | Ask orchestrator to run `ray-exomem session add-agent <path> --agent <name>` |
| `actor_required` | `actor required` | Pass `--actor <name>` |
| `reserved_segment` | `segment "sessions" is reserved` | Pick a different segment name |
| `folder_path_on_exom_command` | `<path> is a folder, not an exom` | Point `--exom` at a leaf exom |
| `session_id_immutable` | `cannot rename session id` | Use `session/label` to change the display label |
| `cannot_nest_inside_exom` | `<segment> is inside exom <parent-path>` | Pick a path that doesn't traverse an existing exom, or `retract` / delete the existing exom first |

## 11. Error handling policy

- Fail fast and explicit. Never silently create a path when the user asked to operate on one.
- Every rejection is structured (see §10.3). `suggestion` is optional but encouraged.
- Reserved system attributes (`session/closed_at`, `branch/claimed_by`, etc.) are retractable. Nothing is permanently locked — archiving and closing are both reversible by retracting the attribute.
- Scaffolding (`init`, `exom new`, `session new`, `branch-create`) is always an explicit step. Writes to non-existent paths are rejected, never auto-scaffold.

## 12. Testing

- **Unit tests** (`cargo test`): path parsing (`::` ↔ `/`), folder-vs-exom detection, `init` idempotency, TOFU enforcement, session metadata mirroring, rename on folder/exom/session-label, reserved segment rejection.
- **Integration tests against the running daemon**: every `/api/actions/*` endpoint, `/api/tree` with depth + activity, SSE `tree-changed` events, rayfall evaluation with the new per-path rule injection.
- **Svelte UI** (`cd ui && npm run check && npm run build`): unchanged pipeline. Adds tests for the drawer tree, the three session view modes, and the rename modal.
- **End-to-end flow** (manual + scripted): `init work::team::project::repo` → `session new` multi-agent → two agents write in parallel to their branches → read peer branch → `session close` → `inspect` → rename a mid-tree folder and verify the SSE event.
- **UI golden path + edge cases**: tree drawer expansion, focus view transitions (folder ↔ project-main ↔ session ↔ archived), rename confirmation modal with a running session in the list, guide rendering.

## 13. Files likely to change

Ground-truth file map from the initial exploration. The plan phase will refine this.

- `src/main.rs` — CLI surface: new verbs (`init`, `exom`, `session`, `inspect`, `guide`), flag changes (path-based `--exom`, `--actor` on writes), rayfall `@file`/stdin support.
- `src/brain.rs` — scaffolding ops, TOFU enforcement, branch allocation, session metadata mirroring.
- `src/storage.rs` — path-based directory resolution, folder-vs-exom detection, rename op, `tree/` rename from `exoms/`.
- `src/exom.rs` — `ExomDir` refactor for nested paths.
- `src/web.rs` — new endpoints (`/api/tree`, `/api/actions/init`, `/api/actions/exom-new`, `/api/actions/session-*`, `/api/actions/branch-create`, `/api/actions/rename`, `/api/guide`), changed endpoints (path-based `exom` params), SSE `tree-changed` event, deprecation of `/api/exoms`.
- `src/system_schema.rs` — reserved system attributes (`session/label`, `session/closed_at`, `session/archived_at`, `branch/claimed_by`), drop user predicate discovery if no longer needed.
- `src/rayfall_ast.rs` — per-exom-path rule keying instead of per-name.
- `ui/src/lib/exomem.svelte.ts` — new tree client, path-based calls, rename modal helper, recent activity.
- `ui/src/routes/+page.svelte` — replaced by tree root focus view.
- `ui/src/routes/tree/[...path]/+page.svelte` — new focus view.
- `ui/src/routes/guide/+page.svelte` — new guide renderer.
- `ui/src/routes/exoms/+page.svelte` — deleted.
- `ui/src/routes/branches/[id]/+page.svelte` — deleted (folded into exom tab).
- `ui/src/routes/dependencies/+page.svelte` — deleted (merged into Graph).

## 14. Open questions / deferred

- CLI parity for rename (UI-only in v1).
- Cross-exom queries.
- Branch merging.
- Remote daemons.
- A separate `ray-exomem doctor` path-walking check that validates the tree for orphaned splay tables, missing `exom.json`, or reserved segment collisions (probably v1.1).
