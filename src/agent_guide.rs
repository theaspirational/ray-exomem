//! Long-form reference for agents and operators (`ray-exomem guide`).
//! Keep in sync with `main.rs` CLI behavior and `web.rs` HTTP routes.

use clap::ValueEnum;

/// Selectable sections for `ray-exomem guide --topic`.
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq, ValueEnum)]
pub enum GuideTopic {
    /// Full guide (all sections).
    #[default]
    All,
    /// What ray-exomem is and how it relates to rayforce2.
    Overview,
    /// Daemon vs foreground, data directory, PID file.
    Workflow,
    /// Every CLI subcommand with usage notes.
    Cli,
    /// REST paths (nested under the compiled BASE_PATH on the bind address).
    Http,
    /// Environment variables and defaults.
    Env,
    /// Known gaps and flags that are not yet wired through.
    Limitations,
}

const OVERVIEW: &str = r#"OVERVIEW
--------
ray-exomem is a thin front-end around native rayforce2: Rayfall parsing, evaluation,
Datalog, and storage live in rayforce2. This binary adds:
  • CLI for scripting and agents (many commands talk to the daemon over HTTP).
  • Web UI + JSON/HTTP API when `ray-exomem daemon` is running (recommended entry point).
  • Multi-exom layout on disk under the data directory (default ~/.ray-exomem).

All user-facing program text uses Rayfall list-style syntax. There is no Teide parser
or Teide-to-Rayfall translation in this crate."#;

const WORKFLOW: &str = r#"WORKFLOW (recommended)
----------------------
1. Start everything (persistence + HTTP API + web UI) in the background — this is the normal flow:
     ray-exomem daemon
   Open http://127.0.0.1:9780{base}/ in a browser. Default bind: 127.0.0.1:9780.
   Stops any previous daemon using the same data dir. PID: <data-dir>/ray-exomem.pid
   (default data dir: ~/.ray-exomem).

2. Use CLI subcommands that take --addr (default 127.0.0.1:9780) to talk to the same daemon:
     ray-exomem status
     ray-exomem eval '(+ 1 2)'
     ray-exomem assert sky-color blue --as-str --exom <your-exom-path>

3. Stop the daemon:
     ray-exomem stop

Optional foreground server (blocks the terminal; same UI + API as daemon — for debugging):
     ray-exomem serve
   Use --no-persist for in-memory-only (no ~/.ray-exomem tables). Prefer `daemon` for normal use.

Offline (no daemon): evaluate a .ray file directly in-process (no shared KB):
     ray-exomem run path/to/file.ray
   Alias: ray-exomem load …"#;

const CLI: &str = r#"CLI REFERENCE
-------------
Global: ray-exomem <command> [options]. Use `ray-exomem <command> --help` for flags.
Global flag: --json (machine-readable stdout; several commands also default to JSON when stdout is not a TTY).

run | load <file>     Run a .ray file via local rayforce2 (no daemon, no shared state).
eval <source>         POST Rayfall text to /api/actions/eval on the daemon. Prints JSON
                      field "output" on success. Requires daemon.
                      Example: ray-exomem eval '(+ 1 2)'

daemon                Start UI + API in the background (recommended). --bind, --data-dir,
                      --ui-dir. Replaces any existing daemon for the same data dir; writes PID file.

serve                 Same as daemon but runs in the foreground (blocks). --no-persist supported.
                      Prefer `daemon` unless you are debugging.

stop                  Read PID from default data dir and stop that process.

status                GET /api/status?exom=… (exom is required; the daemon does not auto-create a default).

assert <pred> <val>   Uses POST /api/actions/assert-fact whenever metadata flags such as
                      --source, --confidence, --valid-from, or --valid-to are provided.
                      Zero-metadata assertions still emit Rayfall `(assert-fact …)` through eval.

retract <fact-id>     Resolves the current tuple via GET /api/facts/<id>, then emits
                      Rayfall `(retract-fact …)` through /api/actions/eval.

query [--request RAY] POST /api/query with a single Rayfall query read form.
                      Accepts either `(query <exom> ...)` or `(in-exom <exom> (query ...))`.
                      Default: (query <exom> (find ?fact ?pred ?value)
                                        (where (?fact 'fact/predicate ?pred)
                                               (?fact 'fact/value ?value)))

expand-query          POST /api/expand-query and print the original, normalized,
                      and fully expanded query after `in-exom` lowering and rule injection.

history <fact-id>     GET /api/facts/<id> — touch history + fact snapshot.

why <fact-id>         GET /api/explain?predicate=<id> (provenance / explanation).

why-not --predicate P [--value V]  Scans exported active facts and reports whether a match exists.

watch                 Stream GET {base}/events (SSE: `event: memory` JSON for mutations,
                      plus heartbeats; optional filters exom/branch/actor/predicate, `since=<id>`).

lint-memory           Hygiene: duplicate (predicate,value) / distinct fact_ids, oversized values,
                      missing provenance, bad predicates, empty fact_id, time-related heuristics.

doctor               Operational checks (see --help). Compares CLI and daemon build identities,
                      then verifies status and query decode for the selected exom.

coord claim|release|depend|agent-session
                      Coordination helpers over claim/*, task/*, and agent/* fact ids using
                      coordination namespace predicates. --exom, --addr, --branch, and --json
                      are accepted before or after the subcommand.
coord list-claims [--owner ...] [--status ...]
                      Read-side summary of current claims without writing Datalog.
coord show-claim <id>
                      Current owner/status/expiry plus field histories for one claim.
coord list-dependencies [--task-id ...]
                      Read-side summary of current task dependency facts.
coord show-task <task-id>
                      Current task dependency set plus fact histories for that task.
coord list-agent-sessions [--agent-id ...]
                      Read-side summary of current agent/session bindings.
coord show-agent <agent-id>
                      Current session binding plus fact history for that agent.

facts                 Pretty-print schema/samples via GET /api/schema?include_samples=…

observe <text>        Emits a simple `(assert-fact …)` observation marker through eval.

import <file|-)       Lossless backup; POST to /api/actions/import-json (requires X-Actor).

export                GET /api/actions/export?exom=… — Rayfall text of facts.

log                   Recent events (GET /api/logs?exom=…).

Branching:
branch <subcommand>   List, create from an explicit parent, diff, merge into an explicit target,
                      or archive branches. --exom, --addr,
                      and --json are accepted before or after the subcommand.
                      eval/assert/retract/observe/import accept --branch for per-operation views.

version               Binary + rayforce2 version string plus build identity.

brain-demo            Print the built-in Brain layer demo (time-travel sample); separate
                      from daemon Brain + rayforce2 integration details.

guide [--topic …]     Print this reference (section or full)."#;

const HTTP: &str = r##"HTTP API (daemon)
-----------------
All requests are nested under the daemon's BASE_PATH (default `/`, set via
$RAY_EXOMEM_BASE_PATH at build time, e.g. `/ray-exomem`). The current build
mounts at: `{base}/`. So `GET /api/status` below means
`http://<bind>{base}/api/status`. The Rust CLI client prepends the prefix.

Common query param: exom=<slash/path>. The daemon does not auto-create a
default exom — every per-exom request must spell out which exom it targets.

GET  /api/status         (stats.sym_entries, rules, server.build.identity)
GET  /api/schema?branch=main&include_samples=true&sample_limit=…
GET  /api/graph
GET  /api/clusters
GET  /api/logs
GET  /api/tree
GET  /api/branches
POST /api/branches                 (JSON: branch_id, name, parent_branch_id)
GET  /api/branches/<id>/diff?base=…
POST /api/branches/<id>/merge     (JSON: policy, target_branch)
DELETE /api/branches/<id>          (archive)
GET  /api/provenance
GET  /api/relation-graph
GET  /api/explain?…
GET  /api/actions/export
GET  /api/actions/export-json
POST /api/actions/import-json
GET  /api/beliefs/<id>/support   (resolve supported_by to fact/observation snapshots)
POST /api/actions/eval           (plain text: Rayfall; advanced / engine path)
POST /api/actions/assert-fact    (structured bitemporal fact assert)
POST /api/actions/init
POST /api/actions/exom-new
POST /api/actions/session-new
POST /api/actions/evaluate
GET  /events?exom=&branch=&actor=&predicate=&since= (SSE; `event: memory` + JSON body with
     id/op/exom/branch/actor; predicate set on fact upserts/eval asserts; heartbeats)

UI static assets: GET {base}/… (SvelteKit base path matches BASE_PATH).

For quick checks:
  curl -s "http://127.0.0.1:9780{base}/api/status?exom=<your-exom-path>"

Queryable system predicates in every exom's datom store:
  fact/predicate, fact/value,
  fact/provenance, fact/valid_from, fact/valid_to, fact/created_by,
  fact/superseded_by, fact/revoked_by,
  tx/id, tx/time, tx/user_email, tx/agent, tx/model, tx/action, tx/branch, tx/parent, tx/session, tx/ref,
  claim/owner, claim/status, claim/expires_at, task/depends_on, agent/session

Built-in derived views:
  fact-row, fact-meta, fact-with-tx, tx-row, observation-row, belief-row,
  branch-row, merge-row, claim-owner-row, claim-status-row, task-dependency-row,
  agent-session-row
"##;

const ENV: &str = r#"ENVIRONMENT
-----------
RAYFORCE2_DIR       If set, build.rs uses this path to find the rayforce2 native library
                    and headers. Otherwise ../rayforce2 relative to this crate.

UI (browser) may use PUBLIC_TEIDE_EXOMEM_BASE_URL to point API calls at a different
origin; when unset and the UI is served from the daemon, it uses the page origin
plus the compiled BASE_PATH."#;

const LIMITATIONS: &str = r#"LIMITATIONS (read before production use)
----------------------------------------
 • Some higher-level memory concepts still live in Rust-side structures; the intended public
   interface remains Rayfall queries and mutations plus the existing structured bitemporal helpers.
 • observe is currently a thin CLI convenience that emits a simple asserted fact.
 • import/eval: very large or complex Rayfall should be validated against rayforce2
   capabilities; there is no separate Teide layer.
 • Raw eval "output" is still the engine formatter; use query --json when you need decoded rows.
 • Metadata now lives in the same datom space as base facts via system predicates. That keeps the
   model Datalog-native, but it does mean `(query <exom> (find ?e ?a ?v) (where (?e ?a ?v)))`
   will include both user facts and system metadata rows.

For version and backend:  ray-exomem version"#;

fn full_guide_string() -> String {
    format!(
        "ray-exomem — agent and operator guide\n\
         ======================================\n\n\
         {OVERVIEW}\n\n\
         {WORKFLOW}\n\n\
         {CLI}\n\n\
         {HTTP}\n\n\
         {ENV}\n\n\
         {LIMITATIONS}\n"
    )
}

/// The agent doctrine markdown — served at `GET /api/guide` and rendered at `/guide` in the UI.
static DOCTRINE: &str = include_str!("../docs/agent_guide.md");

/// Return the agent doctrine as a static markdown string.
pub fn doctrine() -> &'static str {
    DOCTRINE
}

/// Render a section or the full guide. `{base}` placeholders are replaced
/// with the daemon's BASE_PATH (set via $RAY_EXOMEM_BASE_PATH at build time;
/// empty when mounted at root).
pub fn render(topic: GuideTopic) -> String {
    let raw = match topic {
        GuideTopic::All => full_guide_string(),
        GuideTopic::Overview => OVERVIEW.to_string(),
        GuideTopic::Workflow => WORKFLOW.to_string(),
        GuideTopic::Cli => CLI.to_string(),
        GuideTopic::Http => HTTP.to_string(),
        GuideTopic::Env => ENV.to_string(),
        GuideTopic::Limitations => LIMITATIONS.to_string(),
    };
    raw.replace("{base}", crate::server::BASE_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_guide_has_anchors() {
        let g = render(GuideTopic::All);
        for needle in [
            "ray-exomem daemon",
            "/api/status",
            "RAYFORCE2_DIR",
            "LIMITATIONS",
            "eval ",
            "POST /api/actions/eval",
        ] {
            assert!(
                g.contains(needle),
                "guide missing expected substring: {needle:?}"
            );
        }
    }

    #[test]
    fn each_topic_non_empty() {
        use GuideTopic::*;
        for t in [All, Overview, Workflow, Cli, Http, Env, Limitations] {
            assert!(!render(t).is_empty(), "empty topic: {t:?}");
        }
    }

    #[test]
    fn full_contains_sections() {
        let full = render(GuideTopic::All);
        assert!(full.len() > render(GuideTopic::Overview).len());
        assert!(full.contains(OVERVIEW));
        // HTTP has `{base}` placeholders; render(Http) applies the same substitution.
        assert!(full.contains(&render(GuideTopic::Http)));
    }
}
