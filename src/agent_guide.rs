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
    /// REST paths (all under `/ray-exomem` on the bind address).
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
   Open http://127.0.0.1:9780/ray-exomem/ in a browser. Default bind: 127.0.0.1:9780.
   Stops any previous daemon using the same data dir. PID: <data-dir>/ray-exomem.pid
   (default data dir: ~/.ray-exomem).

2. Use CLI subcommands that take --addr (default 127.0.0.1:9780) to talk to the same daemon:
     ray-exomem status
     ray-exomem eval '(+ 1 2)'
     ray-exomem assert sky-color blue --exom main

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

run | load <file>     Run a .ray file via local rayforce2 (no daemon, no shared state).
eval <source>         POST Rayfall text to /api/actions/eval on the daemon. Prints JSON
                      field "output" on success. Requires daemon.
                      Example: ray-exomem eval '(+ 1 2)'

daemon                Start UI + API in the background (recommended). --bind, --data-dir,
                      --ui-dir. Replaces any existing daemon for the same data dir; writes PID file.

serve                 Same as daemon but runs in the foreground (blocks). --no-persist supported.
                      Prefer `daemon` unless you are debugging.

stop                  Read PID from default data dir and stop that process.

status                GET /api/status?exom=… (default exom: main).

assert <pred> <val>   Assert a fact via generated Rayfall (assert-fact). --exom, --addr.
                      Note: --confidence and --source are accepted but not yet applied
                      end-to-end; prefer import/eval for rich metadata.

retract <pred>      Retract by predicate JSON. --exom, --addr.

facts                 Pretty-print schema/samples via GET /api/schema?include_samples=…

observe <text>        Shortcut observation fact (predicate "observation"). Extra flags
                      exist but are not fully wired to Brain metadata yet.

import <file|-)       Read Rayfall source from file or stdin "-"; POST to /api/actions/eval.

export                GET /api/actions/export?exom=… — Rayfall text of facts.

exoms                 List knowledge bases (GET /api/exoms).

log                   Recent events (GET /api/logs?exom=…).

branch <subcommand>   List, create, switch, diff, merge, or archive branches. --exom, --addr.
                      eval/assert/retract/observe/import accept --branch to switch before the op.

version               Binary + rayforce2 version string.

brain-demo            Print the built-in Brain layer demo (time-travel sample); separate
                      from daemon Brain + rayforce2 integration details.

guide [--topic …]     Print this reference (section or full)."#;

const HTTP: &str = r##"HTTP API (daemon)
-----------------
All requests use the path prefix /ray-exomem on the server bind address (e.g.
http://127.0.0.1:9780/ray-exomem/api/...). The Rust CLI client prepends this prefix.

Common query param: exom=<name> (default exom name: main).

GET  /api/status
GET  /api/schema?include_samples=true&sample_limit=…
GET  /api/graph
GET  /api/clusters
GET  /api/logs
GET  /api/exoms
GET  /api/branches
POST /api/branches                 (JSON: branch_id, name)
POST /api/branches/<id>/switch
GET  /api/branches/<id>/diff?base=…
POST /api/branches/<id>/merge     (JSON: policy)
DELETE /api/branches/<id>          (archive)
GET  /api/provenance
GET  /api/relation-graph
GET  /api/explain?…
GET  /api/actions/export
GET  /api/facts/<id>
GET  /api/clusters/<id>
POST /api/actions/clear
POST /api/actions/retract        (JSON body; predicate, etc.)
POST /api/actions/eval           (plain text body: Rayfall source)
POST /api/actions/evaluate
POST /api/actions/import
POST /api/exoms
POST /api/exoms/<name>/manage
GET  /events                     (Server-Sent Events stream for live mutations)

UI static assets: GET /ray-exomem/… (SvelteKit base path /ray-exomem).

For quick checks:
  curl -s "http://127.0.0.1:9780/ray-exomem/api/status?exom=main"
"##;

const ENV: &str = r#"ENVIRONMENT
-----------
RAYFORCE2_DIR       If set, build.rs uses this path to find the rayforce2 native library
                    and headers. Otherwise ../rayforce2 relative to this crate.

UI (browser) may use PUBLIC_TEIDE_EXOMEM_BASE_URL to point API calls at a different
origin; when unset and the UI is served from the daemon, it uses the page origin
with path /ray-exomem."#;

const LIMITATIONS: &str = r#"LIMITATIONS (read before production use)
----------------------------------------
 • Brain (event-sourced memory) and the rayforce2 datoms eval path are not fully unified;
   some features exist in parallel. See plans/dx_improvements.md in the repo.
 • assert/observe: several CLI flags are parsed but not fully propagated to Brain.
 • import/eval: very large or complex Rayfall should be validated against rayforce2
   capabilities; there is no separate Teide layer.
 • No dedicated `query` CLI yet — use eval with appropriate Rayfall or the HTTP API.

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

/// Render a section or the full guide.
pub fn render(topic: GuideTopic) -> String {
    match topic {
        GuideTopic::All => full_guide_string(),
        GuideTopic::Overview => OVERVIEW.to_string(),
        GuideTopic::Workflow => WORKFLOW.to_string(),
        GuideTopic::Cli => CLI.to_string(),
        GuideTopic::Http => HTTP.to_string(),
        GuideTopic::Env => ENV.to_string(),
        GuideTopic::Limitations => LIMITATIONS.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_guide_has_anchors() {
        let g = render(GuideTopic::All);
        for needle in [
            "ray-exomem daemon",
            "/ray-exomem/api/status",
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
        for t in [
            All,
            Overview,
            Workflow,
            Cli,
            Http,
            Env,
            Limitations,
        ] {
            assert!(!render(t).is_empty(), "empty topic: {t:?}");
        }
    }

    #[test]
    fn full_contains_sections() {
        let full = render(GuideTopic::All);
        assert!(full.len() > render(GuideTopic::Overview).len());
        assert!(full.contains(OVERVIEW));
        assert!(full.contains(HTTP));
    }
}
