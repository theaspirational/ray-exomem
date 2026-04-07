use anyhow::{anyhow, bail, Context, Result};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    backend::RayforceEngine,
    brain::{self, Brain, MergePolicy},
    context::MutationContext,
    exom::ExomDir,
    rayfall_parser::{self, FormKind},
    rules::{self, ParsedRule},
    storage::{self, RayObj},
};

pub const UI_MOUNT_PATH: &str = "/ray-exomem";
pub const API_PREFIX: &str = "/ray-exomem/api/";
pub const EVENTS_PATH: &str = "/ray-exomem/events";
pub const DEFAULT_UI_BUILD_DIR: &str = "ui/build";
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:9780";
/// Default knowledge-base name (seeded on first run; CLI/UI omit `?exom=` use this).
pub const DEFAULT_EXOM: &str = "main";

pub fn default_ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_UI_BUILD_DIR)
}

/// URL users should open in a browser (uses `127.0.0.1` when the bind address is `0.0.0.0`).
pub fn http_public_url(bind_addr: SocketAddr) -> String {
    let host = if bind_addr.ip().is_unspecified() {
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        bind_addr.ip()
    };
    match host {
        std::net::IpAddr::V4(v4) => format!("http://{}:{}{}/", v4, bind_addr.port(), UI_MOUNT_PATH),
        std::net::IpAddr::V6(v6) => format!("http://[{}]:{}{}/", v6, bind_addr.port(), UI_MOUNT_PATH),
    }
}

pub struct ExomState {
    pub brain: Brain,
    pub datoms: RayObj,
    pub rules: Vec<ParsedRule>,
}

pub struct DaemonState {
    pub engine: RayforceEngine,
    pub exoms: HashMap<String, ExomState>,
}

/// Shared server state.
pub struct ServerState {
    pub daemon: Mutex<DaemonState>,
    pub exom_dir: Option<ExomDir>,
    pub start_time: Instant,
}

fn get_daemon(state: &ServerState) -> std::sync::MutexGuard<'_, DaemonState> {
    state.daemon.lock().unwrap()
}

fn rules_path(ed: &ExomDir, exom: &str) -> PathBuf {
    ed.exom_path(exom).join("rules.ray")
}

fn datoms_path(ed: &ExomDir, exom: &str) -> PathBuf {
    ed.exom_path(exom).join("datoms")
}

fn load_rules(ed: Option<&ExomDir>, exom: &str) -> Result<Vec<ParsedRule>> {
    let Some(ed) = ed else {
        return Ok(Vec::new());
    };
    let path = rules_path(ed, exom);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let src = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut out = Vec::new();
    for line in src.lines().map(str::trim).filter(|line| !line.is_empty()) {
        out.push(rules::parse_rule_line(
            line,
            MutationContext::default(),
            String::new(),
        )?);
    }
    Ok(out)
}

fn save_rules(ed: Option<&ExomDir>, exom: &str, rules: &[ParsedRule]) -> Result<()> {
    let Some(ed) = ed else {
        return Ok(());
    };
    let path = rules_path(ed, exom);
    let body = if rules.is_empty() {
        String::new()
    } else {
        format!(
            "{}\n",
            rules.iter().map(|r| r.full_text.as_str()).collect::<Vec<_>>().join("\n")
        )
    };
    fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Ensure a rule has the exom name. Rules are always stored with the exom name.
fn ensure_rule_has_exom(exom: &str, rule: &str) -> String {
    let trimmed = rule.trim();
    let prefix = "(rule ";
    if !trimmed.starts_with(prefix) {
        return trimmed.to_string();
    }
    let after_prefix = &trimmed[prefix.len()..];
    // Already has an exom name if the next token is a bare word (not a paren/bracket)
    if !after_prefix.starts_with('(') && !after_prefix.starts_with('[') {
        return trimmed.to_string();
    }
    // Insert exom name
    format!("(rule {} {}", exom, after_prefix)
}

fn build_or_load_datoms(brain: &Brain, ed: Option<&ExomDir>, exom: &str) -> Result<RayObj> {
    if let Some(ed) = ed {
        let dir = datoms_path(ed, exom);
        if storage::table_exists(&dir) {
            return storage::load_table(&dir, &ed.sym_path());
        }
        let datoms = storage::build_datoms_table(&brain.current_facts())?;
        storage::save_table(&datoms, &dir, &ed.sym_path())?;
        storage::sym_save(&ed.sym_path())?;
        return Ok(datoms);
    }
    storage::build_datoms_table(&brain.current_facts())
}

fn persist_exom_state(ed: Option<&ExomDir>, exom: &str, state: &ExomState) -> Result<()> {
    let Some(ed) = ed else {
        return Ok(());
    };
    state.brain.save()?;
    storage::save_table(&state.datoms, &datoms_path(ed, exom), &ed.sym_path())?;
    storage::sym_save(&ed.sym_path())?;
    save_rules(Some(ed), exom, &state.rules)?;
    Ok(())
}

fn restore_runtime(daemon: &DaemonState) -> Result<()> {
    for (name, exom) in &daemon.exoms {
        daemon
            .engine
            .bind_named_db(storage::sym_intern(name), &exom.datoms)?;
    }
    Ok(())
}

/// After a failed `restore_runtime`, C may still hold `ray_retain` refs while Rust is about to drop
/// `RayObj` — unsafe. Clear env + re-init builtins, then bind/eval again from current `daemon`.
fn reconcile_runtime_after_failed_restore(daemon: &DaemonState) -> Result<()> {
    daemon.engine.reconcile_lang_env()?;
    restore_runtime(daemon)
}

fn load_exom_state(ed: Option<&ExomDir>, name: &str) -> Result<ExomState> {
    let brain = if let Some(ed) = ed {
        if ed.is_recovery_mode() {
            eprintln!("[ray-exomem] recovering exom '{}' from JSONL sidecars", name);
            Brain::open_exom_from_jsonl(&ed.exom_path(name), &ed.sym_path())?
        } else {
            let b = Brain::open_exom(&ed.exom_path(name), &ed.sym_path())?;
            // Backfill JSONL sidecars if missing (first run after upgrade)
            b.ensure_jsonl_sidecars()?;
            b
        }
    } else {
        Brain::new()
    };
    let datoms = build_or_load_datoms(&brain, ed, name)?;
    let rules = load_rules(ed, name)?;
    Ok(ExomState { brain, datoms, rules })
}

fn refresh_exom_binding(daemon: &mut DaemonState, exom_dir: Option<&ExomDir>, exom: &str) -> Result<()> {
    let state = daemon
        .exoms
        .get_mut(exom)
        .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
    state.datoms = storage::build_datoms_table(&state.brain.current_facts())?;
    daemon
        .engine
        .bind_named_db(storage::sym_intern(exom), &state.datoms)?;
    persist_exom_state(exom_dir, exom, state)?;
    Ok(())
}

pub fn serve(ui_dir: PathBuf, bind_addr: SocketAddr, data_dir: Option<PathBuf>) -> Result<()> {
    let listener =
        TcpListener::bind(bind_addr).with_context(|| format!("failed to bind {}", bind_addr))?;
    listener
        .set_nonblocking(false)
        .context("failed to configure server socket")?;

    let engine = RayforceEngine::new()?;
    let mut exoms = HashMap::new();
    let exom_dir = match data_dir {
        Some(ref root) => {
            let ed = ExomDir::open(root.clone())?;
            let exom_names = ed.list_exoms()?;
            if exom_names.is_empty() {
                ed.create_exom(DEFAULT_EXOM)?;
                exoms.insert(DEFAULT_EXOM.to_string(), load_exom_state(Some(&ed), DEFAULT_EXOM)?);
            } else {
                for name in &exom_names {
                    eprintln!("[ray-exomem] loading exom '{}'", name);
                    exoms.insert(name.clone(), load_exom_state(Some(&ed), name)?);
                }
            }
            Some(ed)
        }
        None => {
            exoms.insert(DEFAULT_EXOM.to_string(), load_exom_state(None, DEFAULT_EXOM)?);
            None
        }
    };

    let daemon = DaemonState { engine, exoms };
    if let Err(e) = restore_runtime(&daemon) {
        // `ray_env_set` retains each bound datoms table. If `eval` fails mid-restore, unwinding would
        // drop `ExomState` while refcount accounting can disagree with the C heap — SIGSEGV in
        // `ray_release`. `process::exit` aborts without running Rust drops (see std docs).
        eprintln!(
            "[ray-exomem] fatal: rule runtime restore failed: {}\n\
             Hint: fix or temporarily move `<data-dir>/exoms/<exom>/rules.ray`, then restart.",
            e
        );
        std::process::exit(1);
    }

    let state = Arc::new(ServerState {
        daemon: Mutex::new(daemon),
        exom_dir,
        start_time: Instant::now(),
    });

    eprintln!(
        "[ray-exomem] Open {} in your browser — UI + JSON API (assets: {})",
        http_public_url(bind_addr),
        ui_dir.display()
    );
    if let Some(ref dd) = data_dir {
        eprintln!("[ray-exomem] data dir: {}", dd.display());
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let ui_dir = ui_dir.clone();
                let state = state.clone();
                thread::spawn(move || {
                    if let Err(err) = handle_connection(stream, &ui_dir, &state) {
                        eprintln!("request error: {err}");
                    }
                });
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP request parsing
// ---------------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    body: Vec<u8>,
    headers: HashMap<String, String>,
}

fn parse_request(raw: &[u8]) -> Result<Request> {
    let header_end = find_header_end(raw).unwrap_or(raw.len());
    let header_str = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_str.lines();

    let request_line = lines.next().ok_or_else(|| anyhow!("empty request"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("/");

    let (path, query_str) = match raw_path.find('?') {
        Some(i) => (&raw_path[..i], &raw_path[i + 1..]),
        None => (raw_path, ""),
    };

    let mut query = HashMap::new();
    for pair in query_str.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            query.insert(k.to_string(), v.to_string());
        }
    }

    let mut headers = HashMap::new();
    for line in header_str.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    // Extract Content-Length and body
    let mut content_length = 0usize;
    for line in header_str.lines() {
        if let Some(val) = line.strip_prefix("Content-Length:").or_else(|| line.strip_prefix("content-length:")) {
            content_length = val.trim().parse().unwrap_or(0);
        }
    }

    let body_start = header_end + 4; // skip \r\n\r\n
    let body = if body_start < raw.len() && content_length > 0 {
        let end = (body_start + content_length).min(raw.len());
        raw[body_start..end].to_vec()
    } else {
        Vec::new()
    };

    Ok(Request {
        method,
        path: path.to_string(),
        query,
        body,
        headers,
    })
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

fn handle_connection(mut stream: TcpStream, ui_dir: &Path, state: &ServerState) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

    let mut buf = vec![0_u8; 131072];
    let len = stream.read(&mut buf).context("failed to read request")?;
    if len == 0 {
        return Ok(());
    }

    let req = parse_request(&buf[..len])?;

    // API routes
    if req.path.starts_with(API_PREFIX) || req.path == EVENTS_PATH {
        let api_path = req.path.strip_prefix("/ray-exomem").unwrap_or(&req.path);
        return handle_api(&mut stream, &req, api_path, state);
    }

    // Static file serving — GET/HEAD only
    if req.method != "GET" && req.method != "HEAD" {
        return write_response(
            &mut stream, 405, "Method Not Allowed",
            "text/plain; charset=utf-8", b"method not allowed",
            req.method == "HEAD", None,
        );
    }

    if req.path == "/" {
        return write_redirect(&mut stream, "/ray-exomem/");
    }

    let rel = req.path
        .strip_prefix(UI_MOUNT_PATH)
        .unwrap_or(&req.path)
        .trim_start_matches('/');
    let file_path = resolve_asset_path(ui_dir, rel);

    let (status, reason, body_path) = match file_path {
        Some(p) if p.is_file() => (200, "OK", p),
        Some(p) if p.is_dir() => {
            let index = p.join("index.html");
            if index.is_file() { (200, "OK", index) }
            else { (404, "Not Found", ui_dir.join("index.html")) }
        }
        _ => {
            let index = ui_dir.join("index.html");
            if index.is_file() {
                (200, "OK", index)
            } else {
                return write_response(
                    &mut stream, 404, "Not Found",
                    "text/plain; charset=utf-8",
                    b"ray-exomem UI not built; run npm install && npm run build in ray-exomem/ui",
                    req.method == "HEAD", None,
                );
            }
        }
    };

    let body = if req.method == "HEAD" {
        Vec::new()
    } else {
        fs::read(&body_path)
            .with_context(|| format!("failed to read asset {}", body_path.display()))?
    };
    let content_type = content_type_for_path(&body_path);
    write_response(
        &mut stream, status, reason, content_type, &body,
        req.method == "HEAD", Some(("Cache-Control", "no-cache")),
    )
}

// ---------------------------------------------------------------------------
// API router
// ---------------------------------------------------------------------------

fn handle_api(stream: &mut TcpStream, req: &Request, api_path: &str, state: &ServerState) -> Result<()> {
    let exom = req
        .query
        .get("exom")
        .map(|s| s.as_str())
        .unwrap_or(DEFAULT_EXOM);

    let ctx = extract_mutation_context(req);

    // SSE owns the connection for the lifetime of the stream; never send a second HTTP response.
    if req.method == "GET" && api_path == "/events" {
        return handle_events_sse(stream);
    }

    let result = match (req.method.as_str(), api_path) {
        ("GET", "/api/status") => api_status(state, exom),
        ("GET", "/api/schema") => api_schema(state, exom, &req.query),
        ("GET", "/api/graph") => api_graph(state, exom, &req.query),
        ("GET", "/api/clusters") => api_clusters(state, exom),
        ("GET", "/api/logs") => api_logs(state, exom),
        ("GET", "/api/exoms") => api_exoms(state),
        ("GET", "/api/provenance") => api_provenance(state, exom),
        ("GET", "/api/relation-graph") => api_relation_graph(state, exom),
        ("GET", "/api/explain") => api_explain(state, &req.query),
        ("GET", "/api/actions/export") => api_export(state, exom),
        ("GET", "/api/actions/export-json") => api_export_json(state, exom),
        ("POST", "/api/actions/import-json") => api_import_json(state, exom, &req.body, &ctx),
        ("POST", "/api/actions/clear") => api_clear(state, exom, &ctx),
        ("POST", "/api/actions/retract") => api_retract(state, exom, &req.body, &ctx),
        ("POST", "/api/actions/evaluate") => api_evaluate(state),
        ("POST", "/api/actions/eval") => api_eval(state, &req.body, &ctx),
        ("POST", "/api/actions/import") => api_import(state, &req.body, &ctx),
        ("POST", "/api/actions/assert-fact") => api_assert_fact_direct(state, exom, &req.body, &ctx),
        ("GET", "/api/facts/valid-at") => api_facts_valid_at(state, exom, &req.query),
        ("GET", "/api/facts/bitemporal") => api_facts_bitemporal(state, exom, &req.query),
        ("POST", "/api/exoms") => api_exom_create(state, &req.body),
        _ => {
            if (req.method.as_str(), api_path) == ("GET", "/api/branches") {
                api_list_branches(state, exom)
            } else if (req.method.as_str(), api_path) == ("POST", "/api/branches") {
                api_create_branch(state, exom, &req.body, &ctx)
            } else if api_path.starts_with("/api/branches/") {
                api_branches_subpath(state, exom, api_path, req.method.as_str(), &req.body, &req.query, &ctx)
            } else if api_path.starts_with("/api/derived/") {
                let pred = api_path.strip_prefix("/api/derived/").unwrap_or("").trim_end_matches('/');
                api_derived(state, exom, pred)
            } else if api_path.starts_with("/api/facts/") {
                let id = api_path.strip_prefix("/api/facts/").unwrap_or("");
                api_fact_detail(state, id, exom)
            } else if api_path.starts_with("/api/clusters/") {
                let id = api_path.strip_prefix("/api/clusters/").unwrap_or("");
                api_cluster_detail(state, id, exom)
            } else if api_path.starts_with("/api/exoms/") && api_path.ends_with("/manage") {
                let name = api_path.strip_prefix("/api/exoms/").unwrap_or("");
                let name = name.strip_suffix("/manage").unwrap_or(name);
                api_exom_manage(state, name, &req.body)
            } else {
                Ok(json_response(404, r#"{"error":"not found"}"#))
            }
        }
    };

    let (status, body) = match result {
        Ok(resp) => resp,
        Err(err) => (500, serde_json::json!({"error": err.to_string()}).to_string()),
    };

    write_json_response(stream, status, &body)
}

type ApiResult = anyhow::Result<(u16, String)>;

fn json_response(status: u16, body: &str) -> (u16, String) {
    (status, body.to_string())
}

fn json_ok(value: &serde_json::Value) -> ApiResult {
    Ok((200, serde_json::to_string(value).unwrap_or_default()))
}

fn fact_to_json(f: &crate::brain::Fact) -> serde_json::Value {
    serde_json::json!({
        "fact_id": f.fact_id,
        "predicate": f.predicate,
        "value": f.value,
        "confidence": f.confidence,
        "valid_from": f.valid_from,
        "valid_to": f.valid_to,
        "created_by_tx": f.created_by_tx,
        "provenance": f.provenance
    })
}

fn get_exom_state<'a>(daemon: &'a DaemonState, exom: &str) -> Result<&'a ExomState> {
    daemon
        .exoms
        .get(exom)
        .ok_or_else(|| anyhow!("unknown exom '{}'", exom))
}

fn extract_mutation_context(req: &Request) -> MutationContext {
    MutationContext {
        actor: req
            .headers
            .get("x-actor")
            .map(|s| s.as_str())
            .unwrap_or("anonymous")
            .to_string(),
        session: req.headers.get("x-session").cloned(),
        model: req.headers.get("x-model").cloned(),
    }
}

fn extract_exom_from_query_source(source: &str) -> Result<String> {
    let t = source.trim();
    let after = t
        .strip_prefix("(query")
        .ok_or_else(|| anyhow!("expected (query"))?;
    let s = after.trim_start();
    let end = s
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(s.len());
    let name = s[..end].trim();
    if name.is_empty() {
        bail!("query missing database name");
    }
    Ok(name.to_string())
}

fn extract_exom_from_rule_source(source: &str) -> String {
    let t = source.trim();
    let Some(after) = t.strip_prefix("(rule") else {
        return DEFAULT_EXOM.to_string();
    };
    let s = after.trim_start();
    if s.starts_with('(') {
        return DEFAULT_EXOM.to_string();
    }
    let end = s
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(s.len());
    if end == 0 {
        return DEFAULT_EXOM.to_string();
    }
    s[..end].to_string()
}

#[derive(Deserialize)]
struct CreateBranchReq {
    branch_id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, Default)]
struct MergeBranchReq {
    #[serde(default)]
    policy: Option<String>,
}

fn api_list_branches(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;
    let branches: Vec<_> = es
        .brain
        .branches()
        .iter()
        .map(|b| {
            serde_json::json!({
                "branch_id": b.branch_id,
                "name": b.name,
                "parent_branch_id": b.parent_branch_id,
                "created_tx_id": b.created_tx_id,
                "archived": b.archived,
                "is_current": b.branch_id == es.brain.current_branch_id(),
                "fact_count": es.brain.facts_on_branch(&b.branch_id).len(),
            })
        })
        .collect();
    json_ok(&serde_json::json!({ "branches": branches }))
}

fn api_create_branch(state: &ServerState, exom: &str, body: &[u8], ctx: &MutationContext) -> ApiResult {
    let req: CreateBranchReq =
        serde_json::from_slice(body).map_err(|e| anyhow!("invalid JSON: {}", e))?;
    let name = req.name.unwrap_or_else(|| req.branch_id.clone());
    let mut daemon = get_daemon(state);
    let tx_id = {
        let ex = daemon
            .exoms
            .get_mut(exom)
            .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
        ex.brain.create_branch(&req.branch_id, &name, ctx)?
    };
    json_ok(&serde_json::json!({
        "branch_id": req.branch_id,
        "tx_id": tx_id
    }))
}

fn api_switch_branch(state: &ServerState, exom: &str, branch_id: &str, _ctx: &MutationContext) -> ApiResult {
    let mut daemon = get_daemon(state);
    {
        let ex = daemon
            .exoms
            .get_mut(exom)
            .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
        ex.brain.switch_branch(branch_id)?;
    }
    refresh_exom_binding(&mut daemon, state.exom_dir.as_ref(), exom)?;
    if let Err(err) = restore_runtime(&daemon) {
        if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
            eprintln!(
                "[ray-exomem] branch switch restore failed: {} / {}",
                err, e2
            );
        }
    }
    json_ok(&serde_json::json!({ "switched_to": branch_id }))
}

fn api_branch_diff(
    state: &ServerState,
    exom: &str,
    branch_id: &str,
    query: &HashMap<String, String>,
) -> ApiResult {
    let base = query.get("base").map(|s| s.as_str()).unwrap_or("main");
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;
    let branch_facts = es.brain.facts_on_branch(branch_id);
    let base_facts = es.brain.facts_on_branch(base);

    let base_map: HashMap<&str, &brain::Fact> = base_facts
        .iter()
        .map(|f| (f.fact_id.as_str(), *f))
        .collect();
    let branch_map: HashMap<&str, &brain::Fact> = branch_facts
        .iter()
        .map(|f| (f.fact_id.as_str(), *f))
        .collect();

    let added: Vec<_> = branch_facts
        .iter()
        .filter(|f| !base_map.contains_key(f.fact_id.as_str()))
        .map(|f| fact_to_json(f))
        .collect();
    let removed: Vec<_> = base_facts
        .iter()
        .filter(|f| !branch_map.contains_key(f.fact_id.as_str()))
        .map(|f| fact_to_json(f))
        .collect();
    let changed: Vec<_> = branch_facts
        .iter()
        .filter_map(|f| {
            base_map
                .get(f.fact_id.as_str())
                .filter(|base_f| base_f.value != f.value)
                .map(|base_f| {
                    serde_json::json!({
                        "fact_id": f.fact_id,
                        "predicate": f.predicate,
                        "base_value": base_f.value,
                        "branch_value": f.value,
                    })
                })
        })
        .collect();

    json_ok(&serde_json::json!({
        "added": added,
        "removed": removed,
        "changed": changed
    }))
}

fn api_merge_branch(
    state: &ServerState,
    exom: &str,
    source: &str,
    body: &[u8],
    ctx: &MutationContext,
) -> ApiResult {
    let payload: MergeBranchReq = serde_json::from_slice(body).unwrap_or_default();
    let policy = match payload.policy.as_deref().unwrap_or("last-writer-wins") {
        "last-writer-wins" => MergePolicy::LastWriterWins,
        "keep-target" => MergePolicy::KeepTarget,
        "manual" => MergePolicy::Manual,
        _ => bail!("unknown merge policy"),
    };
    let mut daemon = get_daemon(state);
    let target = get_exom_state(&daemon, exom)?
        .brain
        .current_branch_id()
        .to_string();
    let result = {
        let es = daemon
            .exoms
            .get_mut(exom)
            .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
        es.brain.merge_branch(source, &target, policy, ctx)?
    };
    refresh_exom_binding(&mut daemon, state.exom_dir.as_ref(), exom)?;
    if let Err(err) = restore_runtime(&daemon) {
        if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
            eprintln!("[ray-exomem] merge restore failed: {} / {}", err, e2);
        }
    }
    json_ok(&serde_json::json!({
        "added": result.added,
        "conflicts": result.conflicts.iter().map(|c| serde_json::json!({
            "fact_id": c.fact_id,
            "predicate": c.predicate,
            "source_value": c.source_value,
            "target_value": c.target_value,
        })).collect::<Vec<_>>(),
        "tx_id": result.tx_id,
    }))
}

fn api_delete_branch(state: &ServerState, exom: &str, branch_id: &str) -> ApiResult {
    let mut daemon = get_daemon(state);
    {
        let es = daemon
            .exoms
            .get_mut(exom)
            .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
        es.brain.archive_branch(branch_id)?;
    }
    refresh_exom_binding(&mut daemon, state.exom_dir.as_ref(), exom)?;
    if let Err(err) = restore_runtime(&daemon) {
        if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
            eprintln!(
                "[ray-exomem] archive branch restore failed: {} / {}",
                err, e2
            );
        }
    }
    json_ok(&serde_json::json!({ "archived": branch_id }))
}

fn api_branches_subpath(
    state: &ServerState,
    exom: &str,
    api_path: &str,
    method: &str,
    body: &[u8],
    query: &HashMap<String, String>,
    ctx: &MutationContext,
) -> ApiResult {
    let rest = api_path
        .strip_prefix("/api/branches/")
        .unwrap_or("")
        .trim_start_matches('/');
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Ok(json_response(404, r#"{"error":"not found"}"#));
    }
    let id = parts[0];
    if parts.len() == 2 && parts[1] == "switch" && method == "POST" {
        return api_switch_branch(state, exom, id, ctx);
    }
    if parts.len() == 2 && parts[1] == "diff" && method == "GET" {
        return api_branch_diff(state, exom, id, query);
    }
    if parts.len() == 2 && parts[1] == "merge" && method == "POST" {
        return api_merge_branch(state, exom, id, body, ctx);
    }
    if parts.len() == 1 && method == "DELETE" {
        return api_delete_branch(state, exom, id);
    }
    Ok(json_response(404, r#"{"error":"not found"}"#))
}

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

fn api_status(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let uptime = state.start_time.elapsed().as_secs();
    let facts = brain.current_facts();
    let beliefs = brain.current_beliefs();

    json_ok(&serde_json::json!({
        "ok": true,
        "exom": exom,
        "current_branch": brain.current_branch_id(),
        "server": {
            "name": "ray-exomem",
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_sec": uptime
        },
        "storage": {
            "exom_path": state.exom_dir.as_ref().map(|ed| ed.exom_path(exom).display().to_string()).unwrap_or_else(|| "in-memory".into())
        },
        "stats": {
            "relations": 3, // facts, observations, beliefs
            "facts": facts.len(),
            "derived_tuples": beliefs.len(),
            "intervals": facts.iter().filter(|f| f.valid_to.is_some()).count(),
            "directives": 0,
            "events_logged": brain.transactions().len()
        }
    }))
}

fn api_schema(state: &ServerState, exom: &str, query: &HashMap<String, String>) -> ApiResult {
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;
    let brain = &es.brain;
    let include_samples = query.get("include_samples").map(|v| v == "true").unwrap_or(false);
    let sample_limit: usize = query.get("sample_limit").and_then(|v| v.parse().ok()).unwrap_or(10);
    let filter_relation = query.get("relation").map(|s| s.as_str());

    let facts = brain.current_facts();
    let beliefs = brain.current_beliefs();
    let observations = brain.observations();

    let mut relations = Vec::new();

    // Group facts by predicate
    let mut fact_groups: HashMap<&str, Vec<Vec<serde_json::Value>>> = HashMap::new();
    let mut has_intervals_map: HashMap<&str, bool> = HashMap::new();
    for f in &facts {
        let entry = fact_groups.entry(&f.predicate).or_default();
        entry.push(vec![
            serde_json::Value::String(f.fact_id.clone()),
            serde_json::Value::String(f.predicate.clone()),
            serde_json::Value::String(f.value.clone()),
            serde_json::json!(f.confidence),
            serde_json::json!({
                "valid_from": f.valid_from,
                "valid_to": f.valid_to,
                "branch_origin": brain.tx_branch(f.created_by_tx).unwrap_or(""),
                "branch_role": brain.fact_branch_role(f, brain.current_branch_id()),
            }),
        ]);
        if f.valid_to.is_some() {
            has_intervals_map.insert(&f.predicate, true);
        }
    }

    for (pred, tuples) in &fact_groups {
        if let Some(filter) = filter_relation {
            if *pred != filter { continue; }
        }
        let has_intervals = has_intervals_map.get(pred).copied().unwrap_or(false);
        let mut rel = serde_json::json!({
            "name": pred,
            "arity": 5,
            "kind": "base",
            "cardinality": tuples.len(),
            "has_intervals": has_intervals,
            "defined_by": []
        });
        if include_samples {
            let samples: Vec<_> = tuples.iter().take(sample_limit).cloned().collect();
            rel["sample_tuples"] = serde_json::json!(samples);
        }
        relations.push(rel);
    }

    // Observations as a relation
    if filter_relation.is_none() || filter_relation == Some("observation") {
        let obs_tuples: Vec<Vec<serde_json::Value>> = observations.iter().map(|o| {
            vec![
                serde_json::Value::String(o.obs_id.clone()),
                serde_json::Value::String(o.source_type.clone()),
                serde_json::Value::String(o.content.clone()),
                serde_json::json!(o.confidence),
            ]
        }).collect();
        let mut rel = serde_json::json!({
            "name": "observation",
            "arity": 4,
            "kind": "base",
            "cardinality": obs_tuples.len(),
            "has_intervals": false,
            "defined_by": []
        });
        if include_samples && !obs_tuples.is_empty() {
            let samples: Vec<_> = obs_tuples.into_iter().take(sample_limit).collect();
            rel["sample_tuples"] = serde_json::json!(samples);
        }
        relations.push(rel);
    }

    // Beliefs as a derived relation
    if filter_relation.is_none() || filter_relation == Some("belief") {
        let has_belief_intervals = beliefs.iter().any(|b| b.valid_to.is_some());
        let belief_tuples: Vec<Vec<serde_json::Value>> = beliefs.iter().map(|b| {
            vec![
                serde_json::Value::String(b.belief_id.clone()),
                serde_json::Value::String(b.claim_text.clone()),
                serde_json::json!(b.confidence),
                serde_json::Value::String(b.status.to_string()),
                serde_json::json!({
                    "valid_from": b.valid_from,
                    "valid_to": b.valid_to,
                }),
            ]
        }).collect();
        let mut rel = serde_json::json!({
            "name": "belief",
            "arity": 5,
            "kind": "derived",
            "cardinality": belief_tuples.len(),
            "has_intervals": has_belief_intervals,
            "defined_by": ["belief-revision"]
        });
        if include_samples && !belief_tuples.is_empty() {
            let samples: Vec<_> = belief_tuples.into_iter().take(sample_limit).collect();
            rel["sample_tuples"] = serde_json::json!(samples);
        }
        relations.push(rel);
    }

    let mut base_names: std::collections::HashSet<String> =
        fact_groups.keys().map(|s| (*s).to_string()).collect();
    base_names.insert("observation".into());
    base_names.insert("belief".into());

    let derived_preds = rules::derived_predicates(&es.rules);
    for (pred_name, arity) in derived_preds {
        if base_names.contains(&pred_name) {
            continue;
        }
        if let Some(filter) = filter_relation {
            if filter != pred_name.as_str() {
                continue;
            }
        }
        let defined_by_rules: Vec<usize> = es
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.head_predicate == pred_name)
            .map(|(i, _)| i)
            .collect();
        relations.push(serde_json::json!({
            "name": pred_name,
            "arity": arity,
            "kind": "derived",
            "cardinality": serde_json::Value::Null,
            "has_intervals": false,
            "defined_by": defined_by_rules,
        }));
    }

    let base_count = relations.iter().filter(|r| r["kind"] == "base").count();
    let derived_count = relations.iter().filter(|r| r["kind"] == "derived").count();
    let largest = relations
        .iter()
        .max_by_key(|r| r["cardinality"].as_u64().unwrap_or(0))
        .map(|r| {
            serde_json::json!({
                "name": r["name"],
                "cardinality": r["cardinality"]
            })
        });

    json_ok(&serde_json::json!({
        "relations": relations,
        "directives": [],
        "summary": {
            "relation_count": relations.len(),
            "base_relation_count": base_count,
            "derived_relation_count": derived_count,
            "largest_relation": largest
        }
    }))
}

fn api_graph(state: &ServerState, exom: &str, query: &HashMap<String, String>) -> ApiResult {
    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let limit: usize = query.get("limit").and_then(|v| v.parse().ok()).unwrap_or(500);
    let facts = brain.current_facts();

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_nodes = std::collections::HashSet::new();

    for (i, f) in facts.iter().take(limit).enumerate() {
        let entity_id = &f.fact_id;
        let pred_id = format!("pred:{}", f.predicate);

        if seen_nodes.insert(entity_id.clone()) {
            nodes.push(serde_json::json!({
                "id": entity_id,
                "type": "entity",
                "label": format!("{} = {}", f.predicate, f.value),
                "degree": 1
            }));
        }
        if seen_nodes.insert(pred_id.clone()) {
            nodes.push(serde_json::json!({
                "id": pred_id,
                "type": "entity",
                "label": f.predicate,
                "degree": 1
            }));
        }

        edges.push(serde_json::json!({
            "id": format!("e{}", i),
            "type": "fact",
            "source": entity_id,
            "target": pred_id,
            "label": f.value
        }));
    }

    json_ok(&serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "clusters": [],
        "summary": {
            "node_count": nodes.len(),
            "edge_count": edges.len(),
            "cluster_count": 0
        }
    }))
}

fn api_clusters(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let facts = brain.current_facts();

    // Cluster by predicate
    let mut groups: HashMap<&str, usize> = HashMap::new();
    for f in &facts {
        *groups.entry(&f.predicate).or_default() += 1;
    }

    let clusters: Vec<_> = groups.iter().map(|(pred, count)| {
        serde_json::json!({
            "id": format!("cluster:{}", pred),
            "label": pred,
            "kind": "shared_predicate",
            "fact_count": count,
            "active_count": count,
            "deprecated_count": 0
        })
    }).collect();

    json_ok(&serde_json::json!({ "clusters": clusters }))
}

fn api_cluster_detail(state: &ServerState, id: &str, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let pred = id.strip_prefix("cluster:").unwrap_or(id);
    let facts = brain.current_facts();
    let matching: Vec<_> = facts.iter().filter(|f| f.predicate == pred).collect();

    let nodes: Vec<_> = matching.iter().map(|f| {
        serde_json::json!({
            "id": f.fact_id,
            "type": "fact",
            "label": format!("{} = {}", f.predicate, f.value)
        })
    }).collect();

    let fact_entries: Vec<_> = matching.iter().map(|f| {
        serde_json::json!({
            "id": f.fact_id,
            "tuple": [f.fact_id, f.predicate, f.value, f.confidence],
            "status": "active",
            "interval": {
                "start": f.valid_from,
                "end": f.valid_to.as_deref().unwrap_or("inf")
            }
        })
    }).collect();

    json_ok(&serde_json::json!({
        "id": id,
        "label": pred,
        "kind": "shared_predicate",
        "stats": {
            "fact_count": matching.len(),
            "active_count": matching.len(),
            "deprecated_count": 0
        },
        "nodes": nodes,
        "facts": fact_entries,
        "related_clusters": []
    }))
}

fn api_logs(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let txs = brain.transactions();

    let events: Vec<_> = txs.iter().rev().take(24).map(|tx| {
        serde_json::json!({
            "id": format!("tx{}", tx.tx_id),
            "type": tx.action.to_string(),
            "timestamp": tx.tx_time,
            "pattern": tx.note,
            "source": tx.actor
        })
    }).collect();

    json_ok(&serde_json::json!({ "events": events }))
}

fn api_exoms(state: &ServerState) -> ApiResult {
    let daemon = get_daemon(state);
    let exom_list: Vec<_> = daemon.exoms.keys().map(|name| {
        serde_json::json!({
            "name": name,
            "description": if name == DEFAULT_EXOM { "Default exom" } else { "" },
            "created_at": 0,
            "updated_at": 0,
            "archived": false,
            "archived_at": null
        })
    }).collect();

    json_ok(&serde_json::json!({
        "exoms": exom_list
    }))
}

fn api_provenance(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;
    let brain = &es.brain;
    let facts = brain.current_facts();
    let derived_n = rules::derived_predicates(&es.rules).len();

    let base_facts: Vec<_> = facts.iter().map(|f| {
        serde_json::json!({
            "id": f.fact_id,
            "predicate": f.predicate,
            "terms": [f.fact_id, f.predicate, f.value],
            "kind": "base",
            "source": f.provenance,
            "confidence": f.confidence,
            "asserted_at": f.created_by_tx
        })
    }).collect();

    json_ok(&serde_json::json!({
        "derivations": [],
        "base_facts": base_facts,
        "edges": [],
        "timeline": [],
        "summary": {
            "derived_count": derived_n,
            "base_count": base_facts.len(),
            "edge_count": 0,
            "event_count": 0
        }
    }))
}

fn api_relation_graph(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let facts = brain.current_facts();

    let mut preds: HashMap<&str, usize> = HashMap::new();
    for f in &facts {
        *preds.entry(&f.predicate).or_default() += 1;
    }

    let nodes: Vec<_> = preds.iter().map(|(pred, count)| {
        serde_json::json!({
            "id": *pred,
            "label": *pred,
            "degree": count
        })
    }).collect();

    json_ok(&serde_json::json!({
        "nodes": nodes,
        "edges": [],
        "summary": {
            "node_count": nodes.len(),
            "edge_count": 0
        }
    }))
}

fn api_derived(state: &ServerState, exom: &str, pred_name: &str) -> ApiResult {
    if pred_name.is_empty() {
        return Ok(json_response(400, r#"{"error":"missing predicate"}"#));
    }
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;
    let arity = es
        .rules
        .iter()
        .find(|r| r.head_predicate == pred_name)
        .map(|r| r.head_arity)
        .ok_or_else(|| anyhow!("unknown derived predicate"))?;
    let find_vars: Vec<String> = (0..arity).map(|i| format!("?v{i}")).collect();
    let find_vars_str = find_vars.join(" ");
    let bodies: Vec<String> = es.rules.iter().map(|r| r.inline_body.clone()).collect();
    let rules_clause = bodies.join(" ");
    let rayfall = format!(
        "(query {exom} (find {find_vars_str}) (where ({pred_name} {find_vars_str})) (rules {rules_clause}))"
    );
    let output = match daemon.engine.eval(&rayfall) {
        Ok(o) => o,
        Err(e) => {
            return Ok(json_response(
                400,
                &serde_json::json!({ "error": e.to_string() }).to_string(),
            ));
        }
    };
    json_ok(&serde_json::json!({
        "predicate": pred_name,
        "kind": "derived",
        "arity": arity,
        "rows": output,
    }))
}

fn api_explain(state: &ServerState, query: &HashMap<String, String>) -> ApiResult {
    let exom = query.get("exom").map(|s| s.as_str()).unwrap_or(DEFAULT_EXOM);
    let predicate = query.get("predicate").map(|s| s.as_str()).unwrap_or("");
    let _terms_str = query.get("terms").map(|s| s.as_str()).unwrap_or("");
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;
    let brain = &es.brain;
    let facts = brain.current_facts();

    let defining_rules: Vec<&str> = es
        .rules
        .iter()
        .filter(|r| r.head_predicate == predicate)
        .map(|r| r.full_text.as_str())
        .collect();
    if !defining_rules.is_empty() {
        return json_ok(&serde_json::json!({
            "kind": "derived",
            "predicate": predicate,
            "derived_by_rules": defining_rules,
        }));
    }

    // Find matching fact
    let matching = facts.iter().find(|f| {
        f.predicate == predicate || f.fact_id == predicate
    });

    match matching {
        Some(f) => json_ok(&serde_json::json!({
            "predicate": f.predicate,
            "terms": [f.fact_id, f.predicate, f.value],
            "tree": {
                "id": f.fact_id,
                "predicate": f.predicate,
                "terms": [f.fact_id, f.predicate, f.value],
                "kind": "base",
                "source": f.provenance,
                "confidence": f.confidence,
                "asserted_at": f.created_by_tx
            },
            "meta": {
                "source": f.provenance,
                "confidence": f.confidence,
                "asserted_at": f.created_by_tx
            }
        })),
        None => Ok(json_response(404, &serde_json::json!({
            "error": format!("no fact matching predicate '{}'", predicate)
        }).to_string())),
    }
}

fn api_fact_detail(state: &ServerState, id: &str, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let history = brain.fact_history(id);

    match history.last() {
        Some(f) => {
            let status = if f.revoked_by_tx.is_some() { "retracted" } else { "active" };
            let touch_history: Vec<_> = brain.explain(id).iter().map(|tx| {
                serde_json::json!({
                    "event_id": format!("tx{}", tx.tx_id),
                    "event_type": tx.action.to_string()
                })
            }).collect();

            json_ok(&serde_json::json!({
                "fact": {
                    "id": f.fact_id,
                    "predicate": f.predicate,
                    "tuple": [f.fact_id, f.predicate, f.value, f.confidence],
                    "interval": {
                        "start": f.valid_from,
                        "end": f.valid_to.as_deref().unwrap_or("inf")
                    },
                    "status": status,
                    "cluster_ids": [format!("cluster:{}", f.predicate)]
                },
                "provenance": { "type": "base" },
                "touch_history": touch_history
            }))
        }
        None => Ok(json_response(404, r#"{"error":"fact not found"}"#)),
    }
}

fn api_export(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;
    let facts = es.brain.current_facts();

    let mut out = String::new();
    out.push_str(&format!(";; ray-exomem export — exom: {}\n", exom));
    for f in &facts {
        out.push_str(&format!(
            "(assert-fact {} \"{}\" '{} \"{}\")",
            exom,
            f.fact_id.replace('"', "\\\""),
            f.predicate.replace('"', "\\\""),
            f.value.replace('"', "\\\""),
        ));
        // Append bitemporal validity annotation
        let valid_to_str = f.valid_to.as_deref().unwrap_or("inf");
        out.push_str(&format!(" ;; @valid[{}, {}]", f.valid_from, valid_to_str));
        out.push('\n');
    }
    for rule in &es.rules {
        out.push_str(&rule.full_text);
        out.push('\n');
    }

    Ok((200, out))
}

/// Lossless JSON export — all entity types with full metadata.
fn api_export_json(state: &ServerState, exom: &str) -> ApiResult {
    let daemon = get_daemon(state);
    let es = get_exom_state(&daemon, exom)?;

    let payload = serde_json::json!({
        "exom": exom,
        "version": 1,
        "facts": es.brain.all_facts(),
        "transactions": es.brain.transactions(),
        "observations": es.brain.observations(),
        "beliefs": es.brain.all_beliefs(),
        "branches": es.brain.branches(),
        "rules": es.rules.iter().map(|r| &r.full_text).collect::<Vec<_>>(),
    });

    Ok((200, serde_json::to_string_pretty(&payload).unwrap()))
}

/// Lossless JSON import — replaces all data in the exom.
fn api_import_json(
    state: &ServerState,
    exom: &str,
    body: &[u8],
    _ctx: &MutationContext,
) -> ApiResult {
    use crate::brain::{Belief, Branch, Fact, Observation, Tx};

    #[derive(Deserialize)]
    struct ImportPayload {
        facts: Vec<Fact>,
        transactions: Vec<Tx>,
        #[serde(default)]
        observations: Vec<Observation>,
        #[serde(default)]
        beliefs: Vec<Belief>,
        #[serde(default)]
        branches: Vec<Branch>,
        #[serde(default)]
        rules: Vec<String>,
    }

    let payload: ImportPayload = serde_json::from_slice(body)
        .map_err(|e| anyhow!("invalid JSON import payload: {}", e))?;

    let mut daemon = get_daemon(state);

    // Mutate exom state in a block so the mutable borrow is released
    let (n_facts, n_txs) = {
        let es = daemon
            .exoms
            .get_mut(exom)
            .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;

        // Replace brain state wholesale
        es.brain.replace_state(
            payload.facts,
            payload.transactions,
            payload.observations,
            payload.beliefs,
            payload.branches,
        )?;

        // Replace rules
        let mut parsed_rules = Vec::new();
        for line in &payload.rules {
            let line = line.trim();
            if !line.is_empty() {
                parsed_rules.push(rules::parse_rule_line(
                    line,
                    MutationContext::default(),
                    String::new(),
                )?);
            }
        }
        es.rules = parsed_rules;

        // Rebuild datoms and persist
        es.datoms = storage::build_datoms_table(&es.brain.current_facts())?;
        persist_exom_state(state.exom_dir.as_ref(), exom, es)?;

        (es.brain.all_facts().len(), es.brain.transactions().len())
    };

    // Re-bind runtime (needs immutable borrow)
    if let Err(e) = restore_runtime(&daemon) {
        if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
            eprintln!(
                "[ray-exomem] fatal: restore after import failed (first: {}, recover: {})",
                e, e2
            );
            std::process::exit(1);
        }
    }

    json_ok(&serde_json::json!({
        "ok": true,
        "imported": {
            "facts": n_facts,
            "transactions": n_txs,
        }
    }))
}

fn api_clear(state: &ServerState, exom: &str, ctx: &MutationContext) -> ApiResult {
    let mut daemon = get_daemon(state);
    let fact_ids: Vec<String> = {
        let es = get_exom_state(&daemon, exom)?;
        es.brain.current_facts().iter().map(|f| f.fact_id.clone()).collect()
    };
    let count = fact_ids.len();
    for id in &fact_ids {
        if let Some(es) = daemon.exoms.get_mut(exom) {
            let _ = es.brain.retract_fact(id, ctx);
        }
    }
    if let Some(es) = daemon.exoms.get_mut(exom) {
        es.rules.clear();
    }
    if let Some(ed) = state.exom_dir.as_ref() {
        save_rules(Some(ed), exom, &[])?;
    }
    refresh_exom_binding(&mut daemon, state.exom_dir.as_ref(), exom)?;
    if let Err(err) = restore_runtime(&daemon) {
        if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
            eprintln!(
                "[ray-exomem] fatal: restore after clear failed (first: {}, recover: {})",
                err, e2
            );
            std::process::exit(1);
        }
    }
    json_ok(&serde_json::json!({
        "ok": true,
        "tuples_removed": count
    }))
}

fn api_retract(state: &ServerState, exom: &str, body: &[u8], ctx: &MutationContext) -> ApiResult {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .unwrap_or(serde_json::json!({}));
    let predicate = payload["predicate"].as_str().unwrap_or("").to_string();

    let mut daemon = get_daemon(state);
    let matching_ids: Vec<String> = {
        let es = get_exom_state(&daemon, exom)?;
        es.brain.current_facts().iter()
            .filter(|f| f.predicate == predicate)
            .map(|f| f.fact_id.clone())
            .collect()
    };
    let mut retracted = 0;
    for id in &matching_ids {
        if let Some(es) = daemon.exoms.get_mut(exom) {
            if es.brain.retract_fact(id, ctx).is_ok() {
                retracted += 1;
            }
        }
    }
    if retracted > 0 {
        refresh_exom_binding(&mut daemon, state.exom_dir.as_ref(), exom)?;
    }
    json_ok(&serde_json::json!({
        "ok": true,
        "retracted": retracted
    }))
}

fn api_evaluate(_state: &ServerState) -> ApiResult {
    // In the native brain model, evaluation is immediate — no separate step needed.
    json_ok(&serde_json::json!({
        "ok": true,
        "new_derivations": 0,
        "duration_ms": 0
    }))
}

fn api_eval(state: &ServerState, body: &[u8], ctx: &MutationContext) -> ApiResult {
    let source = String::from_utf8_lossy(body).into_owned();
    let exom_dir = state.exom_dir.as_ref();
    let forms = rayfall_parser::split_forms(&source);
    let mut last_result = String::new();
    let mut daemon = get_daemon(state);
    for form in forms {
        match form.kind {
            FormKind::AssertFact => {
                let (exom, entity, pred, val) =
                    rayfall_parser::parse_fact_mutation_args(&form.inner_source)?;
                {
                    let es = daemon
                        .exoms
                        .get_mut(&exom)
                        .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
                    es.brain.assert_fact(
                        &entity,
                        &pred,
                        &val,
                        1.0,
                        "rayfall-eval",
                        None,
                        None,
                        ctx,
                    )?;
                }
                refresh_exom_binding(&mut daemon, exom_dir, &exom)?;
            }
            FormKind::RetractFact => {
                let (exom, entity, pred, val) =
                    rayfall_parser::parse_fact_mutation_args(&form.inner_source)?;
                {
                    let es = daemon
                        .exoms
                        .get_mut(&exom)
                        .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
                    es.brain.retract_fact_exact(&entity, &pred, &val, ctx)?;
                }
                refresh_exom_binding(&mut daemon, exom_dir, &exom)?;
            }
            FormKind::Rule => {
                let exom_name = extract_exom_from_rule_source(&form.source);
                let normalized = form.source.replace('[', "(").replace(']', ")");
                let full = ensure_rule_has_exom(&exom_name, &normalized);
                let pr = rules::parse_rule_line(&full, ctx.clone(), brain::now_iso())?;
                {
                    let es = daemon
                        .exoms
                        .get_mut(&exom_name)
                        .ok_or_else(|| anyhow!("unknown exom '{}'", exom_name))?;
                    es.rules.push(pr);
                }
                let es = daemon
                    .exoms
                    .get(&exom_name)
                    .ok_or_else(|| anyhow!("unknown exom '{}'", exom_name))?;
                persist_exom_state(exom_dir, &exom_name, es)?;
            }
            FormKind::Query => {
                let exom_name = extract_exom_from_query_source(&form.source)?;
                let bodies: Vec<String> = {
                    let es = daemon
                        .exoms
                        .get(&exom_name)
                        .ok_or_else(|| anyhow!("unknown exom '{}'", exom_name))?;
                    es.rules.iter().map(|r| r.inline_body.clone()).collect()
                };
                let q = rayfall_parser::rewrite_query_with_rules(&form.source, &bodies);
                match daemon.engine.eval(&q) {
                    Ok(out) => last_result = out,
                    Err(err) => {
                        if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
                            eprintln!(
                                "[ray-exomem] fatal: restore after eval error (first: {}, recover: {})",
                                err, e2
                            );
                            std::process::exit(1);
                        }
                        return Ok(json_response(
                            400,
                            &serde_json::json!({ "error": err.to_string() }).to_string(),
                        ));
                    }
                }
            }
            FormKind::Other => match daemon.engine.eval(&form.source) {
                Ok(out) => last_result = out,
                Err(err) => {
                    if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
                        eprintln!(
                            "[ray-exomem] fatal: restore after eval error (first: {}, recover: {})",
                            err, e2
                        );
                        std::process::exit(1);
                    }
                    return Ok(json_response(
                        400,
                        &serde_json::json!({ "error": err.to_string() }).to_string(),
                    ));
                }
            },
        }
    }
    json_ok(&serde_json::json!({
        "ok": true,
        "output": last_result,
        "mutated_exom": serde_json::Value::Null,
        "mutation_count": 0
    }))
}

fn api_import(state: &ServerState, body: &[u8], ctx: &MutationContext) -> ApiResult {
    api_eval(state, body, ctx)
}

fn api_assert_fact_direct(state: &ServerState, exom: &str, body: &[u8], ctx: &MutationContext) -> ApiResult {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .unwrap_or(serde_json::json!({}));
    let predicate = payload["predicate"].as_str().unwrap_or("").to_string();
    let value = payload["value"].as_str().unwrap_or("").to_string();
    let fact_id = payload["fact_id"].as_str().unwrap_or(&predicate).to_string();
    let confidence: f64 = payload["confidence"].as_f64().unwrap_or(1.0);
    let provenance = payload["provenance"].as_str().unwrap_or("api").to_string();
    let valid_from = payload["valid_from"].as_str().map(|s| s.to_string());
    let valid_to = payload["valid_to"].as_str().map(|s| s.to_string());

    if predicate.is_empty() || value.is_empty() {
        return Ok(json_response(400, r#"{"error":"predicate and value are required"}"#));
    }

    let mut daemon = get_daemon(state);
    {
        let es = daemon.exoms.get_mut(exom)
            .ok_or_else(|| anyhow!("unknown exom '{}'", exom))?;
        es.brain.assert_fact(
            &fact_id, &predicate, &value, confidence, &provenance,
            valid_from.as_deref(), valid_to.as_deref(),
            ctx,
        )?;
    }
    refresh_exom_binding(&mut daemon, state.exom_dir.as_ref(), exom)?;

    json_ok(&serde_json::json!({
        "ok": true,
        "fact_id": fact_id,
        "valid_from": valid_from,
        "valid_to": valid_to
    }))
}

fn api_facts_valid_at(state: &ServerState, exom: &str, query: &HashMap<String, String>) -> ApiResult {
    let timestamp = query.get("timestamp").map(|s| s.as_str()).unwrap_or("");
    if timestamp.is_empty() {
        return Ok(json_response(400, r#"{"error":"timestamp query parameter is required"}"#));
    }

    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let entries: Vec<_> = brain.facts_valid_at(timestamp).iter().map(|f| fact_to_json(f)).collect();

    json_ok(&serde_json::json!({
        "ok": true,
        "timestamp": timestamp,
        "facts": entries,
        "count": entries.len()
    }))
}

fn api_facts_bitemporal(state: &ServerState, exom: &str, query: &HashMap<String, String>) -> ApiResult {
    let timestamp = query.get("timestamp").map(|s| s.as_str()).unwrap_or("");
    let tx_id: u64 = query.get("tx_id").and_then(|v| v.parse().ok()).unwrap_or(0);

    if timestamp.is_empty() || tx_id == 0 {
        return Ok(json_response(400, r#"{"error":"timestamp and tx_id query parameters are required"}"#));
    }

    let daemon = get_daemon(state);
    let brain = &get_exom_state(&daemon, exom)?.brain;
    let entries: Vec<_> = brain.facts_bitemporal(tx_id, timestamp).iter().map(|f| fact_to_json(f)).collect();

    json_ok(&serde_json::json!({
        "ok": true,
        "timestamp": timestamp,
        "tx_id": tx_id,
        "facts": entries,
        "count": entries.len()
    }))
}

fn api_exom_create(state: &ServerState, body: &[u8]) -> ApiResult {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .unwrap_or(serde_json::json!({}));
    let name = payload["name"].as_str().unwrap_or("new").to_string();

    let mut daemon = get_daemon(state);
    if daemon.exoms.contains_key(&name) {
        return Ok(json_response(409, &serde_json::json!({
            "error": format!("exom '{}' already exists", name)
        }).to_string()));
    }

    let exom_state = if let Some(ref ed) = state.exom_dir {
        ed.create_exom(&name).with_context(|| format!("failed to create exom '{}'", name))?;
        load_exom_state(Some(ed), &name)?
    } else {
        load_exom_state(None, &name)?
    };
    daemon.exoms.insert(name.clone(), exom_state);
    if let Err(e) = restore_runtime(&daemon) {
        if let Err(e2) = daemon.engine.reconcile_lang_env() {
            eprintln!(
                "[ray-exomem] fatal: reconcile env after failed exom create restore: {}",
                e2
            );
            std::process::exit(1);
        }
        daemon.exoms.remove(&name);
        if let Some(ref ed) = state.exom_dir {
            let _ = ed.delete_exom(&name);
        }
        if let Err(e2) = restore_runtime(&daemon) {
            eprintln!(
                "[ray-exomem] fatal: re-restore after rolling back exom create: {}",
                e2
            );
            std::process::exit(1);
        }
        return Ok(json_response(
            500,
            &serde_json::json!({"error": format!("restore_runtime: {}", e)}).to_string(),
        ));
    }

    json_ok(&serde_json::json!({
        "ok": true,
        "name": name
    }))
}

fn api_exom_manage(state: &ServerState, name: &str, body: &[u8]) -> ApiResult {
    let payload: serde_json::Value = serde_json::from_slice(body)
        .unwrap_or(serde_json::json!({}));
    let action = payload["action"].as_str().unwrap_or("").to_string();

    let mut daemon = get_daemon(state);

    match action.as_str() {
        "delete" => {
            if name == DEFAULT_EXOM {
                return Ok(json_response(400, r#"{"error":"cannot delete the default exom"}"#));
            }
            if daemon.exoms.remove(name).is_some() {
                if let Some(ref ed) = state.exom_dir {
                    let _ = ed.delete_exom(name);
                }
                if let Err(e) = restore_runtime(&daemon) {
                    if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
                        eprintln!(
                            "[ray-exomem] fatal: restore after exom delete (first: {}, recover: {})",
                            e, e2
                        );
                        std::process::exit(1);
                    }
                }
                json_ok(&serde_json::json!({ "ok": true, "deleted": name }))
            } else {
                Ok(json_response(404, &serde_json::json!({
                    "error": format!("exom '{}' not found", name)
                }).to_string()))
            }
        }
        "rename" => {
            let new_name = payload["new_name"].as_str().unwrap_or("").to_string();
            if new_name.is_empty() {
                return Ok(json_response(400, r#"{"error":"new_name is required"}"#));
            }
            if name == DEFAULT_EXOM {
                return Ok(json_response(400, r#"{"error":"cannot rename the default exom"}"#));
            }
            if daemon.exoms.contains_key(new_name.as_str()) {
                return Ok(json_response(409, &serde_json::json!({
                    "error": format!("exom '{}' already exists", new_name)
                }).to_string()));
            }
            if let Some(exom_state) = daemon.exoms.remove(name) {
                daemon.exoms.insert(new_name.clone(), exom_state);
                if let Some(ref ed) = state.exom_dir {
                    let _ = ed.rename_exom(name, &new_name);
                }
                if let Err(e) = restore_runtime(&daemon) {
                    if let Err(e2) = reconcile_runtime_after_failed_restore(&daemon) {
                        eprintln!(
                            "[ray-exomem] fatal: restore after exom rename (first: {}, recover: {})",
                            e, e2
                        );
                        std::process::exit(1);
                    }
                }
                json_ok(&serde_json::json!({ "ok": true, "old_name": name, "new_name": new_name }))
            } else {
                Ok(json_response(404, &serde_json::json!({
                    "error": format!("exom '{}' not found", name)
                }).to_string()))
            }
        }
        "archive" => {
            if daemon.exoms.contains_key(name) {
                json_ok(&serde_json::json!({ "ok": true, "archived": name }))
            } else {
                Ok(json_response(404, &serde_json::json!({
                    "error": format!("exom '{}' not found", name)
                }).to_string()))
            }
        }
        _ => Ok(json_response(400, &serde_json::json!({
            "error": format!("unknown action '{}'", action)
        }).to_string())),
    }
}

fn handle_events_sse(stream: &mut TcpStream) -> Result<()> {
    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
    stream.write_all(headers.as_bytes())?;
    stream.write_all(b": connected\n\n")?;
    stream.flush()?;

    loop {
        thread::sleep(Duration::from_secs(15));
        if stream.write_all(b": heartbeat\n\n").is_err() {
            break;
        }
        if stream.flush().is_err() {
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP response helpers
// ---------------------------------------------------------------------------

fn write_json_response(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    write_response(
        stream, status, reason,
        "application/json",
        body.as_bytes(),
        false,
        Some(("Access-Control-Allow-Origin", "*")),
    )
}

fn resolve_asset_path(ui_dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Some(ui_dir.to_path_buf());
    }

    let mut clean = PathBuf::new();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => continue,
            _ => return None,
        }
    }

    Some(ui_dir.join(clean))
}

fn write_redirect(stream: &mut TcpStream, location: &str) -> Result<()> {
    let body = format!("<html><body>Moved to <a href=\"{location}\">{location}</a></body></html>");
    write_response(
        stream, 302, "Found",
        "text/html; charset=utf-8",
        body.as_bytes(),
        false,
        Some(("Location", location)),
    )
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    extra_header: Option<(&str, &str)>,
) -> Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some((key, value)) = extra_header {
        response.push_str(&format!("{key}: {value}\r\n"));
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes())?;
    if !head_only {
        stream.write_all(body)?;
    }
    Ok(())
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_asset_path_rejects_parent_escape() {
        let root = Path::new("/tmp/ui");
        assert!(resolve_asset_path(root, "../secret").is_none());
    }

    #[test]
    fn resolve_asset_path_maps_nested_asset() {
        let root = Path::new("/tmp/ui");
        let resolved = resolve_asset_path(root, "build/_app/immutable/app.js").unwrap();
        assert_eq!(resolved, Path::new("/tmp/ui/build/_app/immutable/app.js"));
    }
}
