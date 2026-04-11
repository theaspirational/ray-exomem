# Ray-exomem agent guide

Ray-exomem persists memory as a tree of folders and exoms.

```
Tree:        work/ath/lynx/orsl/main              (the project's main exom)
             work/ath/lynx/orsl/sessions/<id>     (per-session exoms)
CLI paths:   work::ath::lynx::orsl::main          (`::` == `/`)
Branches:    per-exom; write only to your own (TOFU + orchestrator-allocated)
Writes:      always require --actor <name>
```

---

## 1. Model — folders vs exoms

There are exactly two node kinds in the tree:

- **Folder** — pure grouping. Holds child folders and child exoms. Has no facts, no branches, and no metadata beyond its name.
- **Exom** — a leaf that holds facts and branches. Cannot contain other nodes.

A directory is an exom if and only if it contains an `exom.json` file. Otherwise it is a folder.

**Nesting rule:** Exoms are leaves. You cannot place any node inside an existing exom. Any scaffold operation (`init`, `exom new`, `session new`) that would place a segment under an existing exom is rejected with `cannot_nest_inside_exom`.

**Fixed schema:** Ray-exomem ships one schema. You do not define your own.

- `init <path>` — creates any missing folder segments along `<path>`, then creates `main` (an exom) and `sessions/` (a folder) inside the leaf. Idempotent. Projects nest freely.
- `exom new <path>` — creates a bare exom at the given path. Escape hatch for memory spaces that do not need the `main`/`sessions` layout.

---

## 2. Addressing — `::` vs `/`, `--exom`, `--branch`, `--actor`

- **CLI separator:** `::` between segments. Example: `work::ath::lynx::orsl::main`.
- **Disk / HTTP / UI separator:** `/`. Example: `work/ath/lynx/orsl/main`.
- **Unified address flag:** `--exom <path>`. No `--project`, no `--session` on the addressing side. Paths must end at an exom, except on `inspect` and `init` which accept folder paths.
- **Branch:** `--branch <name>`. Optional, defaults to `main`.
- **Actor:** `--actor <name>`. Required on every write.

**Segment syntax:** `[_A-Za-z0-9-][_A-Za-z0-9.-]*` — alphanumerics, underscore, hyphen, dot. No slashes, no `::`, no whitespace inside a segment.

**Reserved segment:** `sessions`. Can only exist as a folder created by `init`. `exom new <path>::sessions` is rejected.

---

## 3. Starting work — `init`, `exom new`

```
# Scaffold a project at work/ath/lynx/orsl
ray-exomem init work::ath::lynx::orsl

# The above creates:
#   ~/.ray-exomem/tree/work/ath/lynx/orsl/main/   (exom)
#   ~/.ray-exomem/tree/work/ath/lynx/orsl/sessions/   (folder)

# Create a bare exom (no scaffolding)
ray-exomem exom new work::scratch

# Verify
ray-exomem inspect work::ath::lynx::orsl
```

Projects nest freely: `init work::ath` and `init work::ath::lynx::orsl` coexist. Each is a full project with its own `main` exom and `sessions/` folder.

---

## 4. Sessions — id format, multi vs single, lifecycle

### Session id format

Directory name: `<YYYYMMDDTHHMMSSZ>_<multi|single>_agent_<label>`

Example: `20260411T143215Z_multi_agent_landing-page`

The session id (directory name) is **immutable**. To change the display label, mutate `session/label` via assert-fact.

### Creating a session

```
ray-exomem session new work::ath::lynx::orsl \
    --multi --name landing-page \
    --actor orchestrator \
    --agents agent_a,agent_b
```

This creates:
- `sessions/20260411T143215Z_multi_agent_landing-page/` (session exom)
- Branches `main` (orchestrator), `agent_a`, `agent_b` pre-allocated.

For a single-agent session (no parallel branches needed):

```
ray-exomem session new work::scratch \
    --single --name exploration \
    --actor solo
```

### Session lifecycle

```
# Rename the display label (does NOT rename the directory)
ray-exomem assert session/label "new label" \
    --exom work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page \
    --actor orchestrator

# Close the session (all writes rejected after this)
ray-exomem session close work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page

# Archive the session (hidden from default inspect)
ray-exomem session archive work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_landing-page
```

Both close and archive are reversible by retracting `session/closed_at` or `session/archived_at`.

---

## 5. Branching rules — orchestrator allocates, TOFU, read-any-write-own

1. **Orchestrator allocates branches.** `session new --agents a,b,c` pre-creates branches `a`, `b`, `c`. The orchestrator always claims `main` (TOFU on session create).
2. **Non-orchestrator agents cannot create new branches.** Only the orchestrator can add agents mid-session via `session add-agent`.
3. **TOFU ownership.** First write from `<actor>` to an unclaimed branch claims it. Subsequent writes from a different actor are rejected with `branch_owned`.
4. **Read any branch, write only your own.** Reads never require an actor.
5. **Session close.** `session/closed_at` non-null ⇒ all writes rejected. Reads continue to work.
6. **Single-agent sessions.** One `main` branch by default. The agent can create extra branches for exploration via `branch-create`.

```
# Write to your branch
ray-exomem assert task/status done \
    --exom work::ath::sessions::20260411T143215Z_multi_agent_landing-page \
    --branch agent_a \
    --actor agent_a

# Read a peer's branch
ray-exomem query \
    --exom work::ath::sessions::20260411T143215Z_multi_agent_landing-page \
    --branch agent_b \
    '(query (find ?f ?p ?v) (where (?f fact/predicate ?p) (?f fact/value ?v)))'
```

---

## 6. Multi-line commands — always render with `\`; use `@file.ray` or stdin

Always write CLI commands with `\` continuations for readability. For non-trivial rayfall bodies, use `@file.ray` or stdin (`-`).

```
# Inline rayfall
ray-exomem eval \
    --exom work::main \
    --actor orchestrator \
    '(assert-fact task/status "ready")'

# From a file
ray-exomem eval \
    --exom work::main \
    --actor orchestrator \
    @queries/seed.ray

# From stdin
cat queries/seed.ray | ray-exomem eval \
    --exom work::main \
    --actor orchestrator \
    -
```

---

## 7. Reading peer branches — examples with `--branch`

```
# List all branches in an exom
ray-exomem branch list --exom work::main

# Read facts on a peer branch
ray-exomem query \
    --exom work::sessions::20260411T143215Z_multi_agent_landing-page \
    --branch agent_b \
    '(query (find ?f ?p ?v) (where (?f fact/predicate ?p) (?f fact/value ?v)))'

# Read history of a specific fact across all branches
ray-exomem history fact/abc123 \
    --exom work::main
```

---

## 8. Common errors — error taxonomy with suggested fixes

| Code | Cause | Fix |
|---|---|---|
| `no_such_exom` | Path does not point to an exom | `ray-exomem init <path>` or `ray-exomem exom new <path>` |
| `branch_owned` | Branch already claimed by another actor | Write to a branch you own, or ask orchestrator to allocate one |
| `session_closed` | `session/closed_at` is set | Retract `session/closed_at` to reopen |
| `branch_not_in_exom` | Branch was never allocated | Ask orchestrator: `ray-exomem session add-agent <path> --agent <name>` |
| `actor_required` | `--actor` missing on a write | Pass `--actor <name>` |
| `reserved_segment` | Segment `sessions` used as exom name | Pick a different segment name |
| `folder_path_on_exom_command` | Path points to a folder, not an exom | Point `--exom` at a leaf exom path |
| `session_id_immutable` | Tried to rename a session directory | Use `session/label` to change the display label |
| `cannot_nest_inside_exom` | Path traverses an existing exom | Pick a path that does not cross an existing exom |

---

## 9. Cheat sheet — canonical examples for every major verb

```
# --- Scaffolding ---
ray-exomem init work::ath::lynx::orsl
ray-exomem exom new work::scratch

# --- Sessions ---
ray-exomem session new work::ath::lynx::orsl \
    --multi --name sprint-42 \
    --actor orchestrator \
    --agents agent_a,agent_b

ray-exomem session join \
    work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_sprint-42 \
    --actor agent_a

ray-exomem session close \
    work::ath::lynx::orsl::sessions::20260411T143215Z_multi_agent_sprint-42

# --- Reads ---
ray-exomem inspect work --depth 3
ray-exomem query --exom work::main \
    '(query (find ?f ?p ?v) (where (?f fact/predicate ?p) (?f fact/value ?v)))'
ray-exomem history fact/abc123 --exom work::main
ray-exomem why fact/abc123 --exom work::main

# --- Writes ---
ray-exomem assert task/status ready \
    --exom work::main \
    --actor orchestrator
ray-exomem retract fact/abc123 \
    --exom work::main \
    --actor orchestrator

# --- Branches ---
ray-exomem branch list --exom work::main
ray-exomem branch switch main --exom work::main --actor orchestrator

# --- Daemon ---
ray-exomem daemon
ray-exomem stop
ray-exomem doctor --exom work::main

# --- Guide ---
ray-exomem guide
```
