# Ray-exomem agent guide

Ray-exomem persists memory as a tree of folders and exoms.

```
Tree:        work/ath/lynx/orsl/main              (project main exom)
             work/ath/lynx/orsl/sessions/<id>     (per-session exoms)
CLI paths:   work::ath::lynx::orsl::main          (`::` == `/`)
UI / API:    http://127.0.0.1:9780/ray-exomem/
Writes:      pass --actor <name> on mutating commands
```

---

## 1. Model

There are exactly two node kinds in the tree:

- **Folder** — grouping only. Holds child folders and child exoms. No facts or branches.
- **Exom** — a leaf knowledge base. Holds facts, rules, branches, observations, beliefs, and transactions.

A directory is an exom if and only if it contains `exom.json`.

Rules worth remembering:

- Exoms are leaves. You cannot nest anything inside an existing exom.
- `init <path>` scaffolds a project by creating `<path>/main` plus `<path>/sessions/`.
- `exom-new <path>` creates a bare exom with no `main`/`sessions` scaffold.
- A fresh persistent store auto-creates a bare `main` exom at `~/.ray-exomem/tree/main`.

---

## 2. Paths And Command Modes

### Paths

- CLI paths use `::`: `work::ath::lynx::orsl::main`
- Disk, UI, and HTTP paths use `/`: `work/ath/lynx/orsl/main`
- `--exom <path>` must point at a leaf exom path
- `--branch <name>` is available on branch/coord commands and on some writes such as `assert`, `retract`, `observe`, and `eval`
- `query` does **not** currently accept `--branch`

### Command modes

There are two important execution modes:

- **Offline / local tree commands**: `inspect`, `init`, `exom-new`
  These read or write the local data dir directly.
- **Daemon-backed commands**: `status`, `query`, `eval`, `assert`, `retract`, `branch`, `session`, `watch`, and most API-backed workflows
  These require `ray-exomem daemon` or `ray-exomem serve`.

CLI transport is mixed right now:

- Newer tree/session commands route through the global `--daemon-url`
- Older commands still use `--addr 127.0.0.1:9780`

---

## 3. Starting Work

```bash
# See the local tree
ray-exomem inspect

# Scaffold a project
ray-exomem init work::ath::lynx::orsl

# That creates:
#   ~/.ray-exomem/tree/work/ath/lynx/orsl/main/
#   ~/.ray-exomem/tree/work/ath/lynx/orsl/sessions/

# Create a bare exom instead of a full project
ray-exomem exom-new work::scratch

# Inspect a subtree
ray-exomem inspect work::ath::lynx::orsl --depth 3
```

Projects nest freely. `init work::ath` and `init work::ath::lynx::orsl` can coexist.

---

## 4. Sessions

### Session id format

Session exoms live under:

```text
<project>/sessions/<YYYYMMDDTHHMMSSZ>_<multi|single>_agent_<label>
```

Example:

```text
work/ath/lynx/orsl/sessions/20260411T143215Z_multi_agent_landing-page
```

The session id directory is immutable. Use the display label, not the directory name, when you want a human-facing rename.

### Create a session

```bash
ray-exomem session new work::ath::lynx::orsl \
  --multi \
  --name landing-page \
  --actor orchestrator \
  --agents agent_a,agent_b
```

This creates a session exom under `.../sessions/...` and pre-creates:

- branch `main` for the orchestrator
- branch `agent_a`
- branch `agent_b`

Single-agent session:

```bash
ray-exomem session new work::ath::lynx::orsl \
  --single \
  --name exploration \
  --actor solo
```

### Session lifecycle

```bash
# Change the display label
ray-exomem session rename \
  work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page \
  --label new-label \
  --actor orchestrator

# Close the session (future writes rejected)
ray-exomem session close \
  work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page \
  --actor orchestrator

# Archive the session (hidden from default inspect/tree views)
ray-exomem session archive \
  work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page \
  --actor orchestrator
```

There is no dedicated reopen/unarchive command today. Reversal means retracting
`session/closed_at` or `session/archived_at` through the normal mutation path.

### Join a pre-allocated session branch

```bash
ray-exomem session join \
  work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page \
  --actor agent_a
```

That claims the branch named `agent_a`. The orchestrator already owns `main`
after `session new` and should not call `session join`.

### Practical caveat

Prefer listing all participant branches up front with `session new --agents ...`.
The mid-session `session add-agent` flow is still settling, so avoid depending
on it in automation until it is cleaned up.

---

## 5. Branching And Ownership

Branch rules:

1. Session creation pre-allocates branches.
2. The orchestrator gets `main`.
3. Non-orchestrator participants get branches named after their actor ids.
4. First writer claims an unclaimed branch (TOFU).
5. Later writes from a different actor are rejected with `branch_owned`.
6. Closed sessions reject all writes.

Useful commands:

```bash
# List branches on an exom
ray-exomem branch list --exom work::ath::lynx::orsl::main

# Diff a branch against main
ray-exomem branch diff agent_a \
  --exom work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page \
  --base main
```

Important limitation:

- `ray-exomem query` does not currently support `--branch`
- If you need branch-specific reads from the CLI, you must either:
  switch the current branch explicitly first, or
  use the HTTP API directly for branch-scoped facts

The API supports branch-scoped fact reads via:

```text
GET /ray-exomem/api/facts?exom=<path>&branch=<branch>
```

---

## 6. Reading And Writing

### Simple reads

```bash
# Default logical fact listing
ray-exomem query \
  --exom work::ath::lynx::orsl::main \
  --json

# Explicit Rayfall query using the derived fact-row view
ray-exomem query \
  --exom work::ath::lynx::orsl::main \
  --request '(query work/ath/lynx/orsl/main (find ?fact ?pred ?value) (where (fact-row ?fact ?pred ?value)))' \
  --json

# Fact history and explanation
ray-exomem history project/status --exom work::ath::lynx::orsl::main --json
ray-exomem why project/status --exom work::ath::lynx::orsl::main --json
```

### Writes

```bash
# Assert a fact
ray-exomem assert project/status active \
  --exom work::ath::lynx::orsl::main \
  --actor orchestrator \
  --source kickoff-notes

# Assert on a branch
ray-exomem assert task/status done \
  --exom work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page \
  --branch agent_a \
  --actor agent_a

# Retract by stable fact id
ray-exomem retract project/status \
  --exom work::ath::lynx::orsl::main \
  --actor orchestrator
```

### Eval and file input

`eval` is not the same as `query` here:

- `eval` uses a literal source argument or `--file <path>`
- `query` and `expand-query` accept `@file` or `-`

Examples:

```bash
# Eval from a file
ray-exomem eval \
  --exom work::ath::lynx::orsl::main \
  --actor orchestrator \
  --file queries/seed.ray

# Eval from stdin
cat queries/seed.ray | ray-exomem eval \
  --exom work::ath::lynx::orsl::main \
  --actor orchestrator \
  --file -

# Query from a file
ray-exomem query \
  --exom work::ath::lynx::orsl::main \
  --request @queries/list-facts.ray \
  --json
```

---

## 7. Common Errors

| Code | Cause | Fix |
|---|---|---|
| `bad_path` | Invalid path syntax or reserved segment usage | Fix the path; for exoms use `ray-exomem exom-new <path>` |
| `cannot_nest_inside_exom` | Path crosses an existing exom | Choose a path that does not traverse an exom leaf |
| `already_exists_different` | A folder/exom already exists at that location with the wrong kind | Pick a new path or remove the conflicting node |
| `no_such_exom` | `--exom` does not point to an existing exom | `ray-exomem init <project>` or `ray-exomem exom-new <path>` |
| `actor_required` | Missing actor on a write or session join | Pass `--actor <name>` |
| `branch_not_in_exom` | Branch was never allocated in that exom | Pre-allocate it during `session new --agents ...` |
| `branch_owned` | Another actor already claimed the branch | Write to your own branch |
| `session_closed` | Session was closed | Retract `session/closed_at` to reopen |
| `not_orchestrator` | Non-orchestrator tried to create a session branch directly | Only the session initiator may allocate branches |
| `session_id_immutable` | Tried to rename a session directory | Use `session rename` / `session/label` instead |
| `namespace_root_immutable` | Tried to rename a user namespace root in authenticated mode | Rename a descendant instead |

---

## 8. Cheat Sheet

```bash
# --- Daemon ---
ray-exomem daemon
ray-exomem stop

# --- Tree ---
ray-exomem inspect
ray-exomem init work::ath::lynx::orsl
ray-exomem exom-new work::scratch

# --- Sessions ---
ray-exomem session new work::ath::lynx::orsl \
  --multi \
  --name sprint-42 \
  --actor orchestrator \
  --agents agent_a,agent_b

ray-exomem session join \
  work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_sprint-42 \
  --actor agent_a

ray-exomem session rename \
  work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_sprint-42 \
  --label sprint-42b \
  --actor orchestrator

# --- Reads ---
ray-exomem query --exom work::ath::lynx::orsl::main --json
ray-exomem history project/status --exom work::ath::lynx::orsl::main --json
ray-exomem why project/status --exom work::ath::lynx::orsl::main --json

# --- Writes ---
ray-exomem assert project/status active \
  --exom work::ath::lynx::orsl::main \
  --actor orchestrator

ray-exomem retract project/status \
  --exom work::ath::lynx::orsl::main \
  --actor orchestrator

# --- Branches ---
ray-exomem branch list --exom work::ath::lynx::orsl::main
ray-exomem branch diff agent_a \
  --exom work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_sprint-42 \
  --base main

# --- Reference ---
ray-exomem guide
```
