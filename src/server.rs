use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, StatusCode, Uri},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json, Router,
};
use futures::StreamExt;
use include_dir::{include_dir, Dir};
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::{Any, CorsLayer};

// ---------------------------------------------------------------------------
// Embedded UI
// ---------------------------------------------------------------------------

static EMBEDDED_UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/ui/build");

// ---------------------------------------------------------------------------
// Public constants (previously in web.rs)
// ---------------------------------------------------------------------------

pub const UI_MOUNT_PATH: &str = "/ray-exomem";
pub const API_PREFIX: &str = "/ray-exomem/api/";
pub const EVENTS_PATH: &str = "/ray-exomem/events";
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:9780";
pub const DEFAULT_EXOM: &str = "main";

use crate::{
    backend::RayforceEngine,
    brain::{self, Brain, MergePolicy},
    context::{self, MutationContext},
    ffi,
    http_error::ApiError,
    rayfall_ast::{self, CanonicalForm, CanonicalQuery, LoweringOptions},
    rayfall_parser,
    rules::{self, ParsedRule},
    storage::{self, RayObj},
    system_schema,
};

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

pub struct ExomState {
    pub brain: Brain,
    pub datoms: RayObj,
    pub rules: Vec<ParsedRule>,
    pub exom_disk: Option<PathBuf>,
}

pub struct AppState {
    pub exoms: Mutex<HashMap<String, ExomState>>,
    pub engine: crate::backend::RayforceEngine,
    pub tree_root: Option<PathBuf>,
    pub sym_path: Option<PathBuf>,
    pub start_time: Instant,
    pub sse_tx: broadcast::Sender<String>,
}

impl AppState {
    pub fn new(
        engine: RayforceEngine,
        exoms: HashMap<String, ExomState>,
        tree_root: Option<PathBuf>,
        sym_path: Option<PathBuf>,
    ) -> Self {
        let (sse_tx, _) = broadcast::channel(512);
        Self {
            exoms: Mutex::new(exoms),
            engine,
            tree_root,
            sym_path,
            start_time: Instant::now(),
            sse_tx,
        }
    }

    /// Build AppState by loading a data directory (same logic as web::serve).
    pub fn from_data_dir(data_dir: Option<PathBuf>) -> anyhow::Result<Arc<Self>> {
        use crate::exom::ExomDir;

        let engine = RayforceEngine::new()?;
        let mut exoms: HashMap<String, ExomState> = HashMap::new();
        let (tree_root, sym_path) = match data_dir {
            Some(ref root) => {
                let ed = ExomDir::open(root.clone())?;
                let sym = ed.sym_path();
                let tree_dir = root.join("tree");
                if tree_dir.exists() {
                    load_tree_exoms_into(&tree_dir, &sym, &mut exoms);
                }
                // Load legacy flat exoms.
                for name in ed.list_exoms()? {
                    if exoms.contains_key(&name) {
                        continue;
                    }
                    eprintln!("[ray-exomem] loading flat exom '{}'", name);
                    if let Ok(es) = load_flat_exom(&ed, &name) {
                        exoms.insert(name, es);
                    }
                }
                if exoms.is_empty() {
                    ed.create_exom(DEFAULT_EXOM)?;
                    if let Ok(es) = load_flat_exom(&ed, DEFAULT_EXOM) {
                        exoms.insert(DEFAULT_EXOM.to_string(), es);
                    }
                }
                let tree_root = root.join("tree");
                std::fs::create_dir_all(&tree_root).ok();
                (Some(tree_root), Some(sym))
            }
            None => {
                exoms.insert(DEFAULT_EXOM.to_string(), ExomState {
                    brain: Brain::new(),
                    datoms: storage::build_datoms_table(&Brain::new())?,
                    rules: Vec::new(),
                    exom_disk: None,
                });
                (None, None)
            }
        };

        // Bind all exoms into the engine.
        for (name, es) in &exoms {
            engine.bind_named_db(storage::sym_intern(name), &es.datoms)?;
        }

        Ok(Arc::new(AppState::new(engine, exoms, tree_root, sym_path)))
    }
}

fn load_tree_exoms_into(
    tree_root: &std::path::Path,
    sym_path: &std::path::Path,
    out: &mut HashMap<String, ExomState>,
) {
    fn walk(
        tree_root: &std::path::Path,
        current: &std::path::Path,
        sym_path: &std::path::Path,
        out: &mut HashMap<String, ExomState>,
    ) {
        let Ok(rd) = std::fs::read_dir(current) else { return; };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue; };
            if !ft.is_dir() { continue; }
            let disk = entry.path();
            let rel = disk.strip_prefix(tree_root).unwrap_or(&disk);
            let slash_key = rel.components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>().join("/");
            if slash_key.is_empty() { continue; }
            let meta_p = disk.join(crate::exom::META_FILENAME);
            if meta_p.exists() {
                eprintln!("[ray-exomem] loading tree exom '{}'", slash_key);
                match load_exom_from_tree_path_inner(&disk, sym_path, &slash_key) {
                    Ok(es) => { out.insert(slash_key.clone(), es); }
                    Err(e) => { eprintln!("[ray-exomem] WARNING: failed to load '{}': {}", slash_key, e); }
                }
                continue;
            }
            walk(tree_root, &disk, sym_path, out);
        }
    }
    walk(tree_root, tree_root, sym_path, out);
}

fn load_exom_from_tree_path_inner(
    exom_disk: &std::path::Path,
    sym_path: &std::path::Path,
    slash_key: &str,
) -> anyhow::Result<ExomState> {
    load_exom_from_tree_path(exom_disk, sym_path, slash_key)
}

fn load_flat_exom(ed: &crate::exom::ExomDir, name: &str) -> anyhow::Result<ExomState> {
    let brain = {
        let b = Brain::open_exom(&ed.exom_path(name), &ed.sym_path())?;
        b.ensure_jsonl_sidecars()?;
        b
    };
    let datoms = storage::build_datoms_table(&brain)?;
    let rules_p = ed.exom_path(name).join("rules.ray");
    let rules = if rules_p.exists() {
        let src = std::fs::read_to_string(&rules_p)?;
        let mut out = Vec::new();
        for line in src.lines().map(str::trim).filter(|l| !l.is_empty()) {
            out.push(crate::rules::parse_rule_line(
                line,
                context::MutationContext::default(),
                String::new(),
            )?);
        }
        out
    } else {
        Vec::new()
    };
    Ok(ExomState { brain, datoms, rules, exom_disk: None })
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(api_status))
        .route("/tree", get(api_tree))
        .route("/guide", get(api_guide))
        .route("/exoms", get(api_exoms_gone).post(api_exoms_gone))
        .route("/actions/init", post(api_init))
        .route("/actions/exom-new", post(api_exom_new))
        .route("/actions/session-new", post(api_session_new))
        .route("/actions/session-join", post(api_session_join))
        .route("/actions/branch-create", post(api_branch_create))
        .route("/actions/rename", post(api_rename))
        .route("/actions/assert-fact", post(api_assert_fact))
        // Query
        .route("/query", get(api_query_get).post(api_query_post))
        .route("/expand-query", post(api_expand_query))
        // Eval
        .route("/actions/eval", post(api_eval))
        .route("/actions/evaluate", post(api_evaluate_noop))
        // Facts
        .route("/facts", get(api_facts_list))
        .route("/facts/valid-at", get(api_facts_valid_at))
        .route("/facts/bitemporal", get(api_facts_bitemporal))
        .route("/facts/{id}", get(api_fact_detail))
        // Branches
        .route("/branches", get(api_list_branches).post(api_create_branch))
        .route("/branches/{id}", get(api_branch_detail).delete(api_delete_branch_handler))
        .route("/branches/{id}/switch", post(api_switch_branch_handler))
        .route("/branches/{id}/diff", get(api_branch_diff_handler))
        .route("/branches/{id}/merge", post(api_merge_branch_handler))
        // Explain
        .route("/explain", get(api_explain))
        // Export / Import
        .route("/actions/export", get(api_export))
        .route("/actions/export-json", get(api_export_json))
        .route("/actions/import-json", post(api_import_json))
        // Mutations
        .route("/actions/retract-all", post(api_retract_all))
        .route("/actions/wipe", post(api_wipe))
        .route("/actions/factory-reset", post(api_factory_reset))
        .route("/actions/consolidate-propose", post(api_consolidate_propose))
        // Schema / graph / clusters / logs / provenance / relation-graph
        .route("/schema", get(api_schema))
        .route("/graph", get(api_graph))
        .route("/clusters", get(api_clusters))
        .route("/clusters/{id}", get(api_cluster_detail_handler))
        .route("/logs", get(api_logs))
        .route("/provenance", get(api_provenance))
        .route("/relation-graph", get(api_relation_graph))
        // Derived / beliefs
        .route("/derived/{pred}", get(api_derived_handler))
        .route("/beliefs/{id}/support", get(api_belief_support_handler))
        // Exom manage (legacy)
        .route("/exoms/{name}/manage", post(api_exom_manage_handler))
        // Old start-session compat → 410
        .route("/actions/start-session", post(api_start_session_gone))
}

// ---------------------------------------------------------------------------
// SSE endpoint
// ---------------------------------------------------------------------------

async fn api_sse(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.sse_tx.subscribe();
    let stream = BroadcastStream::new(rx)
        .filter_map(|r| async { r.ok() })
        .map(|data| Ok(Event::default().data(data)));
    Sse::new(stream)
}

// ---------------------------------------------------------------------------
// SPA fallback
// ---------------------------------------------------------------------------

fn content_type_for_ext(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn spa_fallback(uri: Uri) -> impl IntoResponse {
    let raw = uri.path().trim_start_matches('/');
    let path = raw.strip_prefix("ray-exomem/").unwrap_or(raw);
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = EMBEDDED_UI.get_file(path) {
        let ct = content_type_for_ext(path);
        return (StatusCode::OK, [(header::CONTENT_TYPE, ct)], file.contents()).into_response();
    }

    // SPA client-side routing fallback
    if let Some(index) = EMBEDDED_UI.get_file("index.html") {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            index.contents(),
        )
            .into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

pub async fn serve(bind: &str, state: Arc<AppState>) -> anyhow::Result<()> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Nested under /ray-exomem/api
        .nest("/ray-exomem/api", api_router())
        // SSE event stream
        .route("/sse", get(api_sse))
        // Compat shim: smoke test calls /api/status
        .route("/api/status", get(api_status))
        .fallback(spa_fallback)
        .with_state(state)
        .layer(cors);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn server_tree_root(state: &AppState) -> PathBuf {
    state
        .tree_root
        .clone()
        .unwrap_or_else(crate::storage::tree_root)
}

/// Lazy-load an exom by slash key, inserting into the map if found on disk.
fn get_or_load_exom<'a>(
    exoms: &'a mut HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    slash_key: &str,
    tree_root: Option<&std::path::Path>,
    sym_path: Option<&std::path::Path>,
) -> anyhow::Result<&'a mut ExomState> {
    if exoms.contains_key(slash_key) {
        return Ok(exoms.get_mut(slash_key).unwrap());
    }
    if let (Some(tr), Some(sp)) = (tree_root, sym_path) {
        let disk = tr.join(slash_key);
        let meta_p = disk.join(crate::exom::META_FILENAME);
        if meta_p.exists() {
            let es = load_exom_from_tree_path(&disk, sp, slash_key)?;
            engine.bind_named_db(storage::sym_intern(slash_key), &es.datoms)?;
            exoms.insert(slash_key.to_string(), es);
            return Ok(exoms.get_mut(slash_key).unwrap());
        }
    }
    Err(anyhow::anyhow!("unknown exom '{}'", slash_key))
}

fn load_exom_from_tree_path(
    exom_disk: &std::path::Path,
    sym_path: &std::path::Path,
    slash_key: &str,
) -> anyhow::Result<ExomState> {
    let mut brain = {
        let b = Brain::open_exom(exom_disk, sym_path)?;
        b.ensure_jsonl_sidecars()?;
        b
    };
    let meta_p = exom_disk.join(crate::exom::META_FILENAME);
    if meta_p.exists() {
        if let Ok(meta) = crate::exom::read_meta(exom_disk) {
            if brain
                .branches()
                .iter()
                .any(|b| b.branch_id == meta.current_branch && !b.archived)
            {
                let _ = brain.switch_branch(&meta.current_branch);
            }
        }
    }
    let datoms = storage::build_datoms_table(&brain)?;
    let rules_p = exom_disk.join("rules.ray");
    let rules = if rules_p.exists() {
        let src = std::fs::read_to_string(&rules_p)?;
        let mut out = Vec::new();
        for line in src.lines().map(str::trim).filter(|l| !l.is_empty()) {
            out.push(crate::rules::parse_rule_line(
                line,
                context::MutationContext::default(),
                String::new(),
            )?);
        }
        out
    } else {
        Vec::new()
    };
    let schema_p = exom_disk.join(system_schema::SCHEMA_FILENAME);
    let ontology = system_schema::build_exom_ontology(slash_key, &brain, &rules);
    let _ = system_schema::save_exom_ontology(&schema_p, &ontology);
    Ok(ExomState {
        brain,
        datoms,
        rules,
        exom_disk: Some(exom_disk.to_path_buf()),
    })
}

fn combined_rules(exom: &str, user_rules: &[ParsedRule]) -> anyhow::Result<Vec<ParsedRule>> {
    let mut rules = system_schema::builtin_rules(exom)?;
    rules.extend_from_slice(user_rules);
    Ok(rules)
}

fn refresh_exom_binding(state: &AppState, exoms: &mut HashMap<String, ExomState>, exom_name: &str) -> anyhow::Result<()> {
    let es = exoms
        .get_mut(exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
    es.datoms = storage::build_datoms_table(&es.brain)?;
    state
        .engine
        .bind_named_db(storage::sym_intern(exom_name), &es.datoms)?;
    if let Some(disk) = es.exom_disk.as_ref() {
        es.brain.save()?;
        let rules_p = disk.join("rules.ray");
        let body: String = es
            .rules
            .iter()
            .map(|r| r.full_text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !body.is_empty() {
            std::fs::write(&rules_p, format!("{}\n", body))?;
        }
    }
    Ok(())
}

fn mutate_exom<T>(
    state: &AppState,
    exom_name: &str,
    f: impl FnOnce(&mut ExomState) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    if !exoms.contains_key(exom_name) {
        let _ = get_or_load_exom(&mut exoms, &state.engine, exom_name, tree_root, sym_path);
    }
    let result = {
        let es = exoms
            .get_mut(exom_name)
            .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
        f(es)
    };
    let out = result?;
    refresh_exom_binding(state, &mut exoms, exom_name)?;
    let _ = state.sse_tx.send(format!(r#"{{"kind":"memory","exom":"{}"}}"#, exom_name));
    Ok(out)
}

fn emit_tree_changed(state: &AppState) {
    let _ = state
        .sse_tx
        .send(r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string());
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn api_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let exom = DEFAULT_EXOM;
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root_val = state.tree_root.as_deref();
    let sym_path_val = state.sym_path.as_deref();
    if !exoms.contains_key(exom) {
        let _ = get_or_load_exom(&mut exoms, &state.engine, exom, tree_root_val, sym_path_val);
    }
    let Some(es) = exoms.get(exom) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "exom not found"})),
        );
    };
    let brain = &es.brain;
    let uptime = state.start_time.elapsed().as_secs();
    let facts = brain.current_facts();
    let beliefs = brain.current_beliefs();
    let all_rules = match combined_rules(exom, &es.rules) {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))),
    };
    let derived_names: Vec<String> = rules::derived_predicates(&all_rules)
        .into_iter()
        .map(|(n, _)| n)
        .take(24)
        .collect();
    let ontology = system_schema::build_exom_ontology(exom, brain, &es.rules);
    let tree_root_display = server_tree_root(&state).display().to_string();
    let status = serde_json::json!({
        "ok": true,
        "exom": exom,
        "current_branch": brain.current_branch_id(),
        "server": {
            "name": "ray-exomem",
            "version": crate::frontend_version(),
            "uptime_sec": uptime,
            "tree_root": tree_root_display,
            "build": {
                "git_sha": crate::build_git_sha(),
                "built_unix": crate::build_unix_timestamp(),
                "identity": crate::build_identity(),
            }
        },
        "storage": {
            "exom_path": "in-memory"
        },
        "stats": {
            "relations": 3,
            "facts": facts.len(),
            "derived_tuples": beliefs.len(),
            "intervals": facts.iter().filter(|f| f.valid_to.is_some()).count(),
            "directives": 0,
            "events_logged": brain.transactions().len(),
            "sym_entries": storage::sym_count(),
            "rules": {
                "count": es.rules.len(),
                "derived_predicates": derived_names,
            }
        },
        "schema": {
            "path": serde_json::Value::Null,
            "system_attribute_count": ontology.system_attributes.len(),
            "coordination_attribute_count": ontology.coordination_attributes.len(),
            "builtin_view_count": ontology.builtin_views.len(),
            "user_predicates": ontology.user_predicates,
        }
    });
    (StatusCode::OK, Json(status))
}

#[derive(Deserialize, Default)]
struct TreeQuery {
    path: Option<String>,
    depth: Option<usize>,
    archived: Option<String>,
    branches: Option<String>,
    activity: Option<String>,
}

async fn api_tree(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TreeQuery>,
) -> impl IntoResponse {
    let tree_root = server_tree_root(&state);
    let opts = crate::tree::WalkOptions {
        depth: q.depth.or(Some(usize::MAX)),
        include_archived: q.archived.as_deref() == Some("true"),
        include_branches: q.branches.as_deref() == Some("true"),
        include_activity: q.activity.as_deref() == Some("true"),
    };
    let result = match q.path.as_deref().filter(|s| !s.is_empty()) {
        None => crate::tree::walk_root(&tree_root, &opts),
        Some(p) => match p.parse::<crate::path::TreePath>() {
            Ok(tp) => crate::tree::walk(&tree_root, &tp, &opts),
            Err(e) => {
                let err = ApiError::new("bad_path", e.to_string());
                return err.into_response();
            }
        },
    };
    match result {
        Ok(node) => Json(node).into_response(),
        Err(e) => ApiError::new("io", e.to_string()).into_response(),
    }
}

async fn api_guide() -> impl IntoResponse {
    let body = crate::agent_guide::doctrine();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        body,
    )
}

async fn api_exoms_gone() -> impl IntoResponse {
    ApiError::new("gone", "/api/exoms is removed; use /api/tree instead")
        .with_status(410)
        .into_response()
}

#[derive(Deserialize)]
struct PathBody {
    path: Option<String>,
}

async fn api_init(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    let tree_root = server_tree_root(&state);
    match crate::scaffold::init_project(&tree_root, &path) {
        Ok(()) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({"ok": true, "path": path.to_slash_string()})).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

async fn api_exom_new(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    let tree_root = server_tree_root(&state);
    match crate::scaffold::new_bare_exom(&tree_root, &path) {
        Ok(()) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({"ok": true, "path": path.to_slash_string()})).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct SessionNewBody {
    project_path: Option<String>,
    #[serde(rename = "type")]
    session_type: Option<String>,
    label: Option<String>,
    actor: Option<String>,
    #[serde(default)]
    agents: Vec<String>,
}

async fn api_session_new(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionNewBody>,
) -> impl IntoResponse {
    let project_path_str = body.project_path.unwrap_or_default();
    let project_path: crate::path::TreePath = match project_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    let session_type = match body.session_type.as_deref().unwrap_or("") {
        "multi" => crate::exom::SessionType::Multi,
        "single" => crate::exom::SessionType::Single,
        other => {
            return ApiError::new(
                "bad_session_type",
                format!("unknown session type {:?}; use 'multi' or 'single'", other),
            )
            .into_response();
        }
    };
    let label = body.label.unwrap_or_default();
    let actor = body.actor.unwrap_or_default();
    let tree_root = server_tree_root(&state);
    match brain::session_new(&tree_root, &project_path, session_type, &label, &actor, &body.agents) {
        Ok(session_path) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({
                "ok": true,
                "session_path": session_path.to_slash_string(),
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct SessionJoinBody {
    session_path: Option<String>,
    actor: Option<String>,
}

async fn api_session_join(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionJoinBody>,
) -> impl IntoResponse {
    let session_path_str = body.session_path.unwrap_or_default();
    let actor = body.actor.unwrap_or_default();
    let session_path: crate::path::TreePath = match session_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if actor.is_empty() {
        return ApiError::new("actor_required", "actor required")
            .with_suggestion("pass actor in request body")
            .into_response();
    }
    let tree_root = server_tree_root(&state);
    match brain::session_join(&tree_root, &session_path, &actor) {
        Ok(branch) => Json(serde_json::json!({
            "ok": true,
            "session_path": session_path.to_slash_string(),
            "actor": actor,
            "branch": branch,
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct BranchCreateBody {
    exom_path: Option<String>,
    branch_name: Option<String>,
    actor: Option<String>,
}

async fn api_branch_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BranchCreateBody>,
) -> impl IntoResponse {
    let exom_path_str = body.exom_path.unwrap_or_default();
    let branch_name = body.branch_name.unwrap_or_default();
    let actor = body.actor.unwrap_or_default();
    let exom_path: crate::path::TreePath = match exom_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if branch_name.is_empty() {
        return ApiError::new("branch_name_required", "branch_name required").into_response();
    }
    if actor.is_empty() {
        return ApiError::new("actor_required", "actor required")
            .with_suggestion("pass actor in request body")
            .into_response();
    }
    let tree_root = server_tree_root(&state);
    let disk = exom_path.to_disk_path(&tree_root);
    let meta = match crate::exom::read_meta(&disk) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::new("no_such_exom", format!("no such exom {exom_path_str}"))
                .with_path(exom_path_str)
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    if let Some(sess) = &meta.session {
        if sess.initiated_by != actor {
            return ApiError::new(
                "not_orchestrator",
                format!(
                    "only the session orchestrator ({}) may create branches",
                    sess.initiated_by
                ),
            )
            .with_actor(actor)
            .into_response();
        }
    }
    match brain::create_branch(&tree_root, &exom_path, &branch_name) {
        Ok(()) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({
                "ok": true,
                "exom_path": exom_path.to_slash_string(),
                "branch_name": branch_name,
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct RenameBody {
    path: Option<String>,
    new_segment: Option<String>,
}

async fn api_rename(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let new_segment = body.new_segment.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    let tree_root = server_tree_root(&state);
    let disk = path.to_disk_path(&tree_root);
    if crate::tree::classify(&disk) == crate::tree::NodeKind::Exom {
        if let Ok(meta) = crate::exom::read_meta(&disk) {
            if meta.kind == crate::exom::ExomKind::Session {
                return ApiError::new(
                    "session_id_immutable",
                    "cannot rename session id; use session/label to change the display label",
                )
                .into_response();
            }
        }
    }
    match crate::tree::rename_last_segment(&tree_root, &path, &new_segment) {
        Ok(new_path) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({"ok": true, "new_path": new_path.to_slash_string()}))
                .into_response()
        }
        Err(e) => ApiError::new("rename_failed", e).into_response(),
    }
}

#[derive(Deserialize)]
struct AssertFactBody {
    #[serde(default)]
    exom: Option<String>,
    #[serde(default)]
    actor: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    fact_id: Option<String>,
    predicate: String,
    value: String,
    #[serde(default = "default_confidence")]
    confidence: f64,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    provenance: Option<String>,
    #[serde(default)]
    valid_from: Option<String>,
    #[serde(default)]
    valid_to: Option<String>,
}

fn default_confidence() -> f64 {
    1.0
}

async fn api_assert_fact(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AssertFactBody>,
) -> impl IntoResponse {
    let exom_raw = req.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    let actor_str = match req.actor.as_deref().filter(|s| !s.is_empty()) {
        Some(a) => a.to_string(),
        None => {
            return ApiError::from(brain::WriteError::ActorRequired).into_response();
        }
    };

    let branch_str = req.branch.as_deref().unwrap_or("main");

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, branch_str, &actor_str) {
            return ApiError::from(e).into_response();
        }
    }

    let write_ctx = MutationContext {
        actor: actor_str.clone(),
        session: None,
        model: None,
    };

    let fact_id = req.fact_id.clone().unwrap_or_else(|| req.predicate.clone());
    let provenance = req
        .source
        .as_deref()
        .or(req.provenance.as_deref())
        .unwrap_or("api")
        .to_string();
    let predicate = req.predicate.clone();
    let value = req.value.clone();
    let confidence = req.confidence;
    let valid_from = req.valid_from.clone();
    let valid_to = req.valid_to.clone();

    let result = mutate_exom(&state, &exom_slash, |ex| {
        ex.brain.assert_fact(
            &fact_id,
            &predicate,
            &value,
            confidence,
            &provenance,
            valid_from.as_deref(),
            valid_to.as_deref(),
            &write_ctx,
        )
    });

    match result {
        Ok(tx_id) => {
            if state.tree_root.is_some()
                && matches!(
                    predicate.as_str(),
                    "session/label" | "session/closed_at" | "session/archived_at"
                )
            {
                let tree_root = server_tree_root(&state);
                if let Err(e) = brain::mirror_session_meta_to_disk(
                    &tree_root,
                    &exom_path,
                    &predicate,
                    &value,
                ) {
                    eprintln!(
                        "[ray-exomem] mirror_session_meta_to_disk failed (best-effort): {}",
                        e
                    );
                }
            }
            Json(serde_json::json!({
                "ok": true,
                "fact_id": fact_id,
                "tx_id": tx_id
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Query helpers (shared by query + eval + expand-query handlers)
// ---------------------------------------------------------------------------

struct ExpandedQuery {
    original_source: String,
    normalized_query: String,
    expanded_query: String,
    #[allow(dead_code)]
    exom_name: String,
}

fn lower_query_request(
    source: &str,
    default_exom: Option<&str>,
    surface: &str,
) -> anyhow::Result<CanonicalQuery> {
    let forms = rayfall_ast::parse_forms(source)?;
    if forms.len() != 1 {
        anyhow::bail!("{surface} expects exactly one top-level Rayfall query form");
    }
    let lowered = rayfall_ast::lower_top_level(
        &forms[0],
        LoweringOptions {
            default_query_exom: default_exom,
            default_rule_exom: Some(DEFAULT_EXOM),
        },
    )?;
    match lowered.as_slice() {
        [CanonicalForm::Query(query)] => Ok(query.clone()),
        [CanonicalForm::AssertFact(_) | CanonicalForm::RetractFact(_) | CanonicalForm::Rule(_)] => {
            anyhow::bail!("{surface} only accepts a Rayfall (query ...) form")
        }
        [] => anyhow::bail!("{surface} expects exactly one top-level Rayfall query form"),
        _ => anyhow::bail!("{surface} accepts exactly one logical query form"),
    }
}

enum EvalForm {
    Canonical(CanonicalForm),
    Raw(String),
}

fn lower_eval_forms(source: &str) -> anyhow::Result<Vec<EvalForm>> {
    let forms = rayfall_ast::parse_forms(source)?;
    let mut out = Vec::new();
    for form in forms {
        match form
            .as_list()
            .and_then(|items| items.first())
            .and_then(|item| item.as_symbol())
        {
            Some("assert-fact" | "retract-fact" | "rule" | "query" | "in-exom") => {
                let lowered = rayfall_ast::lower_top_level(
                    &form,
                    LoweringOptions {
                        default_query_exom: None,
                        default_rule_exom: Some(DEFAULT_EXOM),
                    },
                )?;
                out.extend(lowered.into_iter().map(EvalForm::Canonical));
            }
            _ => out.push(EvalForm::Raw(form.emit())),
        }
    }
    Ok(out)
}

fn expand_canonical_query(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    original_source: String,
    query: &CanonicalQuery,
) -> anyhow::Result<ExpandedQuery> {
    let exom_name = query.exom.clone();
    let rule_inline_bodies: Vec<String> = {
        let es = exoms
            .get(&exom_name)
            .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
        combined_rules(&exom_name, &es.rules)?
            .into_iter()
            .map(|rule| rule.inline_body)
            .collect()
    };
    let normalized_query = query.emit();
    let expanded_query =
        rayfall_parser::rewrite_query_with_rules(&normalized_query, &rule_inline_bodies)?;
    let _ = engine; // engine is accessed separately for eval
    Ok(ExpandedQuery {
        original_source,
        normalized_query,
        expanded_query,
        exom_name,
    })
}

fn expand_query(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    source: &str,
    default_exom: Option<&str>,
    surface: &str,
) -> anyhow::Result<ExpandedQuery> {
    let query = lower_query_request(source, default_exom, surface)?;
    expand_canonical_query(exoms, engine, source.to_string(), &query)
}

fn eval_query_form(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    query: &CanonicalQuery,
) -> anyhow::Result<(String, Option<serde_json::Value>)> {
    let expanded = expand_canonical_query(exoms, engine, query.emit(), query)?;
    let raw = engine.eval_raw(&expanded.expanded_query)?;
    if unsafe { ffi::ray_obj_type(raw.as_ptr()) } == ffi::RAY_TABLE {
        let decoded = storage::decode_query_table(&raw, &expanded.normalized_query)?;
        Ok((storage::format_decoded_query_table(&decoded), Some(decoded)))
    } else {
        Ok((engine.format_obj(&raw)?, None))
    }
}

fn reconcile_engine(state: &AppState, exoms: &HashMap<String, ExomState>) {
    if let Err(e) = state.engine.reconcile_lang_env() {
        eprintln!("[ray-exomem] reconcile_lang_env failed: {}", e);
        return;
    }
    for (name, es) in exoms {
        let _ = state.engine.bind_named_db(storage::sym_intern(name), &es.datoms);
    }
}

// ---------------------------------------------------------------------------
// GET/POST /query
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct QueryParams {
    exom: Option<String>,
}

async fn api_query_get(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QueryParams>,
) -> impl IntoResponse {
    // GET with body is non-standard; just return an error directing to POST
    let _ = params;
    let _ = state;
    ApiError::new("use_post", "Use POST /api/query with a Rayfall query in the request body")
        .with_status(405)
        .into_response()
}

async fn api_query_post(
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let source = String::from_utf8_lossy(&body).into_owned();
    let mut exoms = state.exoms.lock().unwrap();
    let expanded = match expand_query(&exoms, &state.engine, &source, None, "api/query") {
        Ok(e) => e,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    };
    match state.engine.eval_raw(&expanded.expanded_query) {
        Ok(raw) => {
            let (output, decoded) = if unsafe { ffi::ray_obj_type(raw.as_ptr()) } == ffi::RAY_TABLE {
                match storage::decode_query_table(&raw, &expanded.normalized_query) {
                    Ok(d) => (storage::format_decoded_query_table(&d), Some(d)),
                    Err(e) => {
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": e.to_string()}))).into_response();
                    }
                }
            } else {
                match state.engine.format_obj(&raw) {
                    Ok(s) => (s, None),
                    Err(e) => {
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": e.to_string()}))).into_response();
                    }
                }
            };
            let mut payload = serde_json::json!({
                "ok": true,
                "output": output,
                "mutated_exom": serde_json::Value::Null,
                "mutation_count": 0,
                "normalized_query": expanded.normalized_query,
                "expanded_query": expanded.expanded_query
            });
            if let Some(decoded) = decoded {
                if let (Some(dst), Some(src)) = (payload.as_object_mut(), decoded.as_object()) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            }
            Json(payload).into_response()
        }
        Err(err) => {
            reconcile_engine(&state, &exoms);
            drop(exoms);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// POST /expand-query
// ---------------------------------------------------------------------------

async fn api_expand_query(
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let source = String::from_utf8_lossy(&body).into_owned();
    let exoms = state.exoms.lock().unwrap();
    match expand_query(&exoms, &state.engine, &source, None, "api/expand-query") {
        Ok(expanded) => Json(serde_json::json!({
            "ok": true,
            "original_source": expanded.original_source,
            "normalized_query": expanded.normalized_query,
            "expanded_query": expanded.expanded_query,
            "exom": expanded.exom_name,
        }))
        .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/eval
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct EvalActorHeader {
    // actor may come from x-actor header or body — we allow both
}

async fn api_eval(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let actor = headers
        .get("x-actor")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();
    let ctx = MutationContext {
        actor: actor.clone(),
        session: headers.get("x-session").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
        model: headers.get("x-model").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
    };
    api_eval_inner(state, ctx, body).await
}

async fn api_eval_inner(
    state: Arc<AppState>,
    ctx: MutationContext,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let source = String::from_utf8_lossy(&body).into_owned();
    let forms = match lower_eval_forms(&source) {
        Ok(forms) => forms,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    };
    let mut last_result = String::new();
    let mut last_decoded: Option<serde_json::Value> = None;
    let mut exoms = state.exoms.lock().unwrap();

    for form in forms {
        let exec: anyhow::Result<()> = match form {
            EvalForm::Canonical(CanonicalForm::AssertFact(mutation)) => {
                let exom = mutation.exom.clone();
                let pred = mutation.predicate.clone();
                let fact_id = mutation.fact_id.clone();
                let value = mutation.value.clone();
                (|| {
                    let es = exoms
                        .get_mut(&exom)
                        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom))?;
                    es.brain.assert_fact(&fact_id, &pred, &value, 1.0, "rayfall-eval", None, None, &ctx)?;
                    es.datoms = storage::build_datoms_table(&es.brain)?;
                    state.engine.bind_named_db(storage::sym_intern(&exom), &es.datoms)?;
                    if let Some(disk) = es.exom_disk.as_ref() {
                        es.brain.save()?;
                        let body_str: String = es.rules.iter().map(|r| r.full_text.as_str()).collect::<Vec<_>>().join("\n");
                        if !body_str.is_empty() {
                            std::fs::write(disk.join("rules.ray"), format!("{}\n", body_str))?;
                        }
                    }
                    let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"eval_assert_fact","exom":"{}","actor":"{}","predicate":"{}"}}"#, exom, ctx.actor, pred));
                    Ok(())
                })()
            }
            EvalForm::Canonical(CanonicalForm::RetractFact(mutation)) => {
                let exom = mutation.exom.clone();
                let pred = mutation.predicate.clone();
                let fact_id = mutation.fact_id.clone();
                let value = mutation.value.clone();
                (|| {
                    let es = exoms
                        .get_mut(&exom)
                        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom))?;
                    es.brain.retract_fact_exact(&fact_id, &pred, &value, &ctx)?;
                    es.datoms = storage::build_datoms_table(&es.brain)?;
                    state.engine.bind_named_db(storage::sym_intern(&exom), &es.datoms)?;
                    if let Some(disk) = es.exom_disk.as_ref() {
                        es.brain.save()?;
                    }
                    let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"eval_retract_fact","exom":"{}","actor":"{}","predicate":"{}"}}"#, exom, ctx.actor, pred));
                    Ok(())
                })()
            }
            EvalForm::Canonical(CanonicalForm::Rule(rule)) => {
                let full = rule.emit();
                let exom_name = rule.exom.clone();
                (|| {
                    let pr = rules::parse_rule_line(&full, ctx.clone(), brain::now_iso())?;
                    let es = exoms
                        .get_mut(&exom_name)
                        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
                    es.rules.push(pr);
                    es.datoms = storage::build_datoms_table(&es.brain)?;
                    state.engine.bind_named_db(storage::sym_intern(&exom_name), &es.datoms)?;
                    if let Some(disk) = es.exom_disk.as_ref() {
                        es.brain.save()?;
                        let body_str: String = es.rules.iter().map(|r| r.full_text.as_str()).collect::<Vec<_>>().join("\n");
                        std::fs::write(disk.join("rules.ray"), format!("{}\n", body_str))?;
                    }
                    let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"rule_append","exom":"{}","actor":"{}"}}"#, exom_name, ctx.actor));
                    Ok(())
                })()
            }
            EvalForm::Canonical(CanonicalForm::Query(query)) => {
                (|| {
                    let (output, decoded) = eval_query_form(&exoms, &state.engine, &query)?;
                    last_result = output;
                    last_decoded = decoded;
                    Ok(())
                })()
            }
            EvalForm::Raw(source) => {
                (|| {
                    last_result = state.engine.eval(&source)?;
                    last_decoded = None;
                    Ok(())
                })()
            }
        };

        if let Err(err) = exec {
            reconcile_engine(&state, &exoms);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    }

    let mut payload = serde_json::json!({
        "ok": true,
        "output": last_result,
        "mutated_exom": serde_json::Value::Null,
        "mutation_count": 0
    });
    if let Some(decoded) = last_decoded {
        if let (Some(dst), Some(src)) = (payload.as_object_mut(), decoded.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
    Json(payload).into_response()
}

// ---------------------------------------------------------------------------
// POST /actions/evaluate (noop)
// ---------------------------------------------------------------------------

async fn api_evaluate_noop() -> impl IntoResponse {
    Json(serde_json::json!({"ok": true, "new_derivations": 0, "duration_ms": 0}))
}

// ---------------------------------------------------------------------------
// POST /actions/consolidate-propose (501)
// ---------------------------------------------------------------------------

async fn api_consolidate_propose() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"ok": false, "error": "consolidation propose API is not implemented yet"})),
    )
}

// ---------------------------------------------------------------------------
// GET /facts/valid-at
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExomQuery {
    exom: Option<String>,
}

#[derive(Deserialize)]
struct ValidAtQuery {
    exom: Option<String>,
    timestamp: Option<String>,
}

async fn api_facts_valid_at(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ValidAtQuery>,
) -> impl IntoResponse {
    let timestamp = match q.timestamp.as_deref().filter(|s| !s.is_empty()) {
        Some(t) => t.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "timestamp query parameter is required"})),
            )
                .into_response();
        }
    };
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let entries: Vec<_> = es
        .brain
        .facts_valid_at(&timestamp)
        .iter()
        .map(|f| fact_to_json(f))
        .collect();
    Json(serde_json::json!({
        "ok": true,
        "timestamp": timestamp,
        "facts": entries,
        "count": entries.len()
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /facts/bitemporal
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BitemporalQuery {
    exom: Option<String>,
    timestamp: Option<String>,
    tx_id: Option<u64>,
}

async fn api_facts_bitemporal(
    State(state): State<Arc<AppState>>,
    Query(q): Query<BitemporalQuery>,
) -> impl IntoResponse {
    let timestamp = q.timestamp.as_deref().unwrap_or("");
    let tx_id = q.tx_id.unwrap_or(0);
    if timestamp.is_empty() || tx_id == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "timestamp and tx_id query parameters are required"})),
        )
            .into_response();
    }
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let entries: Vec<_> = es
        .brain
        .facts_bitemporal(tx_id, timestamp)
        .iter()
        .map(|f| fact_to_json(f))
        .collect();
    Json(serde_json::json!({
        "ok": true,
        "timestamp": timestamp,
        "tx_id": tx_id,
        "facts": entries,
        "count": entries.len()
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /facts/:id
// ---------------------------------------------------------------------------

async fn api_fact_detail(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let brain = &es.brain;
    let history = brain.fact_history(&id);
    match history.last() {
        Some(f) => {
            let status = if f.revoked_by_tx.is_some() { "retracted" } else { "active" };
            let touch_history: Vec<_> = brain
                .explain(&id)
                .iter()
                .map(|tx| serde_json::json!({
                    "event_id": format!("tx{}", tx.tx_id),
                    "event_type": tx.action.to_string()
                }))
                .collect();
            Json(serde_json::json!({
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
                "metadata": {
                    "predicate": f.predicate,
                    "value": f.value,
                    "confidence": f.confidence,
                    "provenance": f.provenance,
                    "created_at": f.created_at,
                    "valid_from": f.valid_from,
                    "valid_to": f.valid_to,
                    "created_by": format!("tx/{}", f.created_by_tx),
                    "superseded_by": f.superseded_by_tx.map(|tx| format!("tx/{}", tx)),
                    "revoked_by": f.revoked_by_tx.map(|tx| format!("tx/{}", tx))
                },
                "provenance": {"type": "base"},
                "touch_history": touch_history
            }))
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "fact not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Fact JSON helper
// ---------------------------------------------------------------------------

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

fn resolve_branch_key<'a>(brain: &'a Brain, key: &str) -> Option<&'a str> {
    brain
        .branches()
        .iter()
        .find(|b| b.branch_id == key || b.name == key)
        .map(|b| b.branch_id.as_str())
}

fn fact_json_enriched(brain: &Brain, f: &crate::brain::Fact) -> serde_json::Value {
    let tx = brain.transactions().iter().find(|t| t.tx_id == f.created_by_tx);
    let (actor, branch_id, tx_time) = match tx {
        Some(t) => (
            t.actor.as_str(),
            t.branch_id.as_str(),
            t.tx_time.as_str(),
        ),
        None => ("", "", ""),
    };
    let branch_name = brain
        .branches()
        .iter()
        .find(|b| b.branch_id == branch_id)
        .map(|b| b.name.as_str())
        .unwrap_or("");
    serde_json::json!({
        "fact_id": f.fact_id,
        "predicate": f.predicate,
        "value": f.value,
        "confidence": f.confidence,
        "valid_from": f.valid_from,
        "valid_to": f.valid_to,
        "created_by_tx": f.created_by_tx,
        "provenance": f.provenance,
        "actor": actor,
        "branch_id": branch_id,
        "branch_name": branch_name,
        "tx_time": tx_time,
    })
}

#[derive(Deserialize)]
struct FactsListQuery {
    exom: Option<String>,
    /// Branch id or name; omit = current branch.
    branch: Option<String>,
    /// Deduped union of visible facts on every non-archived branch, sorted by `tx_time`.
    #[serde(default)]
    all_branches: bool,
}

async fn api_facts_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<FactsListQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let brain = &es.brain;

    let entries: Vec<serde_json::Value> = if q.all_branches {
        let mut seen: HashMap<String, &crate::brain::Fact> = HashMap::new();
        for b in brain.branches() {
            if b.archived {
                continue;
            }
            for f in brain.facts_on_branch(&b.branch_id) {
                seen.entry(f.fact_id.clone()).or_insert(f);
            }
        }
        let mut rows: Vec<_> = seen
            .into_values()
            .map(|f| fact_json_enriched(brain, f))
            .collect();
        rows.sort_by(|a, b| {
            let ta = a["tx_time"].as_str().unwrap_or("");
            let tb = b["tx_time"].as_str().unwrap_or("");
            ta.cmp(tb).then_with(|| {
                let fa = a["fact_id"].as_str().unwrap_or("");
                let fb = b["fact_id"].as_str().unwrap_or("");
                fa.cmp(fb)
            })
        });
        rows
    } else if let Some(ref key) = q.branch {
        let bid = match resolve_branch_key(brain, key) {
            Some(id) => id,
            None => {
                return ApiError::new(
                    "unknown_branch",
                    format!("no branch matching {:?}", key),
                )
                .into_response();
            }
        };
        brain
            .facts_on_branch(bid)
            .into_iter()
            .map(|f| fact_json_enriched(brain, f))
            .collect()
    } else {
        brain
            .current_facts()
            .into_iter()
            .map(|f| fact_json_enriched(brain, f))
            .collect()
    };

    Json(serde_json::json!({
        "ok": true,
        "exom": exom_slash,
        "facts": entries,
        "count": entries.len()
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /branches + POST /branches
// ---------------------------------------------------------------------------

async fn api_list_branches(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branches: Vec<_> = es
        .brain
        .branches()
        .iter()
        .map(|b| serde_json::json!({
            "branch_id": b.branch_id,
            "name": b.name,
            "parent_branch_id": b.parent_branch_id,
            "created_tx_id": b.created_tx_id,
            "archived": b.archived,
            "is_current": b.branch_id == es.brain.current_branch_id(),
            "fact_count": es.brain.facts_on_branch(&b.branch_id).len(),
            "claimed_by": b.claimed_by,
        }))
        .collect();
    Json(serde_json::json!({"branches": branches})).into_response()
}

#[derive(Deserialize)]
struct CreateBranchBody {
    exom: Option<String>,
    branch_id: String,
    #[serde(default)]
    name: Option<String>,
}

async fn api_create_branch(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateBranchBody>,
) -> impl IntoResponse {
    let actor = headers
        .get("x-actor")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();
    let ctx = MutationContext {
        actor: actor.clone(),
        session: headers.get("x-session").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
        model: headers.get("x-model").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
    };

    let exom_raw = body.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, "main", &actor) {
            return ApiError::from(e).into_response();
        }
    }

    let bid = body.branch_id.clone();
    let name = body.name.unwrap_or_else(|| bid.clone());
    let bid2 = bid.clone();
    let result = mutate_exom(&state, &exom_slash, move |ex| ex.brain.create_branch(&bid2, &name, &ctx));
    match result {
        Ok(tx_id) => Json(serde_json::json!({"branch_id": bid, "tx_id": tx_id})).into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /branches/:id
// ---------------------------------------------------------------------------

async fn api_branch_detail(
    State(state): State<Arc<AppState>>,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    match es.brain.branches().iter().find(|b| b.branch_id == branch_id) {
        Some(b) => Json(serde_json::json!({
            "branch_id": b.branch_id,
            "name": b.name,
            "parent_branch_id": b.parent_branch_id,
            "created_tx_id": b.created_tx_id,
            "archived": b.archived,
            "is_current": b.branch_id == es.brain.current_branch_id(),
            "fact_count": es.brain.facts_on_branch(&b.branch_id).len(),
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("branch '{}' not found", branch_id)})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// DELETE /branches/:id
// ---------------------------------------------------------------------------

async fn api_delete_branch_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let actor = headers
        .get("x-actor")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();
    let ctx = MutationContext {
        actor: actor.clone(),
        session: headers.get("x-session").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
        model: headers.get("x-model").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
    };
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let bid = branch_id.clone();
    let result = mutate_exom(&state, &exom_slash, move |ex| {
        ex.brain.archive_branch(&bid)?;
        Ok(())
    });
    match result {
        Ok(()) => Json(serde_json::json!({"archived": branch_id})).into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /branches/:id/switch
// ---------------------------------------------------------------------------

async fn api_switch_branch_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let actor = headers
        .get("x-actor")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let bid = branch_id.clone();
    let result = mutate_exom(&state, &exom_slash, move |ex| {
        ex.brain.switch_branch(&bid)?;
        Ok(())
    });
    match result {
        Ok(()) => {
            let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"branch_switch","exom":"{}","actor":"{}"}}"#, exom_slash, actor));
            Json(serde_json::json!({"switched_to": branch_id})).into_response()
        }
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /branches/:id/diff
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DiffQuery {
    exom: Option<String>,
    base: Option<String>,
}

async fn api_branch_diff_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<DiffQuery>,
) -> impl IntoResponse {
    let base = q.base.as_deref().unwrap_or("main");
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_facts = es.brain.facts_on_branch(&branch_id);
    let base_facts = es.brain.facts_on_branch(base);
    let base_map: HashMap<&str, &&crate::brain::Fact> = base_facts.iter().map(|f| (f.fact_id.as_str(), f)).collect();
    let branch_map: HashMap<&str, &&crate::brain::Fact> = branch_facts.iter().map(|f| (f.fact_id.as_str(), f)).collect();
    let added: Vec<_> = branch_facts.iter().filter(|f| !base_map.contains_key(f.fact_id.as_str())).map(|f| fact_to_json(f)).collect();
    let removed: Vec<_> = base_facts.iter().filter(|f| !branch_map.contains_key(f.fact_id.as_str())).map(|f| fact_to_json(f)).collect();
    let changed: Vec<_> = branch_facts.iter().filter_map(|f| {
        base_map.get(f.fact_id.as_str()).filter(|bf| bf.value != f.value).map(|bf| serde_json::json!({
            "fact_id": f.fact_id,
            "predicate": f.predicate,
            "base_value": bf.value,
            "branch_value": f.value,
        }))
    }).collect();
    Json(serde_json::json!({"added": added, "removed": removed, "changed": changed})).into_response()
}

// ---------------------------------------------------------------------------
// POST /branches/:id/merge
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct MergeBranchBody {
    exom: Option<String>,
    #[serde(default)]
    policy: Option<String>,
}

async fn api_merge_branch_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(source_branch): AxumPath<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<MergeBranchBody>,
) -> impl IntoResponse {
    let actor = headers.get("x-actor").and_then(|v| v.to_str().ok()).unwrap_or("anonymous").to_string();
    let ctx = MutationContext {
        actor: actor.clone(),
        session: headers.get("x-session").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
        model: headers.get("x-model").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
    };
    let policy = match body.policy.as_deref().unwrap_or("last-writer-wins") {
        "last-writer-wins" => MergePolicy::LastWriterWins,
        "keep-target" => MergePolicy::KeepTarget,
        "manual" => MergePolicy::Manual,
        _ => return ApiError::new("bad_policy", "unknown merge policy").into_response(),
    };
    let exom_raw = body.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let src = source_branch.clone();
    let result = mutate_exom(&state, &exom_slash, move |ex| {
        let target = ex.brain.current_branch_id().to_string();
        ex.brain.merge_branch(&src, &target, policy, &ctx)
    });
    match result {
        Ok(merge_result) => Json(serde_json::json!({
            "added": merge_result.added,
            "conflicts": merge_result.conflicts.iter().map(|c| serde_json::json!({
                "fact_id": c.fact_id,
                "predicate": c.predicate,
                "source_value": c.source_value,
                "target_value": c.target_value,
            })).collect::<Vec<_>>(),
            "tx_id": merge_result.tx_id,
        }))
        .into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /explain
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExplainQuery {
    exom: Option<String>,
    predicate: Option<String>,
    #[allow(dead_code)]
    terms: Option<String>,
}

async fn api_explain(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExplainQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let predicate = q.predicate.as_deref().unwrap_or("").to_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => return ApiError::new("error", e.to_string()).into_response(),
    };
    let defining_rules: Vec<&str> = all_rules
        .iter()
        .filter(|r| r.head_predicate == predicate)
        .map(|r| r.full_text.as_str())
        .collect();
    if !defining_rules.is_empty() {
        return Json(serde_json::json!({
            "kind": "derived",
            "predicate": predicate,
            "derived_by_rules": defining_rules,
        }))
        .into_response();
    }
    let facts = es.brain.current_facts();
    let matching = facts.iter().find(|f| f.predicate == predicate || f.fact_id == predicate);
    match matching {
        Some(f) => Json(serde_json::json!({
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
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("no fact matching predicate '{}'", predicate)})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /actions/export (Rayfall text)
// ---------------------------------------------------------------------------

async fn api_export(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let facts = es.brain.current_facts();
    let mut out = String::new();
    out.push_str(&format!(";; ray-exomem export — exom: {}\n", exom_slash));
    for f in &facts {
        out.push_str(&format!(
            "(assert-fact {} \"{}\" '{} \"{}\")",
            exom_slash,
            f.fact_id.replace('"', "\\\""),
            f.predicate.replace('"', "\\\""),
            f.value.replace('"', "\\\""),
        ));
        let valid_to_str = f.valid_to.as_deref().unwrap_or("inf");
        out.push_str(&format!(" ;; @valid[{}, {}]", f.valid_from, valid_to_str));
        out.push('\n');
    }
    for rule in &es.rules {
        out.push_str(&rule.full_text);
        out.push('\n');
    }
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        out,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// GET /actions/export-json
// ---------------------------------------------------------------------------

async fn api_export_json(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let payload = serde_json::json!({
        "exom": exom_slash,
        "version": 1,
        "facts": es.brain.all_facts(),
        "transactions": es.brain.transactions(),
        "observations": es.brain.observations(),
        "beliefs": es.brain.all_beliefs(),
        "branches": es.brain.branches(),
        "rules": es.rules.iter().map(|r| &r.full_text).collect::<Vec<_>>(),
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(body) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/import-json
// ---------------------------------------------------------------------------

async fn api_import_json(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
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

    let actor = headers.get("x-actor").and_then(|v| v.to_str().ok()).unwrap_or("anonymous").to_string();
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    let payload: ImportPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => return ApiError::new("invalid_payload", format!("invalid JSON import payload: {}", e)).into_response(),
    };

    let ctx = MutationContext {
        actor: actor.clone(),
        session: headers.get("x-session").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
        model: headers.get("x-model").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
    };

    let result = mutate_exom(&state, &exom_slash, move |ex| {
        ex.brain.replace_state(
            payload.facts,
            payload.transactions,
            payload.observations,
            payload.beliefs,
            payload.branches,
        )?;
        let mut parsed_rules = Vec::new();
        for line in &payload.rules {
            let line = line.trim();
            if !line.is_empty() {
                parsed_rules.push(rules::parse_rule_line(line, MutationContext::default(), String::new())?);
            }
        }
        ex.rules = parsed_rules;
        let n_facts = ex.brain.all_facts().len();
        let n_txs = ex.brain.transactions().len();
        Ok((n_facts, n_txs))
    });

    match result {
        Ok((n_facts, n_txs)) => {
            // Re-bind all exoms after import to ensure consistency
            let exoms = state.exoms.lock().unwrap();
            reconcile_engine(&state, &exoms);
            let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"import_json","exom":"{}","actor":"{}"}}"#, exom_slash, actor));
            Json(serde_json::json!({
                "ok": true,
                "imported": {"facts": n_facts, "transactions": n_txs}
            }))
            .into_response()
        }
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/retract-all
// ---------------------------------------------------------------------------

async fn api_retract_all(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let actor = headers.get("x-actor").and_then(|v| v.to_str().ok()).unwrap_or("anonymous").to_string();
    let ctx = MutationContext {
        actor: actor.clone(),
        session: headers.get("x-session").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
        model: headers.get("x-model").and_then(|v| v.to_str().ok()).map(|s| s.to_string()),
    };
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, "main", &actor) {
            return ApiError::from(e).into_response();
        }
    }

    let result = mutate_exom(&state, &exom_slash, move |ex| {
        let fact_ids: Vec<String> = ex.brain.current_facts().iter().map(|f| f.fact_id.clone()).collect();
        let count = fact_ids.len();
        for id in &fact_ids {
            let _ = ex.brain.retract_fact(id, &ctx);
        }
        ex.rules.clear();
        Ok(count)
    });

    match result {
        Ok(count) => {
            let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"retract_all","exom":"{}","actor":"{}"}}"#, exom_slash, actor));
            Json(serde_json::json!({"ok": true, "tuples_removed": count})).into_response()
        }
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/wipe
// ---------------------------------------------------------------------------

async fn api_wipe(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let actor = headers.get("x-actor").and_then(|v| v.to_str().ok()).unwrap_or("anonymous").to_string();
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, "main", &actor) {
            return ApiError::from(e).into_response();
        }
    }

    let result = mutate_exom(&state, &exom_slash, |ex| {
        ex.brain.reset();
        ex.rules.clear();
        if let Some(disk) = ex.exom_disk.as_ref() {
            if disk.exists() {
                std::fs::remove_dir_all(disk)?;
            }
            std::fs::create_dir_all(disk)?;
        }
        Ok(())
    });

    match result {
        Ok(()) => {
            let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"wipe","exom":"{}","actor":"{}"}}"#, exom_slash, actor));
            Json(serde_json::json!({"ok": true, "wiped": exom_slash})).into_response()
        }
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/factory-reset
// ---------------------------------------------------------------------------

async fn api_factory_reset(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let actor = headers.get("x-actor").and_then(|v| v.to_str().ok()).unwrap_or("anonymous").to_string();
    let mut exoms = state.exoms.lock().unwrap();
    let old_names: Vec<String> = exoms.keys().cloned().collect();
    exoms.clear();

    let default_exom = DEFAULT_EXOM;
    let new_es = ExomState {
        brain: Brain::new(),
        datoms: match storage::build_datoms_table(&Brain::new()) {
            Ok(d) => d,
            Err(e) => {
                return ApiError::new("error", e.to_string()).into_response();
            }
        },
        rules: Vec::new(),
        exom_disk: None,
    };
    exoms.insert(default_exom.to_string(), new_es);

    reconcile_engine(&state, &exoms);
    let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"factory_reset","exom":"*","actor":"{}"}}"#, actor));
    Json(serde_json::json!({
        "ok": true,
        "removed_exoms": old_names,
        "state": "clean"
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /schema
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SchemaQuery {
    exom: Option<String>,
    include_samples: Option<String>,
    sample_limit: Option<usize>,
    relation: Option<String>,
}

async fn api_schema(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SchemaQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let include_samples = q.include_samples.as_deref() == Some("true");
    let sample_limit = q.sample_limit.unwrap_or(10);
    let filter_relation = q.relation.clone();

    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let brain = &es.brain;
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => return ApiError::new("error", e.to_string()).into_response(),
    };

    let facts = brain.current_facts();
    let beliefs = brain.current_beliefs();
    let observations = brain.observations();

    let mut relations = Vec::new();
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
        if let Some(ref filter) = filter_relation {
            if *pred != filter.as_str() { continue; }
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
            rel["sample_tuples"] = serde_json::json!(tuples.iter().take(sample_limit).cloned().collect::<Vec<_>>());
        }
        relations.push(rel);
    }

    if filter_relation.is_none() || filter_relation.as_deref() == Some("observation") {
        let obs_tuples: Vec<Vec<serde_json::Value>> = observations.iter().map(|o| vec![
            serde_json::Value::String(o.obs_id.clone()),
            serde_json::Value::String(o.source_type.clone()),
            serde_json::Value::String(o.content.clone()),
            serde_json::json!(o.confidence),
        ]).collect();
        let mut rel = serde_json::json!({
            "name": "observation",
            "arity": 4,
            "kind": "base",
            "cardinality": obs_tuples.len(),
            "has_intervals": false,
            "defined_by": []
        });
        if include_samples && !obs_tuples.is_empty() {
            rel["sample_tuples"] = serde_json::json!(obs_tuples.into_iter().take(sample_limit).collect::<Vec<_>>());
        }
        relations.push(rel);
    }

    if filter_relation.is_none() || filter_relation.as_deref() == Some("belief") {
        let has_belief_intervals = beliefs.iter().any(|b| b.valid_to.is_some());
        let belief_tuples: Vec<Vec<serde_json::Value>> = beliefs.iter().map(|b| vec![
            serde_json::Value::String(b.belief_id.clone()),
            serde_json::Value::String(b.claim_text.clone()),
            serde_json::json!(b.confidence),
            serde_json::Value::String(b.status.to_string()),
            serde_json::json!({"valid_from": b.valid_from, "valid_to": b.valid_to}),
        ]).collect();
        let mut rel = serde_json::json!({
            "name": "belief",
            "arity": 5,
            "kind": "derived",
            "cardinality": belief_tuples.len(),
            "has_intervals": has_belief_intervals,
            "defined_by": ["belief-revision"]
        });
        if include_samples && !belief_tuples.is_empty() {
            rel["sample_tuples"] = serde_json::json!(belief_tuples.into_iter().take(sample_limit).collect::<Vec<_>>());
        }
        relations.push(rel);
    }

    let mut base_names: std::collections::HashSet<String> = fact_groups.keys().map(|s| (*s).to_string()).collect();
    base_names.insert("observation".into());
    base_names.insert("belief".into());

    let derived_preds = rules::derived_predicates(&all_rules);
    for (pred_name, arity) in derived_preds {
        if base_names.contains(&pred_name) { continue; }
        if let Some(ref filter) = filter_relation {
            if filter != pred_name.as_str() { continue; }
        }
        let defined_by_rules: Vec<usize> = all_rules.iter().enumerate()
            .filter(|(_, r)| r.head_predicate == pred_name).map(|(i, _)| i).collect();
        relations.push(serde_json::json!({
            "name": pred_name, "arity": arity, "kind": "derived",
            "cardinality": serde_json::Value::Null,
            "has_intervals": false, "defined_by": defined_by_rules,
        }));
    }

    let base_count = relations.iter().filter(|r| r["kind"] == "base").count();
    let derived_count = relations.iter().filter(|r| r["kind"] == "derived").count();
    let largest = relations.iter().max_by_key(|r| r["cardinality"].as_u64().unwrap_or(0))
        .map(|r| serde_json::json!({"name": r["name"], "cardinality": r["cardinality"]}));

    Json(serde_json::json!({
        "relations": relations,
        "ontology": system_schema::build_exom_ontology(&exom_slash, brain, &es.rules),
        "directives": [],
        "summary": {
            "relation_count": relations.len(),
            "base_relation_count": base_count,
            "derived_relation_count": derived_count,
            "largest_relation": largest
        }
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /graph
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GraphQuery {
    exom: Option<String>,
    limit: Option<usize>,
}

async fn api_graph(
    State(state): State<Arc<AppState>>,
    Query(q): Query<GraphQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let limit = q.limit.unwrap_or(500);
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let facts = es.brain.current_facts();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_nodes = std::collections::HashSet::new();
    for (i, f) in facts.iter().take(limit).enumerate() {
        let entity_id = &f.fact_id;
        let pred_id = format!("pred:{}", f.predicate);
        if seen_nodes.insert(entity_id.clone()) {
            nodes.push(serde_json::json!({"id": entity_id, "type": "entity", "label": format!("{} = {}", f.predicate, f.value), "degree": 1}));
        }
        if seen_nodes.insert(pred_id.clone()) {
            nodes.push(serde_json::json!({"id": pred_id, "type": "entity", "label": f.predicate, "degree": 1}));
        }
        edges.push(serde_json::json!({"id": format!("e{}", i), "type": "fact", "source": entity_id, "target": pred_id, "label": f.value}));
    }
    Json(serde_json::json!({
        "nodes": nodes, "edges": edges, "clusters": [],
        "summary": {"node_count": nodes.len(), "edge_count": edges.len(), "cluster_count": 0}
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /clusters
// ---------------------------------------------------------------------------

async fn api_clusters(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let facts = es.brain.current_facts();
    let mut groups: HashMap<&str, usize> = HashMap::new();
    for f in &facts {
        *groups.entry(&f.predicate).or_default() += 1;
    }
    let clusters: Vec<_> = groups.iter().map(|(pred, count)| serde_json::json!({
        "id": format!("cluster:{}", pred), "label": pred, "kind": "shared_predicate",
        "fact_count": count, "active_count": count, "deprecated_count": 0
    })).collect();
    Json(serde_json::json!({"clusters": clusters})).into_response()
}

// ---------------------------------------------------------------------------
// GET /clusters/:id
// ---------------------------------------------------------------------------

async fn api_cluster_detail_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let pred = id.strip_prefix("cluster:").unwrap_or(&id);
    let facts = es.brain.current_facts();
    let matching: Vec<_> = facts.iter().filter(|f| f.predicate == pred).collect();
    let nodes: Vec<_> = matching.iter().map(|f| serde_json::json!({"id": f.fact_id, "type": "fact", "label": format!("{} = {}", f.predicate, f.value)})).collect();
    let fact_entries: Vec<_> = matching.iter().map(|f| serde_json::json!({
        "id": f.fact_id, "tuple": [f.fact_id, f.predicate, f.value, f.confidence],
        "status": "active", "interval": {"start": f.valid_from, "end": f.valid_to.as_deref().unwrap_or("inf")}
    })).collect();
    Json(serde_json::json!({
        "id": id, "label": pred, "kind": "shared_predicate",
        "stats": {"fact_count": matching.len(), "active_count": matching.len(), "deprecated_count": 0},
        "nodes": nodes, "facts": fact_entries, "related_clusters": []
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /logs
// ---------------------------------------------------------------------------

async fn api_logs(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let events: Vec<_> = es.brain.transactions().iter().rev().take(24).map(|tx| serde_json::json!({
        "id": format!("tx{}", tx.tx_id), "type": tx.action.to_string(),
        "timestamp": tx.tx_time, "pattern": tx.note, "source": tx.actor
    })).collect();
    Json(serde_json::json!({"events": events})).into_response()
}

// ---------------------------------------------------------------------------
// GET /provenance
// ---------------------------------------------------------------------------

async fn api_provenance(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let facts = es.brain.current_facts();
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => return ApiError::new("error", e.to_string()).into_response(),
    };
    let derived_n = rules::derived_predicates(&all_rules).len();
    let base_facts: Vec<_> = facts.iter().map(|f| serde_json::json!({
        "id": f.fact_id, "predicate": f.predicate, "terms": [f.fact_id, f.predicate, f.value],
        "kind": "base", "source": f.provenance, "confidence": f.confidence, "asserted_at": f.created_by_tx
    })).collect();
    Json(serde_json::json!({
        "derivations": [], "base_facts": base_facts, "edges": [], "timeline": [],
        "summary": {"derived_count": derived_n, "base_count": base_facts.len(), "edge_count": 0, "event_count": 0}
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /relation-graph
// ---------------------------------------------------------------------------

async fn api_relation_graph(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let facts = es.brain.current_facts();
    let mut preds: HashMap<&str, usize> = HashMap::new();
    for f in &facts {
        *preds.entry(&f.predicate).or_default() += 1;
    }
    let nodes: Vec<_> = preds.iter().map(|(pred, count)| serde_json::json!({"id": *pred, "label": *pred, "degree": count})).collect();
    Json(serde_json::json!({
        "nodes": nodes, "edges": [],
        "summary": {"node_count": nodes.len(), "edge_count": 0}
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /derived/:pred
// ---------------------------------------------------------------------------

async fn api_derived_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(pred_name): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    if pred_name.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing predicate"}))).into_response();
    }
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => return ApiError::new("error", e.to_string()).into_response(),
    };
    let arity = match all_rules.iter().find(|r| r.head_predicate == pred_name).map(|r| r.head_arity) {
        Some(a) => a,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "unknown derived predicate"}))).into_response(),
    };
    let find_vars: Vec<String> = (0..arity).map(|i| format!("?v{i}")).collect();
    let find_vars_str = find_vars.join(" ");
    let bodies: Vec<String> = all_rules.iter().map(|r| r.inline_body.clone()).collect();
    let rules_clause = bodies.join(" ");
    let rayfall = format!(
        "(query {exom_slash} (find {find_vars_str}) (where ({pred_name} {find_vars_str})) (rules {rules_clause}))"
    );
    match state.engine.eval(&rayfall) {
        Ok(output) => Json(serde_json::json!({"predicate": pred_name, "kind": "derived", "arity": arity, "rows": output})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /beliefs/:id/support
// ---------------------------------------------------------------------------

async fn api_belief_support_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(belief_id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    let bid = belief_id.trim().to_string();
    if bid.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing belief id"}))).into_response();
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let brain = &es.brain;
    let beliefs: Vec<_> = brain.current_beliefs().into_iter().filter(|b| b.belief_id == bid).collect();
    let b = match beliefs.first() {
        Some(x) => *x,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": format!("belief '{}' not found", bid)}))).into_response(),
    };
    let mut support_facts = Vec::new();
    let mut support_obs = Vec::new();
    let mut unresolved: Vec<&str> = Vec::new();
    for id in &b.supported_by {
        if let Some(f) = brain.current_facts().iter().find(|f| f.fact_id == *id) {
            support_facts.push(fact_to_json(f));
        } else if let Some(o) = brain.observations().iter().find(|o| o.obs_id == *id) {
            support_obs.push(serde_json::json!({"obs_id": o.obs_id, "source_type": o.source_type, "source_ref": o.source_ref, "content": o.content}));
        } else {
            unresolved.push(id.as_str());
        }
    }
    Json(serde_json::json!({
        "ok": true,
        "belief_id": b.belief_id,
        "claim_text": b.claim_text,
        "supported_by_resolved": {"facts": support_facts, "observations": support_obs},
        "supported_by_unresolved": unresolved,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// POST /exoms/:name/manage (legacy)
// ---------------------------------------------------------------------------

async fn api_exom_manage_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::json!({}));
    let action = payload["action"].as_str().unwrap_or("").to_string();
    let default_exom = DEFAULT_EXOM;
    let mut exoms = state.exoms.lock().unwrap();

    match action.as_str() {
        "delete" => {
            if name == default_exom {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "cannot delete the default exom"}))).into_response();
            }
            if exoms.remove(&name).is_some() {
                reconcile_engine(&state, &exoms);
                Json(serde_json::json!({"ok": true, "deleted": name})).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": format!("exom '{}' not found", name)}))).into_response()
            }
        }
        "rename" => {
            let new_name = payload["new_name"].as_str().unwrap_or("").to_string();
            if new_name.is_empty() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "new_name is required"}))).into_response();
            }
            if name == default_exom {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "cannot rename the default exom"}))).into_response();
            }
            if exoms.contains_key(new_name.as_str()) {
                return (StatusCode::CONFLICT, Json(serde_json::json!({"error": format!("exom '{}' already exists", new_name)}))).into_response();
            }
            if let Some(es) = exoms.remove(&name) {
                exoms.insert(new_name.clone(), es);
                reconcile_engine(&state, &exoms);
                Json(serde_json::json!({"ok": true, "old_name": name, "new_name": new_name})).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": format!("exom '{}' not found", name)}))).into_response()
            }
        }
        "archive" => {
            if exoms.contains_key(&name) {
                Json(serde_json::json!({"ok": true, "archived": name})).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": format!("exom '{}' not found", name)}))).into_response()
            }
        }
        _ => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("unknown action '{}'", action)}))).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/start-session → 410
// ---------------------------------------------------------------------------

async fn api_start_session_gone() -> impl IntoResponse {
    ApiError::new("gone", "POST /api/actions/start-session is removed; use POST /api/actions/session-new")
        .with_status(410)
        .into_response()
}
