use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};

use ray_exomem::agent_guide::GuideTopic;

#[derive(Subcommand)]
enum BranchCommands {
    /// List all branches.
    List,
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
    },
    /// Switch the active branch.
    Switch {
        branch_id: String,
    },
    /// Show differences between branches.
    Diff {
        branch_id: String,
        #[arg(long, default_value = "main")]
        base: String,
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
    },
    /// Delete (archive) a branch.
    Delete {
        branch_id: String,
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
    #[command(
        after_long_help = "Examples:\n  \
            ray-exomem daemon\n  \
            ray-exomem daemon --bind 0.0.0.0:9780 --data-dir ~/.ray-exomem\n\n\
            Open http://<bind>/ray-exomem/ in a browser. JSON API: /ray-exomem/api/.\n\
            Stop with: ray-exomem stop"
    )]
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
    #[command(
        after_long_help = "Examples:\n  \
            ray-exomem eval '(+ 1 2)'\n  \
            ray-exomem eval --file myscript.ray\n  \
            echo '(+ 1 2)' | ray-exomem eval --file -\n  \
            ray-exomem eval \"(query db (find ?x) (where (?x :edge ?y)))\" --addr 127.0.0.1:9780\n\n\
            POSTs plain text to /ray-exomem/api/actions/eval. Requires `ray-exomem daemon`."
    )]
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

    /// Assert a fact into an exom. Uses the direct assert endpoint when --valid-from/--valid-to are provided.
    #[command(
        after_long_help = "Examples:\n  \
            ray-exomem assert sky-color blue --exom main\n  \
            ray-exomem assert location paris --valid-from 2024-01-01T00:00:00Z --valid-to 2024-06-01T00:00:00Z\n\n\
            For rich Rayfall, use: ray-exomem eval --file script.ray"
    )]
    Assert {
        /// Predicate name (e.g. "sky-color").
        predicate: String,
        /// Value (e.g. "blue").
        value: String,
        /// Confidence score (0.0–1.0).
        #[arg(long, default_value = "1.0")]
        confidence: f64,
        /// Provenance tag.
        #[arg(long, default_value = "cli")]
        source: String,
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

    /// Retract a fact by predicate.
    Retract {
        /// Predicate to retract.
        predicate: String,
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

    /// Print the agent/operator reference (CLI workflows, HTTP routes, env, limitations).
    #[command(visible_alias = "docs")]
    Guide {
        /// Print only this section (default: full guide).
        #[arg(long, value_enum, default_value_t = GuideTopic::All)]
        topic: GuideTopic,
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

fn switch_branch_cli(
    c: &ray_exomem::client::Client,
    branch: &Option<String>,
    exom: &str,
) -> Result<()> {
    if let Some(b) = branch {
        c.post_text(
            &format!("/api/branches/{}/switch?exom={}", b, exom),
            "",
        )?;
    }
    Ok(())
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
        if d.is_absolute() { d }
        else {
            std::env::current_dir()
                .expect("failed to read current working directory")
                .join(d)
        }
    })
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
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
            if let Err(e) = switch_branch_cli(&c, &branch, &exom) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            let h = ctx_headers(&actor, &session, &model);
            let source = match (source, file) {
                (Some(s), _) => s,
                (None, Some(f)) if f == "-" => {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                        .expect("failed to read stdin");
                    buf
                }
                (None, Some(f)) => {
                    std::fs::read_to_string(&f)
                        .unwrap_or_else(|e| { eprintln!("error reading {}: {}", f, e); std::process::exit(1); })
                }
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
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
            }
        }
        Commands::Serve { bind, ui_dir, data_dir, no_persist } => {
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
        Commands::Daemon { bind, ui_dir, data_dir } => {
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
                    eprintln!("[ray-exomem] Open http://{}:{}/ray-exomem/", bind.ip(), bind.port());
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
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
            }
        }
        Commands::Assert {
            predicate,
            value,
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
            if let Err(e) = switch_branch_cli(&c, &branch, &exom) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            let h = ctx_headers(&actor, &session, &model);
            if valid_from.is_some() || valid_to.is_some() {
                // Use the direct assert-fact endpoint for bitemporal assertions
                let mut payload = serde_json::json!({
                    "predicate": predicate,
                    "value": value,
                    "confidence": confidence,
                    "provenance": source,
                    "exom": exom
                });
                if let Some(ref vf) = valid_from {
                    payload["valid_from"] = serde_json::json!(vf);
                }
                if let Some(ref vt) = valid_to {
                    payload["valid_to"] = serde_json::json!(vt);
                }
                match c.post_json_with_headers(
                    &format!("/api/actions/assert-fact?exom={}", exom),
                    &payload.to_string(),
                    &h,
                ) {
                    Ok(body) => println!("{}", body),
                    Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
                }
            } else {
                let ray = format!(
                    "(assert-fact {} \"{}\" '{} \"{}\")",
                    exom,
                    predicate.replace('"', "\\\""),
                    predicate.replace('"', "\\\""),
                    value.replace('"', "\\\""),
                );
                match c.post_text_with_headers("/api/actions/eval", &ray, &h) {
                    Ok(body) => println!("{}", body),
                    Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
                }
            }
        }
        Commands::Retract {
            predicate,
            exom,
            addr,
            actor,
            session,
            model,
            branch,
        } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            if let Err(e) = switch_branch_cli(&c, &branch, &exom) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            let h = ctx_headers(&actor, &session, &model);
            let body = serde_json::json!({ "predicate": predicate }).to_string();
            match c.post_json_with_headers(&format!("/api/actions/retract?exom={}", exom), &body, &h) {
                Ok(body) => println!("{}", body),
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
            }
        }
        Commands::Facts { exom, addr } => {
            let c = ray_exomem::client::Client::new(Some(&addr));
            match c.get(&format!("/api/schema?include_samples=true&sample_limit=10000&exom={}", exom)) {
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
                                            let terms: Vec<String> = arr.iter()
                                                .map(|t| t.as_str().map(|s| s.to_string())
                                                    .unwrap_or_else(|| t.to_string()))
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
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
            }
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
            if let Err(e) = switch_branch_cli(&c, &branch, &exom) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            let h = ctx_headers(&actor, &session, &model);
            let ray = format!(
                "(assert-fact {} \"observation\" 'content \"{}\")",
                exom,
                content.replace('"', "\\\""),
            );
            match c.post_text_with_headers("/api/actions/eval", &ray, &h) {
                Ok(body) => println!("{}", body),
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
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
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
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
                std::fs::read_to_string(&file)
                    .unwrap_or_else(|e| { eprintln!("error reading {}: {}", file, e); std::process::exit(1); })
            };
            match c.post_json_with_headers(
                &format!("/api/actions/import-json?exom={}", exom),
                &source,
                &[],
            ) {
                Ok(body) => println!("{}", body),
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
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
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
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
                Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
            }
        }
        Commands::Version => {
            println!(
                "ray-exomem {} (backend: rayforce2 {}, syntax: rayfall-native)",
                env!("CARGO_PKG_VERSION"),
                ray_exomem::rayforce_version(),
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
            let c = ray_exomem::client::Client::new(Some(&addr));
            match command {
                BranchCommands::List => match c.get(&format!("/api/branches?exom={}", exom)) {
                    Ok(body) => println!("{}", body),
                    Err(e) => {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                },
                BranchCommands::Create {
                    branch_id,
                    name,
                    actor,
                    session,
                    model,
                } => {
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
                        Ok(body) => println!("{}", body),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Switch { branch_id } => {
                    match c.post_text(
                        &format!("/api/branches/{}/switch?exom={}", branch_id, exom),
                        "",
                    ) {
                        Ok(_) => println!("Switched to branch '{}'", branch_id),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Diff { branch_id, base } => {
                    match c.get(&format!(
                        "/api/branches/{}/diff?exom={}&base={}",
                        branch_id, exom, base
                    )) {
                        Ok(body) => println!("{}", body),
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
                } => {
                    let actor_opt = Some(actor.clone());
                    let h = ctx_headers(&actor_opt, &session, &model);
                    let payload = serde_json::json!({ "policy": policy });
                    match c.post_json_with_headers(
                        &format!("/api/branches/{}/merge?exom={}", source, exom),
                        &payload.to_string(),
                        &h,
                    ) {
                        Ok(body) => println!("{}", body),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                BranchCommands::Delete { branch_id } => {
                    match c.delete(&format!("/api/branches/{}?exom={}", branch_id, exom)) {
                        Ok(_) => println!("Archived branch '{}'", branch_id),
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
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
            Commands::Serve { bind, ui_dir, data_dir, no_persist } => {
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
            Commands::Daemon { bind, ui_dir, data_dir } => {
                assert_eq!(bind.to_string(), ray_exomem::web::DEFAULT_BIND_ADDR);
                assert!(ui_dir.is_none());
                assert!(data_dir.is_none());
            }
            _ => panic!("expected daemon command"),
        }
    }
}
