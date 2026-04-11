use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::{net::SocketAddr, path::PathBuf};

use ray_exomem::agent_guide::GuideTopic;

#[derive(Args, Clone, Debug, Default)]
struct CommandScopeArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    exom: Option<String>,
    #[arg(long)]
    addr: Option<String>,
}

#[derive(Args, Clone, Debug, Default)]
struct CoordScopeArgs {
    #[command(flatten)]
    common: CommandScopeArgs,
    #[arg(long)]
    branch: Option<String>,
}

#[derive(Subcommand)]
enum BranchCommands {
    /// List all branches.
    List {
        #[command(flatten)]
        scope: CommandScopeArgs,
    },
    /// Create a new branch from the current branch.
    Create {
        branch_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CommandScopeArgs,
    },
    /// Switch the active branch.
    Switch {
        branch_id: String,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CommandScopeArgs,
    },
    /// Show differences between branches.
    Diff {
        branch_id: String,
        #[arg(long, default_value = "main")]
        base: String,
        #[command(flatten)]
        scope: CommandScopeArgs,
    },
    /// Merge a branch into the current branch.
    Merge {
        source: String,
        #[arg(long, default_value = "last-writer-wins")]
        policy: String,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CommandScopeArgs,
    },
    /// Delete (archive) a branch.
    Delete {
        branch_id: String,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CommandScopeArgs,
    },
}

#[derive(Subcommand)]
enum CoordCommands {
    /// Acquire or refresh a coordination claim.
    Claim {
        claim_id: String,
        owner: String,
        #[arg(long, default_value = "active")]
        status: String,
        #[arg(long)]
        expires_at: Option<String>,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// Release a coordination claim.
    Release {
        claim_id: String,
        #[arg(long, default_value = "released")]
        status: String,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// Record that one task depends on another entity or task.
    Depend {
        task_id: String,
        depends_on: String,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// Record the active session for an agent identity.
    AgentSession {
        agent_id: String,
        session_id: String,
        #[arg(long, default_value = "cli")]
        actor: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// List current claims with optional owner/status filters.
    ListClaims {
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// Show the current coordination state and field histories for one claim.
    ShowClaim {
        claim_id: String,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// List current task dependency facts.
    ListDependencies {
        #[arg(long)]
        task_id: Option<String>,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// Show current task dependency state plus fact details for one task.
    ShowTask {
        task_id: String,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// List current agent session facts.
    ListAgentSessions {
        #[arg(long)]
        agent_id: Option<String>,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
    /// Show current agent/session state plus fact detail for one agent.
    ShowAgent {
        agent_id: String,
        #[command(flatten)]
        scope: CoordScopeArgs,
    },
}

#[derive(Parser)]
#[command(
    name = "ray-exomem",
    version = env!("CARGO_PKG_VERSION"),
    about = "Native rayforce2 exomemory front-end — Rayfall list-style syntax only",
    long_about = "ray-exomem is a thin orchestration layer over native rayforce2.\n\n\
                  Quick start (UI + JSON API):  ray-exomem daemon\n\
                  Then open http://127.0.0.1:9780/ray-exomem/  —  stop with: ray-exomem stop\n\n\
                  All input uses Rayfall list-style syntax. No Teide parser,\n\
                  Teide AST, or Teide-to-Rayfall translation layer is present.\n\n\
                  Full reference for agents:  ray-exomem guide\n\
                  Sections:  ray-exomem guide --topic <overview|workflow|cli|http|env|limitations>",
    after_long_help = "Quick links:\n  \
        ray-exomem daemon             background UI + API (recommended)\n  \
        ray-exomem guide              full CLI + HTTP + env reference\n  \
        ray-exomem guide --topic cli  subcommands only\n  \
        ray-exomem <cmd> --help       per-command flags and examples\n",
)]
struct Cli {
    /// Machine-readable JSON on stdout (also implied when stdout is not a TTY for several commands).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a native Rayfall source file (offline; no daemon).
    #[command(
        visible_alias = "load",
        after_long_help = "Example:\n  ray-exomem run examples/native_smoke.ray\n\n\
            Does not use the daemon; evaluates in-process. Use `eval` + daemon for a shared KB."
    )]
    Run {
        /// Path to a .ray source file.
        file: PathBuf,
    },

    /// Start the web UI and HTTP API in the background (normal use). Replaces any prior daemon for the same data dir.
    #[command(after_long_help = "Examples:\n  \
            ray-exomem daemon\n  \
            ray-exomem daemon --bind 0.0.0.0:9780 --data-dir ~/.ray-exomem\n\n\
            Open http://<bind>/ray-exomem/ in a browser. JSON API: /ray-exomem/api/.\n\
            Stop with: ray-exomem stop")]
    Daemon {
        /// Bind address for the server.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_BIND_ADDR)]
        bind: SocketAddr,

        /// Directory containing the SvelteKit static build.
        #[arg(long)]
        ui_dir: Option<PathBuf>,

        /// Data directory for persistent storage.
        /// Defaults to ~/.ray-exomem.
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Evaluate Rayfall source via the daemon (inline or from file).
    #[command(after_long_help = "Examples:\n  \
            ray-exomem eval '(+ 1 2)'\n  \
            ray-exomem eval --file myscript.ray\n  \
            echo '(+ 1 2)' | ray-exomem eval --file -\n  \
            ray-exomem eval \"(query db (find ?x) (where (?x :edge ?y)))\" --addr 127.0.0.1:9780\n\n\
            POSTs plain text to /ray-exomem/api/actions/eval. Requires `ray-exomem daemon`.")]
    Eval {
        /// Rayfall list-style source expression (omit when using --file).
        source: Option<String>,
        /// Path to a .ray file, or "-" for stdin.
        #[arg(long)]
        file: Option<String>,
        /// Daemon address (host:port — HTTP, no scheme).
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        /// Exom used for `--branch` switch (query source still names its database).
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Attribution: sent as X-Actor (default: anonymous).
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Switch to this branch before evaluating (optional).
        #[arg(long)]
        branch: Option<String>,
    },

    /// Foreground server (same UI + API as `daemon`; blocks the terminal). Prefer `daemon` for daily use.
    Serve {
        /// Bind address for the server.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_BIND_ADDR)]
        bind: SocketAddr,

        /// Directory containing the SvelteKit static build.
        #[arg(long)]
        ui_dir: Option<PathBuf>,

        /// Data directory for persistent storage (rayforce2 splayed tables).
        /// Defaults to ~/.ray-exomem. Pass --no-persist to run in-memory only.
        #[arg(long)]
        data_dir: Option<PathBuf>,

        /// Run without persistence (in-memory only, all data lost on exit).
        #[arg(long)]
        no_persist: bool,
    },

    /// Stop a running daemon.
    Stop,

    /// Check daemon status and exom stats.
    Status {
        /// Target exom (default: "main").
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
    },

    /// Assert a fact into an exom. Uses the direct assert endpoint when metadata flags are provided.
    #[command(after_long_help = "Examples:\n  \
            ray-exomem assert sky-color blue --exom main\n  \
            ray-exomem assert location paris --valid-from 2024-01-01T00:00:00Z --valid-to 2024-06-01T00:00:00Z\n\n\
            For rich Rayfall, use: ray-exomem eval --file script.ray")]
    Assert {
        /// Predicate name (e.g. "sky-color").
        predicate: String,
        /// Value (e.g. "blue").
        value: String,
        /// Stable fact id for future updates/retractions. Defaults to the predicate name.
        #[arg(long)]
        fact_id: Option<String>,
        /// Confidence score (0.0–1.0).
        #[arg(long)]
        confidence: Option<f64>,
        /// Provenance tag.
        #[arg(long)]
        source: Option<String>,
        /// When this fact became true in the real world (ISO 8601). Defaults to now.
        #[arg(long)]
        valid_from: Option<String>,
        /// When this fact ceased being true (ISO 8601). Omit for open-ended.
        #[arg(long)]
        valid_to: Option<String>,
        /// Target exom.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Switch to this branch before asserting (optional).
        #[arg(long)]
        branch: Option<String>,
    },

    /// Retract a fact by stable fact id (resolved to an exact Rayfall retract).
    Retract {
        /// Fact id to retract (same key used on upsert).
        fact_id: String,
        /// Target exom.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Switch to this branch before retracting (optional).
        #[arg(long)]
        branch: Option<String>,
    },

    /// List current facts in an exom.
    Facts {
        /// Target exom.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
    },

    /// Evaluate a Rayfall query against the daemon.
    Query {
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        /// Rayfall query source; default lists visible logical facts via fact/predicate and fact/value.
        #[arg(long)]
        request: Option<String>,
        #[arg(long)]
        json: bool,
    },

    /// Print the normalized and fully expanded form of a query.
    ExpandQuery {
        #[arg(long)]
        exom: Option<String>,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        request: String,
        #[arg(long)]
        json: bool,
    },

    /// Health and consistency checks (daemon, branches, query decode).
    Doctor {
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        json: bool,
    },

    /// Print JSON session contract for agents (exom, URL, branch, required headers).
    StartSession {
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        /// Used as X-Actor when creating the exom if it does not exist yet.
        #[arg(long, default_value = "cli")]
        actor: String,
    },

    /// Record an observation.
    Observe {
        /// Observation content.
        content: String,
        /// Source type (e.g. "agent", "sensor", "user").
        #[arg(long, default_value = "agent")]
        source_type: String,
        /// Source reference.
        #[arg(long, default_value = "cli")]
        source_ref: String,
        /// Confidence score.
        #[arg(long, default_value = "1.0")]
        confidence: f64,
        /// Comma-separated tags.
        #[arg(long, default_value = "")]
        tags: String,
        /// Target exom.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Switch to this branch before recording (optional).
        #[arg(long)]
        branch: Option<String>,
    },

    /// Export all data from an exom as lossless JSON (default) or human-readable Rayfall.
    Export {
        /// Target exom.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        /// Output format: "json" (default, lossless) or "rayfall" (human-readable, facts + rules only).
        #[arg(long, default_value = "json")]
        format: String,
    },

    /// Import a lossless JSON backup into an exom (replaces all data).
    Import {
        /// Path to a .json backup file, or "-" for stdin.
        file: String,
        /// Target exom.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
    },

    /// List all exoms.
    Exoms {
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
    },

    /// Show recent transaction log.
    Log {
        /// Target exom.
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        /// Daemon address.
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
    },

    /// Print version and backend information.
    Version,

    /// Run the brain/memory layer demo showing time-travel queries.
    BrainDemo,

    /// Manage knowledge branches for hypothetical reasoning and parallel agent work.
    Branch {
        #[command(subcommand)]
        command: BranchCommands,
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
    },

    /// Coordination helpers on top of coordination namespace facts.
    Coord {
        #[command(subcommand)]
        command: CoordCommands,
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        branch: Option<String>,
    },

    /// Print the agent/operator reference (CLI workflows, HTTP routes, env, limitations).
    #[command(visible_alias = "docs")]
    Guide {
        /// Print only this section (default: full guide).
        #[arg(long, value_enum, default_value_t = GuideTopic::All)]
        topic: GuideTopic,
    },

    /// Full history and touch log for a fact id (GET /api/facts/<id>).
    History {
        fact_id: String,
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        /// Always print minified JSON (default when stdout is not a TTY).
        #[arg(long)]
        json: bool,
    },

    /// Explain a fact or predicate (GET /api/explain).
    Why {
        fact_id: String,
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        json: bool,
    },

    /// Check whether an active fact matches predicate (and optional value); explains absence.
    WhyNot {
        #[arg(long)]
        predicate: String,
        #[arg(long)]
        value: Option<String>,
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        json: bool,
    },

    /// Stream SSE events from the daemon (blocks; Ctrl+C to stop).
    Watch {
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
    },

    /// Heuristic hygiene report over exported Rayfall facts.
    LintMemory {
        #[arg(long, default_value = ray_exomem::web::DEFAULT_EXOM)]
        exom: String,
        #[arg(long, default_value = "127.0.0.1:9780")]
        addr: String,
        #[arg(long)]
        json: bool,
    },
}

fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .expect("could not determine home directory")
        .join(".ray-exomem")
}

fn pid_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("ray-exomem.pid")
}

fn encode_url_part(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn json_stdout(global: bool, local: bool) -> bool {
    global || local || !std::io::stdout().is_terminal()
}

fn print_json_or_raw(body: &str, force_compact: bool) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        print_json_value(&v, force_compact);
    } else {
        println!("{}", body);
    }
}

fn print_json_value(v: &serde_json::Value, force_compact: bool) {
    let compact = force_compact || !std::io::stdout().is_terminal();
    let s = if compact {
        v.to_string()
    } else {
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    };
    println!("{}", s);
}

fn resolve_scope_value(parent: &str, local: &Option<String>) -> String {
    local.clone().unwrap_or_else(|| parent.to_string())
}

fn resolve_scope_branch(parent: &Option<String>, local: &Option<String>) -> Option<String> {
    local.clone().or_else(|| parent.clone())
}

fn is_unknown_exom_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("unknown exom")
}

fn should_use_structured_assert(
    confidence: Option<f64>,
    source: &Option<String>,
    valid_from: &Option<String>,
    valid_to: &Option<String>,
) -> bool {
    confidence.is_some() || source.is_some() || valid_from.is_some() || valid_to.is_some()
}

fn branch_scope_values(
    global_json: bool,
    parent_exom: &str,
    parent_addr: &str,
    scope: &CommandScopeArgs,
) -> (bool, String, String) {
    (
        json_stdout(global_json, scope.json),
        resolve_scope_value(parent_exom, &scope.exom),
        resolve_scope_value(parent_addr, &scope.addr),
    )
}

fn coord_scope_values(
    global_json: bool,
    parent_exom: &str,
    parent_addr: &str,
    parent_branch: &Option<String>,
    scope: &CoordScopeArgs,
) -> (bool, String, String, Option<String>) {
    (
        json_stdout(global_json, scope.common.json),
        resolve_scope_value(parent_exom, &scope.common.exom),
        resolve_scope_value(parent_addr, &scope.common.addr),
        resolve_scope_branch(parent_branch, &scope.branch),
    )
}

/// Max UTF-8 byte length for a fact `value` before `lint-memory` flags it (roadmap hygiene).
const LINT_MAX_VALUE_BYTES: usize = 65_536;

fn looks_time_sensitive_predicate(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    [
        "deadline", "expires", "due", "ttl", "schedule", "remind", "before", "after",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn run_lint_memory(c: &ray_exomem::client::Client, exom: &str, force_compact: bool) -> Result<()> {
    let body = c.get(&format!("/api/actions/export?exom={}", exom))?;
    let mut groups: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut oversized = Vec::new();
    let mut bad_fact_forms = Vec::new();
    let mut time_sensitive_open_validity = Vec::new();
    let mut rows_per_fact_id: HashMap<String, usize> = HashMap::new();

    for form in ray_exomem::rayfall_parser::split_forms(&body) {
        if !matches!(form.kind, ray_exomem::rayfall_parser::FormKind::AssertFact) {
            continue;
        }
        match ray_exomem::rayfall_parser::parse_fact_mutation_args(&form.inner_source) {
            Ok((_exom_name, fact_id, predicate, value)) => {
                groups
                    .entry((predicate.clone(), value.clone()))
                    .or_default()
                    .push(fact_id.clone());
                *rows_per_fact_id.entry(fact_id.clone()).or_default() += 1;

                if value.len() > LINT_MAX_VALUE_BYTES {
                    oversized.push(serde_json::json!({
                        "fact_id": fact_id,
                        "predicate": predicate,
                        "value_bytes": value.len(),
                    }));
                }

                if looks_time_sensitive_predicate(&predicate) && !form.source.contains(";; @valid[")
                {
                    time_sensitive_open_validity.push(serde_json::json!({
                        "fact_id": fact_id,
                        "predicate": predicate,
                        "note": "heuristic: predicate looks time-related but export line has no explicit validity annotation",
                    }));
                }
            }
            Err(e) => bad_fact_forms.push(serde_json::json!({
                "source": form.source,
                "error": e.to_string(),
            })),
        }
    }

    let mut dupes = Vec::new();
    for ((predicate, value), ids) in groups {
        let uniq: HashSet<_> = ids.iter().collect();
        if uniq.len() > 1 {
            dupes.push(serde_json::json!({
                "predicate": predicate,
                "value": value,
                "fact_ids": ids,
            }));
        }
    }

    let multiple_unrevoked_fact_ids: Vec<_> = rows_per_fact_id
        .into_iter()
        .filter_map(|(fact_id, count)| {
            if count > 1 {
                Some(serde_json::json!({
                    "fact_id": fact_id,
                    "active_export_rows": count,
                }))
            } else {
                None
            }
        })
        .collect();

    let issue_count = dupes.len()
        + oversized.len()
        + bad_fact_forms.len()
        + time_sensitive_open_validity.len()
        + multiple_unrevoked_fact_ids.len();
    let ok = issue_count == 0;

    let report = serde_json::json!({
        "ok": ok,
        "exom": exom,
        "issue_count": issue_count,
        "checks": {
            "duplicate_predicate_value_groups": dupes,
            "oversized_values": oversized,
            "bad_fact_forms": bad_fact_forms,
            "time_sensitive_open_validity": time_sensitive_open_validity,
            "history_multiple_unrevoked_rows": multiple_unrevoked_fact_ids,
        }
    });
    print_json_value(&report, force_compact);
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn ctx_headers<'a>(
    actor: &'a Option<String>,
    session: &'a Option<String>,
    model: &'a Option<String>,
) -> Vec<(&'static str, &'a str)> {
    let mut h = vec![("X-Actor", actor.as_deref().unwrap_or("anonymous"))];
    if let Some(s) = session {
        h.push(("X-Session", s.as_str()));
    }
    if let Some(m) = model {
        h.push(("X-Model", m.as_str()));
    }
    h
}

fn apply_expand_query_exom(source: &str, exom: Option<&str>) -> Result<String> {
    let Some(exom) = exom else {
        return Ok(source.to_string());
    };
    let forms = ray_exomem::rayfall_ast::parse_forms(source)?;
    let [form] = forms.as_slice() else {
        return Ok(source.to_string());
    };
    let is_bare_query = form
        .as_list()
        .and_then(|items| items.first())
        .and_then(|item| item.as_symbol())
        == Some("query");
    if !is_bare_query {
        return Ok(source.to_string());
    }
    let lowered = ray_exomem::rayfall_ast::lower_top_level(
        form,
        ray_exomem::rayfall_ast::LoweringOptions {
            default_query_exom: Some(exom),
            default_rule_exom: Some(ray_exomem::web::DEFAULT_EXOM),
        },
    )?;
    match lowered.as_slice() {
        [ray_exomem::rayfall_ast::CanonicalForm::Query(query)] => Ok(query.emit()),
        _ => Ok(source.to_string()),
    }
}

fn switch_branch_cli(
    c: &ray_exomem::client::Client,
    branch: &Option<String>,
    exom: &str,
    headers: &[(&str, &str)],
) -> Result<()> {
    if let Some(b) = branch {
        c.post_text_with_headers(
            &format!("/api/branches/{}/switch?exom={}", b, exom),
            "",
            headers,
        )?;
    }
    Ok(())
}

fn active_fact_tuple(
    c: &ray_exomem::client::Client,
    exom: &str,
    fact_id: &str,
) -> Result<(String, String)> {
    let body = c.get(&format!(
        "/api/facts/{}?exom={}",
        encode_url_part(fact_id),
        encode_url_part(exom)
    ))?;
    let v: serde_json::Value = serde_json::from_str(&body).context("parse fact detail JSON")?;
    if v["fact"]["status"].as_str() != Some("active") {
        anyhow::bail!("fact {:?} is not active", fact_id);
    }
    let predicate = v["fact"]["predicate"]
        .as_str()
        .or_else(|| v["fact"]["tuple"].get(1).and_then(|x| x.as_str()))
        .context("fact detail missing predicate")?
        .to_string();
    let value = v["fact"]["tuple"]
        .get(2)
        .and_then(|x| x.as_str())
        .context("fact detail missing value")?
        .to_string();
    Ok((predicate, value))
}

fn assert_fact_json(
    c: &ray_exomem::client::Client,
    exom: &str,
    headers: &[(&str, &str)],
    fact_id: &str,
    predicate: &str,
    value: &str,
    confidence: f64,
    provenance: &str,
    valid_from: Option<&str>,
    valid_to: Option<&str>,
) -> Result<String> {
    let mut payload = serde_json::json!({
        "fact_id": fact_id,
        "predicate": predicate,
        "value": value,
        "confidence": confidence,
        "provenance": provenance,
        "exom": exom
    });
    if let Some(vf) = valid_from {
        payload["valid_from"] = serde_json::json!(vf);
    }
    if let Some(vt) = valid_to {
        payload["valid_to"] = serde_json::json!(vt);
    }
    c.post_json_with_headers(
        &format!("/api/actions/assert-fact?exom={}", exom),
        &payload.to_string(),
        headers,
    )
}

fn retract_fact_id(
    c: &ray_exomem::client::Client,
    exom: &str,
    headers: &[(&str, &str)],
    fact_id: &str,
) -> Result<String> {
    let (predicate, value) = active_fact_tuple(c, exom, fact_id)?;
    let ray = format!(
        "(retract-fact {} \"{}\" '{} \"{}\")",
        exom,
        fact_id.replace('"', "\\\""),
        predicate.replace('"', "\\\""),
        value.replace('"', "\\\""),
    );
    c.post_text_with_headers("/api/actions/eval", &ray, headers)
}

fn replace_fact_json(
    c: &ray_exomem::client::Client,
    exom: &str,
    headers: &[(&str, &str)],
    fact_id: &str,
    predicate: &str,
    value: &str,
    provenance: &str,
    valid_from: Option<&str>,
    valid_to: Option<&str>,
) -> Result<String> {
    if let Err(e) = retract_fact_id(c, exom, headers, fact_id) {
        let msg = e.to_string();
        if !(msg.contains("is not active")
            || msg.contains("HTTP 404")
            || msg.contains("fact not found")
            || msg.contains("not found"))
        {
            return Err(e);
        }
    }
    assert_fact_json(
        c,
        exom,
        headers,
        fact_id,
        predicate,
        value,
        1.0,
        provenance,
        valid_from,
        valid_to,
    )
}

fn coord_fact_id(prefix: &str, entity_id: &str, field: &str) -> String {
    format!("{}/{}/{}", prefix, entity_id, field)
}

fn eval_json(
    c: &ray_exomem::client::Client,
    headers: &[(&str, &str)],
    source: &str,
) -> Result<serde_json::Value> {
    let body = c.post_text_with_headers("/api/actions/eval", source, headers)?;
    serde_json::from_str(&body).context("parse eval JSON response")
}

fn query_rows_json(
    c: &ray_exomem::client::Client,
    exom: &str,
    headers: &[(&str, &str)],
    source: &str,
) -> Result<Vec<Vec<serde_json::Value>>> {
    let rendered = source.replace("<exom>", exom);
    let v = eval_json(c, headers, &rendered)?;
    let rows = v["rows"].as_array().cloned().unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|row| row.as_array().cloned())
        .collect())
}

fn query_two_string_cols(
    c: &ray_exomem::client::Client,
    exom: &str,
    headers: &[(&str, &str)],
    source: &str,
) -> Result<Vec<(String, String)>> {
    let rows = query_rows_json(c, exom, headers, source)?;
    let mut out = Vec::new();
    for row in rows {
        let left = row
            .first()
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let right = row
            .get(1)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !left.is_empty() {
            out.push((left, right));
        }
    }
    Ok(out)
}

fn optional_fact_detail(
    c: &ray_exomem::client::Client,
    exom: &str,
    fact_id: &str,
) -> Result<Option<serde_json::Value>> {
    let path = format!(
        "/api/facts/{}?exom={}",
        encode_url_part(fact_id),
        encode_url_part(exom)
    );
    match c.get(&path) {
        Ok(body) => Ok(Some(
            serde_json::from_str(&body).context("parse fact detail JSON")?,
        )),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("HTTP 404")
                || msg.contains("fact not found")
                || msg.contains("not found")
            {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

fn claim_id_from_field_fact_id(fact_id: &str, field: &str) -> Option<String> {
    fact_id
        .strip_prefix("claim/")
        .and_then(|rest| rest.strip_suffix(&format!("/{}", field)))
        .map(str::to_string)
}

fn task_id_from_dependency_fact_id(fact_id: &str) -> Option<String> {
    fact_id
        .strip_prefix("task/")
        .and_then(|rest| rest.split_once("/depends/"))
        .map(|(task_id, _)| task_id.to_string())
}

fn agent_id_from_session_fact_id(fact_id: &str) -> Option<String> {
    fact_id
        .strip_prefix("agent/")
        .and_then(|rest| rest.strip_suffix("/session"))
        .map(str::to_string)
}

fn current_fact_value(detail: Option<&serde_json::Value>) -> serde_json::Value {
    match detail {
        Some(v) if v["fact"]["status"].as_str() == Some("active") => v["fact"]["tuple"]
            .get(2)
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    }
}

/// Stop a running daemon by reading its PID file and sending SIGTERM.
/// Returns true if a daemon was found and stopped.
/// Check whether the given PID belongs to a ray-exomem process.
fn is_ray_exomem_process(pid: u32) -> bool {
    // macOS: `ps -p <pid> -o comm=` prints just the executable name
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let name = String::from_utf8_lossy(&o.stdout);
            name.trim().ends_with("ray-exomem")
        }
        _ => false,
    }
}

fn stop_existing_daemon(data_dir: &std::path::Path) -> bool {
    let pid_file = pid_path(data_dir);
    let pid_str = match std::fs::read_to_string(&pid_file) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            let _ = std::fs::remove_file(&pid_file);
            return false;
        }
    };

    // Verify the PID actually belongs to ray-exomem (PID could have been recycled)
    if !is_ray_exomem_process(pid) {
        let _ = std::fs::remove_file(&pid_file);
        return false;
    }

    eprintln!("[ray-exomem] Stopping existing daemon (pid {})...", pid);
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status();

    // Wait up to 3 seconds for graceful shutdown
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !is_ray_exomem_process(pid) {
            let _ = std::fs::remove_file(&pid_file);
            eprintln!("[ray-exomem] Previous daemon stopped.");
            return true;
        }
    }

    // Force kill only if still ray-exomem
    if is_ray_exomem_process(pid) {
        eprintln!("[ray-exomem] Daemon did not stop gracefully, sending SIGKILL...");
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
    }
    let _ = std::fs::remove_file(&pid_file);
    true
}

fn write_pid(data_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(pid_path(data_dir), std::process::id().to_string());
}

fn remove_pid(data_dir: &std::path::Path) {
    let _ = std::fs::remove_file(pid_path(data_dir));
}

fn resolve_ui_dir(ui_dir: Option<PathBuf>) -> Option<PathBuf> {
    ui_dir.map(|d| {
        if d.is_absolute() {
            d
        } else {
            std::env::current_dir()
                .expect("failed to read current working directory")
                .join(d)
        }
    })
}

fn main() {
    let Cli {
        json: global_json,
        command,
    } = Cli::parse();

    match command {
        Commands::Run { file } => match ray_exomem::run_file(&file) {
            Ok(output) => println!("{}", output),
            Err(err) => {
                eprintln!("error: {}", err);
                std::process::exit(1);
            }
        },
        Commands::Eval {
            source,
            file,
            addr,
            exom,
            actor,
            session,
            model,
            branch,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let h = ctx_headers(&actor, &session, &model);
            if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            let source = match (source, file) {
                (Some(s), _) => s,
                (None, Some(f)) if f == "-" => {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                        .expect("failed to read stdin");
                    buf
                }
                (None, Some(f)) => std::fs::read_to_string(&f).unwrap_or_else(|e| {
                    eprintln!("error reading {}: {}", f, e);
                    std::process::exit(1);
                }),
                (None, None) => {
                    eprintln!("error: provide either a source expression or --file");
                    std::process::exit(1);
                }
            };
            match c.post_text_with_headers("/api/actions/eval", &source, &h) {
                Ok(body) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(err) = v["error"].as_str() {
                            eprintln!("error: {}", err);
                            std::process::exit(1);
                        }
                        println!("{}", v["output"].as_str().unwrap_or(""));
                    } else {
                        println!("{}", body);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Serve {
            bind,
            ui_dir,
            data_dir,
            no_persist,
        } => {
            let root = resolve_ui_dir(ui_dir);
            let resolved_data_dir = if no_persist {
                None
            } else {
                Some(data_dir.unwrap_or_else(default_data_dir))
            };
            if let Err(err) = ray_exomem::web::serve(root, bind, resolved_data_dir) {
                eprintln!("error: {}", err);
                std::process::exit(1);
            }
        }
        Commands::Daemon {
            bind,
            ui_dir,
            data_dir,
        } => {
            let data_dir = data_dir.unwrap_or_else(default_data_dir);

            // Stop any existing daemon
            stop_existing_daemon(&data_dir);

            // Resolve UI dir before fork (needs cwd)
            let root = resolve_ui_dir(ui_dir);

            // Fork into background
            unsafe {
                let pid = libc::fork();
                if pid < 0 {
                    eprintln!("[ray-exomem] fork failed");
                    std::process::exit(1);
                }
                if pid > 0 {
                    // Parent: print info and exit
                    eprintln!("[ray-exomem] Daemon started (pid {})", pid);
                    eprintln!(
                        "[ray-exomem] Open http://{}:{}/ray-exomem/",
                        bind.ip(),
                        bind.port()
                    );
                    eprintln!("[ray-exomem] Stop with: ray-exomem stop");
                    std::process::exit(0);
                }
                // Child: detach from terminal
                libc::setsid();

                // Redirect stdin/stdout/stderr to /dev/null
                let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDWR);
                if devnull >= 0 {
                    libc::dup2(devnull, 0); // stdin
                    libc::dup2(devnull, 1); // stdout
                    libc::dup2(devnull, 2); // stderr
                    if devnull > 2 {
                        libc::close(devnull);
                    }
                }
            }

            // Write child PID
            write_pid(&data_dir);

            // Register signal handler to clean up PID file on SIGTERM/SIGINT
            {
                let cleanup_dir = data_dir.clone();
                std::thread::spawn(move || {
                    unsafe {
                        let mut sigset: libc::sigset_t = std::mem::zeroed();
                        libc::sigemptyset(&mut sigset);
                        libc::sigaddset(&mut sigset, libc::SIGTERM);
                        libc::sigaddset(&mut sigset, libc::SIGINT);
                        libc::pthread_sigmask(libc::SIG_BLOCK, &sigset, std::ptr::null_mut());
                        let mut sig: libc::c_int = 0;
                        libc::sigwait(&sigset, &mut sig);
                    }
                    remove_pid(&cleanup_dir);
                    std::process::exit(0);
                });
            }

            if let Err(err) = ray_exomem::web::serve(root, bind, Some(data_dir.clone())) {
                remove_pid(&data_dir);
                eprintln!("error: {}", err);
                std::process::exit(1);
            }

            remove_pid(&data_dir);
        }
        Commands::Stop => {
            let data_dir = default_data_dir();
            if stop_existing_daemon(&data_dir) {
                eprintln!("[ray-exomem] Daemon stopped.");
            } else {
                eprintln!("[ray-exomem] No running daemon found.");
            }
        }
        Commands::Status { exom, addr } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            match c.get(&format!("/api/status?exom={}", exom)) {
                Ok(body) => println!("{}", body),
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Assert {
            predicate,
            value,
            fact_id,
            confidence,
            source,
            valid_from,
            valid_to,
            exom,
            addr,
            actor,
            session,
            model,
            branch,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let h = ctx_headers(&actor, &session, &model);
            let fact_id = fact_id.unwrap_or_else(|| predicate.clone());
            let uses_structured_assert =
                should_use_structured_assert(confidence, &source, &valid_from, &valid_to);
            let confidence_value = confidence.unwrap_or(1.0);
            let provenance = source.unwrap_or_else(|| "cli".to_string());
            if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            if uses_structured_assert {
                match assert_fact_json(
                    &c,
                    &exom,
                    &h,
                    &fact_id,
                    &predicate,
                    &value,
                    confidence_value,
                    &provenance,
                    valid_from.as_deref(),
                    valid_to.as_deref(),
                ) {
                    Ok(body) => println!("{}", body),
                    Err(e) => {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                let ray = format!(
                    "(assert-fact {} \"{}\" '{} \"{}\")",
                    exom,
                    fact_id.replace('"', "\\\""),
                    predicate.replace('"', "\\\""),
                    value.replace('"', "\\\""),
                );
                match c.post_text_with_headers("/api/actions/eval", &ray, &h) {
                    Ok(body) => println!("{}", body),
                    Err(e) => {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Retract {
            fact_id,
            exom,
            addr,
            actor,
            session,
            model,
            branch,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let h = ctx_headers(&actor, &session, &model);
            if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            let (predicate, value) = match active_fact_tuple(&c, &exom, &fact_id) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            };
            let ray = format!(
                "(retract-fact {} \"{}\" '{} \"{}\")",
                exom,
                fact_id.replace('"', "\\\""),
                predicate.replace('"', "\\\""),
                value.replace('"', "\\\""),
            );
            match c.post_text_with_headers("/api/actions/eval", &ray, &h) {
                Ok(body) => println!("{}", body),
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Facts { exom, addr } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            match c.get(&format!(
                "/api/schema?include_samples=true&sample_limit=10000&exom={}",
                exom
            )) {
                Ok(body) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(relations) = v["relations"].as_array() {
                            for rel in relations {
                                let name = rel["name"].as_str().unwrap_or("?");
                                let card = rel["cardinality"].as_u64().unwrap_or(0);
                                let kind = rel["kind"].as_str().unwrap_or("?");
                                println!("{}  ({}, {} tuples)", name, kind, card);
                                if let Some(tuples) = rel["sample_tuples"].as_array() {
                                    for tuple in tuples {
                                        if let Some(arr) = tuple.as_array() {
                                            let terms: Vec<String> = arr
                                                .iter()
                                                .map(|t| {
                                                    t.as_str()
                                                        .map(|s| s.to_string())
                                                        .unwrap_or_else(|| t.to_string())
                                                })
                                                .collect();
                                            println!("  {}", terms.join(", "));
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        println!("{}", body);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Query {
            exom,
            addr,
            request,
            json: q_json,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let source = if let Some(r) = request {
                r
            } else {
                format!(
                    "(query {} (find ?fact ?pred ?value) (where (?fact 'fact/predicate ?pred) (?fact 'fact/value ?value)))",
                    exom
                )
            };
            let compact = json_stdout(global_json, q_json);
            let h = vec![("X-Actor", "cli-query")];
            match c.post_text_with_headers("/api/query", &source, &h) {
                Ok(out) => {
                    if compact {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                            print_json_value(&v, true);
                        } else {
                            println!("{}", out);
                        }
                    } else if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                        if let Some(s) = v["output"].as_str() {
                            println!("{}", s);
                        } else {
                            print_json_value(&v, false);
                        }
                    } else {
                        println!("{}", out);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::ExpandQuery {
            exom,
            addr,
            request,
            json: eq_json,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let source = match apply_expand_query_exom(&request, exom.as_deref()) {
                Ok(source) => source,
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            };
            let compact = json_stdout(global_json, eq_json);
            match c.post_text("/api/expand-query", &source) {
                Ok(out) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                        if compact {
                            print_json_value(&v, true);
                        } else {
                            let original = v["original_source"].as_str().unwrap_or("");
                            let normalized = v["normalized_query"].as_str().unwrap_or("");
                            let expanded = v["expanded_query"].as_str().unwrap_or("");
                            println!("original:\n{}\n", original);
                            println!("normalized:\n{}\n", normalized);
                            println!("expanded:\n{}", expanded);
                        }
                    } else {
                        println!("{}", out);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Doctor {
            exom,
            addr,
            json: d_json,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let compact = json_stdout(global_json, d_json);
            let mut issues: Vec<String> = Vec::new();
            let local_build_identity = ray_exomem::build_identity();
            let mut status_payload: Option<serde_json::Value> = None;
            let mut exom_known = true;
            let mut daemon_reachable = true;
            match c.get(&format!("/api/status?exom={}", exom)) {
                Ok(s) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
                            issues.push("status did not return ok:true".into());
                        }
                        match v["server"]["build"]["identity"].as_str() {
                            Some(identity) if identity != local_build_identity => issues.push(
                                format!(
                                    "CLI build {} does not match daemon build {}",
                                    local_build_identity, identity
                                ),
                            ),
                            Some(_) => {}
                            None => issues.push(
                                "status response is missing server.build.identity".into(),
                            ),
                        }
                        status_payload = Some(v);
                    } else {
                        issues.push("status response is not JSON".into());
                    }
                }
                Err(e) => {
                    if is_unknown_exom_error(&e) {
                        issues.push(format!("unknown exom '{}'", exom));
                        exom_known = false;
                    } else {
                        issues.push(format!("daemon not reachable: {}", e));
                        daemon_reachable = false;
                    }
                }
            }
            if daemon_reachable && exom_known {
                match c.get(&format!("/api/branches?exom={}", exom)) {
                    Ok(b) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&b) {
                            if let Some(arr) = v["branches"].as_array() {
                                let n = arr
                                    .iter()
                                    .filter(|br| br["is_current"].as_bool() == Some(true))
                                    .count();
                                if n != 1 {
                                    issues.push(format!(
                                        "expected exactly one current branch in /api/branches, found {}",
                                        n
                                    ));
                                }
                            }
                        }
                    }
                    Err(e) => issues.push(format!("branches: {}", e)),
                }
                match c.get(&format!("/api/actions/export?exom={}", exom)) {
                    Ok(_) => {}
                    Err(e) => issues.push(format!("export: {}", e)),
                }
            }
            let eval_h = vec![("X-Actor", "ray-exomem-doctor")];
            if daemon_reachable {
                match c.post_text_with_headers("/api/actions/eval", "(+ 1 1)", &eval_h) {
                    Ok(body) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                            if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
                                issues.push("eval smoke did not return ok:true".into());
                            }
                        } else {
                            issues.push("eval smoke response is not JSON".into());
                        }
                    }
                    Err(e) => issues.push(format!("eval smoke: {}", e)),
                }
            }

            if issues.is_empty() {
                print_json_value(
                    &serde_json::json!({
                        "ok": true,
                        "exom": exom,
                        "checks_passed": 4,
                        "mode": "datalog_native",
                        "build": {
                            "cli": local_build_identity,
                            "daemon": status_payload
                                .as_ref()
                                .and_then(|v| v["server"]["build"]["identity"].as_str())
                                .unwrap_or("")
                        }
                    }),
                    compact,
                );
            } else {
                if compact {
                    print_json_value(
                        &serde_json::json!({ "ok": false, "exom": exom, "issues": issues }),
                        true,
                    );
                } else {
                    eprintln!("ray-exomem doctor: issues:");
                    for i in &issues {
                        eprintln!("  - {}", i);
                    }
                }
                std::process::exit(1);
            }
        }
        Commands::StartSession { exom, addr, actor } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let create_headers = vec![("X-Actor", actor.as_str())];
            let status_path = format!("/api/status?exom={}", exom);
            let smoke_query = format!(
                "(query {} (find ?fact ?pred ?value) (where (?fact 'fact/predicate ?pred) (?fact 'fact/value ?value)))",
                exom
            );
            let exom_exists = match c.get("/api/exoms") {
                Ok(list_body) => serde_json::from_str::<serde_json::Value>(&list_body)
                    .ok()
                    .and_then(|v| {
                        v["exoms"].as_array().map(|arr| {
                            arr.iter()
                                .any(|e| e["name"].as_str() == Some(exom.as_str()))
                        })
                    })
                    .unwrap_or(false),
                Err(e) => {
                    eprintln!("error: daemon not reachable: {}", e);
                    std::process::exit(1);
                }
            };
            if !exom_exists {
                let create = serde_json::json!({ "name": exom, "description": "" });
                if let Err(e) =
                    c.post_json_with_headers("/api/exoms", &create.to_string(), &create_headers)
                {
                    eprintln!("error: could not create exom: {}", e);
                    std::process::exit(1);
                }
            }
            let mut status_body = None;
            let query_headers = vec![("X-Actor", actor.as_str())];
            let mut last_err = None;
            for _ in 0..8 {
                match c.get(&status_path) {
                    Ok(s) => match c.post_text_with_headers(
                        "/api/actions/eval",
                        &smoke_query,
                        &query_headers,
                    ) {
                        Ok(_) => {
                            status_body = Some(s);
                            break;
                        }
                        Err(e) => last_err = Some(format!("query smoke failed: {}", e)),
                    },
                    Err(e) => last_err = Some(format!("status failed: {}", e)),
                }
                std::thread::sleep(std::time::Duration::from_millis(125));
            }
            let status_body = match status_body {
                Some(s) => s,
                None => {
                    eprintln!(
                        "error: exom '{}' was not ready after bootstrap: {}",
                        exom,
                        last_err.unwrap_or_else(|| "unknown error".to_string())
                    );
                    std::process::exit(1);
                }
            };
            let v: serde_json::Value = match serde_json::from_str(&status_body) {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("error: status is not valid JSON");
                    std::process::exit(1);
                }
            };
            let branch = v["current_branch"].as_str().unwrap_or("main");
            let hostport = addr
                .trim_start_matches("http://")
                .trim_start_matches("https://");
            let base_url = format!("http://{}/ray-exomem", hostport);
            let contract = serde_json::json!({
                "exom": exom,
                "base_url": base_url,
                "current_branch": branch,
                "required_headers": ["X-Actor", "X-Session", "X-Model"],
                "query_mode": "rayfall",
                "exom_ready": true,
                "schema": v.get("schema").cloned().unwrap_or(serde_json::Value::Null)
            });
            print_json_value(&contract, global_json || !std::io::stdout().is_terminal());
        }
        Commands::Observe {
            content,
            source_type: _,
            source_ref: _,
            confidence: _,
            tags: _,
            exom,
            addr,
            actor,
            session,
            model,
            branch,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let h = ctx_headers(&actor, &session, &model);
            if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            let ray = format!(
                "(assert-fact {} \"observation\" 'content \"{}\")",
                exom,
                content.replace('"', "\\\""),
            );
            match c.post_text_with_headers("/api/actions/eval", &ray, &h) {
                Ok(body) => println!("{}", body),
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Export { exom, addr, format } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let endpoint = match format.as_str() {
                "rayfall" => format!("/api/actions/export?exom={}", exom),
                _ => format!("/api/actions/export-json?exom={}", exom),
            };
            match c.get(&endpoint) {
                Ok(body) => print!("{}", body),
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Import { file, exom, addr } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let source = if file == "-" {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                    .expect("failed to read stdin");
                buf
            } else {
                std::fs::read_to_string(&file).unwrap_or_else(|e| {
                    eprintln!("error reading {}: {}", file, e);
                    std::process::exit(1);
                })
            };
            let actor_cli = Some("cli".to_string());
            let h = ctx_headers(&actor_cli, &None, &None);
            match c.post_json_with_headers(
                &format!("/api/actions/import-json?exom={}", exom),
                &source,
                &h,
            ) {
                Ok(body) => println!("{}", body),
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Exoms { addr } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            match c.get("/api/exoms") {
                Ok(body) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(exoms) = v["exoms"].as_array() {
                            for e in exoms {
                                let name = e["name"].as_str().unwrap_or("?");
                                let desc = e["description"].as_str().unwrap_or("");
                                if desc.is_empty() {
                                    println!("{}", name);
                                } else {
                                    println!("{}  — {}", name, desc);
                                }
                            }
                        }
                    } else {
                        println!("{}", body);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Log { exom, addr } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            match c.get(&format!("/api/logs?exom={}", exom)) {
                Ok(body) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(events) = v["events"].as_array() {
                            for ev in events {
                                let id = ev["id"].as_str().unwrap_or("?");
                                let typ = ev["type"].as_str().unwrap_or("?");
                                let ts = ev["timestamp"].as_str().unwrap_or("?");
                                let note = ev["pattern"].as_str().unwrap_or("");
                                println!("{} [{}] {} — {}", id, ts, typ, note);
                            }
                        }
                    } else {
                        println!("{}", body);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Version => {
            println!(
                "ray-exomem {} (build: {}, backend: rayforce2 {}, syntax: {})",
                ray_exomem::frontend_version(),
                ray_exomem::build_identity(),
                ray_exomem::rayforce_version(),
                ray_exomem::syntax_name(),
            );
        }
        Commands::BrainDemo => {
            println!("{}", ray_exomem::brain::Brain::run_demo());
        }
        Commands::Branch {
            command,
            exom,
            addr,
        } => {
            match command {
                BranchCommands::List { scope } => {
                    let (compact, exom, addr) =
                        branch_scope_values(global_json, &exom, &addr, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    match c.get(&format!("/api/branches?exom={}", exom)) {
                        Ok(body) => print_json_or_raw(&body, compact),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Create {
                    branch_id,
                    name,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr) =
                        branch_scope_values(global_json, &exom, &addr, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    let payload = serde_json::json!({
                        "branch_id": branch_id,
                        "name": name.unwrap_or_else(|| branch_id.clone()),
                    });
                    match c.post_json_with_headers(
                        &format!("/api/branches?exom={}", exom),
                        &payload.to_string(),
                        &h,
                    ) {
                        Ok(body) => print_json_or_raw(&body, compact),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Switch {
                    branch_id,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr) =
                        branch_scope_values(global_json, &exom, &addr, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    match c.post_text_with_headers(
                        &format!("/api/branches/{}/switch?exom={}", branch_id, exom),
                        "",
                        &h,
                    ) {
                        Ok(_) => {
                            if compact {
                                print_json_value(
                                    &serde_json::json!({
                                        "ok": true,
                                        "command": "switch",
                                        "exom": exom,
                                        "branch_id": branch_id
                                    }),
                                    true,
                                );
                            } else {
                                println!("Switched to branch '{}'", branch_id);
                            }
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Diff {
                    branch_id,
                    base,
                    scope,
                } => {
                    let (compact, exom, addr) =
                        branch_scope_values(global_json, &exom, &addr, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    match c.get(&format!(
                        "/api/branches/{}/diff?exom={}&base={}",
                        branch_id, exom, base
                    )) {
                        Ok(body) => print_json_or_raw(&body, compact),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Merge {
                    source,
                    policy,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr) =
                        branch_scope_values(global_json, &exom, &addr, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    let payload = serde_json::json!({ "policy": policy });
                    match c.post_json_with_headers(
                        &format!("/api/branches/{}/merge?exom={}", source, exom),
                        &payload.to_string(),
                        &h,
                    ) {
                        Ok(body) => print_json_or_raw(&body, compact),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Delete {
                    branch_id,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr) =
                        branch_scope_values(global_json, &exom, &addr, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    match c.delete_with_headers(
                        &format!("/api/branches/{}?exom={}", branch_id, exom),
                        &h,
                    ) {
                        Ok(_) => {
                            if compact {
                                print_json_value(
                                    &serde_json::json!({
                                        "ok": true,
                                        "command": "delete",
                                        "exom": exom,
                                        "branch_id": branch_id
                                    }),
                                    true,
                                );
                            } else {
                                println!("Archived branch '{}'", branch_id);
                            }
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Coord {
            command,
            exom,
            addr,
            branch,
        } => {
            match command {
                CoordCommands::Claim {
                    claim_id,
                    owner,
                    status,
                    expires_at,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let owner_fact_id = coord_fact_id("claim", &claim_id, "owner");
                    let status_fact_id = coord_fact_id("claim", &claim_id, "status");
                    let expires_fact_id = coord_fact_id("claim", &claim_id, "expires_at");
                    let mut results = Vec::new();
                    match replace_fact_json(
                        &c,
                        &exom,
                        &h,
                        &owner_fact_id,
                        ray_exomem::system_schema::attrs::coord::CLAIM_OWNER,
                        &owner,
                        "coord-cli",
                        None,
                        None,
                    ) {
                        Ok(body) => results.push(
                            serde_json::from_str::<serde_json::Value>(&body)
                                .unwrap_or_else(|_| serde_json::json!({ "raw": body })),
                        ),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                    match replace_fact_json(
                        &c,
                        &exom,
                        &h,
                        &status_fact_id,
                        ray_exomem::system_schema::attrs::coord::CLAIM_STATUS,
                        &status,
                        "coord-cli",
                        None,
                        None,
                    ) {
                        Ok(body) => results.push(
                            serde_json::from_str::<serde_json::Value>(&body)
                                .unwrap_or_else(|_| serde_json::json!({ "raw": body })),
                        ),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                    if let Some(expires_at) = expires_at.as_deref() {
                        match replace_fact_json(
                            &c,
                            &exom,
                            &h,
                            &expires_fact_id,
                            ray_exomem::system_schema::attrs::coord::CLAIM_EXPIRES_AT,
                            expires_at,
                            "coord-cli",
                            None,
                            None,
                        ) {
                            Ok(body) => results.push(
                                serde_json::from_str::<serde_json::Value>(&body)
                                    .unwrap_or_else(|_| serde_json::json!({ "raw": body })),
                            ),
                            Err(e) => {
                                eprintln!("error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        match retract_fact_id(&c, &exom, &h, &expires_fact_id) {
                            Ok(body) => results.push(
                                serde_json::from_str::<serde_json::Value>(&body)
                                    .unwrap_or_else(|_| serde_json::json!({ "raw": body })),
                            ),
                            Err(e) => {
                                let msg = e.to_string();
                                if msg.contains("is not active")
                                    || msg.contains("HTTP 404")
                                    || msg.contains("fact not found")
                                    || msg.contains("not found")
                                {
                                    results.push(serde_json::json!({
                                        "ok": true,
                                        "fact_id": expires_fact_id,
                                        "skipped": true
                                    }));
                                } else {
                                    eprintln!("error: {}", e);
                                    std::process::exit(1);
                                }
                            }
                        }
                    }
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "claim",
                        "exom": exom,
                        "claim_id": claim_id,
                        "owner": owner,
                        "status": status,
                        "expires_at": expires_at,
                        "writes": results
                    });
                    print_json_value(&out, compact);
                }
                CoordCommands::Release {
                    claim_id,
                    status,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let owner_fact_id = coord_fact_id("claim", &claim_id, "owner");
                    let status_fact_id = coord_fact_id("claim", &claim_id, "status");
                    let expires_fact_id = coord_fact_id("claim", &claim_id, "expires_at");
                    let mut results = Vec::new();
                    for retract_id in [&owner_fact_id, &expires_fact_id] {
                        match retract_fact_id(&c, &exom, &h, retract_id) {
                            Ok(body) => results.push(
                                serde_json::from_str::<serde_json::Value>(&body)
                                    .unwrap_or_else(|_| serde_json::json!({ "raw": body })),
                            ),
                            Err(e) => {
                                let msg = e.to_string();
                                if msg.contains("is not active")
                                    || msg.contains("HTTP 404")
                                    || msg.contains("not found")
                                {
                                    results.push(serde_json::json!({
                                        "ok": true,
                                        "fact_id": retract_id,
                                        "skipped": true
                                    }));
                                } else {
                                    eprintln!("error: {}", e);
                                    std::process::exit(1);
                                }
                            }
                        }
                    }
                    match replace_fact_json(
                        &c,
                        &exom,
                        &h,
                        &status_fact_id,
                        ray_exomem::system_schema::attrs::coord::CLAIM_STATUS,
                        &status,
                        "coord-cli",
                        None,
                        None,
                    ) {
                        Ok(body) => results.push(
                            serde_json::from_str::<serde_json::Value>(&body)
                                .unwrap_or_else(|_| serde_json::json!({ "raw": body })),
                        ),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "release",
                        "exom": exom,
                        "claim_id": claim_id,
                        "status": status,
                        "writes": results
                    });
                    print_json_value(&out, compact);
                }
                CoordCommands::Depend {
                    task_id,
                    depends_on,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let fact_id = format!("task/{}/depends/{}", task_id, depends_on);
                    match replace_fact_json(
                        &c,
                        &exom,
                        &h,
                        &fact_id,
                        ray_exomem::system_schema::attrs::coord::TASK_DEPENDS_ON,
                        &depends_on,
                        "coord-cli",
                        None,
                        None,
                    ) {
                        Ok(body) => {
                            let out = serde_json::json!({
                                "ok": true,
                                "command": "depend",
                                "exom": exom,
                                "task_id": task_id,
                                "depends_on": depends_on,
                                "write": serde_json::from_str::<serde_json::Value>(&body)
                                    .unwrap_or_else(|_| serde_json::json!({ "raw": body }))
                            });
                            print_json_value(&out, compact);
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                CoordCommands::AgentSession {
                    agent_id,
                    session_id,
                    actor,
                    session,
                    model,
                    scope,
                } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let fact_id = coord_fact_id("agent", &agent_id, "session");
                    match replace_fact_json(
                        &c,
                        &exom,
                        &h,
                        &fact_id,
                        ray_exomem::system_schema::attrs::coord::AGENT_SESSION,
                        &session_id,
                        "coord-cli",
                        None,
                        None,
                    ) {
                        Ok(body) => {
                            let out = serde_json::json!({
                                "ok": true,
                                "command": "agent-session",
                                "exom": exom,
                                "agent_id": agent_id,
                                "session_id": session_id,
                                "write": serde_json::from_str::<serde_json::Value>(&body)
                                    .unwrap_or_else(|_| serde_json::json!({ "raw": body }))
                            });
                            print_json_value(&out, compact);
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                CoordCommands::ListClaims {
                    owner,
                    status,
                    scope,
                } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some("cli-query".to_string());
                    let h = ctx_headers(&actor_opt, &None, &None);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let owner_rows = match query_two_string_cols(
                        &c,
                        &exom,
                        &h,
                        "(query <exom> (find ?fact ?owner) (where (claim-owner-row ?fact ?owner)))",
                    ) {
                        Ok(rows) => rows,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    let status_rows = match query_two_string_cols(
                        &c,
                        &exom,
                        &h,
                        "(query <exom> (find ?fact ?status) (where (claim-status-row ?fact ?status)))",
                    ) {
                        Ok(rows) => rows,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    let expires_rows = match query_two_string_cols(
                        &c,
                        &exom,
                        &h,
                        "(query <exom> (find ?fact ?expires) (where (?fact 'claim/expires_at ?expires)))",
                    ) {
                        Ok(rows) => rows,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };

                    let mut claims: HashMap<String, serde_json::Map<String, serde_json::Value>> =
                        HashMap::new();
                    for (fact_id, value) in owner_rows {
                        if let Some(claim_id) = claim_id_from_field_fact_id(&fact_id, "owner") {
                            let entry = claims.entry(claim_id.clone()).or_default();
                            entry.insert("claim_id".into(), serde_json::json!(claim_id));
                            entry.insert("owner".into(), serde_json::json!(value));
                        }
                    }
                    for (fact_id, value) in status_rows {
                        if let Some(claim_id) = claim_id_from_field_fact_id(&fact_id, "status") {
                            let entry = claims.entry(claim_id.clone()).or_default();
                            entry.insert("claim_id".into(), serde_json::json!(claim_id));
                            entry.insert("status".into(), serde_json::json!(value));
                        }
                    }
                    for (fact_id, value) in expires_rows {
                        if let Some(claim_id) = claim_id_from_field_fact_id(&fact_id, "expires_at")
                        {
                            let entry = claims.entry(claim_id.clone()).or_default();
                            entry.insert("claim_id".into(), serde_json::json!(claim_id));
                            entry.insert("expires_at".into(), serde_json::json!(value));
                        }
                    }

                    let mut rows: Vec<serde_json::Value> = claims
                        .into_values()
                        .map(serde_json::Value::Object)
                        .filter(|row| {
                            let owner_ok = owner
                                .as_ref()
                                .map(|want| row["owner"].as_str() == Some(want.as_str()))
                                .unwrap_or(true);
                            let status_ok = status
                                .as_ref()
                                .map(|want| row["status"].as_str() == Some(want.as_str()))
                                .unwrap_or(true);
                            owner_ok && status_ok
                        })
                        .collect();
                    rows.sort_by(|a, b| {
                        a["claim_id"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["claim_id"].as_str().unwrap_or(""))
                    });
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "list-claims",
                        "exom": exom,
                        "count": rows.len(),
                        "claims": rows
                    });
                    print_json_value(&out, compact);
                }
                CoordCommands::ShowClaim { claim_id, scope } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some("cli-query".to_string());
                    let h = ctx_headers(&actor_opt, &None, &None);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let owner_fact_id = coord_fact_id("claim", &claim_id, "owner");
                    let status_fact_id = coord_fact_id("claim", &claim_id, "status");
                    let expires_fact_id = coord_fact_id("claim", &claim_id, "expires_at");
                    let owner = match optional_fact_detail(&c, &exom, &owner_fact_id) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    let status_detail = match optional_fact_detail(&c, &exom, &status_fact_id) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    let expires = match optional_fact_detail(&c, &exom, &expires_fact_id) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    let current = serde_json::json!({
                        "owner": current_fact_value(owner.as_ref()),
                        "status": current_fact_value(status_detail.as_ref()),
                        "expires_at": current_fact_value(expires.as_ref()),
                    });
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "show-claim",
                        "exom": exom,
                        "claim_id": claim_id,
                        "current": current,
                        "facts": {
                            "owner": owner,
                            "status": status_detail,
                            "expires_at": expires
                        }
                    });
                    print_json_value(&out, compact);
                }
                CoordCommands::ListDependencies { task_id, scope } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some("cli-query".to_string());
                    let h = ctx_headers(&actor_opt, &None, &None);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let mut rows: Vec<serde_json::Value> = match query_two_string_cols(
                        &c,
                        &exom,
                        &h,
                        "(query <exom> (find ?fact ?depends_on) (where (task-dependency-row ?fact ?depends_on)))",
                    ) {
                        Ok(rows) => rows
                            .into_iter()
                            .filter_map(|(fact_id, depends_on)| {
                                let task = task_id_from_dependency_fact_id(&fact_id)?;
                                Some(serde_json::json!({
                                    "fact_id": fact_id,
                                    "task_id": task,
                                    "depends_on": depends_on
                                }))
                            })
                            .filter(|row| {
                                task_id
                                    .as_ref()
                                    .map(|want| row["task_id"].as_str() == Some(want.as_str()))
                                    .unwrap_or(true)
                            })
                            .collect(),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    rows.sort_by(|a, b| {
                        a["fact_id"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["fact_id"].as_str().unwrap_or(""))
                    });
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "list-dependencies",
                        "exom": exom,
                        "count": rows.len(),
                        "dependencies": rows
                    });
                    print_json_value(&out, compact);
                }
                CoordCommands::ShowTask { task_id, scope } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some("cli-query".to_string());
                    let h = ctx_headers(&actor_opt, &None, &None);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let deps = match query_two_string_cols(
                        &c,
                        &exom,
                        &h,
                        "(query <exom> (find ?fact ?depends_on) (where (task-dependency-row ?fact ?depends_on)))",
                    ) {
                        Ok(rows) => rows,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    let mut current = Vec::new();
                    let mut facts = Vec::new();
                    for (fact_id, depends_on) in deps {
                        if task_id_from_dependency_fact_id(&fact_id).as_deref()
                            != Some(task_id.as_str())
                        {
                            continue;
                        }
                        current.push(serde_json::json!({
                            "fact_id": fact_id,
                            "depends_on": depends_on
                        }));
                        match optional_fact_detail(&c, &exom, &fact_id) {
                            Ok(detail) => facts.push(serde_json::json!({
                                "fact_id": fact_id,
                                "detail": detail
                            })),
                            Err(e) => {
                                eprintln!("error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    current.sort_by(|a, b| {
                        a["fact_id"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["fact_id"].as_str().unwrap_or(""))
                    });
                    facts.sort_by(|a, b| {
                        a["fact_id"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["fact_id"].as_str().unwrap_or(""))
                    });
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "show-task",
                        "exom": exom,
                        "task_id": task_id,
                        "current": {
                            "dependencies": current
                        },
                        "facts": facts
                    });
                    print_json_value(&out, compact);
                }
                CoordCommands::ListAgentSessions { agent_id, scope } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some("cli-query".to_string());
                    let h = ctx_headers(&actor_opt, &None, &None);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let mut rows: Vec<serde_json::Value> = match query_two_string_cols(
                        &c,
                        &exom,
                        &h,
                        "(query <exom> (find ?fact ?session) (where (agent-session-row ?fact ?session)))",
                    ) {
                        Ok(rows) => rows
                            .into_iter()
                            .filter_map(|(fact_id, session_id)| {
                                let agent = agent_id_from_session_fact_id(&fact_id)?;
                                Some(serde_json::json!({
                                    "fact_id": fact_id,
                                    "agent_id": agent,
                                    "session_id": session_id
                                }))
                            })
                            .filter(|row| {
                                agent_id
                                    .as_ref()
                                    .map(|want| row["agent_id"].as_str() == Some(want.as_str()))
                                    .unwrap_or(true)
                            })
                            .collect(),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    rows.sort_by(|a, b| {
                        a["fact_id"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["fact_id"].as_str().unwrap_or(""))
                    });
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "list-agent-sessions",
                        "exom": exom,
                        "count": rows.len(),
                        "agent_sessions": rows
                    });
                    print_json_value(&out, compact);
                }
                CoordCommands::ShowAgent { agent_id, scope } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(global_json, &exom, &addr, &branch, &scope);
                    let c = ray_exomem::client::Client::new(Some(&addr));
                    let actor_opt = Some("cli-query".to_string());
                    let h = ctx_headers(&actor_opt, &None, &None);
                    if let Err(e) = switch_branch_cli(&c, &branch, &exom, &h) {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                    let fact_id = coord_fact_id("agent", &agent_id, "session");
                    let detail = match optional_fact_detail(&c, &exom, &fact_id) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    };
                    let out = serde_json::json!({
                        "ok": true,
                        "command": "show-agent",
                        "exom": exom,
                        "agent_id": agent_id,
                        "current": {
                            "session_id": current_fact_value(detail.as_ref())
                        },
                        "facts": {
                            "session": detail
                        }
                    });
                    print_json_value(&out, compact);
                }
            }
        }
        Commands::History {
            fact_id,
            exom,
            addr,
            json: h_json,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let path = format!(
                "/api/facts/{}?exom={}",
                encode_url_part(&fact_id),
                encode_url_part(&exom)
            );
            let compact = json_stdout(global_json, h_json);
            match c.get(&path) {
                Ok(body) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        print_json_value(&v, compact);
                    } else {
                        println!("{}", body);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Why {
            fact_id,
            exom,
            addr,
            json: w_json,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let path = format!(
                "/api/explain?exom={}&predicate={}",
                encode_url_part(&exom),
                encode_url_part(&fact_id)
            );
            let compact = json_stdout(global_json, w_json);
            match c.get(&path) {
                Ok(body) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        print_json_value(&v, compact);
                    } else {
                        println!("{}", body);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::WhyNot {
            predicate,
            value,
            exom,
            addr,
            json: wn_json,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let compact = json_stdout(global_json, wn_json);
            match c.get(&format!("/api/actions/export?exom={}", exom)) {
                Ok(body) => {
                    let mut rows = Vec::new();
                    for form in ray_exomem::rayfall_parser::split_forms(&body) {
                        if !matches!(form.kind, ray_exomem::rayfall_parser::FormKind::AssertFact) {
                            continue;
                        }
                        match ray_exomem::rayfall_parser::parse_fact_mutation_args(
                            &form.inner_source,
                        ) {
                            Ok((_exom_name, fact_id, pred, fact_value)) => {
                                if pred == predicate {
                                    rows.push(serde_json::json!({
                                        "fact_id": fact_id,
                                        "predicate": pred,
                                        "value": fact_value
                                    }));
                                }
                            }
                            Err(_) => continue,
                        }
                    }
                    let active_match = if let Some(ref want) = value {
                        rows.iter()
                            .any(|r| r["value"].as_str() == Some(want.as_str()))
                    } else {
                        !rows.is_empty()
                    };
                    let out = serde_json::json!({
                        "ok": true,
                        "predicate": predicate,
                        "value_filter": value,
                        "rows_for_predicate": rows.len(),
                        "active_match": active_match,
                        "detail": if active_match {
                            "at least one current fact matches the filter"
                        } else if rows.is_empty() {
                            "no active fact uses this predicate"
                        } else {
                            "predicate present but no row matches the requested value"
                        }
                    });
                    print_json_value(&out, compact);
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Watch { addr } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let mut out = std::io::stdout();
            if let Err(e) = c.stream_sse("/events", &mut out) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::LintMemory {
            exom,
            addr,
            json: lm_json,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            let compact = json_stdout(global_json, lm_json);
            if let Err(e) = run_lint_memory(&c, &exom, compact) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Guide { topic } => {
            println!("{}", ray_exomem::agent_guide::render(topic));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ray_exomem::agent_guide::GuideTopic;

    #[test]
    fn guide_parses_default_topic() {
        let cli = Cli::parse_from(["ray-exomem", "guide"]);
        assert!(matches!(
            cli.command,
            Commands::Guide {
                topic: GuideTopic::All
            }
        ));
    }

    #[test]
    fn guide_docs_alias() {
        let cli = Cli::parse_from(["ray-exomem", "docs", "--topic", "cli"]);
        assert!(matches!(
            cli.command,
            Commands::Guide {
                topic: GuideTopic::Cli
            }
        ));
    }

    #[test]
    fn load_alias_parses_as_run() {
        let cli = Cli::parse_from(["ray-exomem", "load", "examples/native_smoke.ray"]);
        assert!(matches!(cli.command, Commands::Run { .. }));
    }

    #[test]
    fn serve_defaults_parse() {
        let cli = Cli::parse_from(["ray-exomem", "serve"]);
        match cli.command {
            Commands::Serve {
                bind,
                ui_dir,
                data_dir,
                no_persist,
            } => {
                assert_eq!(bind.to_string(), ray_exomem::web::DEFAULT_BIND_ADDR);
                assert!(ui_dir.is_none());
                assert!(data_dir.is_none());
                assert!(!no_persist);
            }
            _ => panic!("expected serve command"),
        }
    }

    #[test]
    fn daemon_defaults_parse() {
        let cli = Cli::parse_from(["ray-exomem", "daemon"]);
        match cli.command {
            Commands::Daemon {
                bind,
                ui_dir,
                data_dir,
            } => {
                assert_eq!(bind.to_string(), ray_exomem::web::DEFAULT_BIND_ADDR);
                assert!(ui_dir.is_none());
                assert!(data_dir.is_none());
            }
            _ => panic!("expected daemon command"),
        }
    }

    #[test]
    fn expand_query_parses() {
        let cli = Cli::parse_from([
            "ray-exomem",
            "expand-query",
            "--request",
            "(query (find ?x) (where (p ?x)))",
            "--exom",
            "main",
        ]);
        match cli.command {
            Commands::ExpandQuery { exom, request, .. } => {
                assert_eq!(exom.as_deref(), Some("main"));
                assert_eq!(request, "(query (find ?x) (where (p ?x)))");
            }
            _ => panic!("expected expand-query command"),
        }
    }

    #[test]
    fn branch_flags_parse_after_subcommand() {
        let cli = Cli::parse_from([
            "ray-exomem",
            "branch",
            "--exom",
            "parent",
            "--addr",
            "127.0.0.1:9780",
            "list",
            "--exom",
            "child",
            "--addr",
            "127.0.0.1:9799",
            "--json",
        ]);
        match cli.command {
            Commands::Branch { command, exom, addr } => match command {
                BranchCommands::List { scope } => {
                    let (compact, exom, addr) = branch_scope_values(false, &exom, &addr, &scope);
                    assert!(compact);
                    assert_eq!(exom, "child");
                    assert_eq!(addr, "127.0.0.1:9799");
                }
                _ => panic!("expected branch list command"),
            },
            _ => panic!("expected branch command"),
        }
    }

    #[test]
    fn coord_flags_parse_after_subcommand() {
        let cli = Cli::parse_from([
            "ray-exomem",
            "coord",
            "--exom",
            "parent",
            "--addr",
            "127.0.0.1:9780",
            "--branch",
            "parent-branch",
            "agent-session",
            "agent-1",
            "session-1",
            "--exom",
            "child",
            "--addr",
            "127.0.0.1:9799",
            "--branch",
            "child-branch",
            "--json",
        ]);
        match cli.command {
            Commands::Coord {
                command,
                exom,
                addr,
                branch,
            } => match command {
                CoordCommands::AgentSession { scope, .. } => {
                    let (compact, exom, addr, branch) =
                        coord_scope_values(false, &exom, &addr, &branch, &scope);
                    assert!(compact);
                    assert_eq!(exom, "child");
                    assert_eq!(addr, "127.0.0.1:9799");
                    assert_eq!(branch.as_deref(), Some("child-branch"));
                }
                _ => panic!("expected coord agent-session command"),
            },
            _ => panic!("expected coord command"),
        }
    }

    #[test]
    fn assert_uses_structured_endpoint_for_metadata_flags() {
        assert!(should_use_structured_assert(
            Some(0.7),
            &None,
            &None,
            &None
        ));
        assert!(should_use_structured_assert(
            None,
            &Some("manual".into()),
            &None,
            &None
        ));
        assert!(should_use_structured_assert(
            None,
            &None,
            &Some("2026-04-11T00:00:00Z".into()),
            &None
        ));
        assert!(!should_use_structured_assert(None, &None, &None, &None));
    }

    #[test]
    fn doctor_unknown_exom_detection_is_classified() {
        let err = anyhow::anyhow!("daemon returned HTTP 500: {{\"error\":\"unknown exom 'demo'\"}}");
        assert!(is_unknown_exom_error(&err));
        let other = anyhow::anyhow!("cannot connect to daemon at 127.0.0.1:9780");
        assert!(!is_unknown_exom_error(&other));
    }
}
